// tools/lib/pack-deploy-flat.test.ts — the flat-file convention, which the pi
// adapter needs for its agent files.
//
// Why it is separate from the tree convention, and the reason the two tests that
// matter most are here: pi's agent loader reads `*.md` in ONE flat directory and
// does not recurse, so two Packs land in the same destination root. The tree
// convention's single whole-directory unit cannot express that — both Packs would
// claim the same path — so ownership has to be per file. The tests below deploy two
// Packs into one root and assert each still owns only its own files.
//
// The other load-bearing pair: the transform must be applied on the way out, and the
// digest must be taken over the TRANSFORMED bytes. Hashing the source (the obvious
// implementation) passes a deploy and then reports the deployment as drifted
// forever, because the installed bytes never equal the source again.
//
// Run: bun test tools/lib/pack-deploy-flat.test.ts

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  deployPack,
  getStatuses,
  packUnits,
  readState,
  removePack,
  syncPack,
  type DeployConfig,
} from "./pack-deploy.ts";

let root: string;
let config: DeployConfig;

function writeManifest(pack: string, skills: string[] = []): void {
  const dir = join(root, "Axon", "Packs", pack);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, "pack.toml"),
    `name = "${pack}"\ndescription = "test pack"\nskills = [${skills.map((s) => `"${s}"`).join(", ")}]\n`,
  );
}

function writeAgent(pack: string, file: string, body: string): void {
  const dir = join(root, "Axon", "Packs", pack, "agents");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, file), body);
}

function writeSkill(pack: string, skill: string): void {
  const dir = join(root, "Axon", "Packs", pack, "skills", skill);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "SKILL.md"), `---\nname: ${skill}\ndescription: does a thing\n---\n\nbody\n`);
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "axon-pack-flat-test-"));
  config = {
    axonRoot: join(root, "Axon"),
    // Deliberately a destination nothing deploys to: the point of the convention is
    // that a harness which owns its own skill selection still gets its agent files.
    destination: join(root, "home", ".pi", "agent", "skills"),
    stateFile: join(root, "state", "pi-agents.json"),
    adapter: "pi",
    skipManifestSkills: true,
    flatFileConvention: {
      sourceDir: "agents",
      destinationRoot: join(root, "home", ".pi", "agent", "agents"),
      transform: (content, label) => `${content}translated:${label}\n`,
    },
  };
  writeManifest("deliberation", ["council"]);
  writeSkill("deliberation", "council");
  writeAgent("deliberation", "council-clerk.md", "clerk body\n");
  writeAgent("deliberation", "council-skeptic.md", "skeptic body\n");
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("the flat-file convention", () => {
  test("each file is its own unit", () => {
    expect(packUnits(config, "deliberation").map((u) => u.key))
      .toEqual(["agents/council-clerk.md", "agents/council-skeptic.md"]);
  });

  test("a pack with no such directory yields nothing when skills are skipped", () => {
    writeManifest("empty");
    expect(packUnits(config, "empty")).toEqual([]);
  });

  test("a skill the pack declares is not deployed, because the harness owns that", () => {
    deployPack(config, "deliberation");
    expect(existsSync(join(config.destination, "council"))).toBe(false);
  });

  test("files land directly in the destination root, not a per-pack subdirectory", () => {
    deployPack(config, "deliberation");
    const dest = config.flatFileConvention!.destinationRoot;
    expect(existsSync(join(dest, "council-clerk.md"))).toBe(true);
    expect(existsSync(join(dest, "deliberation"))).toBe(false);
  });

  test("the transform is applied to what lands on disk", () => {
    deployPack(config, "deliberation");
    const installed = readFileSync(join(config.flatFileConvention!.destinationRoot, "council-clerk.md"), "utf8");
    expect(installed).toBe("clerk body\ntranslated:deliberation/agents/council-clerk.md\n");
  });

  test("a fresh deployment reports current, not drifted", () => {
    deployPack(config, "deliberation");
    // The assertion that catches hashing the source instead of the transform: that
    // implementation passes the deploy and then reports every file as drifted.
    for (const row of getStatuses(config, "deliberation")) expect(row.status).toBe("current");
  });

  test("an edit at the destination is reported as drift and sync refuses to erase it", () => {
    deployPack(config, "deliberation");
    const installed = join(config.flatFileConvention!.destinationRoot, "council-clerk.md");
    writeFileSync(installed, "hand edited\n");
    expect(getStatuses(config, "deliberation").some((row) => row.status === "drifted")).toBe(true);
    expect(() => syncPack(config, "deliberation")).toThrow(/local changes/);
    expect(readFileSync(installed, "utf8")).toBe("hand edited\n");
  });

  test("a source edit is published by sync", () => {
    deployPack(config, "deliberation");
    writeAgent("deliberation", "council-clerk.md", "clerk body v2\n");
    expect(syncPack(config, "deliberation")).toContain("✓ agents/council-clerk.md synced");
    expect(readFileSync(join(config.flatFileConvention!.destinationRoot, "council-clerk.md"), "utf8"))
      .toContain("clerk body v2");
  });

  test("remove deletes every file it owns and nothing else", () => {
    deployPack(config, "deliberation");
    const dest = config.flatFileConvention!.destinationRoot;
    writeFileSync(join(dest, "someone-elses.md"), "not ours\n");
    removePack(config, "deliberation");
    expect(existsSync(join(dest, "council-clerk.md"))).toBe(false);
    expect(existsSync(join(dest, "council-skeptic.md"))).toBe(false);
    expect(existsSync(join(dest, "someone-elses.md"))).toBe(true);
  });

  test("the destination is empty after removing the only pack", () => {
    deployPack(config, "deliberation");
    removePack(config, "deliberation");
    expect(readState(config).packs.deliberation).toBeUndefined();
  });
});

describe("two packs sharing one flat destination", () => {
  beforeEach(() => {
    writeManifest("academic-writing", ["academic-writing"]);
    writeAgent("academic-writing", "academic-writing-logic-reviewer.md", "logic body\n");
    writeAgent("academic-writing", "academic-writing-skeptic-claims.md", "claims body\n");
  });

  test("each pack deploys into the same root without colliding", () => {
    deployPack(config, "deliberation");
    deployPack(config, "academic-writing");
    const dest = config.flatFileConvention!.destinationRoot;
    expect(existsSync(join(dest, "council-clerk.md"))).toBe(true);
    expect(existsSync(join(dest, "academic-writing-logic-reviewer.md"))).toBe(true);
  });

  test("removing one pack leaves the other's files in place", () => {
    deployPack(config, "deliberation");
    deployPack(config, "academic-writing");
    removePack(config, "deliberation");
    const dest = config.flatFileConvention!.destinationRoot;
    expect(existsSync(join(dest, "council-clerk.md"))).toBe(false);
    expect(existsSync(join(dest, "academic-writing-logic-reviewer.md"))).toBe(true);
  });

  test("a pack cannot claim a file another pack already owns", () => {
    deployPack(config, "deliberation");
    // Same filename from a second pack is a collision, and the error names the owner.
    writeAgent("academic-writing", "council-clerk.md", "impostor\n");
    expect(() => deployPack(config, "academic-writing")).toThrow(/already owned by Pack 'deliberation'/);
  });
});

describe("flat-file sources that cannot be deployed honestly", () => {
  test("a non-.md file is refused rather than silently inert", () => {
    writeAgent("deliberation", "notes.txt", "not an agent\n");
    expect(() => packUnits(config, "deliberation")).toThrow(/only .md files deploy/);
  });

  test("a subdirectory is refused, because the harness would not read it", () => {
    mkdirSync(join(root, "Axon", "Packs", "deliberation", "agents", "nested"), { recursive: true });
    expect(() => packUnits(config, "deliberation")).toThrow(/is not a file/);
  });
});

describe("an adapter with no flat-file convention never sees the directory", () => {
  test("a claude-style config takes only the tree unit", () => {
    const treeish: DeployConfig = { ...config, skipManifestSkills: false, flatFileConvention: undefined };
    expect(packUnits(treeish, "deliberation").map((u) => u.key)).toEqual(["council"]);
  });
});
