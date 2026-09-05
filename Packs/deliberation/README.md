# deliberation pack

Two ways to test a decision before it is made.

- **`council`** convenes four to five subagents that hold different positions on one decision,
  runs them over one or three parallel rounds, and reports a transcript, one recommendation and
  the position that lost. Five presets cover architecture, investment, travel, security and
  product. `skills/council/references/compose.md` covers everything else.
- **`red-team`** attacks one proposal: atomic claims, a steelman its author would sign, six
  attack lenses, findings ranked `fatal`/`structural`/`cost`/`cosmetic`, each with a test that
  would settle it.

They split on shape, not on tone. Council needs two or more options and gives each one an
advocate. Red-team needs one proposal and gives it none. Each SKILL.md names the other as the
hand-off.

## Provenance

Ported from Daniel Miessler's LifeOS, MIT: <https://github.com/danielmiessler/LifeOS>
(`Council` and `RedTeam`). The register verdict for that upstream is `inspiration` — see the
`[lifeos]` row in `upstreams.toml`. Attribution is repeated in each SKILL.md, which is the copy
that travels when a Pack is deployed into a harness.

What the port removed, and why:

| Removed | Why |
|---|---|
| The voice notification (a `curl` to `localhost:31337` before any action) | It calls a LifeOS daemon that Axon does not run. |
| The execution log (a JSONL append to `~/.claude/LIFEOS/MEMORY/`) | Same: it writes into a harness layout that was uninstalled here on 2026-08-22, per the `[lifeos]` row in `upstreams.toml`. |
| The customization preamble (`check ~/.claude/LIFEOS/USER/CUSTOMIZATIONS/…` first) | Same layout, and it spent the first tokens of every invocation on a directory that does not exist. |
| RedTeam's 32 agents in four types | Eight copies of one role return eight versions of the same finding. Six lenses that ask different questions replace them. `skills/red-team/references/lenses.md` says so at the point of use. |

What the port added: the five council presets, the evidence rule (`[unverified]` marks a claim
with no pointer and the synthesis reports it), the named minority position, and the
progressive-disclosure split — each SKILL.md is a router under 120 lines and every detail sits
one level down in `references/`.

## Activate

```bash
"$AXON_ROOT/tools/packs.sh" link deliberation      # → ~/.claude/skills/{council,red-team}
"$AXON_ROOT/tools/packs-codex" deploy deliberation # → ~/.agents/skills/{council,red-team}
```

`tools/doctor` reports both skills under "Packs (Claude Code materialized)".

## Why this shape: the flip condition

Cut this Pack when a council run stops changing a decision. The test is cheap and it is on the
reader, not on the tooling: after a run, ask whether the recommendation differs from the answer a
single-pass reply would have given. Two consecutive runs where it does not, on decisions that
were worth convening for, and the Pack is theatre — a longer transcript arguing for what was
already going to happen. Delete it and keep `red-team`, which fails loudly instead: it either
names a mechanism or it reports nothing found.

The second flip condition is narrower. If the presets stop being edited while the domains keep
changing, they have become decoration. A preset is a claim about what evidence a decision needs,
and a claim nobody has revised in a year is usually wrong.
