// tools/packs-opencode.ts — materialize Axon and overlay Packs into OpenCode's skill root.
//
// This is the OpenCode counterpart to packs-codex: full skills are copied into
// ~/.config/opencode/skills and tracked in a separate Axon ownership ledger.

import {
  availablePacks,
  defaultCodexDeployConfig,
  deployPack,
  getStatuses,
  readState,
  removePack,
  syncPack,
  type DeployConfig,
} from "./packs-codex.ts";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const AXON_ROOT = resolve(import.meta.dir, "..");
const home = process.env.HOME ?? "";

function expandHome(path: string): string {
  return path === "~" ? home : path.startsWith("~/") ? join(home, path.slice(2)) : path;
}

function overlayRoot(): string | null {
  if (process.env.AXON_OVERLAY_ROOT) return expandHome(process.env.AXON_OVERLAY_ROOT);
  for (const file of [join(AXON_ROOT, "axon.local.toml"), join(AXON_ROOT, "axon.toml")]) {
    if (!existsSync(file)) continue;
    const overlay = (Bun.TOML.parse(readFileSync(file, "utf8")) as Record<string, unknown>).overlay;
    if (typeof overlay === "string" && overlay) return expandHome(overlay);
  }
  return null;
}

function config(): DeployConfig {
  const roots = [join(AXON_ROOT, "Packs")];
  const overlay = overlayRoot();
  if (overlay && existsSync(join(overlay, "Packs"))) roots.push(join(overlay, "Packs"));
  const base = defaultCodexDeployConfig();
  const stateHome = process.env.XDG_STATE_HOME ?? join(home, ".local", "state");
  return {
    ...base,
    axonRoot: AXON_ROOT,
    packRoots: roots,
    destination: resolve(process.env.OPENCODE_SKILLS_DIR ?? join(home, ".config", "opencode", "skills")),
    stateFile: resolve(
      process.env.AXON_OPENCODE_PACKS_STATE_FILE ?? join(stateHome, "axon", "pack-deployments", "opencode-packs.json"),
    ),
    adapter: "opencode",
  };
}

function printStatuses(config: DeployConfig, selected?: string): void {
  let previous = "";
  for (const row of getStatuses(config, selected)) {
    if (row.pack !== previous) {
      if (previous) console.log();
      console.log(row.pack);
      previous = row.pack;
    }
    console.log(`  ${row.skill.padEnd(24)} [${row.status}]${row.detail ? ` ${row.detail}` : ""}`);
  }
}

function usage(): never {
  throw new Error([
    "usage: tools/packs-opencode status [<pack>|--all]",
    "       tools/packs-opencode deploy <pack>...",
    "       tools/packs-opencode sync <pack>|--all",
    "       tools/packs-opencode remove <pack>...",
    "       tools/packs-opencode list",
  ].join("\n"));
}

try {
  const [command = "status", ...args] = process.argv.slice(2);
  const deployConfig = config();
  if (command === "list") {
    for (const pack of availablePacks(deployConfig, true)) console.log(pack);
  } else if (command === "status") {
    printStatuses(deployConfig, args[0] && args[0] !== "--all" ? args[0] : undefined);
  } else if (command === "deploy") {
    if (args.length === 0) usage();
    for (const pack of args) for (const line of deployPack(deployConfig, pack)) console.log(line);
  } else if (command === "sync") {
    const target = args[0];
    if (!target) usage();
    for (const pack of target === "--all" ? Object.keys(readState(deployConfig).packs).sort() : [target]) {
      for (const line of syncPack(deployConfig, pack)) console.log(line);
    }
  } else if (command === "remove") {
    if (args.length === 0) usage();
    for (const pack of args) for (const line of removePack(deployConfig, pack)) console.log(line);
  } else usage();
} catch (error) {
  console.error(`packs-opencode: ${(error as Error).message}`);
  process.exit(1);
}
