#!/usr/bin/env bun
/**
 * lifeos-sync — owns Axon's delta over a stock LifeOS install.
 *
 * LifeOS lives at ~/.claude and is installed, not cloned: no git, no upstream
 * remote. Its installer is strictly non-destructive (DeployCore/DeployComponents
 * use copyMissing, InstallSettings adds only ABSENT keys), so the risk this tool
 * addresses is NOT an update clobbering local edits — it is the opposite:
 * divergence. Edits made directly in ~/.claude are invisible, unversioned, and
 * gone on a reinstall or a second machine. So Axon owns the delta, and this
 * tool projects it back down.
 *
 * Deliberately only the SYSTEM-zone delta. The USER tree (~/.config/LIFEOS/USER)
 * already has tools/lifeos-user-sync.sh and survives updates on its own — see
 * LifeOS' own DOCUMENTATION/SystemUserBoundary.md for the zone model.
 *
 * Two mechanisms, picked per target:
 *   link   — the file has no relative imports and nothing else writes it, so a
 *            symlink makes drift structurally impossible and the installer's
 *            existsSync guard skips it forever.
 *   hooks  — settings.json is co-owned (Claude Code writes permissions into it),
 *            so it cannot be a symlink. The hook-wiring delta is merged in,
 *            matched on command string, making a re-run a no-op.
 *
 * Bazel: deliberately out. No dependency graph to buy anything here, and it
 * writes into $HOME rather than a build output — sandboxing would be pure cost
 * (README.md#argue-bazel-per-case — same call as tools/doctor, the worked case that rule cites).
 *
 * Usage:
 *   tools/lifeos-sync [status]        # what is deployed, what drifted (default)
 *   tools/lifeos-sync deploy          # apply; idempotent, backup before write
 *   tools/lifeos-sync deploy --dry-run
 *
 * Exit 0 = in sync / applied, 1 = drift or partial failure, 2 = usage/setup error.
 */

import { existsSync, lstatSync, readlinkSync, readFileSync, writeFileSync, unlinkSync, symlinkSync, mkdirSync, copyFileSync } from "node:fs";
import { join, dirname, relative } from "node:path";

const HOME = process.env.HOME ?? "";
const AXON_ROOT = process.env.AXON_ROOT ?? "";
const OVERLAY_ROOT = process.env.AXON_PERSONAL_ROOT ?? "";
// LifeOS' config root. CLAUDE_CONFIG_DIR is the harness' own override; ~/.claude
// is the documented default. Never hardcoded anywhere else in this file.
const CONFIG_ROOT = process.env.CLAUDE_CONFIG_DIR ?? join(HOME, ".claude");

if (!AXON_ROOT || !OVERLAY_ROOT) {
  console.error("lifeos-sync: AXON_ROOT/AXON_PERSONAL_ROOT unset — run via tools/lifeos-sync, not directly");
  process.exit(2);
}

type Target = {
  what: string;
  kind: "link" | "hooks";
  src: string;
  dst: string;
  why: string;
};

// The managed set. Small on purpose: every entry here is a place Axon knowingly
// differs from stock LifeOS, and each one carries the reason it exists.
const TARGETS: Target[] = [
  {
    what: "ProseGate.hook.ts",
    kind: "link",
    src: join(AXON_ROOT, "capabilities/lifeos/overlay/hooks/ProseGate.hook.ts"),
    dst: join(CONFIG_ROOT, "hooks/ProseGate.hook.ts"),
    why: "Axon-authored hook, not shipped by LifeOS — a reinstall would drop it. No relative imports, so a symlink is safe.",
  },
  {
    what: "PULSE.toml",
    kind: "link",
    src: join(OVERLAY_ROOT, "config/lifeos/PULSE.toml"),
    dst: join(CONFIG_ROOT, "LIFEOS/PULSE/PULSE.toml"),
    why: "Instance config (which sinks and modules are on for THIS machine) — overlay, not Axon. Pulse reads only PULSE.toml for module sections; its PULSE.user.toml overlay merges [[job]] entries only.",
  },
  {
    what: "LIFEOS_SYSTEM_PROMPT.md",
    kind: "link",
    src: join(OVERLAY_ROOT, "config/lifeos/LIFEOS_SYSTEM_PROMPT.md"),
    dst: join(CONFIG_ROOT, "LIFEOS/LIFEOS_SYSTEM_PROMPT.md"),
    why: "Stock LifeOS ships this file describing an install whose ~/.claude is a private git repo with a remote. This one is not, by design, so the doctrine had to be corrected — and a correction living only in ~/.claude is unversioned. Overlay, not Axon: the edits are instance decisions about an upstream-owned file, the same shape as PULSE.toml. Loaded via --append-system-prompt-file and it has no relative imports, so a symlink is safe.",
  },
  {
    what: "context-budgets.json",
    kind: "link",
    src: join(OVERLAY_ROOT, "config/lifeos/context-budgets.json"),
    dst: join(CONFIG_ROOT, "LIFEOS/TOOLS/context-budgets.json"),
    why: "Which files are always-on and what each may cost. Instance-tuned (the caps are sized to THIS principal's files), and dropping a row is a real decision — BudgetCheck.ts reads it, nothing writes it.",
  },
  {
    what: "settings.json hooks",
    kind: "hooks",
    src: join(AXON_ROOT, "capabilities/lifeos/overlay/settings.hooks.json"),
    dst: join(CONFIG_ROOT, "settings.json"),
    why: "Hook wiring LifeOS ships files for but never registers. Merged, not linked — Claude Code co-owns settings.json.",
  },
];

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run") || args.includes("-n");
const cmd = args.find((a) => !a.startsWith("-")) ?? "status";
if (cmd !== "status" && cmd !== "deploy") {
  console.error(`lifeos-sync: unknown command '${cmd}' — expected 'status' or 'deploy'`);
  process.exit(2);
}

// ── link targets ──────────────────────────────────────────────────────────────

type LinkState = "linked" | "absent" | "foreign" | "no-source";

function linkState(t: Target): LinkState {
  if (!existsSync(t.src)) return "no-source";
  if (!existsSync(t.dst) && !isSymlink(t.dst)) return "absent";
  if (isSymlink(t.dst)) {
    // resolve relative link targets against the link's own directory
    const raw = readlinkSync(t.dst);
    const resolved = raw.startsWith("/") ? raw : join(dirname(t.dst), raw);
    return resolved === t.src ? "linked" : "foreign";
  }
  return "foreign"; // a real file sits there — stock LifeOS content or a hand edit
}

// lstat, not existsSync: a symlink pointing at a missing file must still read as
// a symlink, otherwise a broken link looks like "absent" and gets silently replaced.
function isSymlink(p: string): boolean {
  try { return lstatSync(p).isSymbolicLink(); } catch { return false; }
}

function deployLink(t: Target): boolean {
  const state = linkState(t);
  if (state === "linked") { report("ok", t.what, "already linked"); return true; }
  if (state === "no-source") { report("fail", t.what, `source missing: ${rel(t.src)}`); return false; }

  if (dryRun) {
    report("would", t.what, state === "foreign" ? `back up real file, then link → ${rel(t.src)}` : `link → ${rel(t.src)}`);
    return true;
  }

  try {
    if (state === "foreign") {
      // Never delete content we did not write without keeping a copy: the file
      // sitting there is either stock LifeOS or a hand edit, and both matter.
      const backup = `${t.dst}.axon-backup-${stamp()}`;
      copyFileSync(t.dst, backup);
      unlinkSync(t.dst);
      report("ok", t.what, `backed up → ${rel(backup)}`);
    }
    mkdirSync(dirname(t.dst), { recursive: true });
    symlinkSync(t.src, t.dst);
    report("ok", t.what, `linked → ${rel(t.src)}`);
    return true;
  } catch (err) {
    report("fail", t.what, String(err));
    return false;
  }
}

// ── hook-merge target ─────────────────────────────────────────────────────────

type HookEntry = { type?: string; command?: string; url?: string; timeout?: number; async?: boolean };
type HookBlock = { matcher?: string; hooks?: HookEntry[]; [k: string]: unknown };

/** Documentation keys in the fragment. They explain the delta; settings.json must not carry them. */
function stripDocKeys(block: HookBlock): HookBlock {
  const out: HookBlock = {};
  for (const [k, v] of Object.entries(block)) if (!k.startsWith("_")) out[k] = v;
  return out;
}

/** Identity of a block = its matcher plus the command/url of every entry, in order. */
function blockKey(block: HookBlock): string {
  const cmds = (block.hooks ?? []).map((h) => h.command ?? h.url ?? "?").join("|");
  return `${block.matcher ?? "*"}::${cmds}`;
}

/**
 * Additive merge, pure so the decision is inspectable without touching disk.
 * Returns the merged hooks object and what was added. A block already present
 * (same matcher + same commands) is left exactly as-is — this is what makes a
 * second `deploy` a no-op even if the user retimed a timeout by hand.
 */
export function mergeHooks(
  live: Record<string, HookBlock[]>,
  fragment: Record<string, HookBlock[]>,
): { merged: Record<string, HookBlock[]>; added: string[] } {
  const merged: Record<string, HookBlock[]> = { ...live };
  const added: string[] = [];

  for (const [event, blocks] of Object.entries(fragment)) {
    const existing = merged[event] ? [...merged[event]] : [];
    const seen = new Set(existing.map(blockKey));
    for (const rawBlock of blocks) {
      const block = stripDocKeys(rawBlock);
      const key = blockKey(block);
      if (seen.has(key)) continue;
      existing.push(block);
      seen.add(key);
      for (const h of block.hooks ?? []) added.push(`${event}: ${basename(h.command ?? h.url ?? "?")}`);
    }
    merged[event] = existing;
  }
  return { merged, added };
}

function deployHooks(t: Target): boolean {
  if (!existsSync(t.src)) { report("fail", t.what, `source missing: ${rel(t.src)}`); return false; }
  if (!existsSync(t.dst)) { report("fail", t.what, `target missing: ${rel(t.dst)}`); return false; }

  let live: { hooks?: Record<string, HookBlock[]>; [k: string]: unknown };
  let fragment: { hooks?: Record<string, HookBlock[]> };
  try {
    live = JSON.parse(readFileSync(t.dst, "utf8"));
    // The fragment writes <configRoot> where the harness root belongs. Resolve it the
    // same way CONFIG_ROOT is resolved -- except that on a default install the token
    // becomes the literal "$HOME/.claude" (shell-expanded at hook runtime), so already-
    // deployed blocks keep matching byte-for-byte and a re-run stays a no-op.
    const configRootToken = process.env.CLAUDE_CONFIG_DIR ?? "$HOME/.claude";
    fragment = JSON.parse(readFileSync(t.src, "utf8").replaceAll("<configRoot>", configRootToken));
  } catch (err) {
    // Abort rather than write: a settings.json we cannot parse is one we must not rewrite.
    report("fail", t.what, `parse failed, refusing to write: ${err}`);
    return false;
  }

  const { merged, added } = mergeHooks(live.hooks ?? {}, fragment.hooks ?? {});
  if (added.length === 0) { report("ok", t.what, "all wired"); return true; }

  if (dryRun || cmd === "status") {
    report(cmd === "status" ? "drift" : "would", t.what, `${added.length} missing: ${added.join(", ")}`);
    return cmd !== "status";
  }

  try {
    const backup = `${t.dst}.axon-backup-${stamp()}`;
    copyFileSync(t.dst, backup);
    live.hooks = merged;
    // tmp + rename: settings.json is read on every session start; a partial
    // write would break the harness, a rename is atomic.
    const tmp = `${t.dst}.axon-tmp-${process.pid}`;
    writeFileSync(tmp, JSON.stringify(live, null, 2) + "\n");
    Bun.spawnSync({ cmd: ["mv", tmp, t.dst] });
    report("ok", t.what, `wired ${added.length} (${added.join(", ")}) · backup ${rel(backup)}`);
    return true;
  } catch (err) {
    report("fail", t.what, String(err));
    return false;
  }
}

// ── reporting ─────────────────────────────────────────────────────────────────

const ICON: Record<string, string> = { ok: "✅", drift: "⚠️ ", fail: "❌", would: "→ " };
let failed = false;
let drifted = false;

function report(kind: keyof typeof ICON, what: string, detail: string): void {
  if (kind === "fail") failed = true;
  if (kind === "drift") drifted = true;
  console.log(`  ${ICON[kind]} ${what.padEnd(20)} ${detail}`);
}

function basename(p: string): string { return p.split("/").pop() ?? p; }
function stamp(): string { return new Date().toISOString().replace(/[:.]/g, "-"); }

/** Paths print relative to $HOME so the output stays readable and copy-pasteable. */
function rel(p: string): string {
  if (p.startsWith(AXON_ROOT)) return `Axon/${relative(AXON_ROOT, p)}`;
  if (p.startsWith(OVERLAY_ROOT)) return `${basename(OVERLAY_ROOT)}/${relative(OVERLAY_ROOT, p)}`;
  return p.startsWith(HOME) ? `~${p.slice(HOME.length)}` : p;
}

// ── main ──────────────────────────────────────────────────────────────────────

console.log(`lifeos-sync · ${cmd}${dryRun ? " (dry-run)" : ""}`);
console.log(`  config root: ${rel(CONFIG_ROOT)}\n`);

for (const t of TARGETS) {
  if (t.kind === "link") {
    const state = linkState(t);
    if (cmd === "status") {
      if (state === "linked") report("ok", t.what, `linked → ${rel(t.src)}`);
      else if (state === "no-source") report("fail", t.what, `source missing: ${rel(t.src)}`);
      else if (state === "absent") report("drift", t.what, `not deployed — run: tools/lifeos-sync deploy`);
      else report("drift", t.what, `real file at ${rel(t.dst)}, not our link — deploy backs it up first`);
    } else {
      deployLink(t);
    }
  } else {
    deployHooks(t);
  }
}

console.log();
if (failed) { console.log("── failures above — nothing further applied ──"); process.exit(1); }
if (drifted) { console.log("── drift found · tools/lifeos-sync deploy ──"); process.exit(1); }
console.log("── in sync ──");
process.exit(0);
