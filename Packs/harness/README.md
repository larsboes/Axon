<!-- human-voice: ignore bold_bullets -->
<!-- Definition-list idiom, same as the sibling pack READMEs: the bold span is the term, the
     text after it the definition. -->

# harness pack

Every other Pack does work. This one maintains the thing that does the work: what skills should
exist, and what should stop being loaded into every session.

- **`suggest-skills`** reads this machine's own prompt history and installed skills, finds the work
  that recurred across sessions and the places the user had to correct the agent, and returns at
  most three proposals — each with the evidence that produced it and the coverage check that
  survived. It is read-only and proposal-only by construction: it cannot create or edit a skill,
  because a proposer that can also build grades its own homework.
- **`trim`** measures what actually loads every session — the user and project `CLAUDE.md`, their
  `@`-imports, the memory index — removes what is provably dead, then makes each judgment call in
  front of the user, one at a time, under a gate that refuses any edit which loses a directive.

Both put a deterministic script in front of the model. `collect-signals.ts` gathers and counts;
`measure-context.ts` measures and resolves. Neither judges, and neither writes. Two runs over the
same files return the same corpus, so a proposal can be argued with instead of trusted, and the
model spends its judgment on the part that needs judgment.

## The boundary

Three things sit near each other and do different jobs:

| Question | Where it is answered |
|---|---|
| *What* skill should exist? | `suggest-skills`, here |
| *How* is a skill written and audited? | `writing-skills`, in `Packs/writing` |
| What loads every session, and can it be smaller? | `trim`, here |

`suggest-skills` hands its shortlist to the authoring skill and stops. The authoring skill never
decides what to build. Keeping the two apart is the point: the same agent doing both will propose
what it feels like writing.

The private overlay carries a staging `meta` pack whose `skill-creator` is the intended successor
to `writing-skills`. When that promotion happens, the hand-off target in `suggest-skills`
step 6 changes name and nothing else does — the boundary above is about jobs, not about which
authoring skill currently holds the second row.

## Scripts

```bash
bun skills/suggest-skills/scripts/collect-signals.ts --days 30 --json   # the corpus
bun skills/trim/scripts/measure-context.ts --root "$PWD"                # the always-on set
```

Both read only local files — the harness history, the installed skills, the context files — and
neither sends anything anywhere. `collect-signals.ts` prints raw prompt text, so its output is as
private as the prompts were: keep it local, and do not paste it into anything that leaves the
machine.

## Activate

```bash
"$AXON_ROOT/tools/packs.sh" link harness      # → ~/.claude/skills/{suggest-skills,trim}
"$AXON_ROOT/tools/packs-codex" deploy harness # → ~/.agents/skills/{suggest-skills,trim}
```

## Attribution

Ported from Daniel Miessler's LifeOS, MIT: <https://github.com/danielmiessler/LifeOS>
(`SuggestSkills` and `Trim`, read 2026-09-07). The register verdict for that upstream is
`inspiration` — see the `[lifeos]` row in `upstreams.toml`. No code was vendored: both scripts here
are ours, written against the files this machine actually has.

What the port changed, and why:

| Upstream | Here |
|---|---|
| A satisfaction/rating store as the frustration signal | There is no rating store on this machine. Friction is inferred from the wording of a prompt and the prompt before it, and `suggest-skills` states that this over-fires and under-fires rather than presenting it as data. |
| `Tools/CollectSignals.ts` over the LifeOS session and rating stores | `collect-signals.ts` over `~/.claude/history.jsonl` and the installed skill roots, with every source overridable by flag. |
| Trim's deterministic pass = `ProposalGC.ts` over a LifeOS proposal inbox | No such inbox exists here. The deterministic pass became dead pointers and directives duplicated across two always-on files — both checkable, both free, and both found on the first real run. |
| Trim commits to a private USER_DATA repository | Harness-neutral reversibility: the repository when the file is in one and clean, a timestamped `.bak` copy with a printed restore command when it is not. `~/.claude` is not a repository here. |
| The voice notification, the JSONL execution log, the CUSTOMIZATIONS preamble | Removed, for the reasons the `deliberation` Pack's README already records — they address a LifeOS layout Axon does not run. |

## Why this shape: the flip conditions

`suggest-skills` is worth keeping while it rejects. A run that proposes three skills every time it
is asked is pattern-matching on enthusiasm; the rejection table, with the test each candidate
failed, is the artifact that proves it did the coverage read. Two consecutive runs with an empty
rejection table and it has stopped checking.

`trim` flips on the deterministic half. If the dead-pointer and duplicate checks keep finding
something, the always-on set is drifting and the skill is paying for itself before any judgment
call. If they come back empty three runs running while the files keep growing, the growth is
load-bearing content and the honest answer is that these files are the right size — at which point
the skill's remaining value is the measurement, and a one-line script would do.
