// tools/host-watch.ts — one pass of the host watch, then exit.
//
// Two host conditions are invisible until they hurt, and both bit on 2026-08-15
// (Axon#177). A System Settings Storage pane got stuck at 08:35 and burned 3h29m of CPU
// before anyone noticed; the only symptom was "the Mac feels slow", nine hours later.
// And the disk half was ALREADY solved — tools/storage answered it in seconds — but
// nothing ran it, so it may as well not have existed. A tool nobody runs and a tool
// nobody built are the same tool.
//
// So: notice, then hand off. This writes a `tasks` record per finding and NOTHING when
// the machine is healthy, because a watcher that cries wolf gets muted and a muted
// watcher is worse than none. No new notification machinery — core Axon has never had a
// notifier and does not grow one here (the precedent tools/sparpreis-watch.ts states in
// its own header, and capabilities/tasks' "other capabilities notice things and hand
// them here").
//
//   tools/host-watch              check, write findings to tasks
//   tools/host-watch --dry-run    check and print; write nothing
//   tools/host-watch --json       machine-readable findings
//   tools/host-watch -h           this help
//
// Exit 0 = checked (findings or not). 1 = could not check. 2 = no policy.
//
// Knows no fact about this machine. Every budget, threshold and allowlisted process
// comes from the overlay's config/host-watch-policy.toml, the same split
// tools/storage.ts already uses (README.md#generic-in-axon-specific-in-the-overlay).
// The pure functions below are exported for tools/host-watch.test.ts.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { fmt } from "./storage.ts";
import { axonRoot, overlayRoot } from "./lib/overlay.ts";

const HELP = `tools/host-watch — notice a runaway process or a filling disk, once per run.

  tools/host-watch              check, write findings to tasks
  tools/host-watch --dry-run    check and print; write nothing
  tools/host-watch --json       machine-readable findings
  tools/host-watch -h           this help

Policy: <overlay>/config/host-watch-policy.toml
`;

const CAPABILITY = "host-watch";

export type Proc = { pid: number; cpuSeconds: number; elapsedSeconds: number; comm: string };
export type ProcessBudget = { min_cpu_seconds?: number; min_cpu_ratio?: number };
export type AllowedProcess = { comm: string; reason: string };
export type WatchPolicy = { process?: ProcessBudget; allow_process?: AllowedProcess[] };

export type Finding = { key: string; title: string; note: string };
export type ProcFinding = Finding & { pid: number; comm: string; ratio: number; cpuSeconds: number };

export type TaskRow = {
  id: string;
  status: string;
  source_capability?: string | null;
  source_id?: string | null;
};
export type Emission = { action: "patch"; id: string } | { action: "create"; sourceId: string };

/**
 * ps prints three shapes and only three: `DD-HH:MM:SS`, `HH:MM:SS`, and `MM:SS[.ff]`,
 * where MM is NOT bounded at 60 (a process with 168 minutes of CPU prints "168:50.56").
 * Anything else is a row we do not understand, and the honest value for that is 0 —
 * NaN would propagate into a ratio and a garbage row would become a finding.
 */
function hmsToSeconds(raw: string): number {
  const t = raw.trim();
  const day = /^(\d+)-(\d+):(\d+):(\d+(?:\.\d+)?)$/.exec(t);
  if (day) return +day[1] * 86400 + +day[2] * 3600 + +day[3] * 60 + +day[4];
  const hms = /^(\d+):(\d+):(\d+(?:\.\d+)?)$/.exec(t);
  if (hms) return +hms[1] * 3600 + +hms[2] * 60 + +hms[3];
  const ms = /^(\d+):(\d+(?:\.\d+)?)$/.exec(t);
  if (ms) return +ms[1] * 60 + +ms[2];
  return 0;
}

/** Cumulative CPU time a process has consumed, in seconds. */
export const parseCpuTime = (raw: string): number => hmsToSeconds(raw);

/** Wall-clock time a process has existed, in seconds. */
export const parseElapsed = (raw: string): number => hmsToSeconds(raw);

/**
 * `ps -Aceo pid,time,etime,comm`. The command is LAST because it is the only field that
 * contains spaces ("Spotify Helper (Renderer)"), so it takes the rest of the line; the
 * header has no leading digits and falls out on its own.
 */
export function parsePsOutput(text: string): Proc[] {
  const procs: Proc[] = [];
  for (const line of text.split("\n")) {
    const m = /^\s*(\d+)\s+(\S+)\s+(\S+)\s+(.+?)\s*$/.exec(line);
    if (!m) continue;
    procs.push({
      pid: +m[1],
      cpuSeconds: parseCpuTime(m[2]),
      elapsedSeconds: parseElapsed(m[3]),
      comm: m[4],
    });
  }
  return procs;
}

/**
 * The runaway rule, and the reason it is two conditions rather than one.
 *
 * Ranking by cumulative CPU is the obvious implementation and it is wrong: on the day
 * this was written WindowServer had MORE CPU time than the stuck extension (168m vs
 * 148m) and was perfectly healthy — it had simply been alive four times longer. The
 * signal is the RATIO, how much of a core a process has held for its whole life.
 *
 * The absolute floor is the second condition, and it exists for the opposite error: a
 * compiler at 100% for five minutes has a ratio of 1.0 and is not a runaway, it is a
 * build. Something has to have been wrong for a while before it is worth an interrupt.
 *
 * The ratio is deliberately NOT capped at 1. A process pinning four cores for an hour
 * reads as 4.0, which is exactly how alarming it should look.
 */
export function classifyProcesses(procs: Proc[], policy: WatchPolicy): ProcFinding[] {
  const floor = policy.process?.min_cpu_seconds ?? Infinity;
  const minRatio = policy.process?.min_cpu_ratio ?? Infinity;
  const allowed = new Set((policy.allow_process ?? []).map((a) => a.comm));

  const found: ProcFinding[] = [];
  for (const p of procs) {
    if (allowed.has(p.comm)) continue;
    if (p.cpuSeconds < floor) continue;
    const ratio = p.elapsedSeconds > 0 ? p.cpuSeconds / p.elapsedSeconds : 0;
    if (ratio < minRatio) continue;
    found.push({
      // Keyed on the command, never the pid: a pid is a different number every boot and
      // would make every restart look like a new problem.
      key: `cpu:${p.comm}`,
      pid: p.pid,
      comm: p.comm,
      ratio,
      cpuSeconds: p.cpuSeconds,
      title: `${p.comm} has used ${hours(p.cpuSeconds)} of CPU (${ratio.toFixed(2)} cores sustained)`,
      note:
        `pid ${p.pid} · ${hours(p.cpuSeconds)} CPU over ${hours(p.elapsedSeconds)} wall ` +
        `= ${ratio.toFixed(2)} cores held continuously.\n` +
        `Inspect: ps -p ${p.pid} -o pid,lstart,time,pcpu,command\n` +
        `If it is stuck rather than working: kill ${p.pid}`,
    });
  }
  return found.sort((a, b) => b.ratio - a.ratio);
}

const hours = (s: number) => (s >= 3600 ? `${(s / 3600).toFixed(1)}h` : `${Math.round(s / 60)}m`);

export type StorageReport = {
  disk?: { free?: number; target?: string };
  state?: string;
  classes?: Array<{ name: string; bytes: number; flagged?: boolean }>;
};

/**
 * The volume state is the finding; a large class is not.
 *
 * Decided in Axon#177 against the tempting alternative. `class_flag_gb` legitimately
 * fires today — the cargo target dir is 28 GB against a 20 GB flag — on a machine with
 * 130 GB free and nothing wrong with it. Alerting on that would mean this watcher's
 * FIRST run produced a task nobody needed, which is precisely how a watcher gets muted.
 * The class breakdown stays what it already was: what `sysmon storage` tells you once
 * you are looking.
 */
export function storageFinding(report: StorageReport): Finding | null {
  const state = report.state ?? "ok";
  if (state === "ok") return null;
  const free = report.disk?.free ?? 0;
  const target = report.disk?.target ?? "the data volume";
  return {
    key: "storage:free-below-threshold",
    title: `Disk ${state}: ${fmt(free)} free on ${target}`,
    note:
      `Free space crossed the ${state} threshold in the overlay's storage-policy.toml.\n` +
      `What is filling it, and what is safe to reclaim: tools/sysmon storage\n` +
      `Reclaim the applicable classes: tools/sysmon storage --apply`,
  };
}

/**
 * One task per RUN of a condition, not one per check and not one forever.
 *
 * tasks' partial unique index on (source_capability, source_id) already collapses
 * repeats, which is most of the job. What it cannot express on its own is the case that
 * makes the difference between a watcher that works next year and one that silently
 * stops: the operator marks the task done, and six weeks later the same condition comes
 * back. A fixed source_id would upsert onto the closed row and say nothing, forever. So
 * the id carries a generation, and a new one is minted only once every prior task for
 * this condition is closed.
 */
export function decideEmission(key: string, existing: TaskRow[]): Emission {
  const mine = existing.filter(
    (t) => t.source_capability === CAPABILITY && generationOf(key, t.source_id) !== null,
  );
  const open = mine.find((t) => t.status === "open");
  if (open) return { action: "patch", id: open.id };
  const highest = mine.reduce((max, t) => Math.max(max, generationOf(key, t.source_id) ?? 0), 0);
  return { action: "create", sourceId: `${key}${GEN}${highest + 1}` };
}

/**
 * The generation separator, and why it is not `#`.
 *
 * tasks derives a task's id from `{capability}:{source_id}` (store.rs), and that id goes
 * straight into a URL path on every PATCH. `#` was the first choice and it 404'd on the
 * first real run: a fragment marker truncates the request path at the client, so the
 * server saw `/api/tasks/host-watch:cpu:Google Chrome` and had never heard of it. `~` is
 * unreserved in RFC 3986, survives a path segment untouched, and is no likelier to occur
 * in a process name than `#` was.
 *
 * The separator is load-bearing regardless of which character it is: without one,
 * `cpu:Storage` would claim `cpu:StorageManagementService`'s history and two unrelated
 * conditions would share a task.
 */
const GEN = "~";

function generationOf(key: string, sourceId: string | null | undefined): number | null {
  const prefix = `${key}${GEN}`;
  if (!sourceId?.startsWith(prefix)) return null;
  const n = Number(sourceId.slice(prefix.length));
  return Number.isInteger(n) && n > 0 ? n : null;
}

/**
 * The address of one task. Exported because getting it wrong is silent: tasks derives a
 * task id from `{capability}:{source_id}`, so every id here carries a process name, and
 * process names contain spaces ("Google Chrome Helper"). The id is ONE path segment and
 * has to be encoded as one — interpolating it raw 404'd on the first real run.
 */
export const taskUrl = (base: string, id: string) => `${base}/api/tasks/${encodeURIComponent(id)}`;

// ── I/O ────────────────────────────────────────────────────────────────────────

function fail(message: string, code = 1): never {
  console.error(`host-watch: ${message}`);
  process.exit(code);
}

/** A capability's port, from the one file that declares it. */
function portOf(capability: string): string {
  const manifest = join(axonRoot(), "capabilities", capability, "service.toml");
  if (!existsSync(manifest)) fail(`no ${manifest}`);
  const line = readFileSync(manifest, "utf8")
    .split("\n")
    .find((l) => /^port\s*=/.test(l));
  const port = line?.match(/"([^"]*)"/)?.[1] ?? "";
  if (!port) fail(`no port in ${manifest}`);
  return port;
}

async function runPs(): Promise<Proc[]> {
  const proc = Bun.spawn(["ps", "-Aceo", "pid,time,etime,comm"], { stdout: "pipe", stderr: "pipe" });
  const text = await new Response(proc.stdout).text();
  await proc.exited;
  if (proc.exitCode !== 0) fail("ps failed");
  return parsePsOutput(text);
}

/**
 * storage.ts is invoked rather than imported: it owns the policy file, the du/df
 * arithmetic and the exit code, and re-deriving any of that here would be the second
 * source of truth its own header argues against. A non-zero exit is NOT a failure — it
 * is how storage reports free space below critical, which is the loudest thing it can say.
 */
async function runStorage(): Promise<StorageReport | null> {
  const proc = Bun.spawn(["bun", "run", join(axonRoot(), "tools", "storage.ts"), "--json"], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const text = await new Response(proc.stdout).text();
  await proc.exited;
  try {
    return JSON.parse(text) as StorageReport;
  } catch {
    const err = (await new Response(proc.stderr).text()).trim().slice(0, 200);
    console.error(`host-watch: storage --json unreadable (exit ${proc.exitCode}) ${err}`);
    return null;
  }
}

async function emit(findings: Finding[], base: string): Promise<number> {
  const listed = await fetch(`${base}/api/tasks`);
  if (!listed.ok) fail(`tasks GET /api/tasks returned HTTP ${listed.status}`);
  const existing = ((await listed.json()) as { tasks?: TaskRow[] }).tasks ?? [];

  let created = 0;
  for (const f of findings) {
    const decision = decideEmission(f.key, existing);
    if (decision.action === "patch") {
      const res = await fetch(taskUrl(base, decision.id), {
        method: "PATCH",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ note: f.note, title: f.title }),
      });
      if (!res.ok) console.error(`host-watch: refresh of ${f.key} returned HTTP ${res.status}`);
      else console.log(`host-watch: still open — ${f.title}`);
      continue;
    }
    const res = await fetch(`${base}/api/tasks`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        title: f.title,
        note: f.note,
        source_capability: CAPABILITY,
        source_id: decision.sourceId,
      }),
    });
    if (!res.ok) {
      console.error(`host-watch: task write for ${f.key} returned HTTP ${res.status}`);
      continue;
    }
    created += 1;
    console.log(`host-watch: NEW — ${f.title}`);
  }
  return created;
}

export async function main() {
  const argv = Bun.argv.slice(2);
  if (argv.includes("-h") || argv.includes("--help")) return void process.stdout.write(HELP);
  const dryRun = argv.includes("--dry-run");
  const asJson = argv.includes("--json");

  const policyPath = `${overlayRoot()}/config/host-watch-policy.toml`;
  if (!existsSync(policyPath)) {
    console.error(
      `host-watch: no policy at ${policyPath}\nSee schemas/host-watch-policy.toml.example for the expected shape.`,
    );
    process.exit(2);
  }
  const policy = Bun.TOML.parse(await Bun.file(policyPath).text()) as WatchPolicy;

  const [procs, storage] = await Promise.all([runPs(), runStorage()]);
  const findings: Finding[] = [
    ...classifyProcesses(procs, policy),
    ...(storage ? [storageFinding(storage)].filter((f): f is Finding => f !== null) : []),
  ];

  if (asJson) {
    console.log(JSON.stringify({ checked: procs.length, findings }, null, 2));
    return;
  }

  if (findings.length === 0) {
    console.log(`host-watch: ${procs.length} processes, disk ${storage?.state ?? "unknown"} — nothing to report`);
    return;
  }

  if (dryRun) {
    console.log(`host-watch: ${findings.length} finding(s), writing nothing (--dry-run)`);
    for (const f of findings) console.log(`  ${f.title}`);
    return;
  }

  const created = await emit(findings, `http://127.0.0.1:${portOf("tasks")}`);
  console.log(`host-watch: ${findings.length} finding(s), ${created} new task(s)`);
}

// Guarded so the test file can import the pure functions without running a scan.
if (import.meta.main) await main();
