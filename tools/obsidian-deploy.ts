#!/usr/bin/env bun
/**
 * obsidian-deploy — deploy built Obsidian plugin artifacts into the vault.
 *
 * Copies manifest.json + main.js (+ styles.css) of the ACTIVE plugin set into
 * the vault's .obsidian/plugins/, replacing symlinks with real files so plugins
 * sync to mobile via git. Per-vault settings (data.json) are preserved — rescued
 * from a symlink's target before the link is removed, never overwritten after.
 *
 * Zero hardcoded paths (README.md#one-manifest-per-concern): vault path comes from axon.toml's
 * data_class="vault" state_mount; overlay location from axon.toml `overlay`;
 * source_dir + active set from <overlay>/config/obsidian-plugins.toml.
 *
 * Usage:
 *   bun tools/obsidian-deploy.ts            # deploy + prune, report
 *   bun tools/obsidian-deploy.ts --dry-run  # report only, no writes
 */
import { existsSync, lstatSync, readlinkSync, readdirSync, rmSync, mkdirSync, copyFileSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { homedir } from "node:os";

const DRY = process.argv.includes("--dry-run");
const home = homedir();
const expand = (p: string) => resolve(p.replace(/^~(?=\/|$)/, home));
const repoRoot = resolve(dirname(Bun.main), "..");

const axonToml = Bun.TOML.parse(await Bun.file(join(repoRoot, "axon.toml")).text()) as any;
const overlay = expand(axonToml.platform?.overlay ?? axonToml.overlay);
const vaultMount = (axonToml.state_mount ?? []).find((m: any) => m.data_class === "vault");
if (!vaultMount) throw new Error("axon.toml: no state_mount with data_class=\"vault\"");
const vault = expand(vaultMount.path);

const cfgPath = join(overlay, "config", "obsidian-plugins.toml");
if (!existsSync(cfgPath)) throw new Error(`missing overlay config: ${cfgPath}`);
const cfg = Bun.TOML.parse(await Bun.file(cfgPath).text()) as any;
const sourceDir = expand(cfg.source_dir);
const active: string[] = cfg.active ?? [];
if (active.length === 0) throw new Error("obsidian-plugins.toml: `active` list is empty");

const pluginsDir = join(vault, ".obsidian", "plugins");
const ARTIFACTS = ["manifest.json", "main.js", "styles.css"]; // styles.css optional

let deployed = 0, desktopOnly: string[] = [], failed: string[] = [], rescued: string[] = [];

for (const id of active) {
  const src = join(sourceDir, id);
  const dest = join(pluginsDir, id);
  if (!existsSync(join(src, "manifest.json")) || !existsSync(join(src, "main.js"))) {
    failed.push(id);
    console.error(`✗ ${id}: missing manifest.json/main.js under ${src}`);
    continue;
  }
  const manifest = JSON.parse(await Bun.file(join(src, "manifest.json")).text());
  if (manifest.isDesktopOnly) desktopOnly.push(id);

  // Rescue settings if dest is a symlink (settings lived through the link).
  let rescuedData: string | null = null;
  if (existsSync(dest) && lstatSync(dest).isSymbolicLink()) {
    const target = resolve(dirname(dest), readlinkSync(dest));
    const d = join(target, "data.json");
    if (existsSync(d)) { rescuedData = await Bun.file(d).text(); rescued.push(id); }
    if (!DRY) rmSync(dest);
  }
  if (!DRY) {
    mkdirSync(dest, { recursive: true });
    for (const a of ARTIFACTS) {
      const f = join(src, a);
      if (existsSync(f)) copyFileSync(f, join(dest, a));
    }
    const destData = join(dest, "data.json");
    if (rescuedData !== null && !existsSync(destData)) await Bun.write(destData, rescuedData);
  }
  deployed++;
  console.log(`✓ ${id}${manifest.isDesktopOnly ? " (desktop-only)" : ""}${rescuedData ? " [settings rescued]" : ""}`);
}

// Prune: anything in the vault plugins dir not in the active set.
let pruned: string[] = [];
if (existsSync(pluginsDir)) {
  for (const entry of readdirSync(pluginsDir)) {
    if (!active.includes(entry)) {
      pruned.push(entry);
      if (!DRY) rmSync(join(pluginsDir, entry), { recursive: true });
    }
  }
}

// Enable-state check: community-plugins.json is owned by Obsidian's UI, but an
// enabled id with no deployed dir produces "Failed to load" on every startup.
const cpPath = join(vault, ".obsidian", "community-plugins.json");
if (existsSync(cpPath)) {
  const enabled: string[] = JSON.parse(await Bun.file(cpPath).text());
  const orphans = enabled.filter((id) => !active.includes(id));
  if (orphans.length) console.warn(`⚠ enabled but not deployed (will fail to load): ${orphans.join(", ")}`);
}

console.log(`\n${DRY ? "[dry-run] " : ""}deployed ${deployed}/${active.length} · rescued settings ${rescued.length} · pruned ${pruned.length}${pruned.length ? ` (${pruned.join(", ")})` : ""}`);
if (desktopOnly.length) console.log(`desktop-only (won't enable on mobile): ${desktopOnly.join(", ")}`);
if (failed.length) { console.error(`FAILED: ${failed.join(", ")}`); process.exit(1); }
