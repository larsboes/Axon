# Verify proportionately

Select checks from the changed ownership boundary. Never claim broader validation than ran.

1. Run the nearest unit tests, type checks, or shell syntax checks for changed code.
2. Run repository gates whose declared inputs include the changed files.
3. Check generated artifacts when their source manifests changed.
4. Re-run `scripts/axon-context on <target>` or the relevant API read to verify observable state.
5. Inspect `git diff --check`, the focused diff, and `git status --short` before committing.

Common gates include `tools/self check`, `tools/upstream-checker`, `tools/audit`, and the
architecture freshness test, but only run a gate when its concern is in scope. Read the nearest
`BUILD.bazel`, README, or package configuration to discover exact targets rather than relying on a
static validation list.

Record skipped checks and pre-existing failures separately. A failing unrelated doctor item does
not invalidate a focused change, but it must not be described as passing.
