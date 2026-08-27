# Contributing to Axon

Axon accepts changes that improve the reusable public shell. Personal data and deployment state
stay in a private overlay. The same boundary covers credentials, private host details, and
operator-specific policy.

## Before writing code

Name the consumer and the outcome. An interesting technology without a concrete Axon consumer
remains an idea, not an implementation commitment.

No backlog entry is required to start. Add a claim to the owning `ISA.md` only when something
must outlive the change itself: a defect being left unfixed, or a decision that needs a record.
Write it as a claim with the probe that would falsify it, not as a description. The issue tracker
takes reports from outside the project; it is not where this project's work is planned.

Before external code or adopted design influence enters the tree, record its canonical source and
exact pin in `upstreams.toml`. Record the license and verdict there too, then state precisely what
Axon adopts.

## Work on one change

Start from current `main` and create a branch named for the change: `<area>-<short-slug>`. Keep
the diff inside one coherent boundary. Put reusable code and doctrine in Axon; use synthetic fixtures for
data-shaped tests. Never copy an active overlay or secret value into public work. Workstation paths
and private logs must also stay out of commits and GitHub text, including screenshots and test
failures.

Run `tools/doctor` before editing and record unrelated or machine-only failures separately. A
fresh source checkout does not need a real private overlay for CI; repository tests use synthetic
configuration where a machine contract is required.

## Validate the changed boundary

Run the nearest tests and checks declared by the package you changed. Then inspect the focused
diff, `git diff --check`, and `git status --short` before committing. Common repository checks
are:

~~~sh
cargo test --workspace --locked
bun test
tools/check-publication-hygiene.sh
~~~

Rust packages are members of the root `Cargo.toml` workspace and share the
root `Cargo.lock`. Keep each package's direct dependencies in that package's
manifest; do not add a nested lockfile. For a Rust change, verify the workspace
view and the build:

~~~sh
cargo metadata --locked --no-deps --format-version 1
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo check --workspace --locked
cargo test --workspace --locked
~~~

Run the format and Clippy commands from the repository root. They use the exact
toolchain and components pinned in `rust-toolchain.toml`; do not replace a finding
with a workspace-wide allowance. A narrow allowance belongs beside the affected
item and must explain the invariant that makes the lint inapplicable.

That command needs no database service and no environment variable. The
database-backed suites are `db_tests::` — one module name across the workspace,
which is what makes them selectable — and each test opens a temp SQLite file of
its own. Run them alone with:

~~~sh
cargo test --workspace --locked -- db_tests::
~~~

Until PRD Q45 (2026-08-27) these suites needed a running Postgres and six
`*_TEST_DATABASE_URL` variables, so the hermetic command carried
`--skip postgres_tests::` and CI ran a second job with a service container.
Both are gone; a checkout with no overlay and no server runs everything.

Run `bun run check` in `dashboard/` when dashboard code changes. Manifest or
generated-architecture changes also require:

~~~sh
tools/generate-architecture.sh
tools/check-architecture-fresh.sh
~~~

Do not describe a skipped or unavailable check as passing.

The same rule applies inside a test. An assertion that needs something only one platform has —
`/dev/full`, a container runtime, a specific filesystem — may be given up on a developer machine
and never in an automated run, where "it runs in CI" would otherwise be an assumption nobody can
see failing. Guard it with `skippable` from `tools/lib/test-support.sh`: outside CI it prints what
coverage was lost, and inside CI it fails.

## Open the pull request

Open a draft pull request first. State the outcome, then bound the exact scope. List every
completed validation command with its result and name the known limits. Keep separable follow-ups
out of the pull request rather than widening it. A merge should land one coherent change and
remain easy to review or revert.

Security findings follow [SECURITY.md](SECURITY.md); never report one in a public issue or pull
request.
