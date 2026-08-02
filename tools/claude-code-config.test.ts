// tools/claude-code-config.test.ts — the additive-only contract for overlay fragments.
//
// mergeFragment decides what an overlay is allowed to do to the /etc security policy, so
// the interesting cases are the REFUSALS. A merger that silently dropped an illegal
// contribution would deploy a policy weaker than the fragment's author believes it to be,
// and nothing downstream would notice: the file is valid JSON, the tool exits 0, and the
// missing rule only shows up the day it was supposed to stop something.
// Run: bun test tools/claude-code-config.test.ts

import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  managedHandoffInstructions,
  mergeFragment,
  stageManagedPolicy,
} from "./claude-code-config.ts";

const base = () => ({
  permissions: { deny: ["Read(~/.secrets)"], disableBypassPermissionsMode: "disable" },
  sandbox: {
    enabled: true,
    filesystem: { denyRead: ["~/.secrets"], denyWrite: ["~/.secrets"] },
    credentials: {
      files: [{ path: "~/.secrets", mode: "deny" }],
      envVars: [{ name: "GITHUB_TOKEN", mode: "deny" }],
    },
  },
  allowedMcpServers: [] as unknown[],
});

function merge(fragment: Record<string, unknown>) {
  const appended: string[] = [];
  const out = mergeFragment(base(), fragment, "", appended);
  return { out, appended };
}

describe("mergeFragment — what an overlay may add", () => {
  test("appends to a nested array the base declares, base entries first", () => {
    const { out, appended } = merge({
      sandbox: { credentials: { envVars: [{ name: "EXAMPLE_INTERNAL_API_KEY", mode: "deny" }] } },
    });
    expect((out.sandbox as any).credentials.envVars).toEqual([
      { name: "GITHUB_TOKEN", mode: "deny" },
      { name: "EXAMPLE_INTERNAL_API_KEY", mode: "deny" },
    ]);
    expect(appended).toHaveLength(1);
  });

  test("untouched branches survive the merge intact", () => {
    const { out } = merge({ permissions: { deny: ["Read(~/work-secrets)"] } });
    expect((out.permissions as any).disableBypassPermissionsMode).toBe("disable");
    expect((out.sandbox as any).enabled).toBe(true);
  });

  test("re-stating a base rule is a no-op, not a duplicate", () => {
    const { out, appended } = merge({
      sandbox: { credentials: { envVars: [{ name: "GITHUB_TOKEN", mode: "deny" }] } },
    });
    expect((out.sandbox as any).credentials.envVars).toHaveLength(1);
    expect(appended).toEqual([]);
  });

  test("appends deployment-specific read and write denials", () => {
    const { out } = merge({
      sandbox: { filesystem: { denyRead: ["~/Protected/**"], denyWrite: ["~/Protected/**"] } },
    });
    expect((out.sandbox as any).filesystem.denyRead).toContain("~/Protected/**");
    expect((out.sandbox as any).filesystem.denyWrite).toContain("~/Protected/**");
  });

  test("_-prefixed keys are documentation and never reach the policy", () => {
    const { out, appended } = merge({ _comment: "why this exists", _why: "Rule 21" });
    expect(out).toEqual(base());
    expect(appended).toEqual([]);
    expect(Object.keys(out)).not.toContain("_comment");
  });
});

describe("mergeFragment — what an overlay may NOT do", () => {
  test("rejects expanding an allowlist even when the base declares the array", () => {
    expect(() => merge({ allowedMcpServers: ["https://mcp.example.internal"] })).toThrow(
      /may not extend this array/,
    );
  });

  test("rejects a credential entry that is not deny-only", () => {
    expect(() =>
      merge({ sandbox: { credentials: { envVars: [{ name: "EXAMPLE_INTERNAL_API_KEY", mode: "allow" }] } } }),
    ).toThrow(/mode "deny"/);
  });

  test("reports only the policy path for appended private rules", () => {
    const { appended } = merge({
      sandbox: { credentials: { envVars: [{ name: "PRIVATE_FIXTURE_VALUE", mode: "deny" }] } },
    });
    expect(appended).toEqual(["sandbox.credentials.envVars"]);
    expect(JSON.stringify(appended)).not.toContain("PRIVATE_FIXTURE_VALUE");
  });

  test("rejects a key the base does not declare", () => {
    expect(() => merge({ disableSideloadFlags: true })).toThrow(/disableSideloadFlags/);
  });

  test("rejects flipping a scalar — the sandbox cannot be turned off from an overlay", () => {
    expect(() => merge({ sandbox: { enabled: false } })).toThrow(/sandbox\.enabled/);
  });

  test("rejects replacing an array with a scalar", () => {
    expect(() => merge({ permissions: { deny: "everything" } })).toThrow(/permissions\.deny/);
  });

  test("names the full dotted path so a deep violation is findable", () => {
    expect(() =>
      merge({ sandbox: { credentials: { envVars: { name: "X" } } } }),
    ).toThrow(/sandbox\.credentials\.envVars/);
  });

  test("a partly-legal fragment fails whole — no half-applied security policy", () => {
    expect(() =>
      merge({
        sandbox: { credentials: { envVars: [{ name: "EXAMPLE_INTERNAL_API_KEY", mode: "deny" }] } },
        allowManagedPermissionRulesOnly: false,
      }),
    ).toThrow(/allowManagedPermissionRulesOnly/);
  });
});

describe("public managed-policy boundary", () => {
  const policy = JSON.parse(
    readFileSync(`${import.meta.dir}/templates/claude-code/managed-settings.json`, "utf8"),
  );

  test("contains no real Developer path", () => {
    expect(JSON.stringify(policy)).not.toContain("~/Developer/");
  });

  test("contains only generic public credential families", () => {
    const names = policy.sandbox.credentials.envVars.map((entry: { name: string }) => entry.name);
    for (const name of names) {
      expect(name).toMatch(/^(ANTHROPIC_|AWS_|GITHUB_TOKEN$)/);
    }
  });
});

const cli = join(import.meta.dir, "claude-code-config.ts");

function runManaged(overlay: string, target: string, extra: string[] = []) {
  const capture = mkdtempSync(join(tmpdir(), "axon-managed-output-"));
  const stdoutPath = join(capture, "stdout");
  const stderrPath = join(capture, "stderr");
  try {
    const proc = spawnSync(
      "/bin/sh",
      [
        "-c",
        'exec "$@" >"$AXON_TEST_STDOUT" 2>"$AXON_TEST_STDERR"',
        "axon-managed-test",
        process.execPath,
        "run",
        cli,
        "--managed",
        ...extra,
      ],
      {
      env: {
        ...process.env,
        AXON_OVERLAY_ROOT: overlay,
        MANAGED_SETTINGS_PATH: target,
        AXON_TEST_STDOUT: stdoutPath,
        AXON_TEST_STDERR: stderrPath,
      },
      },
    );
    return {
      exitCode: proc.status,
      stdout: readFileSync(stdoutPath, "utf8"),
      stderr: readFileSync(stderrPath, "utf8"),
    };
  } finally {
    rmSync(capture, { recursive: true, force: true });
  }
}

function fragmentPath(overlay: string): string {
  return join(overlay, "config", "claude-code", "managed-settings.fragment.json");
}

describe("managed-policy CLI deployment boundary", () => {
  test("a configured but missing overlay fails closed without replacing the target", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-managed-missing-"));
    try {
      const target = join(root, "deployed.json");
      writeFileSync(target, "sentinel\n");
      const proc = runManaged(join(root, "unreachable-private-overlay"), target);
      expect(proc.exitCode).toBe(3);
      expect(proc.stderr.toString()).toContain("private overlay is not reachable");
      expect(readFileSync(target, "utf8")).toBe("sentinel\n");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("an absent fragment is explicit and deploys only the public base", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-managed-absent-"));
    try {
      const overlay = join(root, "overlay");
      const target = join(root, "deployed.json");
      mkdirSync(overlay);
      const proc = runManaged(overlay, target);
      expect(proc.exitCode).toBe(0);
      expect(proc.stdout.toString()).toContain("no overlay managed-policy fragment is configured");
      expect(JSON.parse(readFileSync(target, "utf8"))).toBeTruthy();
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("malformed fragment output is redacted and cannot replace the target", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-managed-malformed-"));
    try {
      const overlay = join(root, "private-fragment-coordinate");
      const path = fragmentPath(overlay);
      const target = join(root, "deployed.json");
      mkdirSync(join(overlay, "config", "claude-code"), { recursive: true });
      writeFileSync(path, '{"private":"SYNTHETIC_PRIVATE_VALUE"');
      writeFileSync(target, "sentinel\n");
      const proc = runManaged(overlay, target);
      const output = proc.stdout.toString() + proc.stderr.toString();
      expect(proc.exitCode).toBe(3);
      expect(output).not.toContain(path);
      expect(output).not.toContain("SYNTHETIC_PRIVATE_VALUE");
      expect(readFileSync(target, "utf8")).toBe("sentinel\n");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("normal and dry-run reports reveal neither fragment values nor its path", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-managed-redaction-"));
    try {
      const overlay = join(root, "private-fragment-coordinate");
      const path = fragmentPath(overlay);
      mkdirSync(join(overlay, "config", "claude-code"), { recursive: true });
      writeFileSync(path, JSON.stringify({ permissions: { deny: ["Read(SYNTHETIC_PRIVATE_VALUE)"] } }));

      for (const [name, args] of [["normal", []], ["dry", ["--dry-run"]]] as const) {
        const target = join(root, `${name}.json`);
        const proc = runManaged(overlay, target, [...args]);
        const output = proc.stdout.toString() + proc.stderr.toString();
        expect(proc.exitCode).toBe(0);
        expect(output).not.toContain(path);
        expect(output).not.toContain("SYNTHETIC_PRIVATE_VALUE");
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("privileged staging is unique, private, and cleans only its owned directory", () => {
    const first = stageManagedPolicy('{"private":"SYNTHETIC_PRIVATE_VALUE"}\n');
    const second = stageManagedPolicy("{}\n");
    try {
      expect(first.stageDir).not.toBe(second.stageDir);
      expect(statSync(first.stageDir).mode & 0o777).toBe(0o700);
      expect(statSync(first.source).mode & 0o777).toBe(0o600);
      const lines = managedHandoffInstructions("/etc/claude-code/managed-settings.json", first.source, first.stageDir);
      expect(lines.at(-1)).toBe(`  Cleanup: rm -rf "${first.stageDir}"`);
      expect(lines.join("\n")).not.toContain("SYNTHETIC_PRIVATE_VALUE");
    } finally {
      rmSync(first.stageDir, { recursive: true, force: true });
      rmSync(second.stageDir, { recursive: true, force: true });
    }
  });
});
