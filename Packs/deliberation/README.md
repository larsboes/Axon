<!-- human-voice: ignore bold_bullets -->
<!-- Definition-list idiom, same as the sibling pack READMEs: the bold span is the term, the
     text after it the definition. -->

# deliberation pack

Four moves on one decision, in the order a decision usually needs them.

- **`crystallize`** turns an unstructured mess — a brainstorm, a pile of notes, a half-written
  PRD — into one document where every claim is measured against the repo or the vault and every
  answer is recorded dated, with its reasoning. It drives with batched multiple-choice questions,
  because the user holds the judgment and the agent holds the ability to count. Three scripts do
  the measuring: `census.py`, `docedit.py`, `integrity.py`.
- **`council`** convenes four to five subagents that hold different positions on one decision,
  runs them over one or three parallel rounds, and reports a transcript, one recommendation and
  the position that lost. Five presets cover architecture, investment, travel, security and
  product. `skills/council/references/compose.md` covers everything else.
- **`red-team`** attacks one proposal: atomic claims, a steelman its author would sign, six
  attack lenses, findings ranked `fatal`/`structural`/`cost`/`cosmetic`, each with a test that
  would settle it.
- **`root-cause`** explains a failure that already happened: a timeline with a known-good and a
  known-bad mark, the changes in that window, candidate causes that agents try to *refute* rather
  than confirm, and one named cause carrying the probe that would falsify it.

They split on shape, not on tone. Crystallize needs a mess and produces a decided document.
Council needs two or more options and gives each one an advocate. Red-team needs one proposal and
gives it none. Root-cause needs a failure and gives it a mechanism. Each SKILL.md names the others
as the hand-off.

## The agents

`agents/` holds seven Claude-Code-native subagents (added 2026-09-07). A harness without native
subagents never sees them, and every skill here states the `general-purpose` fallback at the point
it dispatches.

| Agent | Used by | Model |
|---|---|---|
| `council-owner` | council — the member that argues from having built the thing | sonnet |
| `council-skeptic` | council — the member that argues from the failure mode | opus |
| `council-cost` | council — the member that argues from build, run and reversal cost | sonnet |
| `council-evidence` | council — the member that argues from precedent and measurement | sonnet |
| `council-clerk` | council — resolves every citation in a round, after each round | sonnet |
| `red-team-lens` | red-team — one instance per attack lens | opus |
| `cause-hypothesis` | root-cause — one instance per candidate cause, tasked to refute it | sonnet |

Two things they buy that a prompt cannot:

**Tool restriction is enforced, not requested.** Every one of these agents is `Read, Grep, Glob`.
Before this, a council member ran as `general-purpose`, whose tool set is `*` — four subagents
with write access to the repository, in a skill whose entire output is text. Nothing had gone
wrong; nothing was stopping it either. `Bash` is deliberately absent too, because `echo > file` is
a write and a shell is the hole in a read-only claim. The cost is real and is the design: a member
cannot run a measurement, so anything it may cite has to arrive in the prompt, which is what the
skills' evidence-collection step already existed for.

**The contract is separated from the character.** The agent file carries what never changes — read
only, cite or mark, return the round text alone. The brief in the prompt carries what makes this
council different from the last one. So `compose.md` keeps writing topic-specific members, and
they stop being able to edit the repository. Four fixed personas would have been the easy version
of this and the wrong one.

The `model` field is the third lever: the sceptic and the attack lenses run on opus because their
job is to find what nobody else did; the rest run on sonnet because their job is 150 words of
argument from evidence that is already in the prompt. Change it in the agent file, never per run.

### The clerk is the point

`council`'s evidence rule always said a claim with no pointer gets marked `[unverified]` and a
member citing a file that does not exist has its claim dropped. Nothing checked. The member marked
its own homework, and a member that wants to win simply does not mark it.

`council-clerk` runs on each round as it is printed, resolves every pointer, and returns one of
`resolves`, `narrower`, `contradicted`, `missing`, `uncited` per claim. The orchestrator applies
the verdicts before the next round, so round 2 argues against a corrected round 1 rather than
against a confident invention. `narrower` is the verdict that earns the agent: a citation that
exists and supports a weaker claim than the member built on it is the failure mode that a
file-exists check cannot see, and it is the common one.

## Provenance

Ported from Daniel Miessler's LifeOS, MIT: <https://github.com/danielmiessler/LifeOS>
(`Council`, `RedTeam`, and `RootCauseAnalysis` — read again 2026-09-07 for the third). The register
verdict for that upstream is `inspiration` — see the `[lifeos]` row in `upstreams.toml`.
Attribution is repeated in each SKILL.md, which is the copy that travels when a Pack is deployed
into a harness.

`crystallize` is not from that upstream and not from any other: it was written here and lived
unmanaged in `~/.claude/skills` from 2026-08-24 until it was adopted into this Pack on 2026-09-07,
with no copy in any repository for those two weeks.

What the port removed, and why:

| Removed | Why |
|---|---|
| The voice notification (a `curl` to `localhost:31337` before any action) | It calls a LifeOS daemon that Axon does not run. |
| The execution log (a JSONL append to `~/.claude/LIFEOS/MEMORY/`) | Same: it writes into a harness layout that was uninstalled here on 2026-08-22, per the `[lifeos]` row in `upstreams.toml`. |
| The customization preamble (`check ~/.claude/LIFEOS/USER/CUSTOMIZATIONS/…` first) | Same layout, and it spent the first tokens of every invocation on a directory that does not exist. |
| RedTeam's 32 agents in four types | Eight copies of one role return eight versions of the same finding. Six lenses that ask different questions replace them. `skills/red-team/references/lenses.md` says so at the point of use. |
| RootCauseAnalysis's five named methodologies as branded workflows | The brand is not the method. Six methods stay, each keyed to the evidence that selects it and each carrying its own characteristic wrong answer, which is the part a reader cannot get from the name. |

What the port added: the five council presets, the evidence rule (`[unverified]` marks a claim
with no pointer and the synthesis reports it), the named minority position, the clerk that makes
that rule enforceable, the refute-don't-confirm candidate pass in `root-cause`, and the
progressive-disclosure split — each SKILL.md is a router under 120 lines and every detail sits one
level down in `references/`.

Upstream's `Council` still writes its members as inline briefs launched with `general-purpose` and
states that there is no composition tool, so the `agents/` layer above is not an adaptation of
anything. It is this Pack's own answer to a hole the upstream shape leaves open.

## Activate

```bash
"$AXON_ROOT/tools/packs.sh" link deliberation      # → ~/.claude/skills/{crystallize,council,red-team,root-cause}
                                                   # → ~/.claude/agents/deliberation/ (the 7 subagents)
"$AXON_ROOT/tools/packs-codex" deploy deliberation # → ~/.agents/skills/… (skills only; pack-level Claude agents are skipped)
```

`tools/doctor` reports the skills under "Packs (Claude Code materialized)".

## Why this shape: the flip conditions

Cut `council` when a run stops changing a decision. The test is cheap and it is on the reader, not
on the tooling: after a run, ask whether the recommendation differs from the answer a single-pass
reply would have given. Two consecutive runs where it does not, on decisions that were worth
convening for, and the skill is theatre — a longer transcript arguing for what was already going to
happen. Delete it and keep `red-team`, which fails loudly instead: it either names a mechanism or
it reports nothing found.

The clerk has its own flip condition, and it is measurable rather than felt: it reports a count
every round. If ten consecutive rounds return every claim as `resolves`, the members are honest
without supervision and the clerk is a tax — drop it and spot-check instead. If it keeps finding
`narrower` and `missing`, it is the most valuable agent in the Pack and the finding is about the
members, not about the clerk.

The second condition is narrower. If the presets stop being edited while the domains keep
changing, they have become decoration. A preset is a claim about what evidence a decision needs,
and a claim nobody has revised in a year is usually wrong.

`root-cause` flips the other way from `council`: it is worth keeping precisely while it produces
causes that get falsified. A run whose named cause survives its own probe every time is not a
skill working well, it is a probe nobody chose honestly.
