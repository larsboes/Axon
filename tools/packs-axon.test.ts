import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deployPack } from "./packs-codex.ts";
import { type DeployConfig, type StatusRow } from "./packs-codex.ts";
import { readAxonHarnessStatuses, type HarnessStatus } from "./packs-axon.ts";

type Harness = "claude" | "codex" | "opencode";

type EnvMap = Record<string, string | undefined>;

let root: string;
let home: string;
let axonRoot: string;
let originalEnv: EnvMap;

function writeFixture(): void {
  const packDir = join(axonRoot, "Packs", "axon", "skills", "axon");
  mkdirSync(packDir, { recursive: true });
  writeFileSync(
    join(axonRoot, "Packs", "axon", "pack.toml"),
    'name = "axon"\ndescription = "fixture"\nskills = ["axon"]\nlicense = "MIT"\n',
  );
  writeFileSync(
    join(packDir, "SKILL.md"),
    "---\nname: axon\ndescription: Axon Pack skill fixture.\nallowed-tools: Bash\n---\n\n# Axon\n",
  );
}

function restoreEnv(): void {
  // Delete first so deletions from the test run do not persist.
  for (const key of Object.keys(process.env)) {
    delete process.env[key];
  }
  for (const [key, value] of Object.entries(originalEnv)) {
    if (value !== undefined) process.env[key] = value;
  }
}

function setEnv(overrides: EnvMap): void {
  const next = {
    HOME: home,
    CLAUDE_SKILLS_DIR: join(home, ".claude", "skills"),
    CODEX_SKILLS_DIR: join(home, ".agents", "skills"),
    OPENCODE_SKILLS_DIR: join(home, ".config", "opencode", "skills"),
    XDG_STATE_HOME: join(home, ".local", "state"),
    ...overrides,
  };
  restoreEnv();
  for (const [key, value] of Object.entries(next)) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
}

function harnessConfig(harness: Harness): DeployConfig {
  return {
    axonRoot,
    destination: {
      claude: join(home, ".claude", "skills"),
      codex: join(home, ".agents", "skills"),
      opencode: join(home, ".config", "opencode", "skills"),
    }[harness],
    stateFile: {
      claude: join(home, ".local", "state", "axon", "pack-deployments", "claude.json"),
      codex: join(home, ".local", "state", "axon", "pack-deployments", "codex.json"),
      opencode: join(home, ".local", "state", "axon", "pack-deployments", "opencode.json"),
    }[harness],
    adapter: harness,
  };
}

function axonStatusFor(harness: Harness, statuses: HarnessStatus[]): StatusRow {
  const entry = statuses.find((status) => status.harness === harness);
  if (!entry) throw new Error(`missing harness status for ${harness}`);
  const status = entry.rows.find((row) => row.pack === "axon" && row.skill === "axon");
  if (!status) throw new Error(`missing axon status for ${harness}`);
  return status;
}

beforeEach(() => {
  originalEnv = { ...process.env } as EnvMap;
  root = mkdtempSync(join(tmpdir(), "axon-packs-axon-test-"));
  home = join(root, "home");
  axonRoot = join(root, "Axon");
  setEnv({});
  writeFixture();
});

afterEach(() => {
  restoreEnv();
  rmSync(root, { recursive: true, force: true });
});

describe("packs-axon launcher parity and per-harness state", () => {
  test("status helper reports each harness for Axon and can be run from a fixture root", () => {
    const statuses = readAxonHarnessStatuses({
      axonRoot,
      destination: join(home, ".agents", "skills"),
      stateFile: harnessConfig("codex").stateFile,
    });
    expect(statuses.map((status) => status.harness).sort()).toEqual(["claude", "codex", "opencode"]);

    const statusesByHarness = statuses.map((entry) => [entry.harness, entry.rows[0].status] as const);
    expect(statusesByHarness).toEqual([
      ["claude", "not-deployed"],
      ["codex", "not-deployed"],
      ["opencode", "not-deployed"],
    ]);
  });

  test("deploying one harness leaves others untouched", () => {
    expect(deployPack(harnessConfig("claude"), "axon")).toEqual(["✓ axon deployed"]);

    const statuses = readAxonHarnessStatuses({
      axonRoot,
      destination: join(home, ".agents", "skills"),
      stateFile: harnessConfig("codex").stateFile,
    });
    expect(axonStatusFor("claude", statuses)).toMatchObject({ status: "current" });
    expect(axonStatusFor("codex", statuses)).toMatchObject({ status: "not-deployed" });
    expect(axonStatusFor("opencode", statuses)).toMatchObject({ status: "not-deployed" });
    expect(existsSync(join(home, ".claude", "skills", "axon", "SKILL.md"))).toBe(true);
  });

  test("deploying one harness preserves ownership isolation and reports collision state for another", () => {
    expect(deployPack(harnessConfig("claude"), "axon")).toEqual(["✓ axon deployed"]);
    expect(deployPack(harnessConfig("codex"), "axon")).toEqual(["✓ axon deployed"]);

    const opencodeDestination = join(home, ".config", "opencode", "skills", "axon");
    mkdirSync(opencodeDestination, { recursive: true });
    writeFileSync(join(opencodeDestination, "SKILL.md"), "external\n");
    expect(() => deployPack(harnessConfig("opencode"), "axon")).toThrow("not owned by this Axon deployment");

    const statuses = readAxonHarnessStatuses({
      axonRoot,
      destination: join(home, ".agents", "skills"),
      stateFile: harnessConfig("codex").stateFile,
    });
    expect(axonStatusFor("opencode", statuses)).toMatchObject({ status: "collision" });
    expect(axonStatusFor("claude", statuses)).toMatchObject({ status: "current" });
    expect(axonStatusFor("codex", statuses)).toMatchObject({ status: "current" });
  });

  test("status helper reflects harness fallback destinations when no explicit paths are set", () => {
    setEnv({
      CLAUDE_SKILLS_DIR: undefined,
      CODEX_SKILLS_DIR: undefined,
      OPENCODE_SKILLS_DIR: undefined,
      OPENCODE_CONFIG_DIR: undefined,
      XDG_STATE_HOME: undefined,
    });
    const statuses = readAxonHarnessStatuses({
      axonRoot,
      destination: join(home, ".agents", "skills"),
      stateFile: join(home, ".local", "state", "axon", "pack-deployments", "codex.json"),
    });
    expect(statuses.find((entry) => entry.harness === "codex")?.destination).toBe(join(home, ".agents", "skills"));
    expect(statuses.find((entry) => entry.harness === "claude")?.destination).toBe(join(home, ".claude", "skills"));
    expect(statuses.find((entry) => entry.harness === "opencode")?.destination).toBe(
      join(home, ".config", "opencode", "skills"),
    );
  });
});
