# Dependencies and build decisions

## Dependency choice

Apply this order: adopt, contribute upstream, overlay, temporary fork, build. Record the verdict,
pin, license, and reason before consumption. Never use an unpinned floating release for code.

## Implementation defaults

- Add backend logic in Rust unless the existing owner has a justified different runtime.
- Use `uv` for Python execution and `bun` for TypeScript; do not add `pip`, `npm`, or bare `node`
  commands to Axon code or documentation.
- Keep shell compatible with macOS Bash 3.2. Avoid associative arrays and Bash 4 features.
- Express container-backed capabilities as manifests consumed by shared runners, not new bespoke
  lifecycle scripts.
- Import shared schemas and declare cross-capability service requirements; do not import another
  capability's implementation.

## Bazel decision

Argue Bazel per case. Use it when the dependency graph, hermeticity, generated-artifact freshness,
or production build consumption is material. Keep a script outside Bazel when wrapping it adds a
toolchain or runfiles burden without a dependency benefit. Record the reason beside the target or
tool that enforces the decision.
