// tools/doctor.ts — health checks for an already-set-up Axon machine:
// overlay reachability, machine.toml validity, declared state mounts,
// systems.toml coverage + undeclared-connection sweep, Pack deployment state. Real
// TOML parsing via Bun's built-in Bun.TOML — machine.toml uses array-of-tables
// ([[state_mount]]) that tools/lib/toml.sh's grep/sed single-line contract
// can't parse, which is why this is TS, not bash. It stays an interpreted command with no
// build step of its own (README.md#cargo-and-bun-are-the-build-path).
//
// Delegates each rule to the script that owns it rather than reimplementing it: the
// accepted-finding clock to tools/audit --expiry, the host toolchain to
// tools/toolchain-check. Upstream freshness used to be delegated here too, to
// tools/upstream-checker; PRD Q41 retired that script on 2026-08-28 and Renovate answers
// the question now, on GitHub rather than to a local health command. Same
// reasoning applies to systems.toml/connection checks below: this extends the
// existing state-mount reality-check idiom (systems.toml stays the one
// hand-authored registry, README.md#one-manifest-per-concern) rather than adding a second manifest or a
// separate tool — see README.md#documentation-stays-owned-and-current.
//
//   tools/doctor            # full report, offline (no GitHub calls)
//   tools/doctor --online   # also probe declared systems and fetch origin/main
//   tools/doctor --version  # installed vs origin/main version identity only (read-only, exits 0)
//   tools/doctor -h         # this help
//
// Exit 0 = all checks pass, 1 = one or more failed. Invoke via the tools/doctor
// launcher (exec bun run), not this file directly, to match the printctl/uv
// launcher pattern (README.md#language-tooling).

import { existsSync, lstatSync, readdirSync, readFileSync, readlinkSync, statSync } from "node:fs";
import { basename, resolve, join, relative } from "node:path";
// Only the --online reachability probe uses this, to tell a dead hostname from a stopped service.
// The offline doctor path never calls it, so the report stays network-free where it must be.
import { lookup as dnsLookup } from "node:dns/promises";
import { defaultCodexDeployConfig, getStatuses } from "./packs-codex.ts";
import { defaultClaudeDeployConfig } from "./packs-claude.ts";
import { resolveMachineToml, resolveOverlayRoot } from "./lib/overlay.ts";
import { releaseTagGlob } from "./lib/release.ts";

const HELP = `tools/doctor — health checks for an already-set-up Axon machine.

  tools/doctor            full report, offline (no GitHub calls)
  tools/doctor --online   also probe declared systems and fetch origin/main
  tools/doctor --version  installed vs origin/main version identity only
                          (read-only, exits 0; add --online for a live fetch first)
  tools/doctor -h         this help
`;

const AXON_ROOT = resolve(import.meta.dir, "..");
const HOME = process.env.HOME ?? "";

/// Discover Rust sources recursively so policy checks cover conventional
/// multi-file binary roots such as `src/server/main.rs`, not only flat crates.
export function findRustSources(root: string): string[] {
  if (!existsSync(root)) return [];
  const sources: string[] = [];
  const pending = [root];
  while (pending.length > 0) {
    const dir = pending.pop()!;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile() && entry.name.endsWith(".rs")) sources.push(path);
    }
  }
  return sources.sort();
}

function expandHome(p: string): string {
  return p.startsWith("~") ? HOME + p.slice(1) : p;
}

// Pure, unit-testable core of the "Systems (systems.toml)" section: which
// machine.toml state_mount tools are covered by a systems.toml identity entry,
// and which aren't (see tools/doctor.test.ts).
export function checkStateMountCoverage(
  mounts: Array<{ tool: string }>,
  systemIds: Set<string>,
): { covered: string[]; uncovered: string[] } {
  const covered: string[] = [];
  const uncovered: string[] = [];
  for (const m of mounts) (systemIds.has(m.tool) ? covered : uncovered).push(m.tool);
  return { covered, uncovered };
}

// Pure, unit-testable core of the "Undeclared connections" sweep: extract
// candidate sibling-repo names from a blob of text. Repo paths can nest
// (Developer/Personal/Knowledge-Base, Developer/Collab/VBB), so the
// candidate is the LAST path segment (the actual repo dir), matched by
// basename against systems.toml ids — not the first segment after
// Developer/, which would misidentify nested paths as their parent folder.
// Deliberately narrow to $HOME/Developer/* and ~/Developer/* so this stays a
// fast, low-noise signal, not a generic path-linter. Known blind spots (not
// caught by this sweep): paths built from env vars or config indirection, where
// the literal never appears in the file, and Packs/*/pack.toml `links` entries
// that name a system without a literal $HOME path.
export function extractSiblingRepoRefs(text: string): string[] {
  const pathPattern = /(?:\$HOME|~)\/Developer\/((?:[A-Za-z0-9._-]+\/)*[A-Za-z0-9._-]+)/g;
  const names: string[] = [];
  for (const match of text.matchAll(pathPattern)) {
    const segments = match[1].split("/");
    names.push(segments[segments.length - 1]);
  }
  return names;
}

// Pure, unit-testable core of the sweep's skip rule: is this file exempt from the
// hardcoded-path sweep by what it IS, rather than by being named in a list?
//
// Two properties replace two literals the list used to carry (Axon#26). A generated
// artifact says so in its own header, so ARCHITECTURE.md needed no name — and the next
// generated file will not need one either. A `.example` template's whole job is to SHOW a
// path so a reader knows what the field takes, which is the same rule the capability env
// templates already live under.
//
// What stays a literal is what no file property can express: the sanctioned indirection
// itself, the bootstrap that names overlay locations before any resolver exists, and the
// sweep's own test fixtures, whose paths are the specification of what a reference looks
// like rather than references. Naming those three is the rule; naming a generated file was
// a list.
export function isSweepExempt(relPath: string, text: string): boolean {
  if (relPath === "tools/lib/paths.sh" || relPath === "tools/install.sh") return true;
  if (relPath === "tools/doctor.test.ts") return true;
  if (relPath.endsWith(".example")) return true;
  // The header convention is "Auto-generated by <tool>", within the first lines. Scanning
  // the whole file instead would exempt any document that merely mentions the phrase.
  return /auto-generated\b/i.test(text.split("\n", 6).join("\n"));
}

// Pure, unit-testable core of the why-block base set: the prefixes a reference may be
// written against, derived from `git ls-files` (Axon#26).
//
// Three depths, each earning its place. A top-level owner (`capabilities/`), a unit inside
// it (`capabilities/comms/`), and that unit's sources (`capabilities/comms/src/`) — the
// last is the convention of naming a path relative to the crate that owns it
// (`sources/mod.rs`). The referencing document's own directory is added separately by
// findDecisionPathRot and covers the common case.
//
// Derived from tracked paths rather than from readdir for two reasons. It cannot invent a
// base nothing lives under, which the previous hand-list did by appending `<unit>/src/` to
// every unit whether or not it had one; and it cannot pick up an untracked build tree —
// `dashboard/node_modules/` as a resolution base would quietly make missing paths resolve.
export function whyBlockBases(trackedFiles: string[]): string[] {
  const bases = new Set<string>([""]);
  for (const f of trackedFiles) {
    const seg = f.split("/");
    if (seg.length > 1) bases.add(`${seg[0]}/`);
    if (seg.length > 2) bases.add(`${seg[0]}/${seg[1]}/`);
    if (seg.length > 3 && seg[2] === "src") bases.add(`${seg[0]}/${seg[1]}/src/`);
  }
  return [...bases].sort();
}

// Mask cfg(test) items before enforcing production-only Rust source policy.
// Keep newlines so any future diagnostics can still report useful line numbers.
function rustCfgItemEnd(source: string, start: number): number {
  let depth = 0;
  let bodyStarted = false;

  for (let i = start; i < source.length; i++) {
    if (source.startsWith("//", i)) {
      const newline = source.indexOf("\n", i + 2);
      if (newline === -1) return source.length;
      i = newline;
      continue;
    }
    if (source.startsWith("/*", i)) {
      let commentDepth = 1;
      i += 2;
      while (i < source.length && commentDepth > 0) {
        if (source.startsWith("/*", i)) {
          commentDepth++;
          i += 2;
        } else if (source.startsWith("*/", i)) {
          commentDepth--;
          i += 2;
        } else {
          i++;
        }
      }
      i--;
      continue;
    }

    const raw = source.slice(i).match(/^(?:br|r)(#*)"/);
    if (raw) {
      const terminator = `"${raw[1]}`;
      const close = source.indexOf(terminator, i + raw[0].length);
      if (close === -1) return source.length;
      i = close + terminator.length - 1;
      continue;
    }
    if (source[i] === '"') {
      i++;
      while (i < source.length) {
        if (source[i] === "\\") i += 2;
        else if (source[i] === '"') break;
        else i++;
      }
      continue;
    }
    if (source[i] === "'" && /^'(?:\\.|[^\\'\r\n])'/.test(source.slice(i))) {
      const close = source.indexOf("'", i + 1);
      i = close === -1 ? source.length : close;
      continue;
    }

    if (source[i] === "{") {
      bodyStarted = true;
      depth++;
    } else if (source[i] === "}" && bodyStarted) {
      depth--;
      if (depth === 0) return i + 1;
    } else if (source[i] === ";" && !bodyStarted) {
      return i + 1;
    }
  }

  return source.length;
}

export function stripRustCfgTestItems(source: string): string {
  const chars = [...source];
  const cfgTest = /#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/g;
  let match: RegExpExecArray | null;

  while ((match = cfgTest.exec(source)) !== null) {
    const end = rustCfgItemEnd(source, cfgTest.lastIndex);
    for (let i = match.index; i < end; i++) {
      if (chars[i] !== "\n") chars[i] = " ";
    }
    cfgTest.lastIndex = end;
  }

  return chars.join("");
}

const OWN_LISTENER = [
  { name: "axum::serve", pattern: /\baxum::serve\s*\(/ },
  { name: "TcpListener::bind", pattern: /TcpListener::bind\s*\(/ },
] as const;

export function findProductionListenerConstructs(source: string): string[] {
  const production = stripRustCfgTestItems(source);
  return OWN_LISTENER.filter(({ pattern }) => pattern.test(production)).map(({ name }) => name);
}

// Pure helper for the env-template contract check: parse `KEY=VALUE` lines with
// optional quotes and inline comments, skipping blank/comment-only lines.
export function parseEnvTemplateLines(text: string): Array<{ key: string; value: string }> {
  const out: Array<{ key: string; value: string }> = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const hash = line.indexOf(" #");
    const stripped = hash === -1 ? line : line.slice(0, hash).trim();
    const match = stripped.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
    if (!match) continue;
    const key = match[1];
    let value = match[2].trim();
    if (value.length >= 2 && ((value.startsWith("'") && value.endsWith("'")) || (value.startsWith('"') && value.endsWith('"')))) {
      value = value.slice(1, -1);
    }
    out.push({ key, value });
  }
  return out;
}

const SENSITIVE_ENV_HINT = /(PASS|PASSWORD|TOKEN|SECRET|KEY|CREDENTIAL|BEARER|HASH|SIGNATURE|PRIVATE)/i;

function isEnvValuePlaceholder(value: string): boolean {
  const v = value.trim();
  if (!v) return true;
  if (/^<[^>]+>$/.test(v)) return true;
  if (/^\$\{[^}]+\}$/.test(v)) return true;
  if (/^required:/i.test(v)) return true;
  if (/^(example|placeholder|changeme|change me|replace me)/i.test(v)) return true;
  return false;
}

function likelyRawSecret(value: string): boolean {
  const v = value.trim();
  if (!v || isEnvValuePlaceholder(v)) return false;
  if (v.length < 16) return false;
  if (/^[A-Za-z0-9+/=]+$/.test(v) && /[A-Za-z]/.test(v) && /[0-9]/.test(v)) return true;
  if (/^\$argon2/.test(v)) return true; // token hashes belong in overlay, never template
  return false;
}

export function findPlaintextSecretsInEnvTemplate(text: string): string[] {
  const out: string[] = [];
  for (const { key, value } of parseEnvTemplateLines(text)) {
    if (!SENSITIVE_ENV_HINT.test(key)) continue;
    if (likelyRawSecret(value)) out.push(key);
  }
  return out;
}

// Pure, unit-testable core of the "Decision freshness" sweep. A decision entry is a claim
// about the present written in the past tense, and nothing detects when the present changes:
// on 2026-07-16 the root-is-the-spine restructure dissolved `apps/`, and three entries kept
// asserting it for twelve days — one in the `rule:` line the generated index displays.
//
// This catches the half a machine can see: a path an entry names must exist, and a path it
// declares deliberately absent (frontmatter `asserts_absent: ["a/b"]`, single-line array, same
// not-a-full-parser contract as tools/lib/toml.sh) must stay absent. The second direction
// matters as much — something building the thing a decision forbids is rot too.
//
// It lives in doctor rather than a repo gate deliberately. Decisions legitimately name
// gitignored paths (`graphify-out/`, local scratch), and anything that sees tracked files only
// cannot tell an absent path from a rotten one. Measured when this ran as a Bazel test, before
// PRD Q44 retired Bazel on 2026-08-25: 16 findings against the real tree's 0, every one of
// them that blindness. Same reasoning as the topology sweep folded into
// README.md#documentation-stays-owned-and-current: extend doctor, don't add a tool.
//
// Blind to semantic rot (reasoning that stopped applying while the paths stayed valid). Path
// rot was 100% of what the 2026-07-28 audit found by hand, so this is the cheap majority.
export function findDecisionPathRot(
  entries: Array<{ slug: string; text: string; assertsAbsent: string[]; dir?: string }>,
  exists: (p: string) => boolean,
  // Prefixes a reference may be written against. A path is resolved against the referencing
  // document's own directory FIRST -- `references/house-style.md` inside a SKILL.md means
  // exactly that, and reading it any other way produced 171 findings across 98 files, nearly
  // all of them this one mistake. These extra bases then cover the other real convention:
  // naming a path relative to the crate that owns it (`sources/mod.rs`) rather than the root.
  bases: string[] = [""],
): Array<{ slug: string; path: string; kind: "missing" | "present" }> {
  const out: Array<{ slug: string; path: string; kind: "missing" | "present" }> = [];
  // Collapse `a/b/../c` so a parent-relative reference resolves.
  const norm = (p: string): string => {
    const stack: string[] = [];
    for (const seg of p.split("/")) {
      if (seg === "" || seg === ".") continue;
      if (seg === "..") stack.pop();
      else stack.push(seg);
    }
    return stack.join("/");
  };
  for (const { slug, text, assertsAbsent, dir } of entries) {
    const docBases = dir === undefined ? bases : [`${dir}/`, ...bases];
    const named = new Set<string>();
    // A backticked slug inside a link label whose destination is a URL names an external
    // resource, not a repo path: a model id like `org/model-name` written as the label of its
    // own huggingface link is the case that produced the only false positive this check has
    // had. The destination sitting right next to it is the authority on where the thing lives,
    // so drop those labels before scanning.
    const scanned = text.replace(/\[[^\]]*\]\((?:https?|mailto):[^)]*\)/g, " ");
    for (const m of scanned.matchAll(/`([A-Za-z0-9_./-]+\/[A-Za-z0-9_./-]+)`/g)) {
      let p = m[1].split("#")[0].replace(/[.,;:)]+$/, "").replace(/\/$/, "");
      // Absolute paths, URLs, git refs and shell/placeholder forms are not repo paths.
      if (!p || /^[/~$<*]/.test(p) || p.includes("://") || p.startsWith("origin/")) continue;
      if (/\.(com|org|io|dev|net)\//.test(p)) continue;
      named.add(p);
    }
    for (const p of named) {
      if (assertsAbsent.includes(p)) continue;
      if (!docBases.some((b) => exists(norm(b + p)))) out.push({ slug, path: p, kind: "missing" });
    }
    for (const p of assertsAbsent) {
      if (exists(p)) out.push({ slug, path: p, kind: "present" });
    }
  }
  return out;
}

// Pure, unit-testable core of the why-block half of the Decision freshness sweep. README.md#decisions-live-with-their-owner puts
// a decision's reasoning in the README of whatever it governs, under a `## Why this shape:`
// heading, so the rot check has to follow the prose there rather than only watching decisions/.
//
// Scoped to those blocks on purpose. Sweeping every tracked markdown file instead produced 139
// findings across 98 files — GitHub slugs (`gorse-io/gorse`), overlay paths, gitignored runtime
// artifacts — which is a general path linter, not a rot detector, and noise that size trains
// people to ignore the gate. The four why-blocks that exist today produce zero.
//
// `<!-- asserts-absent: a/b, c/d -->` inside a block is the markdown equivalent of the
// frontmatter key decisions/ entries use, for the same "this absence is the point" case.
export function collectWhyBlocks(
  file: string,
  text: string,
): Array<{ slug: string; text: string; assertsAbsent: string[]; dir: string }> {
  const out: Array<{ slug: string; text: string; assertsAbsent: string[]; dir: string }> = [];
  const dir = file.includes("/") ? file.slice(0, file.lastIndexOf("/")) : "";
  for (const m of text.matchAll(/^## Why this shape([^\n]*)\n([\s\S]*?)(?=^## |$(?![\s\S]))/gm)) {
    const body = m[2];
    const declared = body.match(/<!--\s*asserts-absent:([^>]*?)-->/);
    const assertsAbsent = (declared?.[1] ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    out.push({ slug: `${file}${m[1].trim() ? ` (${m[1].replace(/^:\s*/, "").trim()})` : ""}`, text: body, assertsAbsent, dir });
  }
  return out;
}

// Pure, unit-testable core of the third Decision freshness check: a reference to a
// decisions/<slug> that no longer exists. Dissolving an entry means repointing everything that
// cited it, and during the 2026-07-28 curation that step was missed three batches running --
// partly because the generator *emits* decision paths into ARCHITECTURE.md, so a sweep that
// excluded the generated file could not see them. Cheap to check, invisible when skipped.
export function findDanglingDecisionRefs(
  files: Array<{ path: string; text: string }>,
  slugExists: (slug: string) => boolean,
): Array<{ file: string; slug: string }> {
  const out: Array<{ file: string; slug: string }> = [];
  const seen = new Set<string>();
  for (const { path, text } of files) {
    for (const m of text.matchAll(/decisions\/([a-z0-9][a-z0-9-]*)/g)) {
      const slug = m[1];
      const key = `${path}::${slug}`;
      if (seen.has(key) || slugExists(slug)) continue;
      seen.add(key);
      out.push({ file: path, slug });
    }
  }
  return out;
}

// Pure, unit-testable core of the version identity (`--version` fast path and
// the Session orientation version line): render a `git describe
// --tags --always --dirty` string (tag, bare sha, or either with git's own
// "-dirty" suffix — passed through verbatim, never re-derived here) plus the
// commit date. No I/O — callers do the git calls (see tools/doctor.test.ts).
export function formatVersion(describe: string, commitDate: string): string {
  if (!describe) return "(unknown — not a git checkout?)";
  return commitDate ? `${describe} (${commitDate})` : describe;
}

// Pure, unit-testable core of the fetch-age readout: how stale is the cached
// origin/main ref, from .git/FETCH_HEAD's mtime. null = no FETCH_HEAD at all
// (fresh clone that never fetched) — reported honestly, not as an error.
export function formatFetchAge(fetchEpochSeconds: number | null, nowEpochSeconds: number): string {
  if (fetchEpochSeconds === null) return "no fetch recorded";
  const age = Math.max(0, nowEpochSeconds - fetchEpochSeconds);
  if (age < 60) return "fetched just now";
  const minutes = Math.floor(age / 60);
  if (minutes < 60) return `fetched ${minutes} minute(s) ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `fetched ${hours} hour(s) ago`;
  return `fetched ${Math.floor(hours / 24)} day(s) ago`;
}

// How long a single reachability probe may take, absent an overlay saying otherwise. Public Axon
// ships the default; a deployment that knows one of its endpoints is legitimately slow raises it
// for that entry via `probe_timeout_ms` rather than muting the check or raising it for everything.
// Measured case: build.nvidia.com answers 202 in ~9.2s, consistently, for both HEAD and GET.
export const PROBE_TIMEOUT_MS = 5000;

export type ProbeTarget =
  | { id: string; url: string; timeoutMs: number }
  | { id: string; skip: string };

// Pure core of the reachability check: which declared systems have an endpoint worth dialling,
// and where does its URL come from.
//
// Public Axon never holds a private endpoint. A private system's `url` is the literal sentinel
// `overlay:systems.local.toml`, and the real value is looked up by the SAME id in the active
// overlay. That indirection is the whole reason this can probe a private deployment without the
// public repo learning a hostname, so it is resolved here rather than by pattern-matching a URL
// shape somewhere in the section body.
//
// Everything that is not an http(s) endpoint is skipped WITH A REASON, never silently dropped:
// "nothing to probe" and "probe not attempted" are different answers, and a check that conflates
// them reports a green for an endpoint it never touched.
export function resolveProbeTargets(
  systemsToml: Record<string, any>,
  overlaySystems: Record<string, any>,
): ProbeTarget[] {
  const targets: ProbeTarget[] = [];
  for (const [id, entry] of Object.entries(systemsToml ?? {})) {
    if (!entry || typeof entry !== "object") continue;
    const overlayEntry = overlaySystems?.[id];
    // Probe policy is the OVERLAY's call, not Axon's: whether an endpoint should be dialled at
    // all depends on the deployment, and a public default that says "dial it" would be a
    // deployment decision shipped in a public repo.
    if (overlayEntry?.probe === "no") {
      targets.push({ id, skip: "overlay declares probe = \"no\"" });
      continue;
    }
    let url = typeof entry.url === "string" ? entry.url : "";
    if (url === "overlay:systems.local.toml") {
      const resolved = typeof overlayEntry?.url === "string" ? overlayEntry.url : "";
      if (!resolved) {
        targets.push({ id, skip: "private system, no url in the overlay" });
        continue;
      }
      url = resolved;
    }
    if (!url) {
      targets.push({ id, skip: "no url declared" });
      continue;
    }
    if (url === "local") {
      targets.push({ id, skip: "url = \"local\" — not an endpoint" });
      continue;
    }
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch {
      targets.push({ id, skip: "url is not parseable" });
      continue;
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      targets.push({ id, skip: `${parsed.protocol.replace(":", "")} endpoint — only http(s) is probed` });
      continue;
    }
    // A URL carrying userinfo would put a credential on the wire on Axon's initiative. The
    // declaration is the operator's, but dialling it is ours, so this one is refused rather than
    // sent. Note the reason names the shape, never the value.
    if (parsed.username || parsed.password) {
      targets.push({ id, skip: "url embeds credentials — not probed" });
      continue;
    }
    // A per-entry timeout is probe policy, so it comes from the overlay like the rest of it. A
    // non-numeric or non-positive value falls back to the default rather than disabling the
    // timeout: an unbounded probe would hang the whole report on one bad declaration.
    const declared = Number(overlayEntry?.probe_timeout_ms);
    const timeoutMs = Number.isFinite(declared) && declared > 0 ? declared : PROBE_TIMEOUT_MS;
    targets.push({ id, url, timeoutMs });
  }
  return targets;
}

export type ProbeOutcome = "refused" | "timeout" | "unavailable";

// Which failure a thrown fetch error means, GIVEN that the hostname already resolved.
//
// That precondition is not a detail, it is the whole reason the caller resolves DNS separately.
// Bun's fetch reports `code: "ConnectionRefused"` for a genuinely refused socket AND for a
// hostname that does not exist — verified against both, 2026-08-06 on Bun 1.3.14. Classifying
// straight off the error therefore cannot tell a stopped service from a dead name, which is the
// one distinction an operator actually acts on differently. The DNS step upstream removes the
// ambiguity; by the time this runs, a connect failure really is a connect failure.
//
// Kept pure and separate so the mapping is testable without a network.
export function classifyProbeOutcome(err: unknown): ProbeOutcome {
  const name = (err as any)?.name ?? "";
  if (name === "TimeoutError" || name === "AbortError") return "timeout";
  const code = String((err as any)?.code ?? "");
  const message = String((err as any)?.message ?? "");
  if (code === "ETIMEDOUT" || /timed out/i.test(message)) return "timeout";
  // Bun's spelling and Node's, because this file runs under Bun but the shape is not guaranteed.
  if (code === "ConnectionRefused" || code === "ECONNREFUSED" || /refused|unable to connect/i.test(message)) {
    return "refused";
  }
  return "unavailable";
}

async function readToml(path: string): Promise<any> {
  return Bun.TOML.parse(await Bun.file(path).text());
}

// Trimmed stdout of a git command against this checkout, "" on failure —
// enough for the read-only version/orientation readouts below.
function gitOut(...args: string[]): string {
  const proc = Bun.spawnSync({ cmd: ["git", "-C", AXON_ROOT, ...args], stdout: "pipe", stderr: "pipe" });
  return proc.exitCode === 0 ? proc.stdout.toString().trim() : "";
}

// Which tags are release tags — axon.toml [release] tag_glob, the one home shared with
// tools/lib/version.sh (README.md#the-release-line). Resolved once at load; a missing key is a
// broken manifest and should stop the tool, not be papered over with a literal.
const RELEASE_TAG_GLOB = releaseTagGlob(AXON_ROOT);

// Newest semver release tag (vX.Y.Z), or "" if none cut yet. Mirrors tools/lib/delta.sh's
// latest_release_ref so doctor --version and update.sh agree on "the newest release" — the first
// tag in descending version order that is actually a dotted number (a stray -rc/non-version tag
// is skipped, not mistaken for the latest release).
function latestReleaseTag(): string {
  const tags = gitOut("tag", "-l", RELEASE_TAG_GLOB, "--sort=-v:refname");
  if (!tags) return "";
  for (const raw of tags.split("\n")) {
    const t = raw.trim();
    if (/^v?\d+(\.\d+)*$/.test(t)) return t;
  }
  return "";
}

// `tools/doctor --version` fast path — version identity only, no health
// checks. Read-only, always exits 0: "what am I running and is origin newer"
// is a question, not a check that can fail. Offline by default (reads the
// cached origin/main ref + FETCH_HEAD age, says so honestly); --online
// fetches first, same split as the full report.
function printVersion(online: boolean): void {
  console.log(`Axon doctor --version · ${AXON_ROOT}`);
  if (online) {
    Bun.spawnSync({ cmd: ["git", "-C", AXON_ROOT, "fetch", "--quiet", "origin", "main"], stdout: "pipe", stderr: "pipe" });
  }

  const describe = gitOut("describe", "--tags", "--always", "--dirty", "--match", RELEASE_TAG_GLOB);
  console.log(`  installed: ${formatVersion(describe, gitOut("log", "-1", "--format=%cs"))}`);

  // Release-aware: once tags exist, say where this checkout sits relative to the newest release,
  // not only the moving main branch. Silent when no release has been cut yet.
  const latestTag = latestReleaseTag();
  if (latestTag) {
    console.log(`  release:   ${formatVersion(latestTag, gitOut("log", "-1", "--format=%cs", latestTag))} — newest release tag`);
  }

  const originSha = gitOut("rev-parse", "--short", "origin/main");
  if (!originSha) {
    console.log("  latest:    unknown — no origin/main ref cached (run tools/doctor --version --online)");
    return;
  }

  // FETCH_HEAD mtime = when this checkout last asked origin anything. Resolve
  // the git dir properly (worktrees have a .git *file*), fall back gracefully.
  let fetchEpoch: number | null = null;
  try {
    const gitDir = gitOut("rev-parse", "--absolute-git-dir") || join(AXON_ROOT, ".git");
    fetchEpoch = Math.floor(statSync(join(gitDir, "FETCH_HEAD")).mtimeMs / 1000);
  } catch {
    // no FETCH_HEAD — formatFetchAge(null, …) reports it
  }
  const fetchAge = formatFetchAge(fetchEpoch, Math.floor(Date.now() / 1000));
  const liveness = online ? "" : " (offline — run with --online for live)";
  console.log(`  latest:    ${formatVersion(originSha, gitOut("log", "-1", "--format=%cs", "origin/main"))} — origin/main, ${fetchAge}${liveness}`);

  // Same rev-list idiom as the full report's "Repo freshness" section.
  const counts = gitOut("rev-list", "--left-right", "--count", "HEAD...origin/main");
  if (!counts) return;
  const [aheadStr, behindStr] = counts.split(/\s+/);
  const ahead = Number(aheadStr) || 0;
  const behind = Number(behindStr) || 0;
  if (ahead === 0 && behind === 0) console.log("  up to date with origin/main");
  else if (behind > 0 && ahead === 0) console.log(`  ${behind} commit(s) behind origin/main — run tools/update.sh`);
  else if (ahead > 0 && behind === 0) console.log(`  ${ahead} commit(s) ahead of origin/main — push when ready`);
  else console.log(`  diverged from origin/main (${ahead} ahead, ${behind} behind) — merge before tools/update.sh`);
}

// Everything below is the executable report — guarded by the import.meta.main
// line at the bottom so tools/doctor.test.ts can import
// checkStateMountCoverage/extractSiblingRepoRefs without running the whole CLI
// (console output, process.exit) as a side effect of import.

// What one check hands the next. The report is a chain, not a set: the overlay
// path resolved first is what machine.toml is read from, machine.toml is where
// the mounts come from, and the sweep needs both plus systems.toml. Everything
// that crosses a section boundary travels here; anything a section only uses
// itself stays local to its run().
type CheckContext = {
  root: string;
  overlayPath: string;
  machineToml: any;
  mounts: any[];
  systemsToml: Record<string, any>;
  online: boolean;
  ok(msg: string): void;
  bad(msg: string): void;
  warn(msg: string): void;
};

// `name` is printed as the section header, so CHECKS' array order below IS the
// order of the report.
type Check = { name: string; run(ctx: CheckContext): void | Promise<void> };

const CHECKS: Check[] = [
  // overlay location. axon.local.toml (gitignored, per-machine) wins; the tracked
  // axon.toml carries only a shipped default, which is what keeps the repo gates
  // working — a fresh CI clone has the tracked file and no local one.
  // Mirrors tools/lib/paths.sh's resolution order; see
  // schemas/machine.toml.example.
  {
    name: "Overlay",
    async run(ctx) {
      const overlay = resolveOverlayRoot(ctx.root);
      if (!overlay) {
        ctx.bad("no 'overlay' in axon.local.toml or axon.toml — run tools/install.sh");
      } else {
        ctx.overlayPath = overlay.root;
        if (existsSync(ctx.overlayPath)) {
          ctx.ok(`overlay at ${ctx.overlayPath} (from ${overlay.source})`);
          // Falling back means this machine never recorded its own location. It works only
          // as long as the shipped default happens to be right, so say so out loud.
          if (overlay.source === "axon.toml") {
            ctx.warn("no axon.local.toml — this machine is running on the shipped default; run tools/install.sh to pin it");
          }
        } else {
          ctx.bad(`overlay declared (from ${overlay.source}) but missing at ${ctx.overlayPath} — run tools/install.sh`);
        }
      }
    },
  },

  // machine.toml — this machine's whole identity: platform, enabled capabilities,
  // and its state-mount registry.
  {
    name: "Machine identity",
    async run(ctx) {
      if (ctx.overlayPath && existsSync(ctx.overlayPath)) {
        // An overlay may own several machines. Say which one was resolved and how —
        // a report that silently read the wrong machine's manifest would still look
        // clean, and every check below inherits this answer.
        const machine = resolveMachineToml(ctx.overlayPath);
        const machineTomlPath = machine?.path ?? join(ctx.overlayPath, "config", "machine.toml");
        if (!existsSync(machineTomlPath)) {
          const named = machine?.source === "axon.local.toml"
            ? ` — axon.local.toml names machine '${machine.name}', which has no manifest`
            : " — run tools/install.sh";
          ctx.bad(`missing ${machineTomlPath}${named}`);
        } else {
          if (machine?.source === "config/machine.toml") ctx.ok("machine: single-file layout");
          else ctx.ok(`machine: ${machine?.name} (from ${machine?.source})`);
          ctx.machineToml = await readToml(machineTomlPath);
          if (ctx.machineToml.os) ctx.ok(`os = ${ctx.machineToml.os}`);
          else ctx.bad("machine.toml: missing 'os'");
          if (ctx.machineToml.container_runtime) ctx.ok(`container_runtime = ${ctx.machineToml.container_runtime}`);
          else ctx.bad("machine.toml: missing 'container_runtime'");
        }

        // The schema has one home. Three overlay-local copies of it existed until
        // 2026-08-04 and had already drifted — never in structure, only in which OS each
        // happened to name, which is the drift you get for free when a template is copied
        // per consumer. An overlay commits its real machine.toml anyway, so an example
        // beside it is a second thing to update and the first to go stale.
        const strayExamples = [
          join(ctx.overlayPath, "config", "machine.toml.example"),
          join(ctx.overlayPath, "config", "machines", "machine.toml.example"),
        ].filter(existsSync);
        if (strayExamples.length === 0) {
          ctx.ok("machine schema: not duplicated into the overlay");
        } else {
          for (const stray of strayExamples) {
            ctx.bad(
              `${stray} duplicates schemas/machine.toml.example — delete it and drop its ` +
                `.gitignore allowlist line; the schema lives in Axon only`,
            );
          }
        }
      } else {
        ctx.warn("skipped — no overlay to check");
      }
    },
  },

  // Host toolchain — delegate to tools/toolchain-check, don't reimplement (same shape as
  // the tools/audit --expiry delegation below: the script owns the rule, doctor reports). Reads
  // --json so a required miss maps to bad and an optional absence to warn, rather than
  // collapsing everything into one exit code. os/runtime come from machine.toml when we
  // have it; the checker self-resolves from uname otherwise.
  {
    name: "Host toolchain (tools/toolchain-check)",
    run(ctx) {
      const checkerPath = join(ctx.root, "tools", "toolchain-check");
      if (!existsSync(checkerPath)) {
        ctx.warn(`missing ${checkerPath}`);
        return;
      }
      const args = ["--json"];
      if (ctx.machineToml?.os) args.push("--os", ctx.machineToml.os);
      if (ctx.machineToml?.container_runtime) args.push("--runtime", ctx.machineToml.container_runtime);
      const proc = Bun.spawnSync({ cmd: [checkerPath, ...args], stdout: "pipe", stderr: "pipe" });
      let data: any;
      try {
        data = JSON.parse(proc.stdout.toString());
      } catch {
        ctx.warn("toolchain-check did not emit JSON — run tools/toolchain-check for detail");
        return;
      }
      const entries: any[] = Array.isArray(data.entries) ? data.entries : [];
      const missing = entries.filter((e) => e.status === "missing");
      const outdatedReq = entries.filter((e) => e.status === "outdated" && e.class !== "optional");
      const absent = entries.filter((e) => e.status === "absent");
      const outdatedOpt = entries.filter((e) => e.status === "outdated" && e.class === "optional");
      for (const e of missing) ctx.bad(`${e.bin} missing (${e.class}) — install: ${e.install}`);
      for (const e of outdatedReq) ctx.bad(`${e.bin} ${e.note} — install: ${e.install}`);
      for (const e of absent) ctx.warn(`${e.bin} absent (optional) — install: ${e.install}`);
      for (const e of outdatedOpt) ctx.warn(`${e.bin} ${e.note}`);
      if (missing.length === 0 && outdatedReq.length === 0) {
        const tail = absent.length ? `, ${absent.length} optional absent` : "";
        // Say the count is SCOPED. Without this the number silently shrank when needed_by landed,
        // and "8/8 present" on a machine whose manifest declares seventeen tools reads as a
        // partial check rather than a complete one over the applicable set (#163).
        const naCount = entries.filter((e) => e.status === "n/a").length;
        const scope = naCount ? `, ${naCount} n/a here` : "";
        ctx.ok(`${data.totals?.ok ?? 0}/${data.totals?.count ?? 0} required present${tail}${scope}`);
      }
      if (entries.some((e) => e.status === "n/a")) {
        ctx.ok("scoped to this machine — 'tools/toolchain-check --workflow backup|restore|audit|build' before running one");
      }
    },
  },

  // Assistant harness integrations. This section is optional infrastructure, so
  // stale/incomplete state is a warning, not a hard failure; install-time
  // guidance is handled separately in tools/install.sh.
  {
    name: "AI assistant integrations",
    run(ctx) {
      const scriptPath = join(ctx.root, "tools", "agent-integrations.sh");
      if (!existsSync(scriptPath)) {
        ctx.warn(`missing ${scriptPath}`);
        return;
      }
      const proc = Bun.spawnSync({
        cmd: [scriptPath, "status", "--json"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode !== 0) {
        ctx.warn(`agent-integrations status failed (run: tools/agent-integrations.sh status --json)`);
        return;
      }

      let payload: {
        upstream?: string;
      harnesses?: Array<{
          name?: string;
          state?: "runnable" | "configured" | "integrated" | "stale" | string;
          command?: string;
          command_version?: string;
          graph_state?: string;
          install_command?: string;
          config_dir?: string;
        }>;
      };

      try {
        payload = JSON.parse(proc.stdout.toString()) as any;
      } catch {
        ctx.warn("agent-integrations status did not emit JSON — run: tools/agent-integrations.sh status --json");
        return;
      }

      const harnesses = Array.isArray(payload?.harnesses) ? payload.harnesses : [];
      if (harnesses.length === 0) {
        ctx.warn("no assistant integration rows reported");
        return;
      }

      for (const item of harnesses) {
        const name = item.name || "unknown";
        const state = item.state || "unknown";
        const installCommand = (item.install_command || `tools/agent-integrations.sh install ${name}`).trim();
        const location = item.config_dir ? ` (${item.config_dir})` : "";
        const graphState = item.graph_state || "unknown";
        const command = item.command || "unknown";
        const commandVersion = item.command_version ? ` (${item.command_version})` : "";
        const stateSuffix = `graph=${graphState}; command=${command}${commandVersion}`;
        if (state === "integrated") {
          if (graphState === "present") {
            ctx.ok(`${name}: ${state}${location}; ${stateSuffix}`);
          } else {
            ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; check graph with tools/graphify.sh`);
          }
        } else if (state === "runnable") {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; install with: ${installCommand}`);
        } else if (state === "configured") {
          ctx.warn(
            `${name}: ${state}${location}; ${stateSuffix}; install command failed partially — check: ${installCommand}`,
          );
        } else if (state === "stale") {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; refresh with: ${installCommand}`);
        } else if (state === "missing") {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; install flow depends on harness presence`);
        } else {
          ctx.warn(`${name}: ${state}${location}; ${stateSuffix}; run: tools/agent-integrations.sh status --json`);
        }
      }
    },
  },

  // Enabled capability set. These two checks used to live in the repo gate
  // tools/check-manifest-integrity.sh, which could read the enabled set while it sat in
  // the tracked axon.toml. It now sits in the overlay, outside this repo, so
  // the machine-level checks belong to the machine-level tool. The gate keeps the
  // invariants that are intrinsic to the repo (every service.toml `requires =` resolves).
  {
    name: "Capabilities (enabled set)",
    async run(ctx) {
      const enabledCaps: string[] = Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [];
      if (!ctx.machineToml || Object.keys(ctx.machineToml).length === 0) {
        ctx.warn("skipped — no machine.toml to read");
      } else if (enabledCaps.length === 0) {
        ctx.warn("none enabled (tools/capability.sh enable <name>)");
      } else {
        const capRequires = new Map<string, string[]>();
        let allDirsPresent = true;
        for (const name of enabledCaps) {
          // Two roots, same as tools/lib/paths.sh's axon_manifest_for: public Axon holds
          // reusable capabilities, the active overlay holds deployment-specific ones.
          // Reported by root so a missing directory says which tree was searched, and
          // never by listing the overlay's contents — an overlay capability's name is a
          // fact about a private deployment.
          const rootDir = join(ctx.root, "capabilities", name);
          const overlayDir = join(ctx.overlayPath, "capabilities", name);
          const dir = existsSync(rootDir) ? rootDir : overlayDir;
          if (!existsSync(dir)) {
            ctx.bad(
              `'${name}' is enabled but exists in neither capabilities/${name}/ nor the overlay's`,
            );
            allDirsPresent = false;
            continue;
          }
          if (existsSync(rootDir) && existsSync(overlayDir)) {
            ctx.bad(`'${name}' is declared in both roots — rename one`);
            allDirsPresent = false;
            continue;
          }
          const svc = join(dir, "service.toml");
          if (existsSync(svc)) {
            const parsed = await readToml(svc);
            capRequires.set(name, Array.isArray(parsed?.requires) ? parsed.requires : []);
          } else {
            capRequires.set(name, []);
          }
        }
        if (allDirsPresent) ctx.ok(`${enabledCaps.length} enabled, every one a real capabilities/<name>/ dir in Axon or the overlay`);

        // Dependency closure: enabling X without what X requires is a machine that looks
        // configured and fails at start time. capability.sh resolves this on enable, but
        // hand-editing the list is legal, so it gets re-checked here.
        const enabledSet = new Set(enabledCaps);
        const missingDeps: string[] = [];
        for (const [name, reqs] of capRequires) {
          for (const dep of reqs) {
            if (!enabledSet.has(dep)) missingDeps.push(`${name} requires '${dep}', which is not enabled`);
          }
        }
        if (missingDeps.length === 0) ctx.ok("enabled set is dependency-closed");
        else for (const m of missingDeps) ctx.bad(m);
      }
    },
  },

  // Capabilities this machine CONSUMES from another overlay's deployment (retired-tracker#169).
  // The block above answers "what does this machine run"; this is the other half, and it lives
  // here rather than in a repo gate for the same reason the enabled set does — the declaration
  // is in the overlay, outside this repo.
  //
  // No URL is ever printed, matching the rule systems.toml already states for `doctor --online`:
  // a private endpoint resolved from the overlay has to survive being pasted into a report. The
  // id is enough to act on and names nothing.
  {
    name: "Capabilities (external references)",
    async run(ctx) {
      if (!ctx.machineToml || Object.keys(ctx.machineToml).length === 0) {
        ctx.warn("skipped — no machine.toml to read");
        return;
      }
      const perCapability = (ctx.machineToml.capability ?? {}) as Record<string, Record<string, unknown>>;
      const declared = Object.entries(perCapability)
        .filter(([, section]) => typeof section?.provided_by === "string" && section.provided_by !== "")
        .map(([name, section]) => [name, section.provided_by as string] as const);

      if (declared.length === 0) {
        ctx.ok("none declared — every capability here is locally managed");
        return;
      }

      const systemsPath = join(ctx.overlayPath, "config", "systems.local.toml");
      const systems = existsSync(systemsPath) ? ((await readToml(systemsPath)) ?? {}) : null;
      const enabled = new Set<string>(Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : []);

      for (const [name, providerId] of declared) {
        // Both at once is a contradiction rather than a preference to resolve: the runner would
        // hold a local copy up while every client dialled the remote one, and the two would
        // diverge in silence, each perfectly healthy on its own.
        if (enabled.has(name)) {
          ctx.bad(`'${name}' is enabled AND declared as provided by '${providerId}' — it cannot be both`);
          continue;
        }
        if (systems === null) {
          ctx.bad(`'${name}' names provider '${providerId}', but the overlay has no config/systems.local.toml`);
          continue;
        }
        const entry = systems[providerId] as Record<string, unknown> | undefined;
        const url = typeof entry?.url === "string" ? entry.url : "";
        if (!url) {
          ctx.bad(`'${name}' names provider '${providerId}', which has no url in config/systems.local.toml`);
          continue;
        }
        ctx.ok(`'${name}' resolves through systems.local.toml [${providerId}]`);
      }
    },
  },

  // Boot persistence for the autostart set. A capability could declare autostart = true, be
  // enabled, run fine all day, and simply be gone after the next reboot — because nothing ever
  // called install-persistence and nothing ever checked (#9). Delegated to service-runner.sh,
  // which owns the rule; doctor reports. Same shape as the tools/audit --expiry delegation.
  //
  // The second half is the inverse and is not cosmetic: watchdog.sh calls
  // `service-runner.sh start <cap>` every 30s and consults nothing about the enabled set, so a
  // unit left behind by `capability.sh disable` walks a disabled capability back up.
  {
    name: "Boot persistence (autostart + schedule set)",
    async run(ctx) {
      const runner = join(ctx.root, "tools", "service-runner.sh");
      if (!existsSync(runner)) {
        ctx.warn(`missing ${runner}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [runner, "persistence"], stdout: "pipe", stderr: "pipe" });
      const lines = proc.stdout.toString().trim().split("\n").filter(Boolean);
      if (lines.length === 0) {
        ctx.warn("service-runner.sh persistence returned nothing — persistence state unverified");
      }
      let owed = 0;
      for (const line of lines) {
        const [name, state, detail] = line.split("\t");
        switch (state) {
          case "installed":
          case "n/a":
            break;
          case "missing":
            // "owes a unit", not "declares autostart": a capability declaring `schedule` owes one
            // too, and for that one the unit is not a safety net against reboots — it is the only
            // thing that ever runs it.
            ctx.bad(`'${name}' owes a supervisor unit and has none installed — it will not run after a reboot (tools/service-runner.sh install-persistence ${name})`);
            owed++;
            break;
          case "misdeclared":
            // bad, not warn: a manifest claiming both autostart and schedule can never have
            // persistence installed at all, so there is no degraded mode to keep running in.
            ctx.bad(`'${name}': ${detail}`);
            owed++;
            break;
          case "stale":
            ctx.warn(`'${name}': ${detail}`);
            owed++;
            break;
          case "installed-not-loaded":
            ctx.warn(`'${name}': ${detail}`);
            owed++;
            break;
          case "unsupported":
            ctx.warn(`'${name}': ${detail}`);
            break;
          default:
            ctx.warn(`'${name}': unexpected persistence state '${state}'`);
            break;
        }
      }

      // Units for capabilities this machine does not enable. Names come from the unit filenames,
      // which are Axon's own (com.axon.<cap> / axon-<cap>.service), never from listing the
      // overlay — an overlay capability's name is a fact about a private deployment.
      const enabled = new Set<string>(Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : []);
      const os = ctx.machineToml?.os;
      const home = process.env.HOME ?? "";
      const unitDir =
        os === "macos" ? join(home, "Library", "LaunchAgents")
        : os === "linux" ? join(process.env.XDG_CONFIG_HOME ?? join(home, ".config"), "systemd", "user")
        : "";
      // Names that legitimately own a unit while being absent from the enabled set. A spine
      // component is one (tools/lib/paths.sh's axon_manifest_for reads its manifest from the
      // repo root rather than capabilities/), and so is anything that component declares as a
      // `sidecars` entry — a unit it owns that has no manifest of its own because Axon does
      // not own the program.
      //
      // Derived from the manifests rather than from a literal. This used to be `cap !==
      // "dashboard"`, which flagged the dashboard's own macmon sidecar and told the operator
      // to remove the unit its Systems page polls (#65). A second hardcoded name would have
      // fixed that instance and left the next one.
      const exempt = new Set<string>();
      for (const entry of readdirSync(ctx.root, { withFileTypes: true })) {
        if (!entry.isDirectory()) continue;
        const svc = join(ctx.root, entry.name, "service.toml");
        if (!existsSync(svc)) continue;
        exempt.add(entry.name);
        const parsed = await readToml(svc);
        if (Array.isArray(parsed?.sidecars)) {
          for (const s of parsed.sidecars) if (typeof s === "string" && s) exempt.add(s);
        }
      }

      const orphans: string[] = [];
      if (unitDir && existsSync(unitDir)) {
        for (const f of readdirSync(unitDir)) {
          const cap =
            os === "macos" ? (f.startsWith("com.axon.") && f.endsWith(".plist") ? f.slice("com.axon.".length, -".plist".length) : null)
            : (f.startsWith("axon-") && f.endsWith(".service") ? f.slice("axon-".length, -".service".length) : null);
          if (cap && !exempt.has(cap) && !enabled.has(cap)) orphans.push(cap);
        }
      }
      for (const cap of orphans.sort()) {
        ctx.warn(`persistence is installed for '${cap}', which this machine does not enable — its watchdog will start it anyway (tools/service-runner.sh remove-persistence ${cap})`);
      }

      if (owed === 0 && orphans.length === 0 && lines.length > 0) {
        ctx.ok(`${lines.length} enabled capabilities checked, persistence matches the declaration`);
      }
    },
  },

  {
    // The shared SQLite database (PRD Q45, 2026-08-27). Every capability's tables are in one
    // file, so "is it there and does it open" is a machine-level question with one answer,
    // which is what makes it a doctor check rather than nine readiness handlers.
    //
    // Resolved exactly as `axon_config::database_path()` resolves it — AXON_DB_PATH first,
    // then the overlay — because a doctor that checked a different file than the capabilities
    // open would report on nothing. Absent is a WARNING, not a failure: a machine that has
    // never run a capability legitimately has no database yet, and `axon_store::pool_for`
    // creates it on first open.
    name: "Shared store (SQLite)",
    run(ctx) {
      const envPath = (process.env.AXON_DB_PATH ?? "").trim();
      const dbPath = envPath ? expandHome(envPath) : join(ctx.overlayPath, "data", "axon", "axon.db");
      const from = envPath ? "AXON_DB_PATH" : "overlay default";
      if (!ctx.overlayPath && !envPath) return ctx.warn("skipped — no overlay to resolve the database path from");
      if (!existsSync(dbPath)) {
        ctx.warn(`no database at ${dbPath} (${from}) — created on the first write by any capability`);
        return;
      }

      // `integrity_check` on the LIVE file, and this one is a read: sqlite3 opens it
      // read-only through the URI, so a check cannot journal or write to the database every
      // capability is using. Bounded to the first line — a corrupt database answers with a
      // list, and the first entry is the one worth reporting.
      const proc = Bun.spawnSync({
        cmd: ["sqlite3", `file:${dbPath}?mode=ro`, "pragma integrity_check;"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (proc.exitCode === null || proc.exitCode !== 0) {
        const why = proc.stderr.toString().trim() || "sqlite3 is not on PATH";
        ctx.bad(`${dbPath} could not be read: ${why}`);
      } else {
        const first = proc.stdout.toString().trim().split("\n")[0] ?? "";
        if (first === "ok") ctx.ok(`${dbPath} (${from}) — integrity_check ok`);
        else ctx.bad(`${dbPath} failed integrity_check: ${first}`);
      }

      // The file has a backup contract, and the contract only reaches a backup surface
      // through the registry. A machine holding the database without `store` enabled has a
      // database nothing will ever back up — which is exactly the state a cutover from the
      // retired postgres capability leaves behind if only half of it is done.
      const enabled: string[] = Array.isArray(ctx.machineToml?.capabilities) ? ctx.machineToml.capabilities : [];
      if (!enabled.includes("store")) {
        ctx.warn(
          "'store' is not in this machine's enabled set — the database exists and no backup contract covers it (tools/capability.sh enable store)",
        );
      }
    },
  },

  {
    name: "State mounts",
    run(ctx) {
      ctx.mounts = ctx.machineToml?.state_mount ?? [];
      if (ctx.mounts.length === 0) {
        ctx.warn("none declared");
      } else {
        for (const m of ctx.mounts) {
          const p = expandHome(m.path);
          if (existsSync(p)) ctx.ok(`${m.tool} — ${p}`);
          else ctx.bad(`${m.tool} — ${p} missing`);
        }
      }
    },
  },

  // Capability env contract. Every env-backed capability in `capabilities/` must ship a
  // tracked `<name>.env.example` next to service.toml so non-secret defaults are versioned while
  // secret-bearing values remain private. Axon#188 owns the public contract and gate.
  //
  // The repo root only, deliberately, unlike the bind-policy check below. This gate exists
  // so a stranger cloning public Axon can see which variables a capability needs without
  // any of their values. An overlay capability has no such reader: the repository holding
  // it is already private, and its real env file lives beside it. Widening this check
  // would demand a template whose only audience already has the original.
  {
    name: "Capability env templates (public/private split)",
    async run(ctx) {
      const capsDir = join(ctx.root, "capabilities");
      if (!existsSync(capsDir)) return ctx.warn("no capabilities/ dir");
      let checked = 0;
      let foundTemplate = 0;
      for (const capDirEntry of readdirSync(capsDir, { withFileTypes: true })) {
        if (!capDirEntry.isDirectory()) continue;
        const capDir = join(capsDir, capDirEntry.name);
        const svc = join(capDir, "service.toml");
        if (!existsSync(svc)) continue;
        const parsed = await readToml(svc);
        const envFile = parsed?.env_file;
        if (typeof envFile !== "string" || !envFile.trim()) continue;
        checked += 1;
        const envBase = basename(envFile);
        if (!envBase.endsWith(".env")) {
          ctx.warn(`capabilities/${capDirEntry.name}: env_file should probably end with .env`);
          continue;
        }
        const template = join(capDir, `${envBase}.example`);
        if (!existsSync(template)) {
          ctx.bad(`capabilities/${capDirEntry.name}: missing ${envBase}.example for env-backed service`);
          continue;
        }
        foundTemplate += 1;
        let text: string;
        try {
          text = readFileSync(template, "utf8");
        } catch {
          ctx.bad(`capabilities/${capDirEntry.name}: cannot read ${envBase}.example`);
          continue;
        }
        const leaks = findPlaintextSecretsInEnvTemplate(text);
        if (leaks.length > 0) {
          for (const key of leaks) {
            ctx.bad(
              `capabilities/${capDirEntry.name}: ${envBase}.example contains raw-looking secret-like value for ${key} (use placeholders only)`,
            );
          }
        }
      }
      if (checked === 0) ctx.ok("no env_file-backed capabilities found");
      else if (foundTemplate === checked) ctx.ok(`${foundTemplate}/${checked} env-backed capabilities ship .env.example`);
      else ctx.bad(`${foundTemplate}/${checked} env-backed capabilities ship .env.example`);
    },
  },

  // systems.toml — coverage + undeclared-connection sweep. Read-only,
  // offline, mechanical: cross-reference against machine.toml's state_mount
  // (a system declared local="yes" ought to have a monitored path somewhere)
  // and grep this repo's own tracked files for hardcoded sibling-system paths
  // that bypass tools/lib/paths.sh's indirection — the two classes of drift
  // that hand-authored manifests silently accumulate (see
  // README.md#documentation-stays-owned-and-current for the
  // discovered-in-the-wild example: a sync script's env-overridable default
  // path diverging from the mount its own machine.toml declared).
  {
    name: "Systems (systems.toml)",
    async run(ctx) {
      const systemsTomlPath = join(ctx.root, "systems.toml");
      if (!existsSync(systemsTomlPath)) {
        ctx.warn("no systems.toml — skipped");
      } else {
        ctx.systemsToml = await readToml(systemsTomlPath);
        const systemIds = new Set(Object.keys(ctx.systemsToml));
        // Direction that's actually meaningful: machine.toml's [[state_mount]] is the
        // narrower, path-bearing list (README.md#one-manifest-per-concern — "one manifest per concern");
        // systems.toml is the broader identity/why registry. A mount with no
        // matching identity entry is a real gap (doctor already flags via `bad`
        // below). The reverse is NOT generally a gap — most local=yes systems
        // (tools, services, projects with no persisted state Axon backs up) never
        // need a mount by design, so flagging every one would just be noise.
        const { covered, uncovered } = checkStateMountCoverage(ctx.mounts, systemIds);
        for (const tool of covered) ctx.ok(`${tool} — state_mount has a matching systems.toml identity`);
        for (const tool of uncovered) ctx.bad(`${tool} — machine.toml [[state_mount]] with no systems.toml entry — undeclared system`);
        const localCount = Object.values(ctx.systemsToml).filter((e: any) => e?.local === "yes").length;
        const mountedCount = [...systemIds].filter((id) => ctx.mounts.some((m) => m.tool === id)).length;
        ctx.ok(`${mountedCount}/${localCount} local="yes" systems have a state_mount (rest are mount-less by design — tools/services with no persisted state)`);
      }
    },
  },

  // systems.toml reachability. The section above asks whether a system is DECLARED; this one asks
  // whether the declared endpoint answers. Both questions matter and neither substitutes for the
  // other: a complete manifest pointing at a dead host reports perfectly green without this.
  //
  // --online only, and the offline path does no network work at all — not a shortened timeout,
  // not a DNS lookup. `tools/doctor` is the thing an operator runs on a machine with no route out,
  // and a check that quietly dials in that state makes the whole report slow and unreliable
  // exactly where it is most needed.
  //
  // What it never prints: the URL. Not for a private system, not for a public one. The result
  // names the system id, the outcome, and the HTTP status when there was one, which is the bounded
  // evidence the declaration is entitled to. Printing a resolved endpoint would put a private
  // hostname into a report an operator pastes into an issue, and getting that right per-entry is a
  // rule someone eventually forgets. Cheaper to never print any.
  {
    name: "Systems reachability (--online)",
    async run(ctx) {
      if (!ctx.online) {
        ctx.ok("skipped — run 'tools/doctor --online' to probe declared endpoints");
        return;
      }
      if (!ctx.systemsToml || Object.keys(ctx.systemsToml).length === 0) {
        ctx.warn("no systems.toml entries — nothing to probe");
        return;
      }
      let overlaySystems: Record<string, any> = {};
      if (ctx.overlayPath) {
        const overlaySystemsPath = join(ctx.overlayPath, "config", "systems.local.toml");
        if (existsSync(overlaySystemsPath)) {
          try {
            overlaySystems = await readToml(overlaySystemsPath);
          } catch {
            // An unparseable overlay manifest is the overlay's finding, not this check's. Say the
            // probe set is incomplete rather than reporting every private system as unreachable,
            // which is the same false-red a missing file would produce.
            ctx.warn("overlay systems.local.toml is unreadable — private endpoints not resolved");
          }
        }
      }

      const targets = resolveProbeTargets(ctx.systemsToml, overlaySystems);
      const probed = targets.filter((t): t is { id: string; url: string; timeoutMs: number } => "url" in t);
      const skipped = targets.filter((t): t is { id: string; skip: string } => "skip" in t);

      // Concurrent, because these are independent network waits and a serial sweep would make the
      // section's cost the SUM of every timeout rather than the worst one.
      const results = await Promise.all(
        probed.map(async ({ id, url, timeoutMs }) => {
          // Resolve first, connect second. Bun's fetch reports the same ConnectionRefused code for
          // a refused socket and a nonexistent hostname, so without this step "the service is
          // stopped" and "the name is gone" arrive as one answer — and they are the two findings
          // an operator would act on most differently. An IP literal resolves trivially, so this
          // costs nothing for a LAN address.
          try {
            await dnsLookup(new URL(url).hostname);
          } catch {
            return { id, outcome: "unavailable" as ProbeOutcome, status: 0, timeoutMs };
          }
          try {
            const res = await fetch(url, {
              method: "HEAD",
              redirect: "manual",
              signal: AbortSignal.timeout(timeoutMs),
            });
            return { id, outcome: "reachable" as const, status: res.status, timeoutMs };
          } catch (err) {
            return { id, outcome: classifyProbeOutcome(err), status: 0, timeoutMs };
          }
        }),
      );

      for (const r of results) {
        // Any HTTP response proves the service answered. A 401 or a 403 is a reachable service
        // declining an unauthenticated HEAD, which is the correct behaviour for most of these and
        // must not read as an outage — this check asks "is it up", never "am I allowed in".
        if (r.outcome === "reachable") ctx.ok(`${r.id} — reachable (HTTP ${r.status})`);
        else if (r.outcome === "refused") ctx.bad(`${r.id} — connection refused`);
        else if (r.outcome === "timeout") ctx.warn(`${r.id} — no answer within ${r.timeoutMs}ms (overlay probe_timeout_ms raises it)`);
        else ctx.bad(`${r.id} — unavailable (no route, DNS failure, or TLS error)`);
      }
      for (const s of skipped) ctx.ok(`${s.id} — skipped: ${s.skip}`);
      if (probed.length === 0) ctx.warn("no probeable endpoint among the declared systems");
    },
  },

  // Undeclared-connection sweep: every declared canonical path (state mounts +
  // AXON_ROOT + overlay) vs. every hardcoded sibling-repo path actually
  // committed in this tree. tools/lib/paths.sh is the sanctioned indirection
  // (AXON_ROOT / AXON_PERSONAL_ROOT); anything else hardcoding a path to a
  // declared system, or referencing a $HOME path to a system with NO
  // systems.toml entry at all, is exactly the kind of drift systems.toml can't
  // see by construction (it's hand-authored, so it only knows what someone
  // remembered to add).
  {
    name: "Undeclared connections (grep sweep)",
    async run(ctx) {
      const declaredIds = new Set(Object.keys(ctx.systemsToml));
      const declaredPaths = new Set<string>([ctx.root, ctx.overlayPath].filter(Boolean));
      for (const m of ctx.mounts) declaredPaths.add(expandHome(m.path));

      // A path the overlay declares as protected is a DENY rule, not a connection (Axon#147).
      // This sweep's premise is that a hardcoded sibling path implies an undeclared
      // integration, and for a protection zone that inference runs backwards: the path is
      // named precisely so that nothing integrates with it. Demanding a systems.toml entry
      // for it would be asking the operator to declare a connection to the one place they
      // declared off limits. Read here rather than threaded through ctx — this is its only
      // consumer, and the file is the overlay's, not Axon's.
      const protectedNames = new Set<string>();
      if (ctx.overlayPath) {
        const zonesPath = join(ctx.overlayPath, "config", "protection-zones.toml");
        if (existsSync(zonesPath)) {
          try {
            for (const m of readFileSync(zonesPath, "utf8").matchAll(/"([^"]+)"/g)) {
              const base = basename(m[1].replace(/\/+$/, ""));
              if (base) protectedNames.add(base);
            }
          } catch {
            // An unreadable policy is tools/protection-zones' finding to report, not this
            // sweep's. Saying it twice in two vocabularies teaches a reader to skim both.
          }
        }
      }

      const lsFiles = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "ls-files"], stdout: "pipe", stderr: "pipe" });
      if (lsFiles.exitCode !== 0) {
        ctx.warn("git ls-files failed — skipping sweep");
      } else {
        const files = lsFiles.stdout.toString().split("\n").filter(Boolean);
        const hits = new Map<string, Set<string>>(); // repo-name -> files referencing it
        for (const rel of files) {
          const abs = join(ctx.root, rel);
          let text: string;
          try {
            text = await Bun.file(abs).text();
          } catch {
            continue; // binary or unreadable — not a path-reference source
          }
          // Exemptions are a property of the file, not a list of names — see
          // isSweepExempt. Each skip it keeps has a reason no property can express:
          //
          //   tools/lib/paths.sh    the sanctioned indirection itself.
          //   tools/install.sh      the bootstrap namer. It shows the suggested overlay
          //                         path for each recognized boundary and writes the chosen
          //                         one into axon.local.toml, all before paths.sh can
          //                         resolve anything.
          //   tools/doctor.test.ts  its sibling-repo paths are ARGUMENTS to
          //                         extractSiblingRepoRefs, the very function this sweep
          //                         runs — the specification of what a reference looks
          //                         like, not a reference. Reading them as one made the
          //                         sweep report its own fixture permanently, which is the
          //                         kind of finding that teaches a reader to stop reading
          //                         the findings.
          //
          // axon.toml is deliberately NOT exempt: since the state mounts moved to the
          // overlay it holds no paths of its own, so a hardcoded one appearing there is a
          // real regression that should be reported, not hidden.
          if (isSweepExempt(rel, text)) continue;
          for (const name of extractSiblingRepoRefs(text)) {
            // The checkout and selected overlay are self-references. Any other hardcoded
            // sibling path is a real undeclared connection.
            if (name === basename(ctx.root) || (ctx.overlayPath && name === basename(ctx.overlayPath))) continue;
            if (!hits.has(name)) hits.set(name, new Set());
            hits.get(name)!.add(rel);
          }
        }
        if (hits.size === 0) {
          ctx.ok("no hardcoded sibling-repo paths found outside tools/lib/paths.sh");
        } else {
          for (const [name, refFiles] of hits) {
            const slug = name.toLowerCase();
            if (protectedNames.has(name)) {
              ctx.ok(`${name} — declared a protected path in the overlay's protection-zones.toml; a deny rule, not a connection`);
            } else if (declaredIds.has(slug)) {
              ctx.warn(`${name} — hardcoded path in ${[...refFiles].join(", ")} (declared in systems.toml as '${slug}', but bypasses paths.sh indirection)`);
            } else {
              ctx.bad(`${name} — hardcoded path in ${[...refFiles].join(", ")}, no matching systems.toml entry — undeclared connection`);
            }
          }
        }
      }
    },
  },

  // Server bind policy. libs/axon-server exists so a capability server cannot
  // bind the LAN or skip the AXON_PORT contract by accident, and its README said
  // so while two servers contradicted it: scout-server bound 0.0.0.0 with
  // permissive CORS behind a mutating POST, and comms-server hand-rolled its
  // startup. A README claiming a guarantee nothing enforces is worse than no
  // claim, so the guarantee gets a check.
  //
  // Lives in doctor rather than a repo gate for the same reason the decision path-rot
  // sweep does (README.md#documentation-stays-owned-and-current): half the servers it has to
  // cover are in the overlay, outside this repo, and a gate that only globs Axon would report
  // a clean bind policy while an overlay server binds the LAN. doctor reads both real trees.
  {
    name: "Server bind policy (axon-server)",
    run(ctx) {
      // Both roots. This is a security gate, not a public-code style rule: a server the
      // overlay owns can bind 0.0.0.0 just as wrongly as one in Axon, and it would be
      // the more dangerous of the two. Findings print to the terminal only, so naming an
      // overlay capability here does not put it in a tracked artifact.
      const roots = [
        { dir: join(ctx.root, "capabilities"), label: "capabilities" },
        { dir: join(ctx.overlayPath, "capabilities"), label: "overlay capabilities" },
      ];
      const rootCaps = join(ctx.root, "capabilities");
      if (!existsSync(rootCaps)) return ctx.warn("no capabilities/ dir");
      let checked = 0;
      let offenders = 0;
      for (const { dir: capsDir, label } of roots) {
        if (!existsSync(capsDir)) continue; // an overlay need not own any capability
        for (const cap of readdirSync(capsDir, { withFileTypes: true })) {
          if (!cap.isDirectory()) continue;
          const srcDir = join(capsDir, cap.name, "src");
          if (!existsSync(srcDir)) continue;
          for (const path of findRustSources(srcDir)) {
            const sourcePath = relative(srcDir, path);
            const text = readFileSync(path, "utf8");
            const production = stripRustCfgTestItems(text);
            if (!/\bRouter::new\s*\(/.test(production)) continue; // not a server root
            checked++;
            const hand = findProductionListenerConstructs(text);
            if (hand.length === 0) {
              // Either entry point: `serve_local` is loopback with the deployment's
              // inbound token, `serve` spells the reach and the gate out (comms passes
              // its own `api_secret_file` token). Both go through the same bind and the
              // same middleware, so accepting only the first would flag a correct server.
              if (!/axon_server::serve(_local)?\s*\(/.test(production)) {
                ctx.warn(`${label}/${cap.name}/src/${sourcePath} builds a Router but neither serves it nor uses axon_server`);
              }
              continue;
            }
            offenders++;
            ctx.bad(`${label}/${cap.name}/src/${sourcePath} binds its own listener — use axon_server::serve_local (loopback + port contract)`);
          }
        }
      }
      if (checked === 0) return ctx.bad("no capability server sources found — this check is looking in the wrong place");
      if (offenders === 0) ctx.ok(`${checked} capability server(s) across both roots, none binds by hand`);
    },
  },

  // Packs — Claude Code deployment state.
  //
  // This used to assert that every destination was a SYMLINK into Packs/. It stopped being
  // true on 2026-08-09, when deployment changed to copy-with-a-ledger (principal: "we should
  // only deploy from axon overlays never using symlinks"), and the check was not updated. The
  // result was eight hard failures reading "occupied by a non-symlink" against skills that
  // packs-claude reported as `current` — doctor calling correct state broken, which is worse
  // than not checking at all, because it trains you to ignore the report.
  //
  // It now reads the same ledger the deployer writes, exactly as the Codex section below does.
  // One source of truth, and it cannot drift from the tool again.
  {
    name: "Packs (Claude Code materialized)",
    run(ctx) {
      try {
        const rows = getStatuses(defaultClaudeDeployConfig());
        if (rows.length === 0) {
          ctx.warn("no Packs/*/pack.toml found");
          return;
        }
        for (const row of rows) {
          const label = `${row.pack}/${row.skill}`;
          const detail = row.detail ? ` — ${row.detail}` : "";
          switch (row.status) {
            case "current":
              ctx.ok(`${label} current`);
              break;
            case "not-deployed":
              ctx.warn(`${label} not deployed (tools/packs.sh link ${row.pack})`);
              break;
            case "outdated":
              ctx.warn(`${label} outdated (tools/packs-claude sync ${row.pack})${detail}`);
              break;
            case "drifted":
              ctx.bad(`${label} has destination-side changes; sync/remove will refuse`);
              break;
            case "migration-required":
              ctx.warn(`${label} needs generated-artifact ledger migration${detail}`);
              break;
            case "missing":
              ctx.bad(`${label} is ledger-owned but missing from the Claude skill root`);
              break;
            case "collision":
              ctx.bad(`${label} destination is occupied by an unowned skill (tools/packs-claude adopt ${row.pack} if it is identical)`);
              break;
            case "invalid":
              ctx.bad(`${label} invalid${detail}`);
              break;
          }
        }
      } catch (error) {
        ctx.bad(`Claude Pack state unreadable: ${(error as Error).message}`);
      }
    },
  },

  // Codex copies are owned through packs-codex's ledger; getStatuses is the
  // shared read-only source of truth for current/outdated/drift/collision state.
  {
    name: "Packs (Codex materialized)",
    run(ctx) {
      try {
        const rows = getStatuses(defaultCodexDeployConfig());
        if (rows.length === 0) {
          ctx.warn("no Packs/*/pack.toml found");
        } else {
          for (const row of rows) {
            const label = `${row.pack}/${row.skill}`;
            const detail = row.detail ? ` — ${row.detail}` : "";
            switch (row.status) {
              case "current":
                ctx.ok(`${label} current`);
                break;
              case "not-deployed":
                ctx.warn(`${label} not deployed (tools/packs-codex deploy ${row.pack})`);
                break;
              case "outdated":
                ctx.warn(`${label} outdated (tools/packs-codex sync ${row.pack})${detail}`);
                break;
              case "drifted":
                ctx.bad(`${label} has destination-side changes; sync/remove will refuse`);
                break;
              case "migration-required":
                ctx.warn(
                  `${label} needs generated-artifact ledger migration ` +
                    `(tools/packs-codex migrate-generated ${row.pack} --accept-current)${detail}`,
                );
                break;
              case "missing":
                ctx.bad(`${label} is ledger-owned but missing from the Codex skill root`);
                break;
              case "collision":
                ctx.bad(`${label} destination is occupied by an unowned skill`);
                break;
              case "invalid":
                ctx.bad(`${label} invalid${detail}`);
                break;
            }
          }
        }
      } catch (error) {
        ctx.bad(`Codex Pack state unreadable: ${(error as Error).message}`);
      }
    },
  },

  // Decision freshness — do the entries still describe this tree? See
  // findDecisionPathRot above for why this is here and not a repo gate.
  {
    name: "Doctrine freshness (README why-blocks)",
    run(ctx) {
      try {
        // README.md#decisions-live-with-their-owner: a call's reasoning lives beside the thing it governs, under `## Why this shape:`.
        // This checks the prose still matches the tree -- every path a block names must resolve, and
        // every path it declares deliberately absent must stay absent.
        const whyBlocks: Array<{ slug: string; text: string; assertsAbsent: string[]; dir: string }> = [];
        const tracked = gitOut("ls-files").split("\n");
        for (const f of tracked) {
          if (!f.endsWith(".md") || !existsSync(join(ctx.root, f))) continue;
          whyBlocks.push(...collectWhyBlocks(f, readFileSync(join(ctx.root, f), "utf8")));
        }
        // Derived from the tree, not listed (Axon#26). The old list named three parents
        // plus tools/, which left dashboard/ and schemas/ resolving against nothing, and
        // appended a `<unit>/src/` base for every unit whether or not it had one — a
        // prefix nothing could ever resolve under. Both are the same mistake: a hand-list
        // standing in for what `git ls-files` already says.
        const bases = whyBlockBases(tracked);
        const overlay = process.env.AXON_PERSONAL_ROOT ?? "";
        const rot = findDecisionPathRot(
          whyBlocks,
          (p) => existsSync(join(ctx.root, p)) || (overlay !== "" && existsSync(join(overlay, p))),
          bases,
        );

        // decisions/ was dissolved on 2026-07-28 (README.md#decisions-live-with-their-owner). Nothing may cite it again: an empty slug
        // set makes every `decisions/<slug>` reference a finding, which is the guard against the
        // folder quietly coming back one entry at a time.
        const dangling = findDanglingDecisionRefs(
          tracked
            .filter((f) => f && existsSync(join(ctx.root, f)) &&
              !/\.(test|spec)\.[tj]s$/.test(f) && !f.endsWith("test.sh"))
            .map((f) => ({ path: f, text: readFileSync(join(ctx.root, f), "utf8") })),
          () => false,
        );

        for (const r of rot) {
          if (r.kind === "missing") ctx.bad(`${r.slug} names ${r.path}, which no longer exists`);
          else ctx.bad(`${r.slug} declares ${r.path} absent, but it exists now`);
        }
        for (const d of dangling) ctx.bad(`${d.file} cites decisions/${d.slug}; that directory was dissolved (README.md#decisions-live-with-their-owner)`);

        if (rot.length === 0 && dangling.length === 0) {
          ctx.ok(`${whyBlocks.length} why-blocks — every named path resolves, every asserted absence holds`);
        } else {
          console.log(
            "  → repair the paths if the reasoning still holds, delete it if it is spent, or mark the\n" +
            "    absence deliberate with <!-- asserts-absent: <path> --> inside the block",
          );
        }
      } catch (error) {
        ctx.bad(`doctrine sweep failed: ${(error as Error).message}`);
      }
    },
  },

  // self.json freshness — the committed self-model vs the working tree. Stale is a warn,
  // not a fail: an out-of-date self-model is a regenerate away and never breaks a build.
  {
    name: "Self-model freshness (self.json)",
    run(ctx) {
      const selfPath = join(ctx.root, "tools", "self");
      if (!existsSync(selfPath)) {
        ctx.warn(`missing ${selfPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [selfPath, "check"], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) ctx.warn("self.json is stale — run: tools/self generate");
    },
  },

  // Architecture-generator inputs — delegate to tools/check-generator-inputs-tracked.sh.
  // Needs `git ls-files` and the real checkout, which is what doctor already has. Same
  // delegation shape as tools/audit --expiry below: doctor reports, the script owns the rule.
  {
    name: "Architecture-generator input visibility (Axon#30)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "check-generator-inputs-tracked.sh");
      if (!existsSync(checkPath)) {
        ctx.warn(`missing ${checkPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [checkPath], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) {
        ctx.bad("the architecture generator reads an input others cannot see — see above");
      }
    },
  },

  // Public-checkout hygiene — delegate to the index scanner so Doctor and CI enforce
  // the same rule. It inspects tracked blob contents, including binary metadata, rather
  // than pretending .gitignore can remove a file that is already in the index.
  {
    name: "Publication hygiene (tracked tree)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "check-publication-hygiene.sh");
      if (!existsSync(checkPath)) {
        ctx.warn(`missing ${checkPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [checkPath], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) ctx.bad("tracked content is not safe for a public checkout — see above");
    },
  },

  // UI type-check coverage — delegate to tools/discover-ui-packages, the same tool the CI
  // job loops over, so "which UIs are checked" has one answer locally and in CI. Doctor
  // runs discovery only, never the checks themselves: a `bun install` per package is a
  // network operation, and doctor is the fast local sweep. What it catches is the class
  // CI cannot — a UI that would be dropped from coverage, before the commit that drops it.
  {
    name: "UI type-check coverage (Axon#139)",
    run(ctx) {
      const checkPath = join(ctx.root, "tools", "discover-ui-packages");
      if (!existsSync(checkPath)) {
        ctx.warn(`missing ${checkPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [checkPath], stdout: "pipe", stderr: "pipe" });
      const out = [proc.stdout.toString().trim(), proc.stderr.toString().trim()]
        .filter(Boolean)
        .join("\n");
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) ctx.bad("a UI package declares a surface CI cannot type-check — see above");
    },
  },

  // There was an "Upstream audit" check here until 2026-08-28. It delegated to
  // tools/upstream-checker, which PRD Q41 retired along with the rest of the homegrown
  // freshness stack; Renovate reports drift on GitHub now (renovate.json5).
  //
  // Nothing replaces it in this file, and that is the honest outcome rather than a gap
  // worth papering over. doctor answers "is this machine healthy", offline, in a second.
  // "Is a dependency behind" needs the network and a registry per ecosystem — it was
  // always the check here that could not answer offline, and it spent most of its life
  // printing a manifest-format result under a supply-chain heading. What it did answer
  // without the network — that every entry has a verdict and a pin — is doctrine about a
  // documentation file, and README.md#dependency-verdicts-and-provenance now says plainly
  // that a human owns it.

  // Accepted-finding policies — delegate to tools/audit --expiry, don't reimplement.
  //
  // Every finding Axon accepts is accepted UNTIL a date — osv-scanner.toml's ignoreUntil.
  // trivy-ignore/*.txt carried the same until 52aa8c5 (2026-08-28) retired the postgres
  // capability and deleted the last policy in that directory.
  // trivy and osv-scanner each enforce their own dates silently at scan time, so without this
  // the first notice that a decision lapsed is a red build — and the only thing that ever
  // "tracked" the re-decision was one GitHub issue per image whose entire content was "re-scan
  // when the upstream digest changes". The files already carry the dates; a ticket restating
  // them was waiting dressed up as work, and this is where that belongs instead.
  //
  // --expiry reads the files and runs no scanner, so this costs no image pull, no container
  // runtime and no network — it answers offline exactly like the manifest checks above.
  {
    name: "Accepted-finding policies (tools/audit --expiry)",
    run(ctx) {
      const auditPath = join(ctx.root, "tools", "audit");
      if (!existsSync(auditPath)) {
        ctx.warn(`missing ${auditPath}`);
        return;
      }
      const proc = Bun.spawnSync({ cmd: [auditPath, "--expiry"], stdout: "pipe", stderr: "pipe" });
      const out = proc.stdout.toString().trim();
      if (out) console.log(out.split("\n").map((l) => `  ${l}`).join("\n"));
      if (proc.exitCode !== 0) {
        ctx.bad("an accepted-finding policy has lapsed — the scanner is reporting those findings again");
      } else if (out.includes("needing a re-decision")) {
        // warn, not bad: a date that is merely close is notice, not breakage. Failing here
        // would hold the whole of doctor red for a fortnight before anything is wrong.
        ctx.warn("an accepted-finding policy needs a re-decision soon — see above");
      }
    },
  },

  // Repo freshness — is this checkout, on any deployment host, behind origin/main. `--online`
  // fetches first for a live answer; offline reads whatever origin/main ref
  // was last fetched, best-effort. This is the only check left that the flag makes
  // network-dependent apart from the systems probe above — the upstream audit was the
  // third until 2026-08-28. Being behind isn't broken — `warn`, not
  // `bad` — but it's the one thing nothing previously surfaced at all: a
  // multi-host Axon deployments otherwise have no way to
  // tell a stale checkout from a current one short of eyeballing `git log`.
  {
    name: "Repo freshness (origin/main)",
    run(ctx) {
      if (ctx.online) {
        Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "fetch", "--quiet", "origin", "main"], stdout: "pipe", stderr: "pipe" });
      }
      const revList = Bun.spawnSync({
        cmd: ["git", "-C", ctx.root, "rev-list", "--left-right", "--count", "HEAD...origin/main"],
        stdout: "pipe",
        stderr: "pipe",
      });
      if (revList.exitCode !== 0) {
        ctx.warn("no origin/main ref cached — run 'tools/doctor --online' (or 'git fetch') to check freshness");
      } else {
        const [aheadStr, behindStr] = revList.stdout.toString().trim().split(/\s+/);
        const ahead = Number(aheadStr) || 0;
        const behind = Number(behindStr) || 0;
        if (ahead === 0 && behind === 0) ctx.ok("up to date with origin/main");
        else if (behind > 0 && ahead === 0) ctx.warn(`${behind} commit(s) behind origin/main — run tools/update.sh`);
        else if (ahead > 0 && behind === 0) ctx.ok(`${ahead} commit(s) ahead of origin/main — push when ready`);
        else ctx.warn(`diverged from origin/main (${ahead} ahead, ${behind} behind) — merge before tools/update.sh`);
      }
    },
  },

  // Session orientation — the dynamic, always-current answer to "what's
  // the state of this checkout right now," replacing a hand-maintained status
  // doc (README.md#documentation-stays-owned-and-current forbids adding one: nothing executable
  // would reference it). Point of this section: a fresh agent session in this repo runs
  // `tools/doctor` first and gets branch/HEAD/dirty-file-count for free,
  // instead of a static file someone has to remember to update.
  {
    name: "Session orientation",
    run(ctx) {
      ctx.ok(`version: ${formatVersion(gitOut("describe", "--tags", "--always", "--dirty", "--match", RELEASE_TAG_GLOB), gitOut("log", "-1", "--format=%cs"))}`);
      const branchProc = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "branch", "--show-current"], stdout: "pipe" });
      const branch = branchProc.stdout.toString().trim() || "(detached HEAD)";
      const headProc = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "log", "-1", "--format=%h %s (%cr)"], stdout: "pipe" });
      ctx.ok(`${branch} @ ${headProc.stdout.toString().trim()}`);
      const dirtyProc = Bun.spawnSync({ cmd: ["git", "-C", ctx.root, "status", "--porcelain"], stdout: "pipe" });
      const dirtyCount = dirtyProc.stdout.toString().split("\n").filter(Boolean).length;
      if (dirtyCount === 0) ctx.ok("working tree clean");
      else ctx.warn(`${dirtyCount} uncommitted change(s) — git status for detail`);
      // The backlog is ISAs (README.md#the-backlog-is-isas), so this counts unchecked claims
      // across every tracked ISA.md rather than naming a tracker. Derived, not remembered: a
      // hand-maintained number here would be the status doc this section exists to replace.
      // `--others --exclude-standard` so an ISA written this session counts before it is
      // committed; without it the number reads 0 while the file sits in front of you.
      const isaFiles = gitOut("ls-files", "--cached", "--others", "--exclude-standard",
        "ISA.md", "*/ISA.md", "*/*/ISA.md", "*/*/*/ISA.md")
        .split("\n")
        .filter(Boolean);
      const openClaims = isaFiles.reduce((sum, f) => {
        try {
          return sum + (readFileSync(join(ctx.root, f), "utf8").match(/^- \[ \] /gm)?.length ?? 0);
        } catch {
          return sum;
        }
      }, 0);
      ctx.ok(`open backlog: ${openClaims} claim(s) across ${isaFiles.length} ISA(s) · doctrine: README.md`);
    },
  },
];

async function main() {
  if (process.argv.includes("-h") || process.argv.includes("--help")) {
    console.log(HELP);
    process.exit(0);
  }

  // --version: version identity only, skip the full check run entirely.
  if (process.argv.includes("--version")) {
    printVersion(process.argv.includes("--online"));
    process.exit(0);
  }

  let failed = 0;
  const ctx: CheckContext = {
    root: AXON_ROOT,
    overlayPath: "",
    machineToml: {},
    mounts: [],
    systemsToml: {},
    online: process.argv.includes("--online"),
    ok: (msg) => { console.log(`  ✓ ${msg}`); },
    bad: (msg) => { console.log(`  ✗ ${msg}`); failed++; },
    warn: (msg) => { console.log(`  ⚠ ${msg}`); },
  };

  console.log(`Axon doctor · ${AXON_ROOT}`);
  for (const check of CHECKS) {
    console.log(`\n${check.name}`);
    await check.run(ctx);
  }

  console.log();
  if (failed === 0) {
    console.log("doctor: all checks passed");
    process.exit(0);
  } else {
    console.log(`doctor: ${failed} check(s) failed`);
    process.exit(1);
  }
}

if (import.meta.main) await main();
