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
  type Proc,
  type TaskRow,
  type WatchPolicy,
  classifyProcesses,
  decideEmission,
  parseCpuTime,
  parseElapsed,
  parsePsOutput,
  storageFinding,
  taskUrl,
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

  test("the key is stable, so a week-long breach is one task", () => {
    expect(storageFinding(warn)!.key).toBe(storageFinding({ ...warn, disk: { ...warn.disk, free: 69_000_000_000 } })!.key);
  });
});

describe("decideEmission — one task per run of a condition", () => {
  const row = (over: Partial<TaskRow>): TaskRow => ({
    id: "t1",
    status: "open",
    source_capability: "host-watch",
    source_id: "cpu:ApplicationsStorageExtension~1",
    ...over,
  });

  test("no history — create generation 1", () => {
    expect(decideEmission("cpu:ApplicationsStorageExtension", [])).toEqual({
      action: "create",
      sourceId: "cpu:ApplicationsStorageExtension~1",
    });
  });

  test("an open task for this condition — refresh it, do not create a second", () => {
    expect(decideEmission("cpu:ApplicationsStorageExtension", [row({})])).toEqual({
      action: "patch",
      id: "t1",
    });
  });

  test("the operator closed it and the condition returned — create a new generation", () => {
    expect(
      decideEmission("cpu:ApplicationsStorageExtension", [row({ status: "done" })]),
    ).toEqual({ action: "create", sourceId: "cpu:ApplicationsStorageExtension~2" });
  });

  test("generations count from the highest seen, not the row count", () => {
    const history = [
      row({ id: "a", status: "done", source_id: "cpu:ApplicationsStorageExtension~1" }),
      row({ id: "b", status: "dropped", source_id: "cpu:ApplicationsStorageExtension~7" }),
    ];
    expect(decideEmission("cpu:ApplicationsStorageExtension", history)).toEqual({
      action: "create",
      sourceId: "cpu:ApplicationsStorageExtension~8",
    });
  });

  test("another condition's history is not this condition's history", () => {
    const other = [row({ source_id: "storage:free-below-threshold~3" })];
    expect(decideEmission("cpu:ApplicationsStorageExtension", other)).toEqual({
      action: "create",
      sourceId: "cpu:ApplicationsStorageExtension~1",
    });
  });

  test("a key that prefixes another key is not confused with it", () => {
    const history = [row({ source_id: "cpu:Storage~4", status: "open" })];
    expect(decideEmission("cpu:StorageManagementService", history)).toEqual({
      action: "create",
      sourceId: "cpu:StorageManagementService~1",
    });
  });

  test("a task from another capability is ignored even on an identical source_id", () => {
    const foreign = [row({ source_capability: "comms" })];
    expect(decideEmission("cpu:ApplicationsStorageExtension", foreign)).toEqual({
      action: "create",
      sourceId: "cpu:ApplicationsStorageExtension~1",
    });
  });

});

// Regression, Axon#177. tasks derives a task id from `{capability}:{source_id}`, so every
// id carries a process name, and process names contain spaces. Interpolating one raw into
// the refresh URL 404'd on the first real run against the live capability — the condition
// was then re-detected forever and never refreshed. Every unit test was green at the time;
// only the end-to-end run caught it, which is the whole argument for doing one.
describe("taskUrl — the refresh address", () => {
  const id = "host-watch:cpu:Google Chrome Helper~1";

  test("the id is one encoded path segment, so it survives the round trip", () => {
    const url = new URL(taskUrl("http://127.0.0.1:8089", id));
    expect(decodeURIComponent(url.pathname)).toBe(`/api/tasks/${id}`);
  });

  test("no raw space reaches the path — that was the 404", () => {
    expect(taskUrl("http://h", id)).not.toContain(" ");
  });

  test("a fragment marker in an id cannot truncate the request", () => {
    const url = new URL(taskUrl("http://h", "host-watch:cpu:Foo#1"));
    expect(url.hash).toBe("");
    expect(decodeURIComponent(url.pathname)).toBe("/api/tasks/host-watch:cpu:Foo#1");
  });
});
