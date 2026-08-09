// tools/lib/pack-deploy.ts — the Pack deployment engine, shared by every harness adapter.
//
// Axon is the source of truth. A deployer materializes Packs into a harness'
// skill root by COPYING: each unit is assembled into a staging directory,
// validated, and atomically installed. A state ledger records ownership and the
// last installed digest, so sync and remove can refuse to erase destination-side
// edits or anything this deployment does not own.
//
// Extracted from tools/packs-codex.ts on 2026-08-09, when the Claude adapter
// stopped using symlinks (principal: "we should only deploy from axon overlays
// never using symlinks"). A symlink's target WAS its ownership proof — reading it
// told you whether a directory was ours to remove. Copies destroy that proof, so
// the ledger has to supply it, and the ledger already existed here. Two adapters
// hand-rolling the same digest-and-ownership logic is the duplication Axon's own
// "generic in Axon, specific in the overlay" rule exists to prevent.
//
// Nothing in this file may name a specific harness. Adapter differences arrive
// through DeployConfig: the overlay directory name, the state-file env var named
// in errors, an optional extra validator, and an optional whole-directory
// convention. A hardcoded "codex" or "claude" here is a bug.

import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

export const DIGEST_POLICY = "exclude-python-generated-v1" as const;

type SkillRecord = {
  source: string;
  desiredDigest: string;
  installedDigest: string;
  digestPolicy?: typeof DIGEST_POLICY;
  deployedAt: string;
};

type PackRecord = {
  // Named `skills` on the wire because deployed ledgers already use that key and
  // a rename would strand every existing deployment. It holds units — a skill, or
  // a whole-directory convention like Claude's agents/.
  skills: Record<string, SkillRecord>;
};

export type DeploymentState = {
  version: 1;
  destination: string;
  packs: Record<string, PackRecord>;
};

export type DesiredFile = {
  absolutePath: string;
  relativePath: string;
  mode: number;
};

export type DeployConfig = {
  axonRoot: string;
  /** Ordered Pack roots. The first root is the public Axon source of truth. */
  packRoots?: string[];
  destination: string;
  stateFile: string;
  /** Names the per-harness overlay directory in a Pack, and the temp-path prefixes. */
  adapter: string;
  /** Env var named in the "state belongs to another destination" error, so the fix is in the message. */
  stateEnvVar?: string;
  /** Extra per-adapter validation of an assembled skill. Codex checks agents/openai.yaml here. */
  validateAdapterFiles?: (files: Map<string, DesiredFile>, label: string) => void;
  /**
   * A whole directory a Pack may carry, deployed as ONE owned unit under its own
   * root — Claude Code's agents/, where a single directory exposes every agent
   * inside it. Deliberately not part of pack.toml: it is a per-harness
   * convention, and an adapter that knows nothing about it simply omits this.
   */
  treeConvention?: { sourceDir: string; destinationRoot: string };
};

export type SkillStatus =
  | "not-deployed"
  | "current"
  | "outdated"
  | "drifted"
  | "migration-required"
  | "missing"
  | "collision"
  | "invalid";

export type StatusRow = {
  pack: string;
  skill: string;
  status: SkillStatus;
  detail?: string;
};

/**
 * One deployable thing. A skill lives under the Pack's skills/ and validates as a
 * skill; a tree unit is the whole-directory convention and has no SKILL.md.
 *
 * `key` is what the ledger records. A tree's key keeps its trailing slash, which
 * `assertSimpleName` rejects — so a tree can never collide with a skill name, and
 * the impossibility is structural rather than a reserved-word list to maintain.
 */
export type Unit = {
  key: string;
  sourceRoot: string;
  destination: string;
  isSkill: boolean;
};

function emptyState(destination: string): DeploymentState {
  return { version: 1, destination, packs: {} };
}

export function readState(config: DeployConfig): DeploymentState {
  if (!existsSync(config.stateFile)) return emptyState(config.destination);
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(config.stateFile, "utf8"));
  } catch (error) {
    throw new Error(`cannot read ${config.adapter} deployment state ${config.stateFile}: ${error}`);
  }
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    (parsed as any).version !== 1 ||
    typeof (parsed as any).packs !== "object"
  ) {
    throw new Error(`unsupported or malformed ${config.adapter} deployment state: ${config.stateFile}`);
  }
  const state = parsed as DeploymentState;
  if (resolve(state.destination) !== resolve(config.destination)) {
    const hint = config.stateEnvVar ? `; set ${config.stateEnvVar} for this destination` : "";
    throw new Error(`state file belongs to ${state.destination}, not ${config.destination}${hint}`);
  }
  return state;
}

function writeState(config: DeployConfig, state: DeploymentState): void {
  mkdirSync(dirname(config.stateFile), { recursive: true });
  const temp = `${config.stateFile}.tmp-${process.pid}`;
  writeFileSync(temp, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  renameSync(temp, config.stateFile);
}

function assertSimpleName(value: string, label: string): void {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value)) {
    throw new Error(`${label} '${value}' must be lowercase hyphen-case`);
  }
}

function packRoots(config: DeployConfig): string[] {
  return config.packRoots?.length ? config.packRoots : [join(config.axonRoot, "Packs")];
}

function packDir(config: DeployConfig, pack: string): string {
  assertSimpleName(pack, "pack");
  const matches = packRoots(config).filter((root) => existsSync(join(root, pack, "pack.toml")));
  if (matches.length === 0) return join(packRoots(config)[0], pack);
  if (matches.length > 1) {
    throw new Error(`pack '${pack}' is declared in more than one Pack root: ${matches.join(", ")}`);
  }
  return join(matches[0], pack);
}

export function readPackSkills(config: DeployConfig, pack: string): string[] {
  const dir = packDir(config, pack);
  const manifest = join(dir, "pack.toml");
  if (!existsSync(manifest)) throw new Error(`no such pack: ${pack}`);
  const parsed = Bun.TOML.parse(readFileSync(manifest, "utf8")) as Record<string, unknown>;
  if (parsed.name !== pack) throw new Error(`${manifest}: name must match directory '${pack}'`);
  if (!Array.isArray(parsed.skills) || parsed.skills.some((skill) => typeof skill !== "string")) {
    throw new Error(`${manifest}: skills must be an array of names`);
  }
  const skills = parsed.skills as string[];
  const seen = new Set<string>();
  for (const skill of skills) {
    assertSimpleName(skill, "skill");
    if (seen.has(skill)) throw new Error(`${manifest}: duplicate skill '${skill}'`);
    seen.add(skill);
  }
  return skills;
}

/** The tree unit's ledger key. Trailing slash on purpose — see Unit.key. */
export function treeKey(sourceDir: string): string {
  return `${sourceDir}/`;
}

/**
 * Every unit a Pack deploys: its manifest skills, plus the tree convention when
 * the adapter declares one and the Pack actually carries that directory.
 */
export function packUnits(config: DeployConfig, pack: string): Unit[] {
  const dir = packDir(config, pack);
  const units: Unit[] = readPackSkills(config, pack).map((skill) => ({
    key: skill,
    sourceRoot: join(dir, "skills", skill),
    destination: join(config.destination, skill),
    isSkill: true,
  }));
  const tree = config.treeConvention;
  if (tree && existsSync(join(dir, tree.sourceDir))) {
    units.push({
      key: treeKey(tree.sourceDir),
      sourceRoot: join(dir, tree.sourceDir),
      destination: join(tree.destinationRoot, pack),
      isSkill: false,
    });
  }
  return units;
}

function isGeneratedArtifactPath(relativePath: string): boolean {
  const parts = relativePath.split("/");
  return parts.includes("__pycache__") || /\.py[cod]$/.test(parts.at(-1) ?? "");
}

function collectFiles(
  config: DeployConfig,
  root: string,
  sourceLabel: string,
  includeGeneratedArtifacts = false,
): Map<string, DesiredFile> {
  const files = new Map<string, DesiredFile>();
  if (!existsSync(root)) return files;
  const rootStat = lstatSync(root);
  if (rootStat.isSymbolicLink()) {
    throw new Error(`${sourceLabel} is a symlink; ${config.adapter} deployment must be materialized`);
  }
  if (!rootStat.isDirectory()) throw new Error(`${sourceLabel} is not a directory: ${root}`);

  const visit = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === ".DS_Store") continue;
      const absolutePath = join(dir, entry.name);
      const rel = relative(root, absolutePath).split(sep).join("/");
      if (!includeGeneratedArtifacts && isGeneratedArtifactPath(rel)) continue;
      const lst = lstatSync(absolutePath);
      if (lst.isSymbolicLink()) {
        throw new Error(`${sourceLabel} contains a symlink; ${config.adapter} deployment must be materialized: ${rel}`);
      }
      if (lst.isDirectory()) visit(absolutePath);
      else if (lst.isFile()) {
        files.set(rel, { absolutePath, relativePath: rel, mode: lst.mode & 0o777 });
      } else {
        throw new Error(`${sourceLabel} contains unsupported filesystem entry: ${rel}`);
      }
    }
  };
  visit(root);
  return files;
}

/**
 * The assembled file set for a unit: its shared source, with the adapter overlay
 * merged over it. A tree unit takes no overlay — the convention is already
 * per-harness, so there is nothing to specialize it against.
 *
 * SKILL.md may never be overridden by an overlay. That guard used to fire for the
 * codex adapter alone; making it universal is strictly stricter and matches the
 * stated rule that shared instructions stay canonical.
 */
export function desiredFiles(config: DeployConfig, pack: string, unit: Unit): Map<string, DesiredFile> {
  if (!existsSync(unit.sourceRoot)) throw new Error(`${pack}/${unit.key}: source missing at ${unit.sourceRoot}`);
  const files = collectFiles(config, unit.sourceRoot, `${pack}/${unit.key}`);
  if (!unit.isSkill) return files;
  const overlayRoot = join(packDir(config, pack), config.adapter, unit.key);
  const overlay = collectFiles(config, overlayRoot, `${pack}/${unit.key} ${config.adapter} overlay`);
  if (overlay.has("SKILL.md")) {
    throw new Error(`${pack}/${unit.key}: ${config.adapter} overlay may not override canonical SKILL.md`);
  }
  for (const [rel, file] of overlay) files.set(rel, file);
  return files;
}

export function digestFiles(files: Map<string, DesiredFile>): string {
  const hash = createHash("sha256");
  for (const rel of [...files.keys()].sort()) {
    const file = files.get(rel)!;
    hash.update(`${rel}\0${file.mode.toString(8)}\0`);
    hash.update(readFileSync(file.absolutePath));
    hash.update("\0");
  }
  return hash.digest("hex");
}

export function digestTree(config: DeployConfig, root: string): string {
  return digestFiles(collectFiles(config, root, root));
}

function legacyDigestTree(config: DeployConfig, root: string): string {
  return digestFiles(collectFiles(config, root, root, true));
}

function adoptDigestPolicyIfSafe(
  config: DeployConfig,
  record: SkillRecord,
  destination: string,
  unitKey: string,
): void {
  if (record.digestPolicy === DIGEST_POLICY) {
    if (digestTree(config, destination) !== record.installedDigest) {
      throw new Error(`${unitKey}: installed copy has local changes`);
    }
    return;
  }
  if (legacyDigestTree(config, destination) !== record.installedDigest) {
    throw new Error(
      `${unitKey}: legacy digest is ambiguous; review the destination and run ` +
        `migrate-generated <pack> --accept-current`,
    );
  }
  record.installedDigest = digestTree(config, destination);
  record.digestPolicy = DIGEST_POLICY;
}

type GeneratedArtifacts = { files: string[]; directories: string[] };

function knownGeneratedArtifacts(root: string, label: string): GeneratedArtifacts {
  const files: string[] = [];
  const directories: string[] = [];
  const visit = (dir: string, insideCache: boolean): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const absolutePath = join(dir, entry.name);
      const rel = relative(root, absolutePath).split(sep).join("/");
      const inCache = insideCache || entry.name === "__pycache__";
      const generatedFile = /\.py[cod]$/.test(entry.name);
      const lst = lstatSync(absolutePath);
      if (inCache) {
        if (lst.isSymbolicLink()) {
          throw new Error(`${label}: generated-artifact migration refuses symlink ${rel}`);
        }
        if (lst.isDirectory()) {
          directories.push(absolutePath);
          visit(absolutePath, true);
        } else if (lst.isFile() && (generatedFile || entry.name === ".DS_Store")) {
          files.push(absolutePath);
        } else {
          throw new Error(`${label}: unknown content inside __pycache__: ${rel}`);
        }
      } else if (generatedFile) {
        if (!lst.isFile()) {
          throw new Error(`${label}: generated-artifact migration refuses non-file ${rel}`);
        }
        files.push(absolutePath);
      } else if (lst.isDirectory() && !lst.isSymbolicLink()) {
        visit(absolutePath, false);
      }
    }
  };
  visit(root, false);
  return { files, directories };
}

function extractFrontmatter(skillMd: string, label: string): Record<string, unknown> {
  const match = skillMd.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  if (!match) throw new Error(`${label}: SKILL.md has no valid YAML frontmatter`);
  let parsed: unknown;
  try {
    parsed = Bun.YAML.parse(match[1]);
  } catch (error) {
    throw new Error(`${label}: invalid SKILL.md YAML: ${error}`);
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error(`${label}: SKILL.md frontmatter must be a mapping`);
  }
  return parsed as Record<string, unknown>;
}

/** Skill units validate as skills; a tree unit has no SKILL.md and is checked only for the symlink and entry-type rules collectFiles already enforces. */
export function validateUnit(
  config: DeployConfig,
  files: Map<string, DesiredFile>,
  unit: Unit,
  label: string,
): void {
  if (!unit.isSkill) return;
  const skillMd = files.get("SKILL.md");
  if (!skillMd) throw new Error(`${label}: SKILL.md missing`);
  const frontmatter = extractFrontmatter(readFileSync(skillMd.absolutePath, "utf8"), label);
  if (frontmatter.name !== unit.key) {
    throw new Error(`${label}: SKILL.md name must be '${unit.key}'`);
  }
  if (typeof frontmatter.description !== "string" || !frontmatter.description.trim()) {
    throw new Error(`${label}: SKILL.md description must be a non-empty string`);
  }
  if (frontmatter.description.length > 1024) {
    throw new Error(`${label}: SKILL.md description exceeds 1024 characters`);
  }
  config.validateAdapterFiles?.(files, label);
}

function materializeStage(config: DeployConfig, pack: string, unit: Unit): { stage: string; digest: string } {
  const destinationRoot = dirname(unit.destination);
  mkdirSync(destinationRoot, { recursive: true });
  // Keep staging outside the discovery root. A harness scans its skill directory
  // recursively, so even a short-lived half-built tree must not appear there.
  const stage = mkdtempSync(join(dirname(destinationRoot), `.axon-${config.adapter}-stage-${basename(unit.destination)}-`));
  try {
    const files = desiredFiles(config, pack, unit);
    validateUnit(config, files, unit, `${pack}/${unit.key}`);
    for (const file of files.values()) {
      const dest = join(stage, ...file.relativePath.split("/"));
      mkdirSync(dirname(dest), { recursive: true });
      copyFileSync(file.absolutePath, dest);
      chmodSync(dest, file.mode);
    }
    return { stage, digest: digestTree(config, stage) };
  } catch (error) {
    rmSync(stage, { recursive: true, force: true });
    throw error;
  }
}

function ownerOf(state: DeploymentState, unitKey: string): string | null {
  for (const [pack, record] of Object.entries(state.packs)) {
    if (record.skills[unitKey]) return pack;
  }
  return null;
}

function replaceAtomically(config: DeployConfig, stage: string, destination: string): void {
  if (!existsSync(destination)) {
    renameSync(stage, destination);
    return;
  }
  // The rollback copy also stays outside the discovery root so the harness never
  // sees a duplicate during the short rename window.
  const backup = join(
    dirname(dirname(destination)),
    `.axon-${config.adapter}-backup-${basename(destination)}-${process.pid}`,
  );
  renameSync(destination, backup);
  try {
    renameSync(stage, destination);
  } catch (error) {
    renameSync(backup, destination);
    throw error;
  }
  rmSync(backup, { recursive: true, force: true });
}

function recordUnit(config: DeployConfig, state: DeploymentState, pack: string, unit: Unit, digest: string): void {
  state.packs[pack] ??= { skills: {} };
  state.packs[pack].skills[unit.key] = {
    source: relative(config.axonRoot, unit.sourceRoot),
    desiredDigest: digest,
    installedDigest: digest,
    digestPolicy: DIGEST_POLICY,
    deployedAt: new Date().toISOString(),
  };
}

function installOne(
  config: DeployConfig,
  state: DeploymentState,
  pack: string,
  unit: Unit,
  mode: "deploy" | "sync",
): string {
  const destination = unit.destination;
  const owner = ownerOf(state, unit.key);
  const existingRecord = state.packs[pack]?.skills[unit.key];
  if (owner && owner !== pack) throw new Error(`${unit.key}: already owned by Pack '${owner}'`);
  if (existsSync(destination) && !existingRecord) {
    throw new Error(`${unit.key}: ${destination} exists and is not owned by this Axon deployment`);
  }
  if (mode === "deploy" && existingRecord && existsSync(destination)) {
    adoptDigestPolicyIfSafe(config, existingRecord, destination, unit.key);
    const actual = digestTree(config, destination);
    const wanted = digestFiles(desiredFiles(config, pack, unit));
    return wanted === actual ? `= ${unit.key} (already current)` : `= ${unit.key} (deployed; run sync to update)`;
  }

  if (existingRecord && existsSync(destination)) {
    try {
      adoptDigestPolicyIfSafe(config, existingRecord, destination, unit.key);
    } catch (error) {
      throw new Error(`${(error as Error).message}; refusing to overwrite`);
    }
  }

  const { stage, digest } = materializeStage(config, pack, unit);
  try {
    if (existsSync(destination) && digestTree(config, destination) === digest) {
      rmSync(stage, { recursive: true, force: true });
      recordUnit(config, state, pack, unit, digest);
      return `= ${unit.key} (already current)`;
    }
    replaceAtomically(config, stage, destination);
  } catch (error) {
    if (existsSync(stage)) rmSync(stage, { recursive: true, force: true });
    throw error;
  }
  recordUnit(config, state, pack, unit, digest);
  return `✓ ${unit.key} ${mode === "deploy" ? "deployed" : "synced"}`;
}

export function deployPack(config: DeployConfig, pack: string): string[] {
  const units = packUnits(config, pack);
  const state = readState(config);
  // Validate every source and collision before the first write so a bad unit
  // cannot leave a normally-failing Pack only partially deployed.
  for (const unit of units) {
    const files = desiredFiles(config, pack, unit);
    validateUnit(config, files, unit, `${pack}/${unit.key}`);
    const record = state.packs[pack]?.skills[unit.key];
    const owner = ownerOf(state, unit.key);
    if (owner && owner !== pack) throw new Error(`${unit.key}: already owned by Pack '${owner}'`);
    if (existsSync(unit.destination) && !record) {
      throw new Error(`${unit.key}: ${unit.destination} exists and is not owned by this Axon deployment`);
    }
    if (record && existsSync(unit.destination)) {
      try {
        adoptDigestPolicyIfSafe(config, record, unit.destination, unit.key);
      } catch (error) {
        throw new Error(`${(error as Error).message}; refusing to redeploy`);
      }
    }
  }
  const messages: string[] = [];
  const failures: string[] = [];
  for (const unit of units) {
    try {
      messages.push(installOne(config, state, pack, unit, "deploy"));
      writeState(config, state);
    } catch (error) {
      failures.push((error as Error).message);
    }
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return messages;
}

/**
 * Take ownership of destinations that already hold exactly what this Pack would
 * deploy, without writing anything.
 *
 * The migration case it exists for: an adapter that used to deploy by symlink has
 * real directories sitting in place that no ledger knows about, and a plain
 * deploy correctly refuses them as collisions. Adoption is safe ONLY because it
 * demands a digest match — an adopted unit is byte-identical to its source, so
 * recording it asserts nothing that is not already true on disk. Anything that
 * differs is left alone and reported, because a difference is either a hand edit
 * or a stale deployment and both need a human, not a ledger entry.
 */
export function adoptPack(config: DeployConfig, pack: string): string[] {
  const units = packUnits(config, pack);
  const state = readState(config);
  const messages: string[] = [];
  const failures: string[] = [];
  for (const unit of units) {
    const owner = ownerOf(state, unit.key);
    if (owner === pack) { messages.push(`= ${unit.key} (already owned)`); continue; }
    if (owner) { failures.push(`${unit.key}: already owned by Pack '${owner}'`); continue; }
    if (!existsSync(unit.destination)) { messages.push(`= ${unit.key} (not deployed; nothing to adopt)`); continue; }
    const files = desiredFiles(config, pack, unit);
    validateUnit(config, files, unit, `${pack}/${unit.key}`);
    const wanted = digestFiles(files);
    const actual = digestTree(config, unit.destination);
    if (wanted !== actual) {
      failures.push(`${unit.key}: ${unit.destination} differs from the Pack source; refusing to adopt`);
      continue;
    }
    recordUnit(config, state, pack, unit, wanted);
    writeState(config, state);
    messages.push(`✓ ${unit.key} adopted`);
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return messages;
}

/**
 * The destination a recorded unit occupies. Rebuilt from the key rather than
 * stored, so a ledger written before the tree convention existed still resolves.
 */
function recordedDestination(config: DeployConfig, pack: string, unitKey: string): string {
  const tree = config.treeConvention;
  if (tree && unitKey === treeKey(tree.sourceDir)) return join(tree.destinationRoot, pack);
  return join(config.destination, unitKey);
}

function removeOwnedUnit(
  config: DeployConfig,
  state: DeploymentState,
  pack: string,
  unitKey: string,
): string {
  const record = state.packs[pack]?.skills[unitKey];
  if (!record) throw new Error(`${unitKey}: not owned by Pack '${pack}'`);
  const destination = recordedDestination(config, pack, unitKey);
  if (existsSync(destination)) {
    try {
      adoptDigestPolicyIfSafe(config, record, destination, unitKey);
    } catch (error) {
      throw new Error(`${(error as Error).message}; refusing to remove`);
    }
    rmSync(destination, { recursive: true });
  }
  delete state.packs[pack].skills[unitKey];
  if (Object.keys(state.packs[pack].skills).length === 0) delete state.packs[pack];
  return `✓ ${unitKey} removed`;
}

export function syncPack(config: DeployConfig, pack: string): string[] {
  const units = packUnits(config, pack);
  const state = readState(config);
  if (!state.packs[pack]) throw new Error(`${pack}: not deployed; deploy it first`);
  const desired = new Set(units.map((unit) => unit.key));
  const messages: string[] = [];
  const failures: string[] = [];

  // Preflight both stale removals and desired updates before mutating either.
  for (const [unitKey, record] of Object.entries(state.packs[pack].skills)) {
    const destination = recordedDestination(config, pack, unitKey);
    if (existsSync(destination)) {
      try {
        adoptDigestPolicyIfSafe(config, record, destination, unitKey);
      } catch (error) {
        throw new Error(`${(error as Error).message}; refusing to sync`);
      }
    }
  }
  for (const unit of units) {
    const files = desiredFiles(config, pack, unit);
    validateUnit(config, files, unit, `${pack}/${unit.key}`);
    const owner = ownerOf(state, unit.key);
    if (owner && owner !== pack) throw new Error(`${unit.key}: already owned by Pack '${owner}'`);
    if (existsSync(unit.destination) && !state.packs[pack].skills[unit.key]) {
      throw new Error(`${unit.key}: ${unit.destination} exists and is not owned by this Axon deployment`);
    }
  }

  for (const stale of Object.keys(state.packs[pack].skills).filter((key) => !desired.has(key))) {
    try {
      messages.push(removeOwnedUnit(config, state, pack, stale));
      writeState(config, state);
    } catch (error) {
      failures.push((error as Error).message);
    }
  }
  for (const unit of units) {
    try {
      messages.push(installOne(config, state, pack, unit, "sync"));
      writeState(config, state);
    } catch (error) {
      failures.push((error as Error).message);
    }
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return messages;
}

export function removePack(config: DeployConfig, pack: string): string[] {
  const state = readState(config);
  if (!state.packs[pack]) throw new Error(`${pack}: not deployed`);
  for (const [unitKey, record] of Object.entries(state.packs[pack].skills)) {
    const destination = recordedDestination(config, pack, unitKey);
    if (existsSync(destination)) {
      try {
        adoptDigestPolicyIfSafe(config, record, destination, unitKey);
      } catch (error) {
        throw new Error(`${(error as Error).message}; refusing to remove`);
      }
    }
  }
  const messages: string[] = [];
  const failures: string[] = [];
  for (const unitKey of Object.keys(state.packs[pack].skills)) {
    try {
      messages.push(removeOwnedUnit(config, state, pack, unitKey));
      writeState(config, state);
    } catch (error) {
      failures.push((error as Error).message);
    }
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return messages;
}

export function migrateGeneratedArtifacts(
  config: DeployConfig,
  pack: string,
  acceptCurrent: boolean,
): string[] {
  if (!acceptCurrent) {
    throw new Error(
      "migration requires --accept-current after reviewing non-generated destination files",
    );
  }
  const state = readState(config);
  if (!state.packs[pack]) throw new Error(`${pack}: not deployed`);
  const plans = new Map<string, GeneratedArtifacts>();
  for (const [unitKey, record] of Object.entries(state.packs[pack].skills)) {
    if (record.digestPolicy === DIGEST_POLICY) continue;
    const destination = recordedDestination(config, pack, unitKey);
    if (!existsSync(destination)) throw new Error(`${unitKey}: owned destination is missing`);
    plans.set(unitKey, knownGeneratedArtifacts(destination, `${pack}/${unitKey}`));
  }

  const messages: string[] = [];
  for (const [unitKey, artifacts] of plans) {
    const destination = recordedDestination(config, pack, unitKey);
    for (const file of artifacts.files) rmSync(file);
    for (const directory of artifacts.directories.sort((a, b) => b.length - a.length)) {
      if (readdirSync(directory).length === 0) rmdirSync(directory);
    }
    const record = state.packs[pack].skills[unitKey];
    record.installedDigest = digestTree(config, destination);
    record.digestPolicy = DIGEST_POLICY;
    messages.push(`✓ ${unitKey} migrated (${artifacts.files.length} generated artifact(s) removed)`);
  }
  writeState(config, state);
  if (messages.length === 0) messages.push(`= ${pack} (digest policy already current)`);
  return messages;
}

// ── Profiles ────────────────────────────────────────────────────────

export type Profile = {
  name: string;
  description: string;
  packs: string[];
};

export function readProfiles(config: DeployConfig): Profile[] {
  const path = join(config.axonRoot, "profiles.toml");
  if (!existsSync(path)) return [];
  const parsed = Bun.TOML.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  const profiles = parsed.profile;
  if (!Array.isArray(profiles)) return [];
  return profiles as Profile[];
}

export function resolveProfilePacks(config: DeployConfig, profile: Profile): string[] {
  if (profile.packs.length === 1 && profile.packs[0] === "*") {
    return availablePacks(config);
  }
  const allPacks = new Set(availablePacks(config, true));
  for (const pack of profile.packs) {
    if (!allPacks.has(pack)) {
      throw new Error(`profile '${profile.name}': unknown pack '${pack}'`);
    }
    const owner = packDeployer(config, pack);
    if (owner) {
      throw new Error(
        `profile '${profile.name}': pack '${pack}' is deployed by ${owner}; remove it from the profile`,
      );
    }
  }
  return profile.packs;
}

export function activateProfile(config: DeployConfig, profile: Profile): string[] {
  const targetPackNames = new Set(resolveProfilePacks(config, profile));
  const state = readState(config);
  const messages: string[] = [];

  messages.push(`Activating profile '${profile.name}' — ${profile.description}`);

  const currentPacks = Object.keys(state.packs).sort();
  const targetPacks = [...targetPackNames].sort();

  const toRemove = currentPacks.filter((p) => !targetPackNames.has(p));
  const toDeploy = targetPacks.filter((p) => {
    if (!state.packs[p]) return true;
    // Re-deploy if any owned destination is missing from disk
    return Object.keys(state.packs[p].skills).some(
      (unitKey) => !existsSync(recordedDestination(config, p, unitKey)),
    );
  });

  if (toRemove.length === 0 && toDeploy.length === 0) {
    messages.push("  → already current");
    return messages;
  }

  if (toRemove.length > 0) {
    messages.push("", `Removing ${toRemove.length} pack(s) not in profile:`);
    for (const pack of toRemove) {
      try {
        messages.push(...removePack(config, pack).map((l) => `  ${l}`));
      } catch (error) {
        messages.push(`  ✗ ${pack}: ${(error as Error).message}`);
      }
    }
  }

  if (toDeploy.length > 0) {
    messages.push("", `Deploying ${toDeploy.length} pack(s):`);
    for (const pack of toDeploy) {
      try {
        messages.push(...deployPack(config, pack).map((l) => `  ${l}`));
      } catch (error) {
        messages.push(`  ✗ ${pack}: ${(error as Error).message}`);
      }
    }
  }

  return messages;
}

export function profileActivePacks(config: DeployConfig, profile: Profile): string[] {
  const target = new Set(resolveProfilePacks(config, profile));
  const state = readState(config);
  return Object.keys(state.packs).filter((p) => target.has(p)).sort();
}

// ── Discovery ───────────────────────────────────────────────────────

// A pack whose manifest names a `deployer` is owned by that tool alone, so generic
// harness deployment never competes with its dedicated lifecycle.
export function packDeployer(config: DeployConfig, pack: string): string | null {
  const manifest = join(config.axonRoot, "Packs", pack, "pack.toml");
  if (!existsSync(manifest)) return null;
  const parsed = Bun.TOML.parse(readFileSync(manifest, "utf8")) as Record<string, unknown>;
  const deployer = parsed.deployer;
  return typeof deployer === "string" && deployer.length > 0 ? deployer : null;
}

// `includeDedicated` exists so name validation can still see a dedicated pack and
// say who owns it, instead of reporting a pack that plainly exists as unknown.
export function availablePacks(config: DeployConfig, includeDedicated = false): string[] {
  const matches = new Map<string, string[]>();
  for (const root of packRoots(config)) {
    if (!existsSync(root)) continue;
    for (const entry of readdirSync(root, { withFileTypes: true })) {
      if (!entry.isDirectory() || !existsSync(join(root, entry.name, "pack.toml"))) continue;
      matches.set(entry.name, [...(matches.get(entry.name) ?? []), root]);
    }
  }
  for (const [pack, roots] of matches) {
    if (roots.length > 1) throw new Error(`pack '${pack}' is declared in more than one Pack root: ${roots.join(", ")}`);
  }
  return [...matches.keys()]
    .filter((pack) => includeDedicated || packDeployer(config, pack) === null)
    .sort();
}

export function getStatuses(config: DeployConfig, selectedPack?: string): StatusRow[] {
  const state = readState(config);
  const packs = selectedPack
    ? [selectedPack]
    : [...new Set([...availablePacks(config), ...Object.keys(state.packs)])].sort();
  const rows: StatusRow[] = [];
  for (const pack of packs) {
    let units: Unit[];
    try {
      units = packUnits(config, pack);
    } catch (error) {
      rows.push({ pack, skill: "(manifest)", status: "invalid", detail: (error as Error).message });
      continue;
    }
    const seen = new Set(units.map((unit) => unit.key));
    for (const unit of units) {
      const destination = unit.destination;
      const record = state.packs[pack]?.skills[unit.key];
      let wanted: string;
      try {
        const files = desiredFiles(config, pack, unit);
        validateUnit(config, files, unit, `${pack}/${unit.key}`);
        wanted = digestFiles(files);
      } catch (error) {
        rows.push({ pack, skill: unit.key, status: "invalid", detail: (error as Error).message });
        continue;
      }
      if (!record) {
        rows.push({
          pack,
          skill: unit.key,
          status: existsSync(destination) ? "collision" : "not-deployed",
        });
        continue;
      }
      if (!existsSync(destination)) {
        rows.push({ pack, skill: unit.key, status: "missing" });
        continue;
      }
      try {
        const actual = digestTree(config, destination);
        if (record.digestPolicy !== DIGEST_POLICY) {
          if (legacyDigestTree(config, destination) !== record.installedDigest) {
            rows.push({
              pack,
              skill: unit.key,
              status: "migration-required",
              detail: "legacy digest differs; review before adopting generated-artifact exclusions",
            });
            continue;
          }
        } else if (actual !== record.installedDigest) {
          rows.push({ pack, skill: unit.key, status: "drifted" });
          continue;
        }
        rows.push({ pack, skill: unit.key, status: wanted === actual ? "current" : "outdated" });
      } catch (error) {
        rows.push({ pack, skill: unit.key, status: "invalid", detail: (error as Error).message });
      }
    }
    for (const stale of Object.keys(state.packs[pack]?.skills ?? {}).filter((key) => !seen.has(key))) {
      rows.push({ pack, skill: stale, status: "outdated", detail: "removed from pack manifest" });
    }
  }
  return rows;
}

export function printStatuses(rows: StatusRow[]): void {
  let lastPack = "";
  for (const row of rows) {
    if (row.pack !== lastPack) {
      if (lastPack) console.log();
      console.log(row.pack);
      lastPack = row.pack;
    }
    console.log(`  ${row.skill.padEnd(24)} [${row.status}]${row.detail ? ` ${row.detail}` : ""}`);
  }
}
