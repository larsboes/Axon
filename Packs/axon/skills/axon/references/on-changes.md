# Branch, commit, and PR workflow

One auditable branch and one focused change. **A tracker entry is never a precondition for
starting work.**

## Start

1. Start from an up-to-date `main`.
2. If the tree is dirty, preserve it. Do not reset, clean, stash, or absorb work without resolving
   its ownership with the user.
3. Name the branch for the change, not for a ticket: `<area>-<short-slug>`.

## Work

- Keep the diff inside one coherent boundary.
- Write the commit message as the outcome, not the activity.
- Surface separable discoveries instead of silently widening the branch.
- Run the proportionate checks in `references/on-verification.md`.

## Publish

1. Review the focused diff and final status.
2. Push the branch. Unpushed work exists on one machine only, which is the failure this step
   prevents.
3. Open a PR when the change wants review or a record; push a small, verified change straight to
   `main` when it does not.

## When an issue is still worth filing

Only when something must outlive the change itself: a defect being left unfixed, a decision that
needs a record, or work handed to someone else. Read live issue and PR metadata when a task
already references one — never infer its state from local commits.
