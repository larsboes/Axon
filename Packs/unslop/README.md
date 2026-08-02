# unslop pack

Strips the tells that make code and web UI read as AI-generated, and forces a deliberate
project-specific choice instead of the model's default. The skill has no preferred style. It
removes cited tells (ranked by real-world data, not vibes) and pushes toward a choice you can
defend.

One skill, **`unslop`**, covering two domains that share a method:

- **code** — bug-class tells (swallowed errors, hallucinated APIs, unfinished stubs) and
  cosmetic ones (chat artifacts, emoji, narrating comments), plus the structural tells a linter
  can't see (boilerplate/tutorial shape, over-engineering, repo mismatch, the three that
  dominate the data). Scanner: `scripts/unslop_code_scan.py`.
- **UI** — default shadcn/Tailwind, AI-purple gradients, gradient hero text, unprompted neon
  glow, emoji-as-icons, the centered-hero-plus-three-cards layout, and the newer
  cream-plus-serif-plus-sage "tasteful default" that just trades one default for another.
  Complements (doesn't replace) a frontend-design skill. That one builds; this one keeps it
  honest. Scanner: `scripts/devibe_scan.py`.

Both scanners are stdlib-only Python (`os re sys json argparse`), so no install step, `uv run`-
compatible, no network access. Both are CI-gateable: exit code is the high-severity count.

## Merged into one skill (2026-07-28)

`unslop-code` and `unslop-ui` were separate skills until 2026-07-28. They shared an identical
method (same trap-to-avoid argument, same Build/Audit modes, same reporting contract) and split
an ambiguous boundary, since a styled component is both code and UI and each description
claimed it. Merging put the shared method in the body once and pushed the two tell catalogs and
the two build methods into `references/`, where only the domain in play gets loaded: 4,563
tokens of always-both instructions became 1,993. History is preserved via `git mv`.

## Prose moved out (2026-07-25)

`unslop-text` used to live here. It moved to
[`Packs/writing/skills/human-writing`](../writing/skills/human-writing/) and was merged with
[stephenoffer/human-voice](https://github.com/stephenoffer/human-voice) and the `write` skill
from [ryanthedev/oberskills](https://github.com/ryanthedev/oberskills), gaining a linter with
real structural checks (burstiness, lexical diversity, n-gram repetition, over-correction
detection), draft and review modes, and private voice profiles. Its `references/tells.md` and
`references/writing-with-intent.md` are still the `vibecoded-design-tells` text, so
`upstreams.toml [vibecoded-design-tells]` covers that pack too.

Note the two packs use **different ignore directives**. The scanners here read a line
containing `unslop-ignore`; the prose linter in `human-writing` reads
`<!-- human-voice: ignore <categories> -->` and does not honor `unslop-ignore`.

## Activate
```bash
"$AXON_ROOT/tools/packs.sh" link unslop   # → ~/.claude/skills/unslop
"$AXON_ROOT/tools/packs-codex" deploy unslop  # → ~/.agents/skills/unslop
```

## Attribution
The skill (`SKILL.md`, `references/`, `scripts/`) is adapted near-verbatim from
[JCarterJohnson/vibecoded-design-tells](https://github.com/JCarterJohnson/vibecoded-design-tells),
pinned `f7c4aef` (2026-06-23), MIT. Content and instructions are unchanged; each `description`
had a small number of second-person clauses reworded (code half: "for you" / "points you at";
UI half: "hand you taste" / "are still yours" / "you become next year's slop") to clear the
third-person discovery convention `Packs/writing/skills/writing-skills` enforces. The 2026-07-28
merge rewrote the shared body prose; `references/` and `scripts/` remain byte-for-byte the
pinned commit's, renamed only (`tells.md` → `tells-code.md` / `tells-ui.md`). See the SKILL.md
Provenance section for detail, and
`upstreams.toml [vibecoded-design-tells]` for the verdict and `LICENSE` in this pack for the
full grant. The repo's own MIT note applies here too: the license covers the code/docs vendored
into `skills/`; the raw Reddit harvest (`corpus.jsonl`, charts, CSVs) was never copied into
Axon, so its separate data terms don't apply to anything in this pack.

## Further reading
`references/tells-code.md` and `references/tells-ui.md` carry the full ranked-tell catalogs with
cited-vs-matched data shares and real quotes. `references/fitting-the-codebase.md` and
`references/choosing-a-look.md` are the deliberate-choice methods Build mode points to, one per
domain.
