---
name: academic-writing
description: Drafts, critiques, and cite-checks academic writing (thesis chapters, conference/journal papers) across two genre profiles — empirical-CS/paper and DSR/qualitative-thesis — with a genuinely adversarial multi-skeptic critique pass and Quarto/BibTeX-aware citation tooling. Use when drafting a section from bullet points, running a flow/reverse-outline check on a draft, running a hard-critic or red-team review pass on a chapter or paper, or validating/resolving citations and checking citation-key coverage. Do not use for README/user docs, general non-academic prose, or as a ghostwriter — it produces sourced skeletons and critique, not net-new prose, unless the calling context explicitly grants an exception.
allowed-tools: Read, Write, Edit, Bash
---

# Academic Writing

Adds academic-writing-specific judgment a general-purpose model doesn't have by default: which genre
a document belongs to and what that implies structurally, a checkable flow diagnostic, a genuinely
adversarial critique pass (most academic-writing tooling is structurally biased toward gentle
"looks good with minor fixes" feedback), and working citation-resolution/validation tooling. Four
workflows, one skill — they share triggers ("help with this paper/thesis") even though invocation
phrasing differs per workflow.

**Default posture: never ghostwrite.** Every workflow below produces sourced skeletons, critique, and
findings — not net-new prose — unless the calling context (a project's own CLAUDE.md/skill, or an
explicit user instruction this session) grants an exception. This mirrors the discipline a rigorous
thesis/paper process already requires: the author's own reasoning is the thing being examined,
not the assistant's.

## Genre profile — pick this first, every workflow depends on it
Before running any workflow, determine which genre profile the target document belongs to:
- **Empirical-CS / conference paper** (has a method, baselines, ablations) → `references/genre-empirical-cs.md`
- **DSR / qualitative thesis** (design contribution against a knowledge base, single-case study) →
  `references/genre-dsr-qualitative.md`

These are not interchangeable — see Gotchas. If genuinely unclear which applies, ask rather than guess;
picking wrong produces confidently wrong-shaped output.

## Workflow: Draft

Copy this checklist and track progress:
```
- [ ] Step 1: confirm genre profile + read the relevant genre reference
- [ ] Step 2: get the author's bullet points or messy draft (never invent them)
- [ ] Step 3: produce a sourced skeleton or a critique-then-example, per the no-ghostwriting posture
```

**Step 1 — Orient.** Read the genre-profile file (above) for the section being drafted (Abstract,
Introduction, Method, Discussion, etc. — empirical-CS profile names these explicitly; DSR profile
gives the generic Discussion shape and defers to the author's own confirmed chapter outline when one
exists). Read `references/house-style.md` for prose-mechanics rules (B1–B8) to apply to any generated
skeleton language.

**Step 2 — Get material to work with.** If the author has provided bullet points or a messy draft,
work from that. If not, do not write the passage — produce a **sourced skeleton** instead: the claim
each sentence must make, the citation key it needs, and the fact it stands on (see
`references/citation-workflows.md` for the claim-evidence-map format this skeleton should follow).

**Step 3 — Respond per the drafting workflow.** If given a pre-text/messy draft: first state what's
wrong with it (vague claims, missing evidence, structural gap against the genre profile), then show
an improved version as an *example*, explaining the changes — not a silent rewrite. If given nothing
to work from: hand back the sourced skeleton from Step 2 and stop; do not fill it in.

## Workflow: FlowCheck

Copy this checklist and track progress:
```
- [ ] Step 1: reverse-outline the target section/chapter
- [ ] Step 2: apply the reader test to any topic sentence that didn't trace cleanly
- [ ] Step 3: check given-new sentence flow and rhythm variance
- [ ] Step 4: spot-check transitions against the function table
- [ ] Step 5: run scripts/scan-ai-tells.js for the mechanical rhythm/filler patterns
- [ ] Step 6: report paragraph-level findings, not a general verdict
```

Full procedure: `references/flow-diagnostics.md` (reverse-outlining, reader test, transitions,
given-new/rhythm) and `references/ai-cadence-tells.md` (the seven sentence-rhythm AI tells — word-level
filler is `references/house-style.md` B2/B7). Run reverse-outlining first — it's the highest-signal
check and usually surfaces what the other techniques would find individually. Run
`node scripts/scan-ai-tells.js <file-or-dir>` for the two mechanically-detectable rhythm patterns
(em-dash interruption, semicolon splice) and the word-level filler list — treat its output as leads to
verify, not confirmed findings (see Gotchas). Report findings as a list keyed to specific paragraphs/
topic sentences; a bare "flows well" / "doesn't flow" verdict isn't actionable and should be rejected
before it's reported.

## Workflow: CriticReview

Copy this checklist and track progress:
```
- [ ] Step 1: confirm genre profile, prefix every critic brief with it
- [ ] Step 2: dispatch the five lens-briefs in parallel
- [ ] Step 3: fix what they find (or hand findings back to the author, per no-ghostwriting)
- [ ] Step 4: run the adversarial pass on the revised draft, not the rough one
- [ ] Step 5: aggregate verdicts, surface any "fatal" immediately
```

**Step 1–2.** On Claude Code, dispatch the eight `academic-writing-*` subagents (5 lenses + 3 skeptics,
shipped in this pack's `../../agents/`, linked to `~/.claude/agents/academic-writing/`) — they carry
hard tool restrictions (Read/Grep/Glob only, Write/Edit actually blocked) that a prompt-only dispatch
can't enforce. On any other harness, or if the native agents aren't linked, dispatch the five
job-specific critique briefs in `references/critic-briefs.md` (technical, logic, consistency,
bibliography, layout) as parallel `general-purpose` agent dispatches instead — not sequential, so
findings aren't anchored on each other. Either way, prefix every brief/agent call with the genre-profile
content so findings are calibrated correctly (a technical reviewer checking a DSR thesis for "missing
ablations" is asking the wrong question).

**Step 3.** These five lenses find specific, scoped problems — surface them to the author for fixing
(or apply, only under an explicit-exception grant per this skill's default posture above).

**Step 4–5.** Once the lens-brief findings are addressed, run the adversarial multi-skeptic pass — the
three `academic-writing-skeptic-*` subagents on Claude Code, or `references/adversarial-redteam.md`'s
briefs via `general-purpose` elsewhere — on the resulting draft, not the rough one (running it earlier
wastes its signal on issues the lens-briefs would catch more cheaply). This is the piece most
third-party academic-writing tooling doesn't have: three independent skeptics briefed to argue the
work should be rejected, not to suggest improvements. Any single `fatal` verdict surfaces immediately,
regardless of what the other two skeptics found.

## Workflow: Citations

Copy this checklist and track progress:
```
- [ ] Step 1: check citation-key coverage
- [ ] Step 2: validate bib entries against CrossRef (only if network access + author consent)
- [ ] Step 3: resolve new papers by query or DOI, when adding a source
```

**Step 1 — Coverage check (always safe, local-only).** Run
`node scripts/check-citations.js <project-dir> [path/to/references.bib]` — reports citation keys used
in `.tex` and `.qmd` files that don't resolve in the bib file, and bib entries never cited. Omit the
bib path to auto-discover the first `.bib` file under the project directory.

**Step 2 — DOI/metadata validation (hits CrossRef).** Run
`node scripts/validate-bib.js path/to/references.bib` — flags title/author/year mismatches against
CrossRef and placeholder DOIs. Network call per entry with a DOI; ask before running on a large bib
file repeatedly.

**Step 3 — Resolve a new source.** Run
`node scripts/resolve-papers.js --query "search terms" [--year YYYY-YYYY] [--limit N]` or
`node scripts/resolve-papers.js --doi "10.xxxx/yyyy"` — searches Semantic Scholar/Unpaywall/CrossRef
and emits ready-to-paste BibTeX. Does not write to the project's bib file automatically — hand the
result to the author to review and add, per the citation-verification discipline in
`references/citation-workflows.md`.

**Style/backend questions** (APA vs IEEE vs ACM, LaTeX vs Quarto citeproc): `references/citation-workflows.md`.

## Error handling
* If `scripts/check-citations.js` reports "No .bib file found," pass the path explicitly as the
  second argument rather than relying on auto-discovery.
* If `scripts/validate-bib.js` or `resolve-papers.js` report unreachable APIs, the network call
  failed (rate limit or connectivity) — retry once; don't treat a single unreachable result as a
  confirmed mismatch.
* If a `CriticReview` lens-brief agent returns zero findings, treat that as a real "checked, nothing
  found" result only if the brief's own report says so explicitly — a lens brief that goes silent on
  a section it should have covered is a dispatch failure, not a clean pass.

## Gotchas
- **A PDF newer than its inputs is not proof it contains them.** Quarto reads the source files when
  a render *starts* but writes the PDF only when it *finishes*, so an edit that lands mid-render is
  silently absent from a PDF whose mtime is nonetheless newer than that edit. Verify against a build
  that **started** after the last relevant edit, never against output timestamps alone. This bit the
  thesis pipeline twice in one day: numpages and bib content each lagged a build behind while every
  timestamp check said current. (Moved here 2026-07-30 from the repository bootstrap, where it was the only
  Quarto-specific gotcha in the repository bootstrap.)
- **The two genre profiles are not interchangeable, and picking wrong is a structural failure, not a
  style one.** A CV-paper Method/Experiments template applied to a DSR thesis discussion produces
  confidently wrong-shaped prose — the underlying source material this skill draws on is
  empirical-CS-flavored by default (see the pack README's Attribution — `[research-paper-writing-skills]`, `[pengsida-research-notes]`), so the
  DSR profile requires deliberate selection, not the default path.
- **`check-citations.js`'s `.qmd` extractor requires a digit in the matched key** (real citekeys are
  almost always `authorYYYY...`) to avoid false-positiving on prose that happens to contain a literal
  `@word` that isn't a citation (e.g. a placeholder comment like `[@keys]`). If a project uses
  non-standard digit-free citekeys, this will under-report — check manually in that case.
- **`validate-bib.js` and `resolve-papers.js` hit live public APIs** (Semantic Scholar, CrossRef,
  Unpaywall) — no API key required, but rate limits apply on large bibliographies; don't loop them in
  a tight retry.
- **A citation can resolve in the bib file and still be wrong** — `check-citations.js` only checks key
  existence, not whether the source actually supports the claim it's attached to. That's the
  bibliography-auditor lens-brief's job (`references/critic-briefs.md`), not the script's.
- **Dispatch `CriticReview`'s five lens-briefs in parallel, not sequential** — sequential dispatch lets
  later agents anchor on earlier findings and under-report their own lens.
- **Run the adversarial pass (`adversarial-redteam.md`) after the lens-brief fixes, not before** — on
  a rough first draft it will produce a long list dominated by issues the cheaper lens-briefs would
  have caught; that's expected but wastes the adversarial pass's signal if run out of order.
- **`scan-ai-tells.js`'s semicolon-splice pattern truncates its printed preview to 100 characters** —
  the flagged semicolon can be past the visible slice, so a preview that looks like it has no
  semicolon in it is not a false positive on its own; open the file at the reported line number rather
  than judging from the preview text alone. The pattern also can't distinguish a real independent-
  clause splice from a legitimate structured/enumerated semicolon list — markdown table rows are
  already filtered out, but inline enumerated lists in prose are not; verify each hit.
- **`scan-ai-tells.js` patterns 3, 4, 5, 6, 7 from `ai-cadence-tells.md` are NOT scanned by the
  script** (only patterns 1–2, the two mechanically detectable ones, plus the word-level filler list) —
  triads, run-ons, negation-elevation and false-hierarchy modifiers need editorial judgment a regex
  can't supply; check those manually or via the `CriticReview` lens-briefs.

## Examples

**Example 1: Citation coverage check on a live Quarto thesis**
```
User: "check my thesis citations"
→ Invokes Citations workflow, Step 1
→ node scripts/check-citations.js <thesis-dir>/Paper
→ Reports: citation key used in prose but missing from the bib file, plus any unused bib entries
```

**Example 2: Drafting a Discussion chapter section from bullet points**
```
User provides bullet points for a DSR thesis Discussion chapter section.
→ Invokes Draft workflow
→ Step 1 confirms DSR/qualitative genre profile, reads references/genre-dsr-qualitative.md
→ Step 3 critiques the bullets against the profile's Discussion structure, then shows an improved
  version as an example — does not silently rewrite into "clean" prose
```

**Example 3: Hard-critic pass before a submission deadline**
```
User: "review this chapter like a hostile reviewer"
→ Invokes CriticReview workflow
→ Dispatches the five lens-briefs in parallel, author fixes what they find
→ Runs the adversarial 3-skeptic pass on the revised draft
→ Any "fatal" verdict surfaces immediately with the specific claim/section it attacks
```

## Provenance and evaluation

Built 2026-07 from a source-material sweep of several third-party academic-writing tools (citation
scripts, review-agent plugins, prose-template collections) — cherry-picked and rebuilt, not merged
wholesale; see each reference file's own notes for what was ported vs. authored fresh.
`scripts/check-citations.js` was run against a live multi-chapter Quarto thesis as its first real
evaluation (not a synthetic test case): it initially over-matched on sentence-final punctuation and a
literal non-citation `@word` mention in a code comment (both fixed — see the `.qmd`-extractor Gotcha
above) and, once fixed, correctly surfaced a genuine missing bibliography entry cited three times in
the live document. Re-run `scripts/check-citations.js <project-dir>` after any regex change to this
extractor to confirm no regression against a real project before trusting its output again.
