# academic-writing pack

Draft, critique, and cite-check academic writing (thesis chapters, conference/journal papers) across
two genre profiles — empirical-CS/paper and DSR/qualitative-thesis — with an adversarial multi-skeptic
review pass no third-party academic-writing tool checked during this pack's build actually had.

- **`academic-writing`** (skill) — four workflows: `Draft` (genre-aware skeletons/critique, never
  ghostwrites), `FlowCheck` (reverse-outlining, given-new sentence flow, transition audit),
  `CriticReview` (five critique lenses + the adversarial 3-skeptic pass), `Citations` (citation-key
  coverage, DOI/bib validation, paper resolution — Quarto and LaTeX/BibTeX both).
- **`agents/`** (Claude Code only, optional) — the same five critique lenses and three adversarial
  skeptics from `CriticReview`, as native `.claude/agents/*.md` subagents instead of general-purpose
  dispatches with a prompt brief. Real advantage over the skill's own dispatch pattern: hard tool
  restriction (Read/Grep/Glob only — Write/Edit are actually blocked, not just prompt-asked) and
  automatic delegation (Claude can reach for one on its own when it fits, without the skill's
  `CriticReview` workflow explicitly invoking it). Deliberately not part of `pack.toml`'s skills list —
  see Activate below and `README.md#harness-neutral-packs`.

## Activate
```bash
"$AXON_ROOT/tools/packs.sh" link academic-writing
# → ~/.claude/skills/academic-writing (the skill)
# → ~/.claude/agents/academic-writing/ (the 8 Claude-Code-native subagents, if the pack ships agents/)
"$AXON_ROOT/tools/packs-codex" deploy academic-writing
# → ~/.agents/skills/academic-writing (materialized skill; pack-level Claude agents are skipped)
```
The `agents/` link is a `packs.sh` convention (checked by directory presence, not a `pack.toml` field)
so the neutral pack manifest never has to name a Claude-Code-only concept. A harness without a native
subagent system just won't have an `agents/` dir to look for; the skill alone still works everywhere.

## Attribution
Built from a source-material sweep of three third-party academic-writing tools plus a fourth author's
(Peng Sida, 彭思达) publicly shared paper-writing notes, all cherry-picked and rewritten in our own
words per pack — no verbatim third-party text retained except `references/citation-workflows.md`,
which its own header records as ported near-verbatim (MIT source), and no code vendored except
where noted below.
Licenses vary by source, not "MIT throughout" — see each bullet and the linked `upstreams.toml`
entry (`[academic-researcher]`, `[academic-writing-agents]`, `[research-paper-writing-skills]`,
`[pengsida-research-notes]`) for the verdict/pin/license of record; the "unknown license"
placeholders from this pack's initial 2026-07-11 build were resolved 2026-07-14.
- Citation-workflow content (`references/citation-workflows.md`'s bibliography/claim-evidence-map
  sections) and the three citation scripts (`scripts/resolve-papers.js`, `validate-bib.js`,
  `check-citations.js`) are adapted from
  [SiluPanda/academic-researcher](https://github.com/SiluPanda/academic-researcher) (MIT, pinned
  `e75d70d`) — `check-citations.js` was generalized off a hardcoded path and given Quarto `@citekey`
  support beyond the original's LaTeX-only `\cite{}` handling. See `upstreams.toml
  [academic-researcher]` (verdict `overlay` — real code adapted, not just ideas; retroactive entry,
  still needs a `tools/audit` pass against these three files, and an `upstreams.toml`
  verdict written by hand — no script checks the manifest since 2026-08-28).
- The condensed house style (`references/house-style.md`, sections A/B/D/F) is a rewrite of
  [andrehuang/academic-writing-agents](https://github.com/andrehuang/academic-writing-agents) (MIT,
  a Claude Code plugin) — the five critique lenses in `references/critic-briefs.md` and the matching
  `agents/academic-writing-*.md` files (technical/logic/consistency/bibliography/layout) are also
  adapted from that plugin's 10-agent roster, ported as prompt content rather than the original's
  static-persona files, with its hardcoded author-specific path removed. See `upstreams.toml
  [academic-writing-agents]` (verdict `quarry` — ideas/structure mined, no code retained).
- Flow-diagnostic technique (`references/flow-diagnostics.md` §1–4) is distilled from a university
  writing-center handout encountered without clear authorship attribution — ideas only, no verbatim
  text retained, and still no source to record in `upstreams.toml`.
- `references/genre-empirical-cs.md` condenses Peng Sida's (彭思达) publicly shared paper-writing
  notes — [GitHub](https://github.com/pengsida/learning_research) /
  [Notion](https://pengsida.notion.site/c1a22465a0fa4b15a12985223916048e), no license declared — via
  its Claude-skill repackaging,
  [Master-cai/Research-Paper-Writing-Skills](https://github.com/Master-cai/Research-Paper-Writing-Skills)
  (MIT). Rewritten, not copied, from either. See `upstreams.toml [pengsida-research-notes]` and
  `[research-paper-writing-skills]` (both verdict `quarry`).
- **Not from any external source** — built for this pack specifically: `references/genre-dsr-
  qualitative.md` (no analyzed source covered DSR/qualitative-thesis writing at all), §5 of
  `references/flow-diagnostics.md` (given-new sentence flow + rhythm variance),
  `references/ai-cadence-tells.md` and `scripts/scan-ai-tells.js` (generalized from patterns
  battle-tested in real thesis-editing sessions, not derived from the third-party corpus), and
  `references/adversarial-redteam.md` plus the three skeptic agents (no third-party tool checked had a
  genuinely adversarial pass — every one of them was structurally biased toward finding fixable issues
  rather than arguing for outright rejection).

## Further reading
- [Claude Code subagents](https://code.claude.com/docs/en/sub-agents) — the `agents/` convention above
  follows this spec directly; re-check it if this pack's agent files ever look stale against a Claude
  Code release.
