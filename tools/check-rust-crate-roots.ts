// tools/check-rust-crate-roots.ts — every Rust target whose srcs offer more than one
// candidate crate root must name the one it means (Axon#46).
//
// The failure this exists for looked like a working build. Adding
// `//libs/axon-config:src/lib.rs` to capabilities/comms' srcs put a second file named
// lib.rs in the list; rules_rust infers the crate root by basename, picked axon-config's,
// and produced an rlib exposing none of comms' modules while reporting success.
// `//capabilities/comms:comms_test` still passed. Only the downstream binary failed to
// compile, and only under a full `bazel test //...`.
//
// Every target that includes a shared lib now sets crate_root, so the instance is fixed.
// This closes the class: the next capability to include one would hit the same inference,
// and the same "successful" build.
//
// Why the ambiguity rule is "more than one file with the same candidate basename" rather
// than "more than one candidate": one lib.rs and one main.rs is not ambiguous — rules_rust
// picks lib.rs for a library and main.rs for a binary, which is what both mean. Requiring
// crate_root there would manufacture a rule with no failure behind it.
//
// Why a script and not a Bazel test: it reads resolved srcs through `bazel query`, and a
// Bazel test cannot run Bazel. Same shape as tools/check-bazel-package-labels.sh, which is
// a script for its own structural reason. Runs in the CI job that already has Bazel and its
// caches; deliberately not in tools/doctor, because toolchain.toml declares Bazel optional
// on a dev box and doctor is the sweep that works without it.
//
//   tools/check-rust-crate-roots            # exit 1 on the first ambiguous target
//
// TypeScript rather than bash: one `bazel query --output=streamed_jsonproto` answers for
// every target at once, where the shell equivalent is one query per target.

import { basename } from "node:path";

/** One Rust target, reduced to what the rule reads. */
export interface RustTarget {
  /** Bazel label, e.g. //capabilities/comms:comms. */
  label: string;
  /** rust_library, rust_binary, rust_test. */
  kind: string;
  /** Resolved srcs labels — after glob expansion, which is why this reads `bazel query`. */
  srcs: string[];
  /** The explicit crate root, or null when the rule is left to infer one. */
  crateRoot: string | null;
}

export interface Violation {
  target: RustTarget;
  /** The basename that appears more than once, e.g. "lib.rs". */
  ambiguous: string;
  /** The srcs carrying it, in query order — the first is the one inference tends to take. */
  candidates: string[];
}

// rules_rust infers a crate root by looking for these names, in this order, among srcs.
// A second file wearing the same name is what makes the inference a coin flip.
const ROOT_BASENAMES = ["lib.rs", "main.rs"];

/** Targets that leave rules_rust to guess between same-named candidates. */
export function crateRootViolations(targets: RustTarget[]): Violation[] {
  const violations: Violation[] = [];
  for (const target of targets) {
    if (target.crateRoot) continue;
    for (const name of ROOT_BASENAMES) {
      // The label's basename, not the file's: a src is `//libs/axon-config:src/lib.rs`,
      // and it is the trailing path segment that rules_rust matches on.
      const candidates = target.srcs.filter((s) => basename(s) === name);
      if (candidates.length > 1) violations.push({ target, ambiguous: name, candidates });
    }
  }
  return violations;
}

const QUERY = 'kind("rust_(library|binary|test) rule", //...)';

/** Every Rust target in the workspace, with srcs resolved by Bazel. */
function queryTargets(root: string): RustTarget[] {
  const proc = Bun.spawnSync({
    cmd: ["bazel", "query", QUERY, "--output=streamed_jsonproto"],
    cwd: root,
    stdout: "pipe",
    stderr: "pipe",
  });
  if (proc.exitCode !== 0) {
    throw new Error(`bazel query failed:\n${proc.stderr.toString().trim()}`);
  }

  const targets: RustTarget[] = [];
  for (const line of proc.stdout.toString().split("\n")) {
    if (!line.trim()) continue;
    const rule = JSON.parse(line).rule;
    if (!rule) continue;
    const attrs = new Map<string, { stringValue?: string; stringListValue?: string[] }>(
      rule.attribute.map((a: { name: string }) => [a.name, a]),
    );
    targets.push({
      label: rule.name,
      kind: rule.ruleClass,
      srcs: attrs.get("srcs")?.stringListValue ?? [],
      // An unset label attribute comes back as an empty string, not as an absent one.
      crateRoot: attrs.get("crate_root")?.stringValue || null,
    });
  }
  return targets;
}

function main(): number {
  const root = process.env.AXON_ROOT ?? new URL("..", import.meta.url).pathname;

  let targets: RustTarget[];
  try {
    targets = queryTargets(root);
  } catch (err) {
    console.error(`check-rust-crate-roots: ${err instanceof Error ? err.message : String(err)}`);
    return 1;
  }

  if (targets.length === 0) {
    // A query that matched nothing is a broken gate reporting a clean repository — the
    // same false green this check exists to prevent, one level up.
    console.error(`check-rust-crate-roots: '${QUERY}' matched no target — the query is wrong, not the repository empty`);
    return 1;
  }

  const violations = crateRootViolations(targets);
  for (const v of violations) {
    console.error(
      `FAIL ${v.target.label} (${v.target.kind}): srcs carry ${v.candidates.length} files named ${v.ambiguous} and no crate_root — rules_rust picks one by basename, and picking the wrong one still reports a successful build (Axon#46)`,
    );
    for (const c of v.candidates) console.error(`       ${c}`);
  }
  if (violations.length > 0) {
    console.error(`check-rust-crate-roots: ${violations.length} target(s) leave their crate root to inference.`);
    return 1;
  }

  console.log(`check-rust-crate-roots: ${targets.length} Rust target(s), none ambiguous.`);
  return 0;
}

if (import.meta.main) process.exit(main());
