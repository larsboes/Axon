// tools/pack-extensions.test.ts — gate the pi extensions the Packs deploy.
//
// WHAT THIS CHECKS, and why it did not exist before: `Packs/*/extensions/*.ts` is loaded
// by pi at startup as agent code with the power to block tool calls, and it was the only
// TypeScript in this repository that nothing checked. `bun test` never imported it, no
// tsconfig reached it, cargo cannot see it. The first time these files were type-checked
// (2026-09-13) three type errors fell out of two extensions, and the secrets guard's
// env-dump pattern turned out to allow every real `env` dump while blocking `rg
// process.env`. Both classes of defect are invisible at runtime and invisible to CI.
//
// It checks four things: every `Packs/*/extensions/*.ts` typechecks against the *installed
// pi's own types* (drift from pi's API is the failure mode, so a local shim would defeat the
// point); the extensions *pi itself has registered* typecheck too; every extension loads
// under a stub ExtensionAPI without throwing; and secrets-guard's bash heuristics block the
// commands that leak secrets and leave the rest of bash alone, case by case.
//
// Registered extensions live outside this repository (`~/.pi/agent/settings.json`) and are
// included because that is where a third extension, and a third type error, was found. The
// consequence for whoever runs this on their own machine: an extension kept there is held to
// the same standard, and a broken one turns this gate red until it is fixed or unregistered.
// That is the intent — such an extension is otherwise invisible until it runs at startup.
//
// WHY IT SKIPS, and this is the honest cost: the extensions import pi's packages
// (@earendil-works/pi-tui, typebox) and pi's types, none of which the bun-tests job can
// install — that job is deliberately dependency-free (Axon#116), and its comment says
// every test there imports bun:test, node builtins and local sources only. pi is also
// where these files can run at all, so the gate opens exactly where the code is live: on a
// machine with pi installed, visible as a skip anywhere else. A green run on a machine
// without pi means "not applicable", not "verified", and bun prints the skip count so that
// reads as a skip.
//
// THE COMPILER IS RESOLVED, NEVER FETCHED. A gate that downloads a compiler goes red on a
// plane, so this uses the dashboard's typescript, then `tsc` on PATH, then a typescript
// already in bun's cache — and fails with instructions when none of the three is there,
// rather than reporting "typecheck passed" after checking nothing.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { harnessById, isInstalled } from "./lib/harness-registry.ts";

const AXON_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PI = harnessById("pi");

/* ── where pi lives ──────────────────────────────────────────────────────── */

interface PiInstall {
  /** The installed @earendil-works/pi-coding-agent package directory. */
  packageDir: string;
  /** Where its dependencies (pi-tui, pi-ai, typebox, @types/node) resolve from. */
  modulesDir: string;
}

/**
 * The marker in `harness-registry` decides whether to run (it is what CI lacks), and this
 * finds the package that makes the check possible. Both are needed: the marker without a
 * package is a broken install and should be loud, not a skip.
 */
function findPiInstall(): PiInstall | undefined {
  const candidates: string[] = [];
  const onPath = spawnSync("sh", ["-c", "command -v pi"], { encoding: "utf8" }).stdout?.trim();
  if (onPath) {
    // Realpath first, then climb: `pi` is normally a symlink into the package (Homebrew,
    // ~/.bun/bin), and a lexical climb from the link lands in /opt/homebrew/bin or ~/.bun/bin
    // — directories that hold no package at all.
    try {
      let dir = dirname(realpathSync(onPath));
      for (let i = 0; i < 6 && dir !== dirname(dir); i++) {
        candidates.push(dir);
        dir = dirname(dir);
      }
    } catch {
      // A wrapper that is not a file we can resolve: the known global roots below still apply.
    }
  }
  candidates.push(
    join(homedir(), ".bun", "install", "global", "node_modules", "@earendil-works", "pi-coding-agent"),
    "/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent",
    "/usr/local/lib/node_modules/@earendil-works/pi-coding-agent",
    "/usr/lib/node_modules/@earendil-works/pi-coding-agent",
  );
  for (const candidate of candidates) {
    const modulesDir = join(candidate, "node_modules");
    // Both must be there: the manifest proves it is pi, the dependency proves we can
    // resolve the packages the extensions import.
    if (!existsSync(join(candidate, "package.json")) || !existsSync(join(modulesDir, "typebox"))) continue;
    try {
      const manifest = JSON.parse(readFileSync(join(candidate, "package.json"), "utf8")) as { name?: string };
      if (manifest.name !== "@earendil-works/pi-coding-agent") continue;
    } catch {
      continue;
    }
    return { packageDir: candidate, modulesDir };
  }
  return undefined;
}

/**
 * pi is installed when its agent config exists. That is also the CI condition: the runner
 * has neither, so the gate skips there instead of failing on packages it cannot install.
 */
const piConfigured = isInstalled(PI);
const piInstall = piConfigured ? findPiInstall() : undefined;

/* ── the extensions under test ───────────────────────────────────────────── */

interface ExtensionFile {
  pack: string;
  /** Absolute path in the checkout. */
  path: string;
  /** Path relative to the repository root, which is also its path in the fixture. */
  rel: string;
}

/** Discovered, never listed: a new `Packs/<pack>/extensions/*.ts` is gated by existing. */
function packExtensions(): ExtensionFile[] {
  const found: ExtensionFile[] = [];
  for (const pack of readdirSync(join(AXON_ROOT, "Packs")).sort()) {
    const dir = join(AXON_ROOT, "Packs", pack, "extensions");
    if (!existsSync(dir)) continue;
    for (const entry of readdirSync(dir).sort()) {
      if (!entry.endsWith(".ts") || entry.endsWith(".test.ts")) continue;
      found.push({ pack, path: join(dir, entry), rel: join("Packs", pack, "extensions", entry) });
    }
  }
  return found;
}

/**
 * The extensions pi actually has registered, read from its settings. They live outside the
 * repository on purpose (a machine's own harness config), and they are included because one
 * of them is where a type error was found: the gate is about what loads at startup, not
 * about what happens to be committed. Ones that are pack files are left to the pack test,
 * so a broken file is reported once rather than twice.
 */
function registeredExtensions(): ExtensionFile[] {
  const settingsPath = join(homedir(), ".pi", "agent", "settings.json");
  if (!existsSync(settingsPath)) return [];
  const inPacks = new Set(packExtensions().map((file) => resolve(file.path)));
  try {
    const settings = JSON.parse(readFileSync(settingsPath, "utf8")) as { extensions?: unknown };
    if (!Array.isArray(settings.extensions)) return [];
    return settings.extensions
      .filter((entry): entry is string => typeof entry === "string" && entry.endsWith(".ts"))
      .filter((entry) => existsSync(entry) && !inPacks.has(resolve(entry)))
      .map((entry) => ({ pack: "pi settings", path: entry, rel: join("registered", entry.replace(/^\//, "")) }));
  } catch {
    return [];
  }
}

/* ── fixture: a directory where the extensions can resolve pi ────────────── */

let fixtureDir: string | undefined;

/**
 * Extension imports resolve from the importing file's real path, so a copy has to sit
 * somewhere with a `node_modules` that reaches pi — the checkout has none, and the gate
 * must not write into it. Copies keep the repository's own layout so a diagnostic prints a
 * path that maps back to the file it came from.
 */
function fixture(): string {
  if (fixtureDir) return fixtureDir;
  const pi = piInstall;
  if (!pi) throw new Error("pi is not installed");

  // Realpath, because macOS resolves /var to /private/var when a process is spawned:
  // the compiler then prints diagnostics relative to a cwd that does not share a prefix
  // with the path in the tsconfig, which turns every line into ../../../../../../ noise.
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "axon-pack-extensions-")));
  const modules = join(dir, "node_modules");
  mkdirSync(join(modules, "@earendil-works"), { recursive: true });
  mkdirSync(join(modules, "@types"), { recursive: true });
  symlinkSync(pi.packageDir, join(modules, "@earendil-works", "pi-coding-agent"));
  for (const name of ["pi-tui", "pi-ai"]) {
    symlinkSync(join(pi.modulesDir, "@earendil-works", name), join(modules, "@earendil-works", name));
  }
  symlinkSync(join(pi.modulesDir, "typebox"), join(modules, "typebox"));
  symlinkSync(join(pi.modulesDir, "@types", "node"), join(modules, "@types", "node"));

  for (const file of [...packs, ...registered]) {
    const destination = join(dir, file.rel);
    mkdirSync(dirname(destination), { recursive: true });
    cpSync(file.path, destination);
  }

  fixtureDir = dir;
  return dir;
}

afterAll(() => {
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

/* ── the compiler ───────────────────────────────────────────────────────── */

interface Compiler {
  cmd: string;
  via: string;
}

/**
 * Resolved in preference order, never fetched: a cached typescript for bunx counts as
 * available, an empty cache does not (that would be a download).
 */
function findCompiler(): Compiler | undefined {
  const dashboard = join(AXON_ROOT, "dashboard", "node_modules", ".bin", "tsc");
  if (existsSync(dashboard)) return { cmd: dashboard, via: "the dashboard's typescript" };

  const onPath = spawnSync("sh", ["-c", "command -v tsc"], { encoding: "utf8" }).stdout?.trim();
  if (onPath) return { cmd: onPath, via: "tsc on PATH" };

  const cache = join(homedir(), ".bun", "install", "cache");
  if (existsSync(cache) && readdirSync(cache).some((entry) => entry.startsWith("typescript"))) {
    return { cmd: "bunx", via: "a typescript already in bun's cache" };
  }
  return undefined;
}

const compiler = piConfigured ? findCompiler() : undefined;

/** The pi version the extensions were checked against, named in a failure. */
function piVersion(): string {
  try {
    const manifest = JSON.parse(readFileSync(join(piInstall!.packageDir, "package.json"), "utf8")) as { version?: string };
    return manifest.version ?? "(version unknown)";
  } catch {
    return "(version unknown)";
  }
}

const NO_COMPILER =
  "no TypeScript compiler found. Looked for dashboard/node_modules/.bin/tsc, tsc on PATH " +
  "and a typescript in bun's install cache. Install one (`cd dashboard && bun install`, or " +
  "`bun add -g typescript`) and re-run: this gate resolves a compiler rather than fetching " +
  "one, so it never reports a typecheck it did not run.";

/** Typechecks one extension in isolation, the way pi loads it. */
function typecheckWe(file: ExtensionFile, dir: string): { ok: boolean; output: string } {  const tsconfig = join(dir, `tsconfig.${file.pack}-${file.rel.split("/").pop()?.replace(/\.ts$/, "")}.json`);
  writeFileSync(
    tsconfig,
    `${JSON.stringify(
      {
        compilerOptions: {
          target: "es2023",
          module: "esnext",
          moduleResolution: "bundler",
          strict: true,
          noEmit: true,
          skipLibCheck: true,
          allowImportingTsExtensions: true,
          types: ["node"],
        },
        files: [file.rel],
      },
      null,
      2,
    )}\n`,
  );
  const args = compiler?.cmd === "bunx" ? ["tsc", "-p", tsconfig] : ["-p", tsconfig];
  const run = spawnSync(compiler!.cmd, args, { cwd: dir, encoding: "utf8" });
  // Diagnostics arrive with the throwaway fixture's path on the front of every line. The
  // copy keeps the repository layout, so stripping the prefix leaves the path a reader can
  // open. A failure that names a temp directory is a failure nobody can act on.
  const output = `${run.stdout ?? ""}${run.stderr ?? ""}`
    .trim()
    .split("\n")
    .map((line) => line.replaceAll(`${dir}/`, ""))
    .join("\n");
  return { ok: run.status === 0, output };
}

/* ── loading an extension under a stub ExtensionAPI ──────────────────────── */

type Handler = (event: unknown, ctx: unknown) => Promise<unknown>;

interface Loaded {
  factory: unknown;
  handlers: Map<string, Handler[]>;
  tools: string[];
  commands: string[];
  providers: string[];
}

async function load(file: ExtensionFile, dir: string): Promise<Loaded> {
  const loaded: Loaded = { factory: undefined, handlers: new Map(), tools: [], commands: [], providers: [] };
  const api = {
    on: (event: string, handler: Handler) => {
      loaded.handlers.set(event, [...(loaded.handlers.get(event) ?? []), handler]);
    },
    registerTool: (definition: { name: string }) => loaded.tools.push(definition.name),
    registerCommand: (name: string) => loaded.commands.push(name),
    registerProvider: (id: string) => loaded.providers.push(id),
    sendMessage: () => {},
  };
  const module = (await import(join(dir, file.rel))) as { default?: (api: unknown) => void };
  loaded.factory = module.default;
  if (typeof module.default === "function") module.default(api);
  return loaded;
}

/* ── the gate ────────────────────────────────────────────────────────────── */

const packs = packExtensions();
const registered = registeredExtensions();

/**
 * The skip has to be legible: `bun test` prints a skip count but not what was skipped or
 * why, and "not applicable" reading as "verified" is the failure this file exists to
 * prevent. A skipped test's body never runs, so the reason is stated at load time.
 */
if (!piConfigured) {
  console.log(
    `pack-extensions: not checked — ${PI.marker} does not exist, so pi's packages and types ` +
      `are unavailable. This gate opens where pi is installed, which is the only place these ` +
      `extensions can run. No Packs/*/extensions/*.ts was type-checked or loaded in this run.`,
  );
}

describe.skipIf(!piConfigured)("pack extensions", () => {
  beforeAll(() => {
    if (piConfigured) fixture();
  });

  test("the installed pi was found, or the reason it was not is stated", () => {
    if (!piInstall) {
      // The marker exists, so pi is configured on this machine: a package search that
      // comes up empty is a broken install and must not pass as "nothing to check".
      throw new Error(
        `pi is configured (${PI.marker}) but its package was not found. Set it up so the ` +
          `extensions' imports resolve, or remove the config — this gate will not report a ` +
          `typecheck it could not run.`,
      );
    }
    expect(existsSync(join(piInstall.packageDir, "package.json"))).toBe(true);
    expect(existsSync(join(piInstall.modulesDir, "typebox"))).toBe(true);
  });

  test("every Packs/*/extensions/*.ts typechecks against pi's own types", () => {
    expect(packs.length).toBeGreaterThan(0);
    if (!compiler) throw new Error(NO_COMPILER);
    const dir = fixture();
    const failures: string[] = [];
    for (const file of packs) {
      const { ok, output } = typecheckWe(file, dir);
      if (!ok) failures.push(output);
    }
    // Thrown rather than asserted so the diagnostics print as written: a compiler's
    // file/line/column message is the whole value of this test, and a diff around it is noise.
    if (failures.length > 0) {
      throw new Error(
        `these extensions do not typecheck against pi ${piVersion()}, using ${compiler?.via}:\n\n${failures.join("\n\n")}`,
      );
    }
  });

  test("the extensions pi has registered typecheck too (they live outside this repository)", () => {
    if (registered.length === 0) return;
    if (!compiler) throw new Error(NO_COMPILER);
    const dir = fixture();
    const failures: string[] = [];
    for (const file of registered) {
      const { ok, output } = typecheckWe(file, dir);
      if (!ok) failures.push(output);
    }
    if (failures.length > 0) {
      throw new Error(`these pi-registered extensions do not typecheck:\n\n${failures.join("\n\n")}`);
    }
  });

  test("every extension loads and registers under a stub api", async () => {
    const dir = fixture();
    for (const file of packs) {
      const loaded = await load(file, dir);
      expect(`${file.rel}: ${typeof loaded.factory}`).toBe(`${file.rel}: function`);
      // An extension that loads but registers nothing is dead code, or a registration
      // guarded by something that is not there — either way, not what it looks like.
      const registeredSomething = loaded.tools.length + loaded.commands.length + loaded.handlers.size;
      expect(`${file.rel}: ${registeredSomething > 0}`).toBe(`${file.rel}: true`);
    }
  });

  test("secrets-guard blocks what leaks secrets and leaves the rest of bash alone", async () => {
    const guard = packs.find((file) => file.path.endsWith("secrets-guard.ts"));
    expect(guard).toBeDefined();
    const dir = fixture();
    const loaded = await load(guard!, dir);
    const toolCall = loaded.handlers.get("tool_call")?.[0];
    expect(typeof toolCall).toBe("function");
    const ctx = { ui: { notify: () => {} } };
    const mismatches: string[] = [];

    const blocked = async (toolName: string, input: Record<string, string>) =>
      (await toolCall!({ toolName, input }, ctx) as { block?: boolean } | undefined)?.block === true;

    // Each case is a command plus what must happen to it. The two failures this table was
    // written for are marked: the old pattern required a trailing newline, so a bare `env`
    // passed while a multi-line command merely containing "env" was refused.
    const cases: [what: string, toolName: string, input: Record<string, string>, want: "block" | "allow"][] = [
      ["bare env", "bash", { command: "env" }, "block"],
      ["bare env with a trailing newline", "bash", { command: "env\n" }, "block"],
      ["env piped to grep", "bash", { command: "env | grep -i key" }, "block"],
      ["env piped to sort", "bash", { command: "env | sort" }, "block"],
      ["env redirected to a file", "bash", { command: "env > /tmp/env.txt" }, "block"],
      ["env after a separator", "bash", { command: "echo hi; env" }, "block"],
      ["env on a later line", "bash", { command: "cd /tmp && ls\nenv" }, "block"],
      ["env under sudo", "bash", { command: "sudo env" }, "block"],
      ["env -0", "bash", { command: "env -0" }, "block"],
      ["env in a subshell", "bash", { command: "echo $(env)" }, "block"],
      ["printenv", "bash", { command: "printenv" }, "block"],
      ["printenv of one variable", "bash", { command: "printenv HOME" }, "block"],
      ["echo $API_KEY", "bash", { command: "echo $API_KEY" }, "block"],
      ["cat .env", "bash", { command: "cat .env" }, "block"],
      ["grep a token out of .env", "bash", { command: "grep -n TOKEN .env" }, "block"],
      ["cat a named env file", "bash", { command: "cat myapp.env" }, "block"],
      ["cat .env.local", "bash", { command: "cat .env.local" }, "block"],
      ["bw get", "bash", { command: "bw get item infra" }, "block"],
      ["read of a .env path", "read", { path: "/tmp/x/.env" }, "block"],
      ["edit of a .env path", "edit", { path: "/tmp/x/.env", oldText: "a", newText: "b" }, "block"],
      // Code that mentions env, not code that dumps it.
      ["grep for process.env", "bash", { command: 'grep -n "process.env" index.ts' }, "allow"],
      ["rg for process.env", "bash", { command: "rg process.env src/" }, "allow"],
      ["grep of a code namespace plus the real .env", "bash", { command: "grep -n process.env .env" }, "block"],
      ["rg for import.meta.env", "bash", { command: "rg 'import.meta.env' src/" }, "allow"],
      ["rg for os.environ", "bash", { command: "rg 'os.environ' src/" }, "allow"],
      ["rg for the word env", "bash", { command: "rg 'env' --type ts" }, "allow"],
      ["env as a command prefix", "bash", { command: 'env -i bash -c "echo hi"' }, "allow"],
      ["env with an assignment prefix", "bash", { command: "env FOO=1 make test" }, "allow"],
      ["env var in code, mid-script", "bash", { command: 'ls\nFOO="bar"\nprocess.env.MISSING = 1\necho done' }, "allow"],
      [
        "heredoc writing code-namespace assignments",
        "bash",
        { command: 'cat > t.ts <<\'EOF\'\nprocess.env.PI_BW_BIN = "/tmp/w.sh";\nconst x = process.env.FOO ?? "";\nEOF' },
        "allow",
      ],
      ["a yaml env_file key", "bash", { command: "printf 'env_file: .env\\n' > compose.yml" }, "allow"],
      ["a yaml environment key", "bash", { command: "rg 'environment:' docker-compose.yml" }, "allow"],
      ["the word environment in prose", "bash", { command: 'echo "check the environment first"' }, "allow"],
      ["env in a filename", "bash", { command: "tail -f logs/environment.log" }, "allow"],
      ["source .env, the allowed form", "bash", { command: "source .env && ls" }, "allow"],
      ["read of an ordinary config file", "read", { path: "/tmp/x/config.yaml" }, "allow"],
    ];

    for (const [what, toolName, input, want] of cases) {
      const got = (await blocked(toolName, input)) ? "block" : "allow";
      // Collected rather than asserted per case so one run reports every mismatch: a gate
      // that shows the first of six problems is a gate someone re-runs six times.
      if (got !== want) mismatches.push(`${want.padEnd(5)} ${what} — got ${got}`);
    }
    expect(mismatches).toEqual([]);
  });
});

/**
 * A skipped test's body never runs, so this exists only to put a skip in the count that
 * the message above explains. The discovery assertion next to it is the one check that
 * runs on every machine, pi or not.
 */
describe.skipIf(piConfigured)("pack extensions (not checked in this run)", () => {
  test("needs a machine with pi installed", () => {});
});

test("this gate found the extensions it means to check", () => {
  expect(packs.map((file) => relative(AXON_ROOT, file.path)).sort()).toContain(
    relative(AXON_ROOT, join(AXON_ROOT, "Packs", "security", "extensions", "secrets-guard.ts")),
  );
});
