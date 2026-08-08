// tools/storage.test.ts — planted-fixture tests for the pure functions storage.ts's
// scan and --apply are built on.
//
// The one that earns its keep is reclaimArgv: it decides whether a policy-supplied
// string reaches a shell, and whether --apply deletes the paths we measured or a
// path a policy string claimed. Everything else here guards the arithmetic that
// turns raw du/df bytes into the numbers a human acts on.
// Run: bun test tools/storage.test.ts

import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  GB,
  diskState,
  expandGlob,
  expandHome,
  fmt,
  isApplicable,
  parseDf,
  parseDu,
  reclaimArgv,
  type StorageClass,
} from "./storage.ts";

const cls = (over: Partial<StorageClass> = {}): StorageClass => ({
  name: "fixture",
  paths: ["~/nowhere"],
  ...over,
});

describe("reclaimArgv — the --apply safety boundary", () => {
  test("rm -rf deletes the paths we measured, never a policy-supplied string", () => {
    const measured = ["/tmp/a/target", "/tmp/b/target"];
    expect(reclaimArgv(cls({ reclaim: "rm -rf", apply: true }), measured)).toEqual([
      "rm",
      "-rf",
      "/tmp/a/target",
      "/tmp/b/target",
    ]);
  });

  test("rm -rf with nothing measured is a no-op, not a bare `rm -rf`", () => {
    // The failure this guards: argv collapsing to ["rm","-rf"] and the command
    // inheriting a cwd, or a policy path expanding to "" and taking "/" with it.
    expect(reclaimArgv(cls({ reclaim: "rm -rf", apply: true }), [])).toBeNull();
  });

  test("a named tool's cleanup verb runs verbatim through a shell", () => {
    expect(reclaimArgv(cls({ reclaim: "brew cleanup --prune=all", apply: true }), [])).toEqual([
      "bash",
      "-lc",
      "brew cleanup --prune=all",
    ]);
  });

  test("shell metacharacters in a measured path stay one argv element", () => {
    // rm goes through argv, so a path is a path even when it looks like a command.
    const nasty = "/tmp/weird; rm -rf ~";
    expect(reclaimArgv(cls({ reclaim: "rm -rf", apply: true }), [nasty])).toEqual(["rm", "-rf", nasty]);
  });

  test("report-only classes yield no command whatever they declare", () => {
    expect(reclaimArgv(cls({ reclaim: "rm -rf", apply: false }), ["/tmp/x"])).toBeNull();
    expect(reclaimArgv(cls({ reclaim: "rm -rf" }), ["/tmp/x"])).toBeNull();
  });

  test("apply without a reclaim command is a policy bug, not a licence to guess", () => {
    expect(reclaimArgv(cls({ apply: true }), ["/tmp/x"])).toBeNull();
    expect(isApplicable(cls({ apply: true }))).toBe(false);
  });
});

describe("expandGlob", () => {
  test("expands a single * segment and keeps the tail, sorted", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-storage-glob-"));
    try {
      for (const crate of ["comms", "scouting", "trips"]) {
        mkdirSync(join(root, "capabilities", crate, "target"), { recursive: true });
      }
      // A crate with no target dir must not appear: the tail has to exist, not just
      // the globbed segment, or the scan reports paths that were never built.
      mkdirSync(join(root, "capabilities", "agentbox"), { recursive: true });

      expect(expandGlob(`${root}/capabilities/*/target`)).toEqual([
        join(root, "capabilities", "comms", "target"),
        join(root, "capabilities", "scouting", "target"),
        join(root, "capabilities", "trips", "target"),
      ]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("a trailing * segment needs no tail", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-storage-glob-tail-"));
    try {
      mkdirSync(join(root, "cache-a"));
      mkdirSync(join(root, "cache-b"));
      mkdirSync(join(root, "other"));
      expect(expandGlob(`${root}/cache-*`)).toEqual([join(root, "cache-a"), join(root, "cache-b")]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("a literal path is returned only when it exists", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-storage-lit-"));
    try {
      writeFileSync(join(root, "real"), "x");
      expect(expandGlob(join(root, "real"))).toEqual([join(root, "real")]);
      expect(expandGlob(join(root, "absent"))).toEqual([]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("a missing base directory yields nothing rather than throwing", () => {
    expect(expandGlob("/nonexistent-axon-fixture/*/target")).toEqual([]);
  });
});

describe("expandHome", () => {
  test("expands ~/ and bare ~, leaves absolute paths alone", () => {
    expect(expandHome("~/.omlx/cache", "/Users/fixture")).toBe("/Users/fixture/.omlx/cache");
    expect(expandHome("~", "/Users/fixture")).toBe("/Users/fixture");
    expect(expandHome("/var/log", "/Users/fixture")).toBe("/var/log");
  });

  test("a path merely starting with ~ is not a home reference", () => {
    expect(expandHome("~backup/data", "/Users/fixture")).toBe("~backup/data");
  });
});

describe("parseDu / parseDf", () => {
  test("parseDu sums 1K-block totals across paths", () => {
    expect(parseDu("1024\t/a\n2048\t/b\n")).toBe(3072 * 1024);
  });

  test("parseDu treats absent paths as zero, since du omits them entirely", () => {
    expect(parseDu("")).toBe(0);
  });

  test("parseDf reads the second line, not the header", () => {
    const df = [
      "Filesystem   1024-blocks      Used Available Capacity  Mounted on",
      "/dev/disk3s5   482797652 308596224 148500000    68%    /System/Volumes/Data",
      "",
    ].join("\n");
    expect(parseDf(df)).toEqual({
      total: 482797652 * 1024,
      used: 308596224 * 1024,
      free: 148500000 * 1024,
    });
  });
});

describe("diskState", () => {
  test("classifies against the policy thresholds", () => {
    expect(diskState(200 * GB, 80, 40)).toBe("ok");
    expect(diskState(60 * GB, 80, 40)).toBe("warn");
    expect(diskState(10 * GB, 80, 40)).toBe("CRITICAL");
  });

  test("critical wins on the boundary where the bands would overlap", () => {
    expect(diskState(39 * GB, 80, 40)).toBe("CRITICAL");
    expect(diskState(40 * GB, 80, 40)).toBe("warn");
  });

  test("absent thresholds never manufacture an alarm", () => {
    expect(diskState(0)).toBe("ok");
  });
});

describe("fmt", () => {
  test("switches unit at 1 GB and keeps the report columns comparable", () => {
    expect(fmt(46 * GB)).toBe("46.0 GB");
    expect(fmt(GB)).toBe("1.0 GB");
    expect(fmt(512 * 1024 ** 2)).toBe("512 MB");
    expect(fmt(0)).toBe("0 MB");
  });
});
