# Contributing to Axon

Axon accepts changes that improve the reusable public shell. Personal data and deployment state
stay in a private overlay. The same boundary covers credentials, private host details, and
operator-specific policy.

## Before writing code

Search the issue tracker first. Open a focused issue when the change affects architecture, data
ownership, a capability contract, an upstream dependency, or more than one package. Name the
consumer and the outcome. An interesting technology without a concrete Axon consumer remains an
idea, not an implementation commitment.

Before external code or adopted design influence enters the tree, record its canonical source and
exact pin in `upstreams.toml`. Record the license and verdict there too, then state precisely what
Axon adopts.

## Work on one issue

Start from current `main` and create `issue-<number>-<short-slug>`. Keep the diff inside that
issue's acceptance boundary. Put reusable code and doctrine in Axon; use synthetic fixtures for
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
bazel test //...
bun test ./tools/
tools/check-publication-hygiene.sh
~~~

The default Bazel command is hermetic and does not require Postgres. Store
integration tests are deliberately manual and fail rather than skip when their
database is absent. Run all four against a synthetic database with:

~~~sh
bazel test //:postgres_integration_tests \
  --test_env=SCOUTING_TEST_DATABASE_URL=postgresql://axon:axon@127.0.0.1:5432/axon \
  --test_env=TRANSIT_TEST_DATABASE_URL=postgresql://axon:axon@127.0.0.1:5432/axon \
  --test_env=COMMS_TEST_DATABASE_URL=postgresql://axon:axon@127.0.0.1:5432/axon \
  --test_env=TASKS_TEST_DATABASE_URL=postgresql://axon:axon@127.0.0.1:5432/axon
~~~

Run `bun run check` in `dashboard/` when dashboard code changes. Manifest or
generated-architecture changes also require:

~~~sh
bazel run //:generate_architecture
bazel test //:architecture_up_to_date_test
~~~

Do not describe a skipped or unavailable check as passing.

The same rule applies inside a test. An assertion that needs something only one platform has —
`/dev/full`, a container runtime, a specific filesystem — may be given up on a developer machine
and never in an automated run, where "it runs in CI" would otherwise be an assumption nobody can
see failing. Guard it with `skippable` from `tools/lib/test-support.sh`: outside CI it prints what
coverage was lost, and inside CI it fails.

## Open the pull request

Open a draft pull request first. State the outcome and linked issue, then bound the exact scope.
List every completed validation command with its result and name the known limits. Keep separable
follow-ups as issues rather than widening the pull request. A merge should close one coherent issue
and remain easy to review or revert.

Security findings follow [SECURITY.md](SECURITY.md), not the public issue workflow.
