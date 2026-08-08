// tools/storage.ts — what is filling this disk, and what is safe to reclaim.
//
// `sysmon report` answers "am I full" with one df line. This answers "what is
// filling me", which df structurally cannot: the 46 GB that started this was 215
// individually-unremarkable 230 MB cache blocks, invisible to every per-file view.
//
// Knows no path on this machine. Classes, protected paths, expected services and
// thresholds all come from the overlay's config/storage-policy.toml, so core Axon
// stays generic and "normal here" stays machine-specific
// (README.md#generic-in-axon-specific-in-the-overlay). TS rather than bash because the
// policy is array-of-tables, which tools/lib/toml.sh's single-line contract cannot
// parse — the same reason doctor.ts is TS. Invoke via the tools/storage launcher.
//
//   tools/storage           report, read-only
//   tools/storage --apply   run each applicable class's reclaim command
//   tools/storage --json    machine-readable, for the dashboard or a hook
//   tools/storage -h        this help
//
// Exit 0 = free space above the critical threshold, 1 = below it.
//
// The pure functions below are exported for tools/storage.test.ts. reclaimArgv is
// the one that matters: it decides whether a policy string reaches a shell, and a
// mistake there deletes the wrong thing.

import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { overlayRoot } from "./lib/overlay.ts";

const HELP = `tools/storage — what is filling this disk, and what is safe to reclaim.

  tools/storage           report, read-only
  tools/storage --apply   run each applicable class's reclaim command
  tools/storage --json    machine-readable output
  tools/storage -h        this help

Policy: <overlay>/config/storage-policy.toml
`;

export const GB = 1024 ** 3;

export type StorageClass = {
  name: string;
  paths: string[];
  reclaim?: string;
  apply?: boolean;
  regrows?: boolean;
  note?: string;
};
export type ProtectedEntry = { path: string; reason: string };
export type ExpectedService = { name: string; kind: string; note?: string };
export type Policy = {
  thresholds?: { free_warn_gb?: number; free_critical_gb?: number; class_flag_gb?: number };
  class?: StorageClass[];
  protected?: ProtectedEntry[];
  expected_service?: ExpectedService[];
};

export const expandHome = (p: string, home = homedir()) =>
  p === "~" ? home : p.startsWith("~/") ? resolve(home, p.slice(2)) : p;

export const fmt = (b: number) =>
  b >= GB ? `${(b / GB).toFixed(1)} GB` : `${Math.round(b / 1024 ** 2)} MB`;

/**
 * A class is only touchable by --apply when the policy both allows it and says how.
 * `apply = true` with no reclaim command is a policy bug, not a licence to guess.
 */
export const isApplicable = (c: StorageClass): boolean => Boolean(c.apply && c.reclaim);

/**
 * The safety boundary. A policy-supplied string is data, not code — the one case
 * where it reaches a shell is an explicit `rm -rf`, and even then we substitute the
 * paths WE measured rather than let the policy hand us a command line. Anything
 * else runs verbatim because it is a named tool's own cleanup verb
 * (`brew cleanup`, `cargo clean`) that only its own CLI can express.
 */
export function reclaimArgv(c: StorageClass, measuredPaths: string[]): string[] | null {
  if (!isApplicable(c)) return null;
  if (c.reclaim === "rm -rf") {
    if (measuredPaths.length === 0) return null;
    return ["rm", "-rf", ...measuredPaths];
  }
  return ["bash", "-lc", c.reclaim as string];
}

/** Free-space verdict against the policy thresholds. */
export function diskState(freeBytes: number, warnGb = 0, criticalGb = 0): "ok" | "warn" | "CRITICAL" {
  if (freeBytes < criticalGb * GB) return "CRITICAL";
  if (freeBytes < warnGb * GB) return "warn";
  return "ok";
}

/** Parse `df -k <target>`: second line, columns are total/used/free in 1K blocks. */
export function parseDf(text: string): { total: number; used: number; free: number } {
  const cols = (text.split("\n")[1] ?? "").trim().split(/\s+/);
  return { total: Number(cols[1]) * 1024, used: Number(cols[2]) * 1024, free: Number(cols[3]) * 1024 };
}

/** Sum `du -sx -k` output. Missing paths simply do not appear, so they contribute 0. */
export function parseDu(text: string): number {
  return text
    .split("\n")
    .filter(Boolean)
    .reduce((sum, line) => sum + Number(line.split("\t")[0] ?? 0) * 1024, 0);
}

/**
 * Glob only where the policy actually uses it: a single `*` in one path segment,
 * which is what "every crate's target dir" needs. Anything richer and the policy
 * would be doing work that belongs in the scanner.
 */
export function expandGlob(pattern: string, home = homedir()): string[] {
  const full = expandHome(pattern, home);
  if (!full.includes("*")) return existsSync(full) ? [full] : [];
  const star = full.indexOf("*");
  const base = full.slice(0, full.lastIndexOf("/", star));
  const segEnd = full.indexOf("/", star);
  const segment = segEnd === -1 ? full.slice(base.length + 1) : full.slice(base.length + 1, segEnd);
  const tail = segEnd === -1 ? "" : full.slice(segEnd);
  if (!existsSync(base)) return [];
  const re = new RegExp(`^${segment.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*")}$`);
  const out: string[] = [];
  for (const entry of new Bun.Glob("*").scanSync({ cwd: base, onlyFiles: false, depth: 0 })) {
    if (!re.test(entry)) continue;
    const candidate = `${base}/${entry}${tail}`;
    if (existsSync(candidate)) out.push(candidate);
  }
  return out.sort();
}

// ── Impure edges ───────────────────────────────────────────────────────────────

// `du -sx` rather than a recursive walk: it is the kernel's own accounting, it stays
// on one filesystem, and it reports allocated blocks — which is what actually frees.
async function sizeOf(paths: string[]): Promise<number> {
  if (paths.length === 0) return 0;
  const proc = Bun.spawn(["du", "-sx", "-k", ...paths], { stdout: "pipe", stderr: "ignore" });
  const text = await new Response(proc.stdout).text();
  await proc.exited;
  return parseDu(text);
}

async function diskUsage(): Promise<{ free: number; used: number; total: number; target: string }> {
  // APFS volume groups: / is the sealed read-only System volume and is always small.
  // The Data volume is where a cleanup actually changes a number.
  const target = existsSync("/System/Volumes/Data") ? "/System/Volumes/Data" : "/";
  const proc = Bun.spawn(["df", "-k", target], { stdout: "pipe", stderr: "ignore" });
  const text = await new Response(proc.stdout).text();
  await proc.exited;
  return { ...parseDf(text), target };
}

export async function main() {
  const argv = Bun.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) return void process.stdout.write(HELP);
  const doApply = argv.includes("--apply");
  const asJson = argv.includes("--json");

  const policyPath = `${overlayRoot()}/config/storage-policy.toml`;
  if (!existsSync(policyPath)) {
    console.error(
      `storage: no policy at ${policyPath}\nSee schemas/storage-policy.toml.example for the expected shape.`,
    );
    process.exit(2);
  }
  const policy = Bun.TOML.parse(await Bun.file(policyPath).text()) as Policy;
  const warnGb = policy.thresholds?.free_warn_gb ?? 0;
  const critGb = policy.thresholds?.free_critical_gb ?? 0;
  const flagGb = policy.thresholds?.class_flag_gb ?? Infinity;

  const measured = await Promise.all(
    (policy.class ?? []).map(async (cls) => {
      const paths = cls.paths.flatMap((p) => expandGlob(p));
      return { cls, paths, bytes: await sizeOf(paths) };
    }),
  );
  measured.sort((a, b) => b.bytes - a.bytes);

  const prot = await Promise.all(
    (policy.protected ?? []).map(async (p) => ({ ...p, bytes: await sizeOf(expandGlob(p.path)) })),
  );

  const disk = await diskUsage();

  if (asJson) {
    console.log(
      JSON.stringify(
        {
          disk,
          state: diskState(disk.free, warnGb, critGb),
          classes: measured.map((m) => ({
            name: m.cls.name,
            bytes: m.bytes,
            applicable: isApplicable(m.cls),
            flagged: m.bytes > flagGb * GB,
          })),
          protected: prot.map((p) => ({ path: p.path, bytes: p.bytes, reason: p.reason })),
        },
        null,
        2,
      ),
    );
    return void process.exit(disk.free < critGb * GB ? 1 : 0);
  }

  const pct = Math.round((disk.used / disk.total) * 100);
  console.log(`Axon storage · ${disk.target}`);
  console.log(
    `  ${fmt(disk.used)} used / ${fmt(disk.total)} (${pct}%) · ${fmt(disk.free)} free · ${diskState(disk.free, warnGb, critGb)}\n`,
  );

  console.log("Reclaimable by class");
  let reclaimable = 0;
  for (const { cls, bytes, paths } of measured) {
    if (bytes === 0) continue;
    if (isApplicable(cls)) reclaimable += bytes;
    const marks = [
      isApplicable(cls) ? "" : "report-only",
      bytes > flagGb * GB ? "OVER FLAG" : "",
      cls.regrows ? "regrows" : "",
    ].filter(Boolean);
    console.log(`  ${fmt(bytes).padStart(9)}  ${cls.name}${marks.length ? `  [${marks.join(", ")}]` : ""}`);
    console.log(
      `             ${paths.length} path${paths.length === 1 ? "" : "s"} · ${cls.reclaim ?? "no reclaim command"}`,
    );
  }
  console.log(`\n  ${fmt(reclaimable)} reclaimable without --apply restrictions\n`);

  if (prot.length) {
    console.log("Protected — reported, never applied");
    for (const p of prot) console.log(`  ${fmt(p.bytes).padStart(9)}  ${p.path}\n             ${p.reason}`);
    console.log();
  }

  if (policy.expected_service?.length) {
    console.log("Expected running (not findings)");
    for (const s of policy.expected_service) console.log(`  ${s.name} (${s.kind})${s.note ? ` — ${s.note}` : ""}`);
    console.log();
  }

  if (!doApply) {
    console.log("Read-only. Re-run with --apply to execute the applicable reclaims.");
    return void process.exit(disk.free < critGb * GB ? 1 : 0);
  }

  console.log("Applying");
  for (const { cls, bytes, paths } of measured) {
    const cmd = reclaimArgv(cls, paths);
    if (!cmd || bytes === 0) continue;
    const proc = Bun.spawn(cmd, { stdout: "ignore", stderr: "pipe" });
    const err = await new Response(proc.stderr).text();
    await proc.exited;
    const after = await sizeOf(paths.filter((p) => existsSync(p)));
    const failed = proc.exitCode === 0 ? "" : ` (exit ${proc.exitCode}: ${err.trim().slice(0, 120)})`;
    console.log(`  ${cls.name}: freed ${fmt(bytes - after)}${failed}`);
  }

  const post = await diskUsage();
  console.log(`\n  ${fmt(post.free)} free (was ${fmt(disk.free)}, +${fmt(post.free - disk.free)})`);
  process.exit(post.free < critGb * GB ? 1 : 0);
}

// Guarded so the test file can import the pure functions without running a scan.
if (import.meta.main) await main();
