// tools/doctor.test.ts — planted-fixture regression test for the two pure
// checks doctor.ts's "Systems (systems.toml)" section and "Undeclared
// connections" sweep are built on. Guards against the silent-green failure
// mode a grep-pattern sweep is otherwise prone to (pattern typo, path
// convention change) — see README.md#documentation-stays-owned-and-current.
// Run: bun test tools/doctor.test.ts

import { describe, expect, test } from "bun:test";
import {
  checkStateMountCoverage,
  collectWhyBlocks,
  findDanglingDecisionRefs,
  extractSiblingRepoRefs,
  parseMirrorDivergence,
  parseProjectManifestPaths,
  pointerSearchRoots,
  findDecisionPathRot,
  findPlaintextSecretsInEnvTemplate,
  parseEnvTemplateLines,
  formatFetchAge,
  formatVersion,
  findProductionListenerConstructs,
  isSweepExempt,
  stripRustCfgTestItems,
  whyBlockBases,
} from "./doctor.ts";

describe("production Rust server policy", () => {
  test("test-only listener constructs are excluded", () => {
    const source = `
fn build_router() -> Router { Router::new() }
fn main() { axon_server::serve_local("fixture", 1234, build_router()); }

#[cfg(test)]
mod tests {
  async fn serve() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    axum::serve(listener, build_router()).await.unwrap();
  }
}`;

    expect(findProductionListenerConstructs(source)).toEqual([]);
    expect(stripRustCfgTestItems(source)).toContain("axon_server::serve_local");
  });

  test("production listener constructs remain findings", () => {
    const source = `
fn build_router() -> Router { Router::new() }
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  axum::serve(listener, build_router()).await.unwrap();
}`;

    expect(findProductionListenerConstructs(source)).toEqual(["axum::serve", "TcpListener::bind"]);
  });

  test("a test module cannot hide a production listener", () => {
    const source = `
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
}

#[cfg(test)]
mod tests {
  async fn serve() { axum::serve(listener, app).await.unwrap(); }
}`;

    expect(findProductionListenerConstructs(source)).toEqual(["TcpListener::bind"]);
  });
});

describe("checkStateMountCoverage", () => {
  test("mount with a matching systems.toml identity is covered", () => {
    const { covered, uncovered } = checkStateMountCoverage(
      [{ tool: "lifeos" }, { tool: "knowledge-base" }],
      new Set(["lifeos", "knowledge-base", "mach-mono"]),
    );
    expect(covered).toEqual(["lifeos", "knowledge-base"]);
    expect(uncovered).toEqual([]);
  });

  test("mount with no systems.toml identity is uncovered — the real gap direction", () => {
    const { covered, uncovered } = checkStateMountCoverage(
      [{ tool: "lifeos" }, { tool: "some-new-tool" }],
      new Set(["lifeos"]),
    );
    expect(covered).toEqual(["lifeos"]);
    expect(uncovered).toEqual(["some-new-tool"]);
  });

  test("empty mounts is trivially fully covered", () => {
    expect(checkStateMountCoverage([], new Set(["lifeos"]))).toEqual({ covered: [], uncovered: [] });
  });
});

describe("extractSiblingRepoRefs", () => {
  test("plain top-level repo path", () => {
    expect(extractSiblingRepoRefs("see ~/Developer/mach-mono for the Swift monorepo")).toEqual(["mach-mono"]);
  });

  test("a direct sibling path planted in a public JSON example is detected", () => {
    const example = JSON.stringify({
      source: { path: "~/Developer/private-knowledge" },
    });
    expect(extractSiblingRepoRefs(example)).toEqual(["private-knowledge"]);
  });

  test("nested path resolves to the LAST segment, not the first — the planted regression case", () => {
    // This is the exact false positive the live run against Axon caught before the fix:
    // a naive first-segment match on ~/Developer/Personal/Knowledge-Base surfaced "Personal"
    // instead of "Knowledge-Base".
    expect(extractSiblingRepoRefs('"path": "~/Developer/Personal/Knowledge-Base"')).toEqual(["Knowledge-Base"]);
    expect(extractSiblingRepoRefs("~/Developer/Collab/VBB")).toEqual(["VBB"]);
  });

  test("$HOME form matches the same as ~", () => {
    expect(extractSiblingRepoRefs("$HOME/Developer/pi-agent")).toEqual(["pi-agent"]);
  });

  test("multiple distinct references in one blob", () => {
    expect(extractSiblingRepoRefs("~/Developer/Axon and ~/Developer/axon-overlay and $HOME/Developer/mach-mono")).toEqual([
      "Axon",
      "axon-overlay",
      "mach-mono",
    ]);
  });

  test("no match returns empty, not undefined/throw", () => {
    expect(extractSiblingRepoRefs("nothing relevant here, just prose")).toEqual([]);
  });

  test("does not match Developer/ paths outside $HOME/~ (narrow-by-design blind spot)", () => {
    expect(extractSiblingRepoRefs("/opt/Developer/something-else")).toEqual([]);
  });
});

describe("formatVersion", () => {
  test("tag-based describe shape", () => {
    expect(formatVersion("v1.2-3-gabc1234", "2026-07-16")).toBe("v1.2-3-gabc1234 (2026-07-16)");
  });

  test("bare-sha describe fallback (no tags in repo)", () => {
    expect(formatVersion("abc1234", "2026-07-16")).toBe("abc1234 (2026-07-16)");
  });

  test("git's own -dirty suffix passes through verbatim", () => {
    expect(formatVersion("abc1234-dirty", "2026-07-16")).toBe("abc1234-dirty (2026-07-16)");
  });

  test("missing commit date degrades to describe alone", () => {
    expect(formatVersion("abc1234", "")).toBe("abc1234");
  });

  test("empty describe reports honestly instead of ' ()'", () => {
    expect(formatVersion("", "2026-07-16")).toBe("(unknown — not a git checkout?)");
  });
});

describe("formatFetchAge", () => {
  const NOW = 1_800_000_000; // fixed epoch — the function is pure in (fetch, now)

  test("fresh fetch (<60s) is 'just now'", () => {
    expect(formatFetchAge(NOW - 5, NOW)).toBe("fetched just now");
  });

  test("minutes-old fetch", () => {
    expect(formatFetchAge(NOW - 25 * 60, NOW)).toBe("fetched 25 minute(s) ago");
  });

  test("hours-old fetch", () => {
    expect(formatFetchAge(NOW - 3 * 3600 - 40, NOW)).toBe("fetched 3 hour(s) ago");
  });

  test("days-old fetch", () => {
    expect(formatFetchAge(NOW - 2 * 86_400 - 3600, NOW)).toBe("fetched 2 day(s) ago");
  });

  test("missing FETCH_HEAD (null) reported honestly, not thrown", () => {
    expect(formatFetchAge(null, NOW)).toBe("no fetch recorded");
  });

  test("clock skew (fetch mtime in the future) clamps to 'just now' rather than negative", () => {
    expect(formatFetchAge(NOW + 120, NOW)).toBe("fetched just now");
  });
});

describe("findDecisionPathRot", () => {
  // The real 2026-07-16 failure: root-is-the-spine dissolved apps/, three entries kept
  // asserting it for twelve days, and nothing noticed.
  const present = (real: string[]) => (p: string) => real.includes(p);

  test("a named path that no longer exists is rot", () => {
    const rot = findDecisionPathRot(
      [{ slug: "stale", text: "consumes `apps/dashboard` over HTTP", assertsAbsent: [] }],
      present(["dashboard"]),
    );
    expect(rot).toEqual([{ slug: "stale", path: "apps/dashboard", kind: "missing" }]);
  });

  test("a path declared absent that came back is rot in the other direction", () => {
    const rot = findDecisionPathRot(
      [{ slug: "forbids", text: "no `tools/topology` binary", assertsAbsent: ["tools/topology"] }],
      present(["tools/topology"]),
    );
    expect(rot).toEqual([{ slug: "forbids", path: "tools/topology", kind: "present" }]);
  });

  test("asserts_absent silences the missing-path direction for the same path", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "no `tools/topology` binary", assertsAbsent: ["tools/topology"] }],
      present([]),
    )).toEqual([]);
  });

  test("a crate-relative reference resolves against the passed bases", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "an arm in `sources/mod.rs`", assertsAbsent: [] }],
      present(["capabilities/scouting/src/sources/mod.rs"]),
      ["", "capabilities/scouting/src/"],
    )).toEqual([]);
  });

  test("URLs, absolute paths, git refs and placeholders are not repo paths", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "`https://a.com/b` `/usr/local/bin/x` `origin/main` `<vault>/Atlas` `~/Developer/x`", assertsAbsent: [] }],
      present([]),
    )).toEqual([]);
  });

  // The only false positive this check has produced: a model id backticked as the label of
  // its own huggingface link read as a repo path that had gone missing.
  test("a backticked slug labelling an external link is not a repo path", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "[`org/model-name` at the audited commit](https://huggingface.co/org/model-name/tree/abc)", assertsAbsent: [] }],
      present([]),
    )).toEqual([]);
  });

  test("a real repo path outside a link label is still checked", () => {
    expect(findDecisionPathRot(
      [{ slug: "stale", text: "[docs](https://a.com/b) and `apps/dashboard`", assertsAbsent: [] }],
      present([]),
    )).toEqual([{ slug: "stale", path: "apps/dashboard", kind: "missing" }]);
  });
});

describe("collectWhyBlocks", () => {
  const doc = [
    "# punctuality", "", "Some prose naming `capabilities/other/thing.rs`.", "",
    "## Why this shape: Rust over a second engine", "",
    "It reads parquet from `src/aggregate.rs`.", "",
    "## Considered and declined", "", "Naming `nope/gone.rs` here must not be swept.", "",
  ].join("\n");

  test("captures only the why-block, not the surrounding README", () => {
    const blocks = collectWhyBlocks("capabilities/punctuality/README.md", doc);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].text).toContain("src/aggregate.rs");
    expect(blocks[0].text).not.toContain("nope/gone.rs");
    expect(blocks[0].text).not.toContain("capabilities/other/thing.rs");
    expect(blocks[0].dir).toBe("capabilities/punctuality");
  });

  test("the heading topic lands in the slug so a finding is locatable", () => {
    expect(collectWhyBlocks("a/README.md", doc)[0].slug).toBe("a/README.md (Rust over a second engine)");
  });

  test("an asserts-absent comment inside the block is honoured", () => {
    const blocks = collectWhyBlocks("a/README.md",
      "## Why this shape: x\n\n<!-- asserts-absent: apps/dashboard, tools/topology -->\nno `apps/dashboard` here.\n");
    expect(blocks[0].assertsAbsent).toEqual(["apps/dashboard", "tools/topology"]);
  });

  test("a README with no why-block yields nothing", () => {
    expect(collectWhyBlocks("a/README.md", "# a\n\nplain prose with `some/path.rs`.\n")).toEqual([]);
  });

  test("two why-blocks in one file are captured separately", () => {
    const two = "## Why this shape: one\n\na\n\n## Why this shape: two\n\nb\n";
    expect(collectWhyBlocks("a/README.md", two)).toHaveLength(2);
  });
});

describe("parseEnvTemplateLines", () => {
  test("parses key/value pairs and strips inline comments", () => {
    const parsed = parseEnvTemplateLines(
      [
        "FOO=bar",
        "BAZ=\"quoted value\" # inline comment",
        "  # ignored comment",
        "",
        "X= # value can be blank",
      ].join("\n"),
    );
    expect(parsed).toEqual([
      { key: "FOO", value: "bar" },
      { key: "BAZ", value: "quoted value" },
      { key: "X", value: "" },
    ]);
  });
});

describe("findPlaintextSecretsInEnvTemplate", () => {
  test("ignores placeholders and flags obvious secret-like literals", () => {
    const leaks = findPlaintextSecretsInEnvTemplate(
      [
        "DOMAIN=example.local",
        "ADMIN_TOKEN=$argon2id$v=19$m=65536,t=3,p=4$...",
        "POSTGRES_PASSWORD=<required: private password>",
        "HA_TOKEN=<required: private token>",
        "DB_KEY=abc", // short, not enough entropy to be flagged
      ].join("\n"),
    );
    expect(leaks).toEqual(["ADMIN_TOKEN"]);
  });
});

describe("findDanglingDecisionRefs", () => {
  const alive = (s: string) => s === "root-is-the-spine-three-nouns";

  test("a citation of a dissolved entry is reported", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "README.md", text: "See `decisions/bazel-build-spine/README.md`." }], alive,
    )).toEqual([{ file: "README.md", slug: "bazel-build-spine" }]);
  });

  test("a live entry is not reported", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "README.md", text: "See `README.md#three-architectural-nouns`." }], alive,
    )).toEqual([]);
  });

  test("the same dead slug twice in one file reports once", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "a.md", text: "decisions/gone and again decisions/gone/README.md" }], alive,
    )).toHaveLength(1);
  });

  test("catches it in a generator that emits the path rather than citing it", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "tools/gen.sh", text: 'echo "See decisions/gone/README.md."' }], alive,
    )).toEqual([{ file: "tools/gen.sh", slug: "gone" }]);
  });
});

// The sweep's skip rule used to be a list of five file names, two of which described
// properties rather than exceptions (Axon#26). Repository and overlay names here are
// synthetic: a test that hardcodes this deployment's names would be the same mistake one
// level down.
describe("isSweepExempt", () => {
  const generated = "# Fixture Architecture\n\n> Auto-generated by tools/generate-fixture.sh. Do not edit manually.\n\n`~/Developer/example-repo/x`\n";

  test("a generated artifact is exempt by its own header, not by its name", () => {
    expect(isSweepExempt("FIXTURE.md", generated)).toBe(true);
  });

  test("the header only counts near the top, so a mention deep in prose is not an escape hatch", () => {
    const buried = `${"filler\n".repeat(20)}This file is auto-generated, honest.\n`;
    expect(isSweepExempt("notes.md", buried)).toBe(false);
  });

  test("a .example template is exempt — showing the path is its job", () => {
    expect(isSweepExempt("fixture.local.toml.example", 'overlay = "~/Developer/example-overlay"\n')).toBe(true);
  });

  test("the sanctioned indirection, the bootstrap namer and the sweep's own fixture stay exempt", () => {
    for (const f of ["tools/lib/paths.sh", "tools/install.sh", "tools/doctor.test.ts"]) {
      expect(isSweepExempt(f, '~/Developer/example-overlay')).toBe(true);
    }
  });

  test("an ordinary manifest is not exempt, whatever it holds", () => {
    expect(isSweepExempt("axon.toml", 'overlay = "~/Developer/example-overlay"\n')).toBe(false);
  });

  test("a file merely named like a template is not exempt", () => {
    // `.example` is a suffix rule, matching the env-template convention. A file called
    // example.md is documentation.
    expect(isSweepExempt("docs/example.md", "~/Developer/example-repo")).toBe(false);
  });
});

describe("whyBlockBases", () => {
  test("an owner, a unit inside it, and that unit's sources", () => {
    expect(whyBlockBases([
      "capabilities/example/README.md",
      "capabilities/example/src/lib.rs",
      "capabilities/example/src/sources/mod.rs",
    ])).toEqual([
      "",
      "capabilities/",
      "capabilities/example/",
      "capabilities/example/src/",
    ]);
  });

  test("a unit with no src/ gets no src/ base — the phantom the hand-list appended", () => {
    expect(whyBlockBases(["Packs/example/pack.toml", "Packs/example/skills/thing/SKILL.md"]))
      .not.toContain("Packs/example/src/");
  });

  test("every top-level owner the tree has, not the three someone remembered", () => {
    const bases = whyBlockBases([
      "dashboard/src/routes/+page.svelte",
      "schemas/service.toml.example",
      "tools/doctor.ts",
    ]);
    expect(bases).toContain("dashboard/");
    expect(bases).toContain("schemas/");
    expect(bases).toContain("tools/");
  });

  test("an untracked build tree cannot become a resolution base", () => {
    // node_modules/ is on disk beside a tracked package. Reading the directory instead of
    // the index would let a missing path resolve under a dependency and report clean.
    expect(whyBlockBases(["dashboard/package.json"])).toEqual(["", "dashboard/"]);
  });

  test("root-level files contribute the root base and nothing else", () => {
    expect(whyBlockBases(["README.md", "axon.toml"])).toEqual([""]);
  });
});

describe("LifeOS USER mirror divergence", () => {
  test("the clean report reads as zero", () => {
    expect(parseMirrorDivergence([
      "── LifeOS USER mirror (dry-run) ──",
      "  source: /somewhere/LIFEOS/USER",
      "  mirror: /elsewhere/resources/backups/lifeos/USER",
      "  up to date ✅",
    ].join("\n"))).toBe(0);
  });

  test("the count is read from the summary line, not from the file listing above it", () => {
    expect(parseMirrorDivergence([
      "── LifeOS USER mirror (dry-run) ──",
      "  Files /a/CACHE/freshness.json and /b/CACHE/freshness.json differ",
      "  Files /a/TELOS/SUMMARY.md and /b/TELOS/SUMMARY.md differ",
      "  2 path(s) diverged — run with --apply",
    ].join("\n"))).toBe(2);
  });

  test("a reworded tool reads as unrecognized, never as clean", () => {
    // The failure this exists for: a looser regex returning 0 here would report a
    // healthy mirror on a tool that no longer says anything about divergence.
    expect(parseMirrorDivergence("mirror sync complete, nothing to do")).toBeNull();
  });
});

describe("LifeOS PROJECTS.md pointers", () => {
  const manifest = [
    "## Projects Table",
    "",
    "| Project | Path | URL | Visibility |",
    "|---------|------|-----|-----------|",
    "| **Axon** | `~/Developer/Axon` | `github.com/x/Axon` | public-safe |",
    "| **widget-app** (Codename) | `~/Developer/Projects/widget-app` | _(no git)_ | private |",
    "",
    "## Open Sessions",
    "",
    "| Session | Stand | Offen |",
    "|---|---|---|",
    "| **Some Session** | still open | the next step |",
    "",
    "## Routing Aliases",
    "",
    "When the principal says... | the DA routes to...",
    "---|---",
    '"LifeOS", "this system" | `~/.claude`',
  ].join("\n");

  test("only the project table's rows are pointers", () => {
    expect(parseProjectManifestPaths(manifest)).toEqual([
      { name: "Axon", path: "~/Developer/Axon" },
      { name: "widget-app", path: "~/Developer/Projects/widget-app" },
    ]);
  });

  test("a bolded row with prose in cell two is not a pointer", () => {
    // Open Sessions satisfies the bold half and nothing else. Matching on bold alone
    // would turn every session line into a path that fails to resolve.
    expect(parseProjectManifestPaths(manifest).map((r) => r.name)).not.toContain("Some Session");
  });

  test("a backticked path with no bolded name is not a pointer", () => {
    // Routing Aliases satisfies the path half. It resolves today, so matching it would
    // pass — and would break the day an alias names something that is not a directory.
    expect(parseProjectManifestPaths('"LifeOS" | `~/.claude`')).toEqual([]);
  });

  test("a relative or bare-word cell is not a path", () => {
    expect(parseProjectManifestPaths("| **Thing** | `Developer/Thing` |")).toEqual([]);
    expect(parseProjectManifestPaths("| **Thing** | Axon |")).toEqual([]);
  });

  test("search roots are the manifest's own parents, deduped", () => {
    expect(pointerSearchRoots([
      "/h/Developer/Axon",
      "/h/Developer/axon-overlay",
      "/h/Developer/Projects/VBB",
      "/h/.claude",
    ])).toEqual(["/h", "/h/Developer", "/h/Developer/Projects"]);
  });
});
