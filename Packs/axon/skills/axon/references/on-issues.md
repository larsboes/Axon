# Issue, branch, and PR workflow

Use one auditable branch and one focused PR per GitHub issue.

## Start

1. Read the live issue, labels, relationships, and acceptance criteria.
2. Verify dependent issues and current PRs instead of inferring state from local commits.
3. Start from an up-to-date, clean `main`.
4. If the tree is dirty, preserve it. Do not reset, clean, stash, or absorb work without resolving
   its ownership with the user.
5. Create `issue-<number>-<short-slug>`.

## Work

- Keep the diff inside the issue acceptance boundary.
- Update the issue before implementation when evidence changes scope or acceptance.
- Commit coherent changes with the issue number in the message.
- Add follow-up issues for separable discoveries; do not silently expand the current PR.
- Run the proportionate checks in `references/on-verification.md`.

## Publish

1. Review the focused diff and final status.
2. Push the issue branch.
3. Open a draft PR with outcome, evidence, validation, known limits, and `Closes #<number>`.
4. Make the PR reviewable before starting the next dependent issue.
5. Start the next issue from updated `main` after the dependency is merged. Use a stacked PR only
   when the dependency must remain unmerged, and state that relationship explicitly.

Emergency fixes or genuinely inseparable acceptance criteria may share a boundary, but the PR must
name the exception and why splitting would make review or rollback less safe.
