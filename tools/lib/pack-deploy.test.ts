// tools/lib/pack-deploy.test.ts — the two behaviours the shared engine grew when
// the Claude adapter stopped using symlinks on 2026-08-09: the whole-directory
// tree convention, and adoption of copies that already sit at the destination.
//
// The pre-existing engine behaviour is covered by tools/packs-codex.test.ts,
// which was deliberately left pointing at packs-codex.ts so the extraction had a
// check it could not quietly pass.
//
// Run: bun test tools/lib/pack-deploy.test.ts

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  adoptPack,
  deployPack,
  getStatuses,
  packUnits,
  readState,
  removePack,
  treeKey,
  type DeployConfig,
} from "./pack-deploy.ts";

let root: string;
let config: DeployConfig;

function writeSkill(pack: string, skill: string, body = "shared instructions"): void {
  const dir = join(root, "Axon", "Packs", pack, "skills", skill);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "SKILL.md"), `---\nname: ${skill}\ndescription: does a thing\n---\n\n${body}\n`);
}

function writeAgents(pack: string, ...names: string[]): void {
  const dir = join(root, "Axon", "Packs", pack, "agents");
  mkdirSync(dir, { recursive: true });
  for (const name of names) writeFileSync(join(dir, `${name}.md`), `# ${name}\n`);
}

function writeManifest(pack: string, skills: string[]): void {
  const dir = join(root, "Axon", "Packs", pack);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, "pack.toml"),
    `name = "${pack}"\ndescription = "test pack"\nskills = [${skills.map((s) => `"${s}"`).join(", ")}]\n`,
  );
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "axon-pack-deploy-test-"));
  config = {
    axonRoot: join(root, "Axon"),
    destination: join(root, "home", ".claude", "skills"),
    stateFile: join(root, "state", "claude.json"),
    adapter: "claude",
    stateEnvVar: "AXON_CLAUDE_STATE_FILE",
    treeConvention: {
      sourceDir: "agents",
      destinationRoot: join(root, "home", ".claude", "agents"),
    },
  };
  writeManifest("demo", ["demo-skill"]);
  writeSkill("demo", "demo-skill");
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("the tree convention", () => {
  test("a pack without the directory yields skills only", () => {
    expect(packUnits(config, "demo").map((u) => u.key)).toEqual(["demo-skill"]);
  });

  test("a pack carrying it gains one unit, not one per file", () => {
    writeAgents("demo", "reviewer", "skeptic", "auditor");
    const units = packUnits(config, "demo");
    expect(units.map((u) => u.key)).toEqual(["demo-skill", "agents/"]);
    expect(units[1].isSkill).toBe(false);
  });

  test("an adapter that declares no convention never sees the directory", () => {
    writeAgents("demo", "reviewer");
    const codexish: DeployConfig = { ...config, adapter: "codex", treeConvention: undefined };
    expect(packUnits(codexish, "demo").map((u) => u.key)).toEqual(["demo-skill"]);
  });

  test("the tree deploys to its own root, under the pack name", () => {
    writeAgents("demo", "reviewer");
    deployPack(config, "demo");
    expect(readFileSync(join(config.treeConvention!.destinationRoot, "demo", "reviewer.md"), "utf8")).toContain("reviewer");
  });

  test("its ledger key cannot collide with a skill name", () => {
    // Skill names are lowercase-hyphen-case; the trailing slash makes the tree key
    // unrepresentable as one, so the impossibility is structural.
    expect(treeKey("agents")).toBe("agents/");
    expect(/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(treeKey("agents"))).toBe(false);
  });

  test("remove takes the tree with it", () => {
    writeAgents("demo", "reviewer");
    deployPack(config, "demo");
    removePack(config, "demo");
    expect(getStatuses(config, "demo").map((r) => r.status)).toEqual(["not-deployed", "not-deployed"]);
  });
});

describe("adoptPack", () => {
  function placeIdenticalCopy(): void {
    mkdirSync(join(config.destination, "demo-skill"), { recursive: true });
    writeFileSync(
      join(config.destination, "demo-skill", "SKILL.md"),
      readFileSync(join(config.axonRoot, "Packs", "demo", "skills", "demo-skill", "SKILL.md")),
    );
  }

  test("an unowned copy is a collision until adopted", () => {
    placeIdenticalCopy();
    expect(getStatuses(config, "demo")[0].status).toBe("collision");
    adoptPack(config, "demo");
    expect(getStatuses(config, "demo")[0].status).toBe("current");
  });

  test("adoption writes the ledger without touching the destination", () => {
    placeIdenticalCopy();
    const before = readFileSync(join(config.destination, "demo-skill", "SKILL.md"), "utf8");
    adoptPack(config, "demo");
    expect(readFileSync(join(config.destination, "demo-skill", "SKILL.md"), "utf8")).toBe(before);
    expect(readState(config).packs.demo.skills["demo-skill"]).toBeDefined();
  });

  test("a destination that differs is refused, never claimed", () => {
    // The whole safety argument for adoption is the digest match. A copy that
    // differs is a hand edit or a stale deploy, and recording it would assert
    // something untrue about what is on disk.
    mkdirSync(join(config.destination, "demo-skill"), { recursive: true });
    writeFileSync(
      join(config.destination, "demo-skill", "SKILL.md"),
      `---\nname: demo-skill\ndescription: does a thing\n---\n\nEDITED BY HAND\n`,
    );
    expect(() => adoptPack(config, "demo")).toThrow("refusing to adopt");
    expect(readState(config).packs.demo).toBeUndefined();
  });

  test("nothing at the destination adopts nothing rather than failing", () => {
    expect(adoptPack(config, "demo")).toEqual(["= demo-skill (not deployed; nothing to adopt)"]);
  });

  test("re-adopting is a no-op, so it is safe to run twice", () => {
    placeIdenticalCopy();
    adoptPack(config, "demo");
    expect(adoptPack(config, "demo")).toEqual(["= demo-skill (already owned)"]);
  });

  test("a unit another pack owns is refused", () => {
    placeIdenticalCopy();
    adoptPack(config, "demo");
    writeManifest("rival", ["demo-skill"]);
    writeSkill("rival", "demo-skill");
    expect(() => adoptPack(config, "rival")).toThrow("already owned by Pack 'demo'");
  });

  test("the tree unit adopts on the same terms", () => {
    writeAgents("demo", "reviewer");
    placeIdenticalCopy();
    const treeDest = join(config.treeConvention!.destinationRoot, "demo");
    mkdirSync(treeDest, { recursive: true });
    writeFileSync(join(treeDest, "reviewer.md"), "# reviewer\n");
    expect(adoptPack(config, "demo")).toEqual(["✓ demo-skill adopted", "✓ agents/ adopted"]);
  });
});
