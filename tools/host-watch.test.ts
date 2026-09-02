// tools/host-watch.test.ts — planted-fixture tests for the pure functions host-watch.ts
// decides with. Every fixture row below is a real measurement from the 2026-08-15
// incident (Axon#177), not an invented number, because the whole question this tool
// answers is "which of these two processes is the runaway" and the honest answer is
// only interesting when both rows look alarming.
//
// The one that earns its keep is classifyProcesses: WindowServer had MORE cumulative
// CPU than the stuck extension (168m vs 148m) and was fine. A tool that ranks by CPU
// time flags the compositor every day and gets muted in a week.
// Run: bun test tools/host-watch.test.ts

import { describe, expect, test } from "bun:test";
import {
  type Finding,
  type FindingRow,
  type NetReport,
  type Proc,
  type WatchPolicy,
  classifyProcesses,
  decideEmission,
  decideResolutions,
  parseCpuTime,
  parseElapsed,
  netFinding,
  parsePsOutput,
  storageFinding,
} from "./host-watch.ts";

// The real `ps -Aceo pid,time,etime,comm` block from 2026-08-15 17:52, trimmed to the
// rows that matter. ApplicationsStorageExtension is the stuck System Settings Storage
// pane; it had run since 08:35 and burned 2h28m of CPU by this sample.
const PS_FIXTURE = `  PID      TIME     ELAPSED COMM
13105 148:50.33    09:17:29 ApplicationsStorageExtension
  402 168:50.56 01-02:00:53 WindowServer
12791  39:47.47    09:17:29 Storage
 1436  40:11.42 01-02:00:41 Spotify Helper (Renderer)
  330   0:00.10 01-02:00:54 smd
`;

const policy = (over: Partial<WatchPolicy> = {}): WatchPolicy => ({
  process: { min_cpu_seconds: 3600, min_cpu_ratio: 0.15 },
  allow_process: [{ comm: "WindowServer", reason: "the compositor; busy is its job" }],
  ...over,
});

describe("parseCpuTime — ps TIME", () => {
  test("MMM:SS.ff, minutes past 60", () => {
    expect(parseCpuTime("148:50.33")).toBeCloseTo(8930.33, 1);
    expect(parseCpuTime("168:50.56")).toBeCloseTo(10130.56, 1);
  });

  test("sub-minute", () => {
    expect(parseCpuTime("0:00.10")).toBeCloseTo(0.1, 2);
  });

  test("day form, for a process that has burned days of CPU", () => {
    expect(parseCpuTime("02-03:04:05")).toBe(2 * 86400 + 3 * 3600 + 4 * 60 + 5);
  });

  test("unparseable is zero, never NaN — a bad row must not become a finding", () => {
    expect(parseCpuTime("-")).toBe(0);
    expect(parseCpuTime("")).toBe(0);
  });
});

describe("parseElapsed — ps ELAPSED", () => {
  test("DD-HH:MM:SS", () => {
    expect(parseElapsed("01-02:00:53")).toBe(86400 + 2 * 3600 + 53);
  });

  test("HH:MM:SS", () => {
    expect(parseElapsed("09:17:29")).toBe(9 * 3600 + 17 * 60 + 29);
  });

  test("MM:SS", () => {
    expect(parseElapsed("04:12")).toBe(4 * 60 + 12);
  });

  test("unparseable is zero, never NaN", () => {
    expect(parseElapsed("garbage")).toBe(0);
  });
});

describe("parsePsOutput", () => {
  test("skips the header and keeps a command containing spaces", () => {
    const procs = parsePsOutput(PS_FIXTURE);
    expect(procs).toHaveLength(5);
    expect(procs.map((p) => p.comm)).toContain("Spotify Helper (Renderer)");
    expect(procs.find((p) => p.pid === 13105)?.cpuSeconds).toBeCloseTo(8930.33, 1);
  });
});

describe("classifyProcesses — the runaway rule", () => {
  test("flags the stuck Storage extension", () => {
    const found = classifyProcesses(parsePsOutput(PS_FIXTURE), policy());
    expect(found.map((f) => f.comm)).toContain("ApplicationsStorageExtension");
  });

  test("spares WindowServer, which had MORE cumulative CPU than the runaway", () => {
    const procs = parsePsOutput(PS_FIXTURE);
    const ws = procs.find((p) => p.comm === "WindowServer")!;
    const stuck = procs.find((p) => p.comm === "ApplicationsStorageExtension")!;
    expect(ws.cpuSeconds).toBeGreaterThan(stuck.cpuSeconds); // the premise of the test
    expect(classifyProcesses(procs, policy()).map((f) => f.comm)).not.toContain("WindowServer");
  });

  test("the ratio alone would also spare WindowServer — the allowlist is belt, not the whole trousers", () => {
    const noAllow = policy({ allow_process: [] });
    expect(classifyProcesses(parsePsOutput(PS_FIXTURE), noAllow).map((f) => f.comm)).not.toContain(
      "WindowServer",
    );
  });

  test("a short burst at 100% is not a runaway — the CPU floor holds", () => {
    const burst: Proc[] = [{ pid: 1, comm: "cc1plus", cpuSeconds: 300, elapsedSeconds: 305 }];
    expect(classifyProcesses(burst, policy())).toHaveLength(0);
  });

  test("a process older than the sample cannot exceed a ratio of 1", () => {
    const found = classifyProcesses(
      [{ pid: 1, comm: "runaway", cpuSeconds: 9000, elapsedSeconds: 9000 }],
      policy(),
    );
    expect(found[0]?.ratio).toBeLessThanOrEqual(1);
  });

  test("zero elapsed does not divide by zero", () => {
    expect(() =>
      classifyProcesses([{ pid: 1, comm: "x", cpuSeconds: 9000, elapsedSeconds: 0 }], policy()),
    ).not.toThrow();
  });

  test("the finding key is stable across runs — it names the process, not the pid or the clock", () => {
    const a = classifyProcesses(parsePsOutput(PS_FIXTURE), policy())[0];
    const shifted = parsePsOutput(PS_FIXTURE.replace("13105", "99999"));
    const b = classifyProcesses(shifted, policy())[0];
    expect(a.key).toBe(b.key);
  });

  // Regression, found by the first end-to-end run against host-watch's own table
  // (2026-08-28): a browser has several helper processes under one command name, so one
  // run produced several findings on one key and the store's unique index refused the
  // second. `tasks` had been swallowing that silently and this tool counted a task it
  // never wrote. The key is the condition, so one run yields one finding per key.
  test("several processes sharing a command are one finding, the worst of them", () => {
    const helpers: Proc[] = [
      { pid: 1, comm: "Google Chrome Helper", cpuSeconds: 7200, elapsedSeconds: 14_400 },
      { pid: 2, comm: "Google Chrome Helper", cpuSeconds: 7200, elapsedSeconds: 7_200 },
      { pid: 3, comm: "Google Chrome Helper", cpuSeconds: 7200, elapsedSeconds: 36_000 },
    ];
    const found = classifyProcesses(helpers, policy({ allow_process: [] }));
    expect(found).toHaveLength(1);
    expect(found[0].pid).toBe(2); // ratio 1.00, against 0.50 and 0.20
    expect(found[0].note).toContain("2 other process(es) named Google Chrome Helper");
  });
});

describe("storageFinding — fires on the volume state, never on a class being large", () => {
  const ok = { disk: { free: 140_000_000_000, target: "/System/Volumes/Data" }, state: "ok" };
  const warn = { disk: { free: 70_000_000_000, target: "/System/Volumes/Data" }, state: "warn" };

  test("a healthy volume is not a finding", () => {
    expect(storageFinding(ok)).toBeNull();
  });

  test("state warn is a finding naming the free space", () => {
    const f = storageFinding(warn)!;
    expect(f).not.toBeNull();
    expect(f.title).toMatch(/65\.2 GB|70/);
  });

  test("an over-flag class on a healthy volume stays silent (Axon#177 decision)", () => {
    const withFlagged = {
      ...ok,
      classes: [{ name: "rust-workspace-target", bytes: 30_500_000_000, flagged: true }],
    };
    expect(storageFinding(withFlagged)).toBeNull();
  });

  test("the key is stable, so a week-long breach is one finding", () => {
    expect(storageFinding(warn)!.key).toBe(storageFinding({ ...warn, disk: { ...warn.disk, free: 69_000_000_000 } })!.key);
  });
});

describe("decideEmission — one row per run of a condition", () => {
  const row = (over: Partial<FindingRow>): FindingRow => ({
    id: "cpu:ApplicationsStorageExtension~1",
    key: "cpu:ApplicationsStorageExtension",
    generation: 1,
    status: "open",
    ...over,
  });

  test("no history — create generation 1", () => {
    expect(decideEmission("cpu:ApplicationsStorageExtension", [])).toEqual({
      action: "create",
      id: "cpu:ApplicationsStorageExtension~1",
      generation: 1,
    });
  });

  test("an open row for this condition — refresh it, do not create a second", () => {
    expect(decideEmission("cpu:ApplicationsStorageExtension", [row({})])).toEqual({
      action: "refresh",
      id: "cpu:ApplicationsStorageExtension~1",
    });
  });

  test("it cleared and the condition returned — create a new generation", () => {
    expect(decideEmission("cpu:ApplicationsStorageExtension", [row({ status: "resolved" })])).toEqual({
      action: "create",
      id: "cpu:ApplicationsStorageExtension~2",
      generation: 2,
    });
  });

  test("generations count from the highest seen, not the row count", () => {
    const history = [
      row({ id: "a", status: "resolved", generation: 1 }),
      row({ id: "b", status: "resolved", generation: 7 }),
    ];
    expect(decideEmission("cpu:ApplicationsStorageExtension", history)).toEqual({
      action: "create",
      id: "cpu:ApplicationsStorageExtension~8",
      generation: 8,
    });
  });

  test("another condition's history is not this condition's history", () => {
    const other = [row({ key: "storage:free-below-threshold", generation: 3 })];
    expect(decideEmission("cpu:ApplicationsStorageExtension", other)).toEqual({
      action: "create",
      id: "cpu:ApplicationsStorageExtension~1",
      generation: 1,
    });
  });

  // The Axon#177 bug that a packed `{key}~{n}` id made possible: `cpu:Storage` is a
  // prefix of `cpu:StorageManagementService`, so a startsWith comparison gave one
  // condition the other's history. `key` is a column now, so this is exact — the test
  // stays because the guarantee is what matters, not how it is obtained.
  test("a key that prefixes another key is not confused with it", () => {
    const history = [row({ id: "cpu:Storage~4", key: "cpu:Storage", generation: 4 })];
    expect(decideEmission("cpu:StorageManagementService", history)).toEqual({
      action: "create",
      id: "cpu:StorageManagementService~1",
      generation: 1,
    });
  });
});

// The half that could not exist while `tasks` owned the lifecycle: nothing closes a
// finding now except the watcher itself, so a run that no longer sees a condition has to
// say so. Without this every row written since 2026-08 would stay open forever.
describe("decideResolutions — a run closes what it no longer sees", () => {
  const finding = (key: string): Finding => ({ key, title: key, note: "" });
  const open = (id: string, key: string): FindingRow => ({ id, key, generation: 1, status: "open" });

  test("a condition the run did not see is closed", () => {
    const existing = [open("cpu:Foo~1", "cpu:Foo"), open("cpu:Bar~1", "cpu:Bar")];
    expect(decideResolutions([finding("cpu:Foo")], existing)).toEqual(["cpu:Bar~1"]);
  });

  test("a healthy run closes everything that was open", () => {
    const existing = [open("cpu:Foo~1", "cpu:Foo"), open("cpu:Bar~1", "cpu:Bar")];
    expect(decideResolutions([], existing)).toEqual(["cpu:Foo~1", "cpu:Bar~1"]);
  });

  test("an already-resolved row is not resolved twice", () => {
    const existing = [{ ...open("cpu:Foo~1", "cpu:Foo"), status: "resolved" }];
    expect(decideResolutions([], existing)).toEqual([]);
  });
});

// The third condition, delegated whole to `host-net check --json`. These fixtures are the
// payload shape that command emits, with the process names replaced: this file is public and
// which daemons a given machine runs is an overlay fact.
describe("netFinding — one finding for the whole condition", () => {
  const report = (unexpected: NetReport["unexpected"]): NetReport => ({
    listeners: 46,
    wildcard: 24,
    policy: "<overlay>/config/host-net-policy.toml",
    unexpected,
  });

  test("a host whose wildcard listeners are all declared files nothing", () => {
    expect(netFinding(report([]))).toBeNull();
  });

  test("host-net could not run: no finding, no throw", () => {
    expect(netFinding(null)).toBeNull();
  });

  // Three listeners are one condition. The key carries no port because a mesh VPN's wildcard
  // ports change on every restart, and a key that moves mints a new generation every hour.
  test("three unexpected listeners are one finding with a port-free key", () => {
    const three = netFinding(
      report([
        { process: "example-daemon", port: "19222", protos: "tcp46", pid: 22458 },
        { process: "example-vpn-extension", port: "443", protos: "tcp4+tcp6", pid: 11481 },
        { process: "example-vpn-extension", port: "41641", protos: "udp4+udp6", pid: 11481 },
      ]),
    );
    expect(three).not.toBeNull();
    expect(three!.key).toBe("net:unexpected-exposure");
    expect(three!.note).toContain("example-daemon on *:19222");
    expect(three!.note).toContain("*:443");
    expect(three!.note).toContain("*:41641");

    // The same three listeners after a restart, on different ephemeral ports: the same key.
    const later = netFinding(
      report([
        { process: "example-daemon", port: "19222", protos: "tcp46", pid: 31002 },
        { process: "example-vpn-extension", port: "443", protos: "tcp4+tcp6", pid: 31111 },
        { process: "example-vpn-extension", port: "50007", protos: "udp4+udp6", pid: 31111 },
      ]),
    );
    expect(later!.key).toBe(three!.key);
  });

  test("the title names the distinct processes, not one line per port", () => {
    const finding = netFinding(
      report([
        { process: "example-vpn-extension", port: "443", protos: "tcp4", pid: 1 },
        { process: "example-vpn-extension", port: "41641", protos: "udp4", pid: 1 },
      ]),
    );
    expect(finding!.title).toBe("2 wildcard listener(s) not in the host-net policy (example-vpn-extension)");
  });
});
