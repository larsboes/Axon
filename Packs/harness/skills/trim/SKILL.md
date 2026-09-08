---
name: trim
description: Shrinks the files that load into every session — the user and project CLAUDE.md, their @-imports, the memory index — by removing what is provably dead first and then merging, tightening or relocating the rest behind a human gate, without ever dropping a distinct directive. Use when an always-on context file has grown too big, when asked to trim, shrink or prune CLAUDE.md, AGENTS.md or a memory index, when a rule set has turned into a document, or when the session's instruction budget needs to come down. Do not use for editing ordinary documentation, for refactoring code, or for removing AI writing tells from prose.
license: MIT
---

# trim

Every byte in an always-on file is paid on every session, forever. This skill measures that cost,
removes what is provably dead, and then makes the judgment calls one at a time in front of the
person whose rules they are.

Adapted from the Trim skill in LifeOS by Daniel Miessler
(https://github.com/danielmiessler/LifeOS), MIT.

## Procedure

Copy this checklist and track progress:

```
- [ ] 1. Measure
- [ ] 2. Make the edit reversible
- [ ] 3. Remove what is provably dead
- [ ] 4. Propose the semantic moves, one at a time
- [ ] 5. Run the safety gate
- [ ] 6. Re-measure and report
```

**1 — Measure.** Never start from an impression of which file is too big.

```bash
bun scripts/measure-context.ts --root "$PWD"
```

It resolves the always-on set — `~/.claude/CLAUDE.md`, the project `CLAUDE.md` and `AGENTS.md`,
every `@`-import they pull in, and the memory index for this project — and reports bytes, an
approximate token count, the largest sections, dead pointers and directives duplicated across two
always-on files. Show the totals to the user before proposing anything. Without a target size,
"smaller" has no end.

**2 — Make it reversible.** Before the first edit:

- **The file is in a git repository** — check `git status --short` on that path. Clean means the
  repository is the undo. Dirty means say so and ask before touching it; a trim mixed into
  somebody else's uncommitted work cannot be reverted on its own.
- **The file is not in a repository** — this is the normal case for `~/.claude/CLAUDE.md`. Copy it
  to `<file>.bak-<UTC timestamp>` and print the exact `cp` command that restores it. Do this once,
  before the first edit, not per move.

**3 — Remove what is provably dead.** These need no judgment, so they are free. Take them all
before proposing anything that costs attention:

- **Dead pointers.** A rule that points at a file which no longer exists has no reader. Delete the
  rule, or repair the pointer when the file merely moved — say which, and how you checked.
- **Duplicated directives.** A line loaded by two always-on files is paid twice. Keep the copy in
  the narrower scope, delete the other, and say which one survived.
- **Rules about things that no longer exist.** A tool, a path, a service, a workflow that is gone.
  Check before claiming it: `ls`, the repository, the deployment ledger. An unverified "this looks
  obsolete" is not a deterministic removal, it is a semantic one, and it belongs in step 4.

Re-measure after this step. It often clears enough that step 4 is unnecessary, which is the best
outcome available: no judgment call was spent.

**4 — Propose the semantic moves.** Read `references/moves.md`. Three moves, and each proposal
goes to the user as a diff with the byte count it saves:

| Move | When |
|---|---|
| MERGE | Two rules say overlapping things and one sentence can carry both |
| TIGHTEN | One rule says a correct thing in three times the words it needs |
| RELOCATE | Detail that matters rarely, and a pointer can stand where it stood |

One move at a time, largest saving first, and stop when the user stops approving. A batch of
fifteen approved at once is a batch nobody read.

**5 — Run the safety gate.** Before writing any semantic edit, check the result against the
original: every proper noun, path, command, tool name, number and imperative in the original still
appears in the replacement or in the file it was relocated to. If one does not, the move is wrong
— keep the original. This gate is not optional and it is the whole reason a trim is safe to run on
a file the agent depends on to behave correctly.

**6 — Re-measure and report.** Run the script again. Report before, after, the difference, and
what moved where. A relocation that nobody can find is a deletion with extra steps.

## What a trim never does

- **Never drops a distinct directive.** Shrinking is the goal; losing a rule is a defect, and the
  gate in step 5 exists to catch it.
- **Never merges two rules that disagree.** Two contradictory rules are a finding — report the
  contradiction and let the user resolve it. A merge that silently picks a winner hides a decision.
- **Never rewrites for style.** Voice is not size. A rule that reads awkwardly and is unambiguous
  stays as it is.
- **Never edits a file the user did not name or approve.** The measurement covers the whole set;
  the edits cover what was agreed.

## The relocation that is really a skill

A procedure in an always-on file — numbered steps, a checklist, a runbook — is a skill that has not
been written yet. It is loaded into every session and needed in one of fifty. When step 4 finds one,
say so: the relocation target is a skill, not a reference file, and the always-on file keeps one
line naming it. This is the single largest saving available in most context files and the one that
is easiest to miss, because a procedure looks like it belongs where the rules are.

## Error handling

- **The file changed since it was read.** Re-read before writing. Another session, a hook or the
  memory writer may have appended while the proposal was being drafted, and writing from a stale
  read silently reverts their change.
- **The target is already small.** Report the number and stop. A file that costs a few hundred
  tokens is not the reason a session feels heavy.
- **Every section is load-bearing.** That is a legitimate result. Report the measurement, say that
  nothing can go without losing a rule, and name the largest section so the user can decide whether
  its subject deserves that much of every session.
- **The user asks to trim a file that is not always-on.** Say that it costs nothing per session and
  ask what problem the trim is for.
