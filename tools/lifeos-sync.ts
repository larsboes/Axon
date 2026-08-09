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
 *   copy   — the overlay's content is written into the config root as a real
 *            file. `~/.claude` is a deployment target, not a set of pointers
 *            into two git repos: nothing under it can write into a checkout by
 *            accident, and a LifeOS reinstall skips it the same way it skipped
 *            a symlink, because copyMissing only fills absent paths.
 *   hooks  — settings.json is co-owned (Claude Code writes permissions into it),
 *            so it cannot be replaced wholesale. The hook-wiring delta is merged
 *            in, matched on command string, making a re-run a no-op.
 *
 * Copies replaced symlinks on 2026-08-09 (principal directive: "we should only
 * deploy from axon overlays never using symlinks"). A symlink made drift
 * structurally impossible, and that guarantee is not free to give up — so it is
 * replaced in the same change by a CONTENT comparison. `status` hashes both
 * sides; a hand edit in the live tree is reported as drift and exits 1, where
 * the old link-identity check could not have seen it at all.
 *
 * `writeback` is the other half. A linked target that something writes to wrote
 * straight into the overlay; a copied one does not, so generated state would be
 * stranded in an untracked tree and lost on the next deploy. Targets that are
 * written to declare `writeback = true` and deploy pulls newer live content into
 * the overlay BEFORE writing back down. It is opt-in and defaults false on
 * purpose: for a read-only target, a difference in the live tree is drift to be
 * overwritten, and blindly pulling it back would let a LifeOS reinstall that
 * clobbered a file launder stock content into the overlay as if it were ours.
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

import { existsSync, lstatSync, statSync, readdirSync, readFileSync, writeFileSync, unlinkSync, rmSync, mkdirSync, copyFileSync, utimesSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname, relative } from "node:path";

const HOME = process.env.HOME ?? "";
const AXON_ROOT = process.env.AXON_ROOT ?? "";
const OVERLAY_ROOT = process.env.AXON_PERSONAL_ROOT ?? "";
// LifeOS' config root. CLAUDE_CONFIG_DIR is the harness' own override; ~/.claude
// is the documented default. Never hardcoded anywhere else in this file.
const CONFIG_ROOT = process.env.CLAUDE_CONFIG_DIR ?? join(HOME, ".claude");

type Target = {
  what: string;
  kind: "copy" | "hooks";
  src: string;
  dst: string;
  why: string;
  /** Pull newer live content into the overlay before deploying over it. Only for targets something writes to. */
  writeback: boolean;
};

// What Axon itself owns: the two targets Axon AUTHORS. Which files a given
// install additionally differs in is not Axon's business — that is an instance
// fact and it lives in the overlay's config/lifeos/overlay.toml, read below.
// Public core owns the mechanism; the overlay owns the list.
const AXON_TARGETS: Target[] = [
  {
    what: "ProseGate.hook.ts",
    kind: "copy",
    src: join(AXON_ROOT, "capabilities/lifeos/overlay/hooks/ProseGate.hook.ts"),
    dst: join(CONFIG_ROOT, "hooks/ProseGate.hook.ts"),
    why: "Axon-authored hook, not shipped by LifeOS — a reinstall would drop it. No relative imports, so it stands alone as a copy.",
    writeback: false,
  },
  {
    what: "settings.json hooks",
    kind: "hooks",
    src: join(AXON_ROOT, "capabilities/lifeos/overlay/settings.hooks.json"),
    dst: join(CONFIG_ROOT, "settings.json"),
    why: "Hook wiring LifeOS ships files for but never registers. Merged, not copied — Claude Code co-owns settings.json.",
    writeback: false,
  },
];

const OVERLAY_MANIFEST = join(OVERLAY_ROOT, "config/lifeos/overlay.toml");

/**
 * The overlay's declared files, as sync targets. Pure so the parsing is testable
 * without an overlay on disk.
 *
 * Every field is required and a bad entry throws rather than being skipped: a
 * silently dropped target is a file that stops being synced while the report
 * still says "in sync", which is the one failure this tool must not have. `kind`
 * is not read from the manifest at all — only copies are declarable, because the
 * merge path is co-owned with Claude Code and belongs with the code that
 * implements it.
 *
 * `writeback` is the one optional field. Absent means false, and a non-boolean
 * throws rather than being coerced — `writeback = "yes"` silently truthy would
 * turn a read-only target into one that laundres live content into the overlay.
 */
export function parseOverlayTargets(
  toml: any,
  overlayRoot: string,
  configRoot: string,
): Target[] {
  const files = toml?.file ?? [];
  if (!Array.isArray(files)) throw new Error("overlay.toml: [[file]] must be an array of tables");
  return files.map((f: any, i: number) => {
    for (const key of ["what", "src", "dst", "why"]) {
      if (typeof f?.[key] !== "string" || !f[key].trim()) {
        throw new Error(`overlay.toml: [[file]] #${i + 1} is missing '${key}'`);
      }
    }
    if (f.writeback !== undefined && typeof f.writeback !== "boolean") {
      throw new Error(`overlay.toml: [[file]] #${i + 1} has a non-boolean 'writeback'`);
    }
    return {
      what: f.what,
      kind: "copy" as const,
      src: join(overlayRoot, f.src),
      dst: join(configRoot, f.dst),
      why: f.why,
      writeback: f.writeback === true,
    };
  });
}

/**
 * Axon's own targets plus whatever the overlay declares. An overlay without a
 * manifest is legal — it just contributes nothing.
 *
 * Order: Axon's hook, the overlay's files, then the settings merge last, so a
 * run that ends by touching settings.json reports that last.
 */
function resolveTargets(): Target[] {
  if (!existsSync(OVERLAY_MANIFEST)) return AXON_TARGETS;
  const declared = parseOverlayTargets(
    Bun.TOML.parse(readFileSync(OVERLAY_MANIFEST, "utf8")),
    OVERLAY_ROOT,
    CONFIG_ROOT,
  );
  return [AXON_TARGETS[0], ...declared, AXON_TARGETS[1]];
}

// Set by main(). Module-level because deployLink/deployHooks read them, and
// threading two flags through every call site buys nothing.
let dryRun = false;
let cmd = "status";

// ── copy targets ──────────────────────────────────────────────────────────────

type CopyState = "synced" | "absent" | "drifted" | "no-source";

/** One entry per file. A file target is a one-entry tree keyed "" so files and directories share every code path below. */
type Tree = Map<string, { hash: string; mtimeMs: number }>;

function hashOf(p: string): string {
  return createHash("sha256").update(readFileSync(p)).digest("hex");
}

/**
 * Content fingerprint of a path. Missing → null, so callers distinguish "absent"
 * from "empty directory" — the two look identical through a bare file count and
 * only one of them is a reason to deploy.
 */
function treeOf(root: string): Tree | null {
  if (!existsSync(root)) return null;
  const tree: Tree = new Map();
  const walk = (abs: string, relPath: string): void => {
    const st = statSync(abs);
    if (st.isDirectory()) {
      for (const entry of readdirSync(abs).sort()) walk(join(abs, entry), relPath ? join(relPath, entry) : entry);
    } else if (st.isFile()) {
      tree.set(relPath, { hash: hashOf(abs), mtimeMs: st.mtimeMs });
    }
  };
  walk(root, "");
  return tree;
}

/**
 * What deploy would pull from the live tree into the overlay, for a `writeback`
 * target. Pure so the decision is inspectable without touching disk.
 *
 * A path qualifies when the live side is BOTH different and newer, or exists
 * only live. Newer alone is not enough — a touch is not an edit — and different
 * alone would let a stale live copy overwrite a deliberate overlay change.
 */
export function writebackPlan(src: Tree, dst: Tree): string[] {
  const plan: string[] = [];
  for (const [path, live] of dst) {
    const overlay = src.get(path);
    if (!overlay) { plan.push(path); continue; }
    if (overlay.hash !== live.hash && live.mtimeMs > overlay.mtimeMs) plan.push(path);
  }
  return plan.sort();
}

/** Identical content on both sides. Timestamps deliberately ignored: a copy is in sync when its bytes are. */
function treesMatch(a: Tree, b: Tree): boolean {
  if (a.size !== b.size) return false;
  for (const [path, entry] of a) if (b.get(path)?.hash !== entry.hash) return false;
  return true;
}

function copyState(t: Target): CopyState {
  const src = treeOf(t.src);
  if (!src) return "no-source";
  // lstat, not existsSync: a dangling symlink left by the pre-2026-08-09 link
  // mechanism must read as present-and-wrong, not absent, or deploy would treat
  // replacing it as a fresh install and skip the backup.
  const dstPresent = existsSync(t.dst) || isSymlink(t.dst);
  if (!dstPresent) return "absent";
  if (isSymlink(t.dst)) return "drifted"; // a legacy link is drift by definition now
  const dst = treeOf(t.dst);
  return dst && treesMatch(src, dst) ? "synced" : "drifted";
}

function isSymlink(p: string): boolean {
  try { return lstatSync(p).isSymbolicLink(); } catch { return false; }
}

/** Recursive copy that preserves mtimes, so a deployed file does not read as newer than its overlay source and trigger a phantom writeback next run. */
function copyTree(src: string, dst: string): void {
  const st = statSync(src);
  if (st.isDirectory()) {
    mkdirSync(dst, { recursive: true });
    for (const entry of readdirSync(src)) copyTree(join(src, entry), join(dst, entry));
    return;
  }
  mkdirSync(dirname(dst), { recursive: true });
  copyFileSync(src, dst);
  utimesSync(dst, st.atime, st.mtime);
}

function pullBack(t: Target): string[] {
  const src = treeOf(t.src);
  const dst = treeOf(t.dst);
  if (!src || !dst) return [];
  const plan = writebackPlan(src, dst);
  if (dryRun || cmd === "status") return plan;
  for (const path of plan) copyTree(join(t.dst, path), join(t.src, path));
  return plan;
}

function deployCopy(t: Target): boolean {
  const state = copyState(t);
  if (t.writeback && (state === "drifted" || state === "synced")) {
    const pulled = pullBack(t);
    if (pulled.length) {
      // A file target is one entry keyed "" — print the target's name there, or the row reads "pulled back 1: " and names nothing.
      const named = pulled.map((p) => p || basename(t.dst));
      report(dryRun ? "would" : "ok", t.what, `${dryRun ? "pull back" : "pulled back"} ${pulled.length} newer from live: ${named.slice(0, 3).join(", ")}${named.length > 3 ? ", …" : ""}`);
    }
  }
  if (copyState(t) === "synced") { report("ok", t.what, "content matches"); return true; }
  if (state === "no-source") { report("fail", t.what, `source missing: ${rel(t.src)}`); return false; }

  if (dryRun) {
    report("would", t.what, state === "drifted" ? `back up live copy, then write ← ${rel(t.src)}` : `write ← ${rel(t.src)}`);
    return true;
  }

  try {
    if (state === "drifted") {
      // Never delete content we did not write without keeping a copy: what sits
      // there is stock LifeOS, a hand edit, or a legacy link, and all three matter.
      const backup = `${t.dst}.axon-backup-${stamp()}`;
      if (isSymlink(t.dst)) unlinkSync(t.dst);
      else { copyTree(t.dst, backup); rmSync(t.dst, { recursive: true, force: true }); report("ok", t.what, `backed up → ${rel(backup)}`); }
    }
    copyTree(t.src, t.dst);
    report("ok", t.what, `written ← ${rel(t.src)}`);
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
//
// Guarded by import.meta.main so lifeos-sync.test.ts can import mergeHooks and
// parseOverlayTargets without running the CLI as a side effect of the import —
// the same split tools/doctor.ts uses, and the reason both are testable at all.

function main(): never {
  if (!AXON_ROOT || !OVERLAY_ROOT) {
    console.error("lifeos-sync: AXON_ROOT/AXON_PERSONAL_ROOT unset — run via tools/lifeos-sync, not directly");
    process.exit(2);
  }

  const args = process.argv.slice(2);
  dryRun = args.includes("--dry-run") || args.includes("-n");
  cmd = args.find((a) => !a.startsWith("-")) ?? "status";
  if (cmd !== "status" && cmd !== "deploy") {
    console.error(`lifeos-sync: unknown command '${cmd}' — expected 'status' or 'deploy'`);
    process.exit(2);
  }

  let targets: Target[];
  try {
    targets = resolveTargets();
  } catch (err) {
    // Refuse to run on a manifest we cannot read. Falling back to the Axon-only
    // set would deploy a strictly smaller delta and report success doing it.
    console.error(`lifeos-sync: ${err instanceof Error ? err.message : err}`);
    process.exit(2);
  }

  console.log(`lifeos-sync · ${cmd}${dryRun ? " (dry-run)" : ""}`);
  console.log(`  config root: ${rel(CONFIG_ROOT)}\n`);

  for (const t of targets) {
    if (t.kind === "copy") {
      const state = copyState(t);
      if (cmd === "status") {
        if (state === "synced") report("ok", t.what, `content matches ${rel(t.src)}`);
        else if (state === "no-source") report("fail", t.what, `source missing: ${rel(t.src)}`);
        else if (state === "absent") report("drift", t.what, `not deployed — run: tools/lifeos-sync deploy`);
        else {
          const pending = t.writeback ? pullBack(t) : [];
          report("drift", t.what, pending.length
            ? `${rel(t.dst)} differs; ${pending.length} newer live file(s) would be pulled back first`
            : `${rel(t.dst)} differs from the overlay — deploy backs it up, then overwrites`);
        }
      } else {
        deployCopy(t);
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
}

if (import.meta.main) main();
