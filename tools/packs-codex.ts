// tools/packs-codex.ts — materialize Axon Packs into Codex's user skill root.
//
// Axon remains the source of truth. Unlike tools/packs.sh's Claude adapter,
// this deployer never creates symlinks: it copies each complete skill into a
// staging directory, applies an optional Codex-only overlay, validates the
// result, and atomically installs the finished tree. A state ledger records
// ownership and the last installed digest so sync/remove can refuse to erase
// destination-side edits or unrelated skills.
//
// Optional harness overlay (not part of pack.toml):
//   Packs/<pack>/codex/<skill>/
// Its contents are merged over the shared skill at deploy time. This is where
// Codex-only agents/openai.yaml metadata and any assets it references belong.
// SKILL.md may not be overridden: shared instructions stay canonical.
//
//   tools/packs-codex status [<pack>|--all]
//   tools/packs-codex deploy <pack>...
//   tools/packs-codex sync <pack>|--all
//   tools/packs-codex remove <pack>...
//   tools/packs-codex use [<profile>]

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
import { createInterface } from "node:readline/promises";

const AXON_ROOT = resolve(import.meta.dir, "..");
const DIGEST_POLICY = "exclude-python-generated-v1" as const;
const HELP = `tools/packs-codex — materialize Axon Packs.

  tools/packs-codex status [<pack>|--all]  show source/install/drift state
  tools/packs-codex deploy <pack>...       install one or more Packs
  tools/packs-codex sync <pack>|--all      update already-deployed Packs
  tools/packs-codex remove <pack>...       remove one or more owned Packs
  tools/packs-codex migrate-generated <pack> --accept-current
                                            adopt generated-artifact exclusions after review
  tools/packs-codex use [<profile>]        activate a profile (or pick interactively)

Environment:
  CODEX_SKILLS_DIR       destination (default: $HOME/.agents/skills)
  AXON_CODEX_STATE_FILE ownership ledger override (mainly for tests)
`;

type SkillRecord = {
  source: string;
  desiredDigest: string;
  installedDigest: string;
  digestPolicy?: typeof DIGEST_POLICY;
  deployedAt: string;
};

type PackRecord = {
  skills: Record<string, SkillRecord>;
};

export type DeploymentState = {
  version: 1;
  destination: string;
  packs: Record<string, PackRecord>;
};

export type DeployConfig = {
  axonRoot: string;
  destination: string;
  stateFile: string;
  adapter?: string;
};

const DEFAULT_ADAPTER = "codex";

type DesiredFile = {
  absolutePath: string;
  relativePath: string;
  mode: number;
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

export function defaultCodexDeployConfig(): DeployConfig {
  const home = process.env.HOME ?? "";
  const stateHome = process.env.XDG_STATE_HOME ?? join(home, ".local", "state");
  return {
    axonRoot: AXON_ROOT,
    destination: resolve(process.env.CODEX_SKILLS_DIR ?? join(home, ".agents", "skills")),
    stateFile: resolve(
      process.env.AXON_CODEX_STATE_FILE ?? join(stateHome, "axon", "pack-deployments", "codex.json"),
    ),
    adapter: DEFAULT_ADAPTER,
  };
}

function resolveAdapter(config: DeployConfig): string {
  return config.adapter && config.adapter.trim().length > 0 ? config.adapter.trim() : DEFAULT_ADAPTER;
}

function emptyState(destination: string): DeploymentState {
  return { version: 1, destination, packs: {} };
}

export function readState(config: DeployConfig): DeploymentState {
  if (!existsSync(config.stateFile)) return emptyState(config.destination);
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(config.stateFile, "utf8"));
  } catch (error) {
    throw new Error(`cannot read Codex deployment state ${config.stateFile}: ${error}`);
  }
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    (parsed as any).version !== 1 ||
    typeof (parsed as any).packs !== "object"
  ) {
    throw new Error(`unsupported or malformed Codex deployment state: ${config.stateFile}`);
  }
  const state = parsed as DeploymentState;
  if (resolve(state.destination) !== resolve(config.destination)) {
    throw new Error(
      `state file belongs to ${state.destination}, not ${config.destination}; set AXON_CODEX_STATE_FILE for this destination`,
    );
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

function packDir(config: DeployConfig, pack: string): string {
  assertSimpleName(pack, "pack");
  return join(config.axonRoot, "Packs", pack);
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

function isGeneratedArtifactPath(relativePath: string): boolean {
  const parts = relativePath.split("/");
  return parts.includes("__pycache__") || /\.py[cod]$/.test(parts.at(-1) ?? "");
}

function collectFiles(
  root: string,
  sourceLabel: string,
  includeGeneratedArtifacts = false,
): Map<string, DesiredFile> {
  const files = new Map<string, DesiredFile>();
  if (!existsSync(root)) return files;
  const rootStat = lstatSync(root);
  if (rootStat.isSymbolicLink()) {
    throw new Error(`${sourceLabel} is a symlink; Codex deployment must be materialized`);
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
        throw new Error(`${sourceLabel} contains a symlink; Codex deployment must be materialized: ${rel}`);
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

function desiredFiles(config: DeployConfig, pack: string, skill: string): Map<string, DesiredFile> {
  const sharedRoot = join(packDir(config, pack), "skills", skill);
  if (!existsSync(sharedRoot)) throw new Error(`${pack}/${skill}: source missing at ${sharedRoot}`);
  const files = collectFiles(sharedRoot, `${pack}/${skill}`);
  const adapter = resolveAdapter(config);
  const overlayRoot = join(packDir(config, pack), adapter, skill);
  const overlay = collectFiles(overlayRoot, `${pack}/${skill} ${adapter} overlay`);
  if (adapter === DEFAULT_ADAPTER && overlay.has("SKILL.md")) {
    throw new Error(`${pack}/${skill}: ${DEFAULT_ADAPTER} overlay may not override canonical SKILL.md`);
  }
  for (const [rel, file] of overlay) files.set(rel, file);
  return files;
}

function digestFiles(files: Map<string, DesiredFile>): string {
  const hash = createHash("sha256");
  for (const rel of [...files.keys()].sort()) {
    const file = files.get(rel)!;
    hash.update(`${rel}\0${file.mode.toString(8)}\0`);
    hash.update(readFileSync(file.absolutePath));
    hash.update("\0");
  }
  return hash.digest("hex");
}

export function digestTree(root: string): string {
  return digestFiles(collectFiles(root, root));
}

function legacyDigestTree(root: string): string {
  return digestFiles(collectFiles(root, root, true));
}

function adoptDigestPolicyIfSafe(record: SkillRecord, destination: string, skill: string): void {
  if (record.digestPolicy === DIGEST_POLICY) {
    if (digestTree(destination) !== record.installedDigest) {
      throw new Error(`${skill}: installed copy has local changes`);
    }
    return;
  }
  if (legacyDigestTree(destination) !== record.installedDigest) {
    throw new Error(
      `${skill}: legacy digest is ambiguous; review the destination and run ` +
        `tools/packs-codex migrate-generated <pack> --accept-current`,
    );
  }
  record.installedDigest = digestTree(destination);
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

function validateDesiredFiles(files: Map<string, DesiredFile>, expectedName: string, label: string): void {
  const skillMd = files.get("SKILL.md");
  if (!skillMd) throw new Error(`${label}: SKILL.md missing`);
  const frontmatter = extractFrontmatter(readFileSync(skillMd.absolutePath, "utf8"), label);
  if (frontmatter.name !== expectedName) {
    throw new Error(`${label}: SKILL.md name must be '${expectedName}'`);
  }
  if (typeof frontmatter.description !== "string" || !frontmatter.description.trim()) {
    throw new Error(`${label}: SKILL.md description must be a non-empty string`);
  }
  if (frontmatter.description.length > 1024) {
    throw new Error(`${label}: SKILL.md description exceeds 1024 characters`);
  }
  const openaiYaml = files.get("agents/openai.yaml");
  if (openaiYaml) {
    try {
      const parsed = Bun.YAML.parse(readFileSync(openaiYaml.absolutePath, "utf8"));
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new Error("top level must be a mapping");
      }
    } catch (error) {
      throw new Error(`${label}: invalid agents/openai.yaml: ${error}`);
    }
  }
}

function materializeStage(config: DeployConfig, pack: string, skill: string): { stage: string; digest: string } {
  mkdirSync(config.destination, { recursive: true });
  // Keep staging outside the skill discovery root. Codex recursively scans
  // .agents/skills, so even a short-lived half-built tree must not appear there.
  const stage = mkdtempSync(join(dirname(config.destination), `.axon-codex-stage-${skill}-`));
  try {
    const files = desiredFiles(config, pack, skill);
    validateDesiredFiles(files, skill, `${pack}/${skill}`);
    for (const file of files.values()) {
      const dest = join(stage, ...file.relativePath.split("/"));
      mkdirSync(dirname(dest), { recursive: true });
      copyFileSync(file.absolutePath, dest);
      chmodSync(dest, file.mode);
    }
    return { stage, digest: digestTree(stage) };
  } catch (error) {
    rmSync(stage, { recursive: true, force: true });
    throw error;
  }
}

function ownerOf(state: DeploymentState, skill: string): string | null {
  for (const [pack, record] of Object.entries(state.packs)) {
    if (record.skills[skill]) return pack;
  }
  return null;
}

function replaceAtomically(stage: string, destination: string): void {
  if (!existsSync(destination)) {
    renameSync(stage, destination);
    return;
  }
  // The rollback copy also stays outside .agents/skills so Codex never sees a
  // duplicate skill during the short rename window.
  const backup = join(
    dirname(dirname(destination)),
    `.axon-codex-backup-${basename(destination)}-${process.pid}`,
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

function installOne(
  config: DeployConfig,
  state: DeploymentState,
  pack: string,
  skill: string,
  mode: "deploy" | "sync",
): string {
  const destination = join(config.destination, skill);
  const owner = ownerOf(state, skill);
  const existingRecord = state.packs[pack]?.skills[skill];
  if (owner && owner !== pack) throw new Error(`${skill}: already owned by Pack '${owner}'`);
  if (existsSync(destination) && !existingRecord) {
    throw new Error(`${skill}: ${destination} exists and is not owned by this Axon deployment`);
  }
  if (mode === "deploy" && existingRecord && existsSync(destination)) {
    adoptDigestPolicyIfSafe(existingRecord, destination, skill);
    const actual = digestTree(destination);
    const wanted = digestFiles(desiredFiles(config, pack, skill));
    return wanted === actual ? `= ${skill} (already current)` : `= ${skill} (deployed; run sync to update)`;
  }

  if (existingRecord && existsSync(destination)) {
    try {
      adoptDigestPolicyIfSafe(existingRecord, destination, skill);
    } catch (error) {
      throw new Error(`${(error as Error).message}; refusing to overwrite`);
    }
  }

  const { stage, digest } = materializeStage(config, pack, skill);
  try {
    if (existsSync(destination) && digestTree(destination) === digest) {
      rmSync(stage, { recursive: true, force: true });
      state.packs[pack] ??= { skills: {} };
      state.packs[pack].skills[skill] = {
        source: relative(config.axonRoot, join(packDir(config, pack), "skills", skill)),
        desiredDigest: digest,
        installedDigest: digest,
        digestPolicy: DIGEST_POLICY,
        deployedAt: new Date().toISOString(),
      };
      return `= ${skill} (already current)`;
    }
    replaceAtomically(stage, destination);
  } catch (error) {
    if (existsSync(stage)) rmSync(stage, { recursive: true, force: true });
    throw error;
  }
  state.packs[pack] ??= { skills: {} };
  state.packs[pack].skills[skill] = {
    source: relative(config.axonRoot, join(packDir(config, pack), "skills", skill)),
    desiredDigest: digest,
    installedDigest: digest,
    digestPolicy: DIGEST_POLICY,
    deployedAt: new Date().toISOString(),
  };
  return `✓ ${skill} ${mode === "deploy" ? "deployed" : "synced"}`;
}

export function deployPack(config: DeployConfig, pack: string): string[] {
  const skills = readPackSkills(config, pack);
  const state = readState(config);
  // Validate every source and collision before the first write so a bad skill
  // cannot leave a normally-failing Pack only partially deployed.
  for (const skill of skills) {
    const files = desiredFiles(config, pack, skill);
    validateDesiredFiles(files, skill, `${pack}/${skill}`);
    const destination = join(config.destination, skill);
    const record = state.packs[pack]?.skills[skill];
    const owner = ownerOf(state, skill);
    if (owner && owner !== pack) throw new Error(`${skill}: already owned by Pack '${owner}'`);
    if (existsSync(destination) && !record) {
      throw new Error(`${skill}: ${destination} exists and is not owned by this Axon deployment`);
    }
    if (record && existsSync(destination)) {
      try {
        adoptDigestPolicyIfSafe(record, destination, skill);
      } catch (error) {
        throw new Error(`${(error as Error).message}; refusing to redeploy`);
      }
    }
  }
  const messages: string[] = [];
  const failures: string[] = [];
  for (const skill of skills) {
    try {
      messages.push(installOne(config, state, pack, skill, "deploy"));
      writeState(config, state);
    } catch (error) {
      failures.push((error as Error).message);
    }
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return messages;
}

function removeOwnedSkill(
  config: DeployConfig,
  state: DeploymentState,
  pack: string,
  skill: string,
): string {
  const record = state.packs[pack]?.skills[skill];
  if (!record) throw new Error(`${skill}: not owned by Pack '${pack}'`);
  const destination = join(config.destination, skill);
  if (existsSync(destination)) {
    try {
      adoptDigestPolicyIfSafe(record, destination, skill);
    } catch (error) {
      throw new Error(`${(error as Error).message}; refusing to remove`);
    }
    rmSync(destination, { recursive: true });
  }
  delete state.packs[pack].skills[skill];
  if (Object.keys(state.packs[pack].skills).length === 0) delete state.packs[pack];
  return `✓ ${skill} removed`;
}

export function syncPack(config: DeployConfig, pack: string): string[] {
  const skills = readPackSkills(config, pack);
  const state = readState(config);
  if (!state.packs[pack]) throw new Error(`${pack}: not deployed; run tools/packs-codex deploy ${pack}`);
  const desired = new Set(skills);
  const messages: string[] = [];
  const failures: string[] = [];

  // Preflight both stale removals and desired updates before mutating either.
  for (const [skill, record] of Object.entries(state.packs[pack].skills)) {
    const destination = join(config.destination, skill);
    if (existsSync(destination)) {
      try {
        adoptDigestPolicyIfSafe(record, destination, skill);
      } catch (error) {
        throw new Error(`${(error as Error).message}; refusing to sync`);
      }
    }
  }
  for (const skill of skills) {
    const files = desiredFiles(config, pack, skill);
    validateDesiredFiles(files, skill, `${pack}/${skill}`);
    const owner = ownerOf(state, skill);
    if (owner && owner !== pack) throw new Error(`${skill}: already owned by Pack '${owner}'`);
    const destination = join(config.destination, skill);
    if (existsSync(destination) && !state.packs[pack].skills[skill]) {
      throw new Error(`${skill}: ${destination} exists and is not owned by this Axon deployment`);
    }
  }

  for (const staleSkill of Object.keys(state.packs[pack].skills).filter((skill) => !desired.has(skill))) {
    try {
      messages.push(removeOwnedSkill(config, state, pack, staleSkill));
      writeState(config, state);
    } catch (error) {
      failures.push((error as Error).message);
    }
  }
  for (const skill of skills) {
    try {
      messages.push(installOne(config, state, pack, skill, "sync"));
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
  for (const [skill, record] of Object.entries(state.packs[pack].skills)) {
    const destination = join(config.destination, skill);
    if (existsSync(destination)) {
      try {
        adoptDigestPolicyIfSafe(record, destination, skill);
      } catch (error) {
        throw new Error(`${(error as Error).message}; refusing to remove`);
      }
    }
  }
  const messages: string[] = [];
  const failures: string[] = [];
  for (const skill of Object.keys(state.packs[pack].skills)) {
    try {
      messages.push(removeOwnedSkill(config, state, pack, skill));
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
  for (const [skill, record] of Object.entries(state.packs[pack].skills)) {
    if (record.digestPolicy === DIGEST_POLICY) continue;
    const destination = join(config.destination, skill);
    if (!existsSync(destination)) throw new Error(`${skill}: owned destination is missing`);
    plans.set(skill, knownGeneratedArtifacts(destination, `${pack}/${skill}`));
  }

  const messages: string[] = [];
  for (const [skill, artifacts] of plans) {
    const destination = join(config.destination, skill);
    for (const file of artifacts.files) rmSync(file);
    for (const directory of artifacts.directories.sort((a, b) => b.length - a.length)) {
      if (readdirSync(directory).length === 0) rmdirSync(directory);
    }
    const record = state.packs[pack].skills[skill];
    record.installedDigest = digestTree(destination);
    record.digestPolicy = DIGEST_POLICY;
    messages.push(`✓ ${skill} migrated (${artifacts.files.length} generated artifact(s) removed)`);
  }
  writeState(config, state);
  if (messages.length === 0) messages.push(`= ${pack} (digest policy already current)`);
  return messages;
}

// ── Profiles ────────────────────────────────────────────────────────

type Profile = {
  name: string;
  description: string;
  packs: string[];
};

function readProfiles(config: DeployConfig): Profile[] {
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

function activateProfile(config: DeployConfig, profile: Profile): string[] {
  const targetPackNames = new Set(resolveProfilePacks(config, profile));
  const state = readState(config);
  const messages: string[] = [];

  messages.push(`Activating profile '${profile.name}' — ${profile.description}`);

  const currentPacks = Object.keys(state.packs).sort();
  const targetPacks = [...targetPackNames].sort();

  const toRemove = currentPacks.filter((p) => !targetPackNames.has(p));
  const toDeploy = targetPacks.filter((p) => {
    if (!state.packs[p]) return true;
    // Re-deploy if any owned skill directory is missing from disk
    return Object.keys(state.packs[p].skills).some(
      (skill) => !existsSync(join(config.destination, skill)),
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

function profileActivePacks(config: DeployConfig, profile: Profile): string[] {
  const target = new Set(resolveProfilePacks(config, profile));
  const state = readState(config);
  return Object.keys(state.packs).filter((p) => target.has(p)).sort();
}

// ── Discovery ───────────────────────────────────────────────────────

// A pack whose manifest names a `deployer` is owned by that tool alone. Without
// this, three mechanisms claimed ~/.claude/skills/axon at once — a packs.sh
// symlink, this sweep, and packs-axon's ledger — and doctor reported the same
// path as linked, as an unowned collision and as not-deployed in one run.
export function packDeployer(config: DeployConfig, pack: string): string | null {
  const manifest = join(config.axonRoot, "Packs", pack, "pack.toml");
  if (!existsSync(manifest)) return null;
  const parsed = Bun.TOML.parse(readFileSync(manifest, "utf8")) as Record<string, unknown>;
  const deployer = parsed.deployer;
  return typeof deployer === "string" && deployer.length > 0 ? deployer : null;
}

// `includeDedicated` exists so name validation can still see a dedicated pack and
// say who owns it, instead of reporting a pack that plainly exists as unknown.
function availablePacks(config: DeployConfig, includeDedicated = false): string[] {
  const root = join(config.axonRoot, "Packs");
  if (!existsSync(root)) return [];
  return readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(root, entry.name, "pack.toml")))
    .map((entry) => entry.name)
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
    let skills: string[];
    try {
      skills = readPackSkills(config, pack);
    } catch (error) {
      rows.push({ pack, skill: "(manifest)", status: "invalid", detail: (error as Error).message });
      continue;
    }
    const seen = new Set(skills);
    for (const skill of skills) {
      const destination = join(config.destination, skill);
      const record = state.packs[pack]?.skills[skill];
      let wanted: string;
      try {
        const files = desiredFiles(config, pack, skill);
        validateDesiredFiles(files, skill, `${pack}/${skill}`);
        wanted = digestFiles(files);
      } catch (error) {
        rows.push({ pack, skill, status: "invalid", detail: (error as Error).message });
        continue;
      }
      if (!record) {
        rows.push({
          pack,
          skill,
          status: existsSync(destination) ? "collision" : "not-deployed",
        });
        continue;
      }
      if (!existsSync(destination)) {
        rows.push({ pack, skill, status: "missing" });
        continue;
      }
      try {
        const actual = digestTree(destination);
        if (record.digestPolicy !== DIGEST_POLICY) {
          if (legacyDigestTree(destination) !== record.installedDigest) {
            rows.push({
              pack,
              skill,
              status: "migration-required",
              detail: "legacy digest differs; review before adopting generated-artifact exclusions",
            });
            continue;
          }
        } else if (actual !== record.installedDigest) {
          rows.push({ pack, skill, status: "drifted" });
          continue;
        }
        rows.push({ pack, skill, status: wanted === actual ? "current" : "outdated" });
      } catch (error) {
        rows.push({ pack, skill, status: "invalid", detail: (error as Error).message });
      }
    }
    for (const stale of Object.keys(state.packs[pack]?.skills ?? {}).filter((skill) => !seen.has(skill))) {
      rows.push({ pack, skill: stale, status: "outdated", detail: "removed from pack manifest" });
    }
  }
  return rows;
}

function printStatuses(rows: StatusRow[]): void {
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

async function main(): Promise<void> {
  const [command = "status", ...args] = process.argv.slice(2);
  if (command === "-h" || command === "--help" || command === "help") {
    console.log(HELP);
    return;
  }
  const config = defaultCodexDeployConfig();
  try {
    switch (command) {
      case "status": {
        const target = args[0];
        printStatuses(getStatuses(config, target && target !== "--all" ? target : undefined));
        break;
      }
      case "deploy":
        if (args.length === 0) throw new Error("usage: tools/packs-codex deploy <pack>...");
        for (const pack of args) {
          for (const line of deployPack(config, pack)) console.log(line);
        }
        break;
      case "sync": {
        const target = args[0];
        if (!target) throw new Error("usage: tools/packs-codex sync <pack>|--all");
        if (target === "--all") {
          const state = readState(config);
          for (const pack of Object.keys(state.packs).sort()) {
            console.log(pack);
            for (const line of syncPack(config, pack)) console.log(`  ${line}`);
          }
        } else {
          for (const line of syncPack(config, target)) console.log(line);
        }
        break;
      }
      case "remove":
        if (args.length === 0) throw new Error("usage: tools/packs-codex remove <pack>...");
        for (const pack of args) {
          for (const line of removePack(config, pack)) console.log(line);
        }
        break;
      case "migrate-generated": {
        const pack = args.find((arg) => !arg.startsWith("--"));
        if (!pack || args.some((arg) => arg !== pack && arg !== "--accept-current")) {
          throw new Error(
            "usage: tools/packs-codex migrate-generated <pack> --accept-current",
          );
        }
        for (const line of migrateGeneratedArtifacts(config, pack, args.includes("--accept-current"))) {
          console.log(line);
        }
        break;
      }
      case "profile":
      case "use": {
        const profileName = args[0];
        if (profileName) {
          const profiles = readProfiles(config);
          const profile = profiles.find((p) => p.name === profileName);
          if (!profile) throw new Error(`no such profile: '${profileName}'`);
          for (const line of activateProfile(config, profile)) console.log(line);
        } else {
          const profiles = readProfiles(config);
          if (profiles.length === 0) { console.log("No profiles defined. Add them to profiles.toml."); break; }
          console.log();
          for (let i = 0; i < profiles.length; i++) {
            const active = profileActivePacks(config, profiles[i]).length > 0 ? " [active]" : "";
            console.log(`  ${(i + 1).toString().padEnd(3)} ${profiles[i].name.padEnd(20)} ${profiles[i].description}${active}`);
          }
          const rl = createInterface({ input: process.stdin, output: process.stdout });
          const answer = await rl.question("\nSelect profile (number or name): ");
          rl.close();
          const num = parseInt(answer, 10);
          let profile: Profile | undefined;
          if (!isNaN(num) && num >= 1 && num <= profiles.length) {
            profile = profiles[num - 1];
          } else {
            profile = profiles.find((p) => p.name === answer);
          }
          if (!profile) { console.log(`No profile matches '${answer}'`); break; }
          for (const line of activateProfile(config, profile)) console.log(line);
        }
        break;
      }
      default:
        throw new Error(`unknown command '${command}'\n\n${HELP}`);
    }
  } catch (error) {
    console.error(`packs-codex: ${(error as Error).message}`);
    process.exit(1);
  }
}

if (import.meta.main) await main();
