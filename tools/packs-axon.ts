// tools/packs-axon.ts — materialize the Axon Pack across agent harnesses.
//
// The Axon Pack is Axon-specific doctrine; this harness deployer keeps it
// materialized and drift-checked without symlinks.
//
// Commands:
//   tools/packs-axon status [all|claude|codex|opencode] [--json]
//   tools/packs-axon deploy [all|claude|codex|opencode]
//   tools/packs-axon sync   [all|claude|codex|opencode]
//   tools/packs-axon remove [all|claude|codex|opencode]

import {
  deployPack,
  getStatuses,
  readPackSkills,
  removePack,
  syncPack,
  type DeployConfig,
  defaultCodexDeployConfig,
} from "./packs-codex.ts";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const AXON_PACK = "axon";

export type AxonHarness = "claude" | "codex" | "opencode";

type HarnessDefinition = {
  harness: AxonHarness;
  destination: string;
  stateFile: string;
  adapter: string;
};

export type HarnessStatus = {
  harness: AxonHarness;
  destination: string;
  rows: Array<{
    pack: string;
    skill: string;
    status: string;
    detail?: string;
  }>;
};

function expandHome(path: string): string {
  const home = process.env.HOME ?? "";
  return path.startsWith("~") ? `${home}${path.slice(1)}` : path;
}

function stateHome(): string {
  return process.env.XDG_STATE_HOME || resolve(process.env.HOME ?? "", ".local", "state");
}

function stateFileFor(harness: AxonHarness): string {
  const env = {
    codex: process.env.AXON_CODEX_STATE_FILE,
    claude: process.env.AXON_CLAUDE_STATE_FILE,
    opencode: process.env.AXON_OPENCODE_STATE_FILE,
  }[harness];
  return resolve(env ?? join(stateHome(), "axon", "pack-deployments", `${harness}.json`));
}

function unique<T>(values: T[]): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const value of values) {
    const key = `${value}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(value);
  }
  return out;
}

function assertHarness(value: string): asserts value is AxonHarness {
  if (value !== "claude" && value !== "codex" && value !== "opencode") {
    throw new Error(`invalid harness '${value}'`);
  }
}

function resolveOpencodeDestination(): string {
  const candidates = unique(
    [
      process.env.OPENCODE_SKILLS_DIR ? expandHome(process.env.OPENCODE_SKILLS_DIR) : undefined,
      expandHome(process.env.OPENCODE_CONFIG_DIR || join(process.env.HOME ?? "", ".config", "opencode")),
      join(process.env.HOME ?? "", ".opencode", "skills"),
    ].filter(Boolean) as string[],
  );

  const existingRoots = candidates.filter((dir) => existsSync(dir));
  const existingWithAxon = existingRoots.filter((dir) => existsSync(join(dir, "axon")));

  if (existingWithAxon.length > 1) {
    throw new Error(
      `opencode: multiple compatible skill roots already contain 'axon': ${existingWithAxon.join(", ")}. ` +
        "set OPENCODE_SKILLS_DIR to the target root",
    );
  }

  if (existingWithAxon.length === 1) {
    return existingWithAxon[0];
  }

  if (existingRoots.length > 1) {
    throw new Error(
      `opencode: multiple compatible skill roots discovered: ${existingRoots.join(", ")}. ` +
        "set OPENCODE_SKILLS_DIR before deploy/sync/remove",
    );
  }

  if (existingRoots.length === 1) {
    return existingRoots[0];
  }

  return expandHome(process.env.OPENCODE_CONFIG_DIR || join(process.env.HOME ?? "", ".config", "opencode", "skills"));
}

function resolveBaseConfig(): DeployConfig {
  return defaultCodexDeployConfig();
}

function discoverHarnesses(): HarnessDefinition[] {
  const claude = process.env.CLAUDE_SKILLS_DIR
    ? resolve(expandHome(process.env.CLAUDE_SKILLS_DIR))
    : resolve(join(process.env.HOME ?? "", ".claude", "skills"));
  const codex = process.env.CODEX_SKILLS_DIR
    ? resolve(expandHome(process.env.CODEX_SKILLS_DIR))
    : resolve(join(process.env.HOME ?? "", ".agents", "skills"));

  return [
    { harness: "claude", destination: claude, stateFile: stateFileFor("claude"), adapter: "claude" },
    { harness: "codex", destination: codex, stateFile: stateFileFor("codex"), adapter: "codex" },
    { harness: "opencode", destination: resolveOpencodeDestination(), stateFile: stateFileFor("opencode"), adapter: "opencode" },
  ];
}

function resolveHarnessConfig(config: DeployConfig, harness: AxonHarness): DeployConfig {
  const definition = discoverHarnesses().find((item) => item.harness === harness);
  if (!definition) throw new Error(`no such harness '${harness}'`);
  return {
    ...config,
    destination: definition.destination,
    stateFile: definition.stateFile,
    adapter: definition.adapter,
  };
}

function parseTargets(args: string[]): { targets: AxonHarness[]; asJson: boolean } {
  const targets: AxonHarness[] = [];
  let asJson = false;
  for (const arg of args) {
    if (arg === "--json") {
      asJson = true;
      continue;
    }
    if (arg === "all") continue;
    assertHarness(arg);
    targets.push(arg);
  }
  if (targets.length === 0) targets.push("codex", "claude", "opencode");
  return { targets: Array.from(new Set(targets)), asJson };
}

export function readAxonHarnessStatuses(config = resolveBaseConfig()): HarnessStatus[] {
  const manifest = readPackSkills(config, AXON_PACK);
  if (!manifest.includes(AXON_PACK)) {
    throw new Error(`pack manifest missing skill '${AXON_PACK}'`);
  }

  return discoverHarnesses().map((definition) => {
    const harnessConfig = resolveHarnessConfig(config, definition.harness);
    const rows = getStatuses(harnessConfig, AXON_PACK).map((row) => ({
      pack: row.pack,
      skill: row.skill,
      status: row.status,
      detail: row.detail,
    }));

    return {
      harness: definition.harness,
      destination: definition.destination,
      rows,
    };
  });
}

function printStatus(statuses: HarnessStatus[], asJson: boolean): void {
  if (asJson) {
    const output = statuses.map((entry) => ({
      harness: entry.harness,
      destination: entry.destination,
      rows: entry.rows,
    }));
    process.stdout.write(`${JSON.stringify(output)}\n`);
    return;
  }

  for (const status of statuses) {
    console.log(`${status.harness}:`);
    for (const row of status.rows) {
      const detail = row.detail ? ` — ${row.detail}` : "";
      console.log(`  ${row.pack}/${row.skill}: ${row.status}${detail}`);
    }
  }
}

function runPerHarness(config: DeployConfig, targets: AxonHarness[], mode: "deploy" | "sync" | "remove"): string[] {
  const messages: string[] = [];
  for (const harness of targets) {
    const harnessConfig = resolveHarnessConfig(config, harness);
    try {
      const lines =
        mode === "deploy"
          ? deployPack(harnessConfig, AXON_PACK)
          : mode === "sync"
            ? syncPack(harnessConfig, AXON_PACK)
            : removePack(harnessConfig, AXON_PACK);
      for (const line of lines) messages.push(`[${harness}] ${line}`);
    } catch (error) {
      messages.push(`[${harness}] ${String((error as Error).message)}`);
    }
  }
  return messages;
}

function usage(): never {
  throw new Error([
    "usage: tools/packs-axon status [all|claude|codex|opencode] [--json]",
    "       tools/packs-axon deploy [all|claude|codex|opencode]",
    "       tools/packs-axon sync [all|claude|codex|opencode]",
    "       tools/packs-axon remove [all|claude|codex|opencode]",
  ].join("\n"));
}

async function main(): Promise<void> {
  const [command = "status", ...args] = process.argv.slice(2);
  const base = resolveBaseConfig();
  try {
    switch (command) {
      case "status": {
        const { targets, asJson } = parseTargets(args);
        const statuses = readAxonHarnessStatuses(base).filter((status) => targets.includes(status.harness));
        printStatus(statuses, asJson);
        break;
      }
      case "deploy": {
        const { targets } = parseTargets(args);
        for (const line of runPerHarness(base, targets, "deploy")) console.log(line);
        break;
      }
      case "sync": {
        const { targets } = parseTargets(args);
        for (const line of runPerHarness(base, targets, "sync")) console.log(line);
        break;
      }
      case "remove": {
        const { targets } = parseTargets(args);
        for (const line of runPerHarness(base, targets, "remove")) console.log(line);
        break;
      }
      default:
        usage();
    }
  } catch (error) {
    console.error(`packs-axon: ${(error as Error).message}`);
    process.exit(1);
  }
}

if (import.meta.main) {
  await main();
}
