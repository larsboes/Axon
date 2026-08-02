# writing pack

Writing for two audiences: readers, and agents.

- **`human-writing`**: prose that reads as one deliberate human voice instead of the model's
  default register. Drafts, edits, reviews, audits. Three layers. A CI-gateable linter with
  real structural checks (burstiness, lexical diversity, n-gram repetition, SVO monotony,
  over-correction, dialect drift). A catalog of the tells no regex catches, from uniform
  rhythm to sycophancy to saying nothing at length. And a guard against the over-corrected
  "trying not to sound like AI" register that just swaps one default for another. Facts,
  numbers, and citations stay invariant. Optional private voice profiles handle "make it sound
  like me".
- **`writing-skills`**: meta-skill. Scaffolds and audits SKILL.md plus scripts/references/assets, validates metadata (name/description/dir-match/reserved-words/triggers), and enforces progressive disclosure. Dogfoods itself.

## Activate
```bash
"$AXON_ROOT/tools/packs.sh" link writing   # → ~/.claude/skills/writing-skills, human-writing
"$AXON_ROOT/tools/packs-codex" deploy writing  # → ~/.agents/skills/writing-skills, human-writing
```

## Attribution

Full grant text for everything vendored here lives in [`LICENSE`](LICENSE).

### human-writing

Merged 2026-07-25 from three upstreams. It supersedes the former `unslop-text` skill, moved
here from `Packs/unslop` with `git mv` so its history survives. Full per-source detail sits in
that skill's `references/maintenance.md` (§ Provenance) and in `upstreams.toml`.

- [stephenoffer/human-voice](https://github.com/stephenoffer/human-voice) (`[human-voice]`,
  MIT, pinned `9bcba2f`) is the substrate. Its linter is vendored wholesale
  (`scripts/human_voice_linter/`, `detect_ai_prose.py`, `ai_prose_patterns.json`), plus four
  reference files verbatim under an attribution header. Two local deltas, both documented in
  the skill's `references/maintenance.md` (§ Local deltas against upstream): the em-dash check now fires on presence rather than upstream's
  `count >= 2` density gate (zero false positives measured across the bundled human and
  ESL-formal corpora), and a handful of additive pattern entries.
- [ryanthedev/oberskills](https://github.com/ryanthedev/oberskills) (`[oberskills]`, pinned
  `5050537`) supplied the architecture: mode router, reader-job taxonomy, voice profiles,
  review protocol. **MIT is declared in `.claude-plugin/plugin.json` only, with no LICENSE file
  at the repo root.** So everything taken is paraphrased and rewritten rather than copied, and
  both derived reference files say so explicitly. Revisit if upstream adds a real LICENSE.
- [JCarterJohnson/vibecoded-design-tells](https://github.com/JCarterJohnson/vibecoded-design-tells)
  (`[vibecoded-design-tells]`, MIT, pinned `f7c4aef`) supplied the doctrine and the ranked tell
  catalog: cited-vs-matched weighting, the density model, the over-corrected-register framing.
  `references/tells.md` and `references/writing-with-intent.md` are still its text.

[hardikpandya/stop-slop](https://github.com/hardikpandya/stop-slop) was evaluated and declined
twice, most recently during this merge. Its distinctive contributions are already pattern
categories in the vendored linter, and its absolutist framing ("kill all adverbs", "no em
dashes ever") manufactures exactly the uniform signature the skill exists to prevent. See
`upstreams.toml [stop-slop]`.

### writing-skills

`writing-skills` reconciles several sources into one local convention, all distilled into `references/` in our own words. No verbatim third-party text retained, no code vendored, MIT throughout.
- Anthropic's [skill best-practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices) plus [Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview), the [agentskills.io](https://agentskills.io) `skill-creator` spec, and the dense variable-slotted "Fable" skill style. That was the original three-source synthesis.
- Anthropic's [Extend Claude with skills](https://code.claude.com/docs/en/skills) doc, covering the Claude-Code-specific frontmatter and runtime (`context: fork`, dynamic context injection, skill stacking, `skillOverrides`, the description-listing budget), in `references/claude-code-extensions.md`.
- Anthropic's [new rules of context engineering for Claude 5](https://claude.com/blog/the-new-rules-of-context-engineering-for-claude-5-generation-models) (2026): over 80% of Claude Code's own system prompt removed without measurable loss; prescriptive rules replaced by principles, detail moved behind progressive disclosure. Read 2026-07-28; it is why `validate_metadata.py` now enforces the Level-2 body budget this pack had documented but never checked.
- A third-party PDF skill-authoring guide and a "bootstrap a whole skill library" mega-prompt (both unknown author and license, encountered without provenance), mined for *ideas only*: the three-tier eval framework, the five workflow-shape patterns, the discover→parallel-author→adversarial-review pattern. Rewritten from scratch, no text retained. `references/evaluation.md`, `references/patterns.md`, `references/bootstrap-library.md`.

## Considered and declined

Evaluated 2026-07-28 while auditing this pack, none adopted. Recorded here under
`README.md#decisions-live-with-their-owner` rather than deleted, so the same sources
do not get re-evaluated from scratch.

- **[ASD-STE100 Simplified Technical English](https://asd-ste100.org)** (ASD, Brussels;
  copyrighted, free official copy on request — never paste it in full). A controlled natural
  language from 1986 aircraft maintenance documentation: one word one meaning, active voice,
  max 20 words per instruction, no semicolons or contractions. Declined as a *skill*, because it
  strips voice on purpose and `human-writing` exists to specify voice, not remove it. Its natural
  home here is an eleventh `references/registers.md` profile, stricter than `technical`, for
  genuinely voiceless operational text (error messages, runbooks, CLI help) — not built yet.
- **[woosal1337/blog `ep01-the-cure-for-ai-slop`](https://github.com/woosal1337/blog/tree/main/videos/ep01-the-cure-for-ai-slop)**
  — an STE-condensed skill plus `ste-lint.py`, benchmarked at −74% "violations per 100 words" on
  Claude and −50% on GPT-5.5 against a plain baseline. Declined as a dependency: the linter counts
  the STE rules themselves (sentences over 20 words, semicolons, contractions, passive voice, its
  own banned-word list), so a skill that states those rules scores well by construction. What
  survives the circularity is the *comparison* — a banned-words list moved Claude only 3% against
  STE's 74%, so a writing system beats a word list. That conclusion is already this pack's design.

## Further reading
- Anthropic's live docs. Check these directly when this pack's distillation might be stale (they're versioned, this is a 2026-07-10 snapshot): [Agent Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview) · [authoring best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices) · [Extend Claude with skills](https://code.claude.com/docs/en/skills) · [agentskills.io spec](https://agentskills.io).
- [NVIDIA/skills](https://github.com/NVIDIA/skills) (`upstreams.toml [nvidia-skills]`, dual CC-BY-4.0/Apache-2.0): roughly 230 published skills, mostly GPU and datacenter ML infra with no direct overlap here, but a genuinely large real-world corpus of skill *structure* to compare against when scoping a new one. Mined for patterns only, not a dependency.
