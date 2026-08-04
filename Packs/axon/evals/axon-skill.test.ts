import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const evalDir = dirname(import.meta.path);
const packDir = resolve(evalDir, "..");
const skillDir = join(packDir, "skills", "axon");
const referencesDir = join(skillDir, "references");
const repoDir = resolve(packDir, "../..");
const cases = JSON.parse(readFileSync(join(evalDir, "axon-skill-cases.json"), "utf8"));
const doctrine = JSON.parse(readFileSync(join(evalDir, "bootstrap-doctrine.json"), "utf8"));

describe("axon skill contract", () => {
  test("covers every intended harness and evaluation kind", () => {
    expect(cases.harnesses).toEqual(["claude", "codex", "opencode", "pi"]);
    expect(new Set(cases.cases.map((entry: any) => entry.kind))).toEqual(
      new Set(["triggering", "functional", "cross-harness", "negative-trigger"]),
    );
  });

  test("every routed reference is a direct skill resource", () => {
    for (const entry of cases.cases) {
      for (const reference of entry.references) {
        expect(reference).not.toContain("/");
        expect(existsSync(join(referencesDir, reference))).toBe(true);
      }
    }
  });

  test("bootstrap delegates to the skill and README doctrine", () => {
    for (const path of Object.values(doctrine.bootstrap) as string[]) {
      expect(existsSync(join(repoDir, path))).toBe(true);
    }
    expect(readFileSync(join(repoDir, "CLAUDE.md"), "utf8")).toBe("@AGENTS.md\n");
    expect(readFileSync(join(repoDir, "AGENTS.md"), "utf8")).toContain("Use the `axon` skill");

    const readme = readFileSync(join(repoDir, "README.md"), "utf8");
    for (const heading of doctrine.readme_headings) {
      expect(readme).toMatch(new RegExp(`^#{2,3} ${heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`, "m"));
    }
  });

  test("scripts are executable and the retired skill is absent", () => {
    for (const script of ["axon-root", "axon-context", "axapi"]) {
      expect(statSync(join(skillDir, "scripts", script)).mode & 0o111).not.toBe(0);
    }
    expect(existsSync(join(packDir, "skills", "axon-operate"))).toBe(false);
  });

  test("repository entrypoints delegate to the Pack-owned scripts", () => {
    for (const script of ["axon-context", "axapi"]) {
      const entrypoint = join(repoDir, "scripts", script);
      expect(statSync(entrypoint).mode & 0o111).not.toBe(0);
      expect(readFileSync(entrypoint, "utf8")).toContain(
        `Packs/axon/skills/axon/scripts/${script}`,
      );
    }
  });

  test("skill contains no fixed localhost port or endpoint inventory", () => {
    const skill = readFileSync(join(skillDir, "SKILL.md"), "utf8");
    expect(skill).not.toMatch(/127\.0\.0\.1:\d+/);
    expect(skill).not.toContain("axon-operate");
  });
});
