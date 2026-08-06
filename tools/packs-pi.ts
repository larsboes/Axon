// tools/packs-pi.ts — select Axon and overlay Packs for Pi's settings-managed skill registry.
// Pi scans each path in ~/.pi/agent/settings.json:skills. This tool owns only paths
// recorded in its ledger, so remove never deletes a skill selected outside Axon.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { availablePacks, readPackSkills, type DeployConfig } from "./packs-codex.ts";

const AXON_ROOT = resolve(import.meta.dir, "..");
const home = process.env.HOME ?? "";
const settingsPath = resolve(process.env.PI_SETTINGS_FILE ?? join(home, ".pi", "agent", "settings.json"));
const statePath = resolve(process.env.AXON_PI_STATE_FILE ?? join(process.env.XDG_STATE_HOME ?? join(home, ".local", "state"), "axon", "pack-deployments", "pi.json"));

type State = { version: 1; settingsPath: string; packs: Record<string, string[]> };

function overlayRoot(): string | null {
  if (process.env.AXON_OVERLAY_ROOT) return expandHome(process.env.AXON_OVERLAY_ROOT);
  for (const file of [join(AXON_ROOT, "axon.local.toml"), join(AXON_ROOT, "axon.toml")]) {
    if (!existsSync(file)) continue;
    const overlay = (Bun.TOML.parse(readFileSync(file, "utf8")) as Record<string, unknown>).overlay;
    if (typeof overlay === "string" && overlay) return expandHome(overlay);
  }
  return null;
}

function expandHome(path: string): string {
  return path === "~" ? home : path.startsWith("~/") ? join(home, path.slice(2)) : path;
}

function config(): DeployConfig {
  const roots = [join(AXON_ROOT, "Packs")];
  const overlay = overlayRoot();
  if (overlay && existsSync(join(overlay, "Packs"))) roots.push(join(overlay, "Packs"));
  // destination is unused by Pi, but getStatuses needs a valid configuration.
  return { axonRoot: AXON_ROOT, packRoots: roots, destination: join(home, ".pi", "agent", "skills"), stateFile: statePath, adapter: "pi" };
}

function readSettings(): Record<string, unknown> {
  if (!existsSync(settingsPath)) return {};
  const value = JSON.parse(readFileSync(settingsPath, "utf8"));
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error(`${settingsPath}: expected a JSON object`);
  return value as Record<string, unknown>;
}

function settingSkills(settings: Record<string, unknown>): string[] {
  const skills = settings.skills;
  if (skills === undefined) return [];
  if (!Array.isArray(skills) || skills.some((value) => typeof value !== "string")) throw new Error(`${settingsPath}: skills must be an array of strings`);
  return skills as string[];
}

function writeSettings(settings: Record<string, unknown>): void {
  mkdirSync(dirname(settingsPath), { recursive: true });
  const temporary = `${settingsPath}.tmp-${process.pid}`;
  writeFileSync(temporary, `${JSON.stringify(settings, null, 2)}\n`, { mode: 0o600 });
  renameSync(temporary, settingsPath);
}

function readState(): State {
  if (!existsSync(statePath)) return { version: 1, settingsPath, packs: {} };
  const state = JSON.parse(readFileSync(statePath, "utf8")) as State;
  if (state.version !== 1 || !state.packs || resolve(state.settingsPath) !== settingsPath) throw new Error(`${statePath}: malformed or belongs to another Pi settings file`);
  return state;
}

function writeState(state: State): void {
  mkdirSync(dirname(statePath), { recursive: true });
  const temporary = `${statePath}.tmp-${process.pid}`;
  writeFileSync(temporary, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  renameSync(temporary, statePath);
}

function pathsForPack(pack: string): string[] {
  const cfg = config();
  return readPackSkills(cfg, pack).map((skill) => {
    const matches = (cfg.packRoots ?? []).map((root) => join(root, pack, "skills", skill)).filter(existsSync);
    if (matches.length !== 1) throw new Error(`${pack}/${skill}: source is missing or ambiguous`);
    return matches[0];
  });
}

function status(packs: string[]): void {
  const selected = packs.length ? packs : availablePacks(config(), true);
  const settings = settingSkills(readSettings());
  const state = readState();
  for (const pack of selected) {
    for (const path of pathsForPack(pack)) {
      const selected = settings.includes(path);
      const owned = state.packs[pack]?.includes(path) ?? false;
      console.log(`${pack}/${path.split("/").slice(-1)[0]}: ${selected ? (owned ? "current" : "selected-unmanaged") : "not-deployed"}`);
    }
  }
}

function canonicalPath(path: string): string {
  return resolve(expandHome(path));
}

function deploy(packs: string[]): void {
  if (!packs.length) throw new Error("deploy needs one or more Pack names");
  const settings = readSettings();
  const skills = settingSkills(settings);
  const state = readState();
  for (const pack of packs) {
    const paths = pathsForPack(pack);
    for (const path of paths) {
      const duplicate = skills.findIndex((existing) => canonicalPath(existing) === canonicalPath(path));
      if (duplicate === -1) skills.push(path);
      else if (skills[duplicate] !== path) skills[duplicate] = path;
    }
    state.packs[pack] = paths;
    console.log(`✓ ${pack}: ${paths.length} skill(s) selected for Pi`);
  }
  // Pi treats equivalent absolute and ~/ paths as the same source. Keep one path
  // so its startup discovery does not report a duplicate skill name.
  const deduplicated = skills.filter((path, index) =>
    skills.findIndex((candidate) => canonicalPath(candidate) === canonicalPath(path)) === index,
  );
  settings.skills = deduplicated;
  writeSettings(settings);
  writeState(state);
}

function remove(packs: string[]): void {
  if (!packs.length) throw new Error("remove needs one or more Pack names");
  const settings = readSettings();
  const state = readState();
  const removePaths = new Set<string>();
  for (const pack of packs) {
    const paths = state.packs[pack];
    if (!paths) throw new Error(`${pack}: not owned by Axon Pi deployment`);
    paths.forEach((path) => removePaths.add(path));
    delete state.packs[pack];
    console.log(`✓ ${pack}: removed from Pi selection`);
  }
  settings.skills = settingSkills(settings).filter((path) => !removePaths.has(path));
  writeSettings(settings);
  writeState(state);
}

function usage(): never {
  throw new Error("usage: tools/packs-pi list | status [pack ...] | deploy <pack ...> | sync <pack ...> | remove <pack ...>");
}

try {
  const [command = "list", ...args] = process.argv.slice(2);
  if (command === "list") {
    for (const pack of availablePacks(config(), true)) console.log(pack);
  } else if (command === "status") status(args);
  else if (command === "deploy" || command === "sync") deploy(args);
  else if (command === "remove") remove(args);
  else usage();
} catch (error) {
  console.error(`packs-pi: ${(error as Error).message}`);
  process.exit(1);
}
