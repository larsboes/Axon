# Citation workflows — bibliography, claim-evidence mapping, style

Ported near-verbatim from a third-party citation-management reference (flagged as sharp and directly
reusable), generalized to also cover Quarto/citeproc, not just raw LaTeX/BibTeX.

## Bibliography source of truth
Keep a single `.bib` file as the source of truth (`references.bib` is the conventional name) — no
inline ad-hoc citations, no duplicate bibliographies. Citation key convention:
`firstauthorYYYYkeyword` (e.g. `smith2023transformers`) — stable, searchable, doesn't change when
other metadata changes. Avoid auto-generated hash keys.

**DOI-first policy**: for peer-reviewed work, store the DOI in the `.bib` entry whenever one exists;
add `url =` only when a DOI is missing or the URL is the canonical access point (standards, datasets,
software, policy docs).

**Minimal `.bib` entry rules**: protect proper nouns/acronyms in titles (`title = {A Study of {NLP}
and {BERT}}`); always include year; include pages when available; include publisher/booktitle for
proceedings.

## Backend by output format
- **Raw LaTeX**: `\bibliographystyle{IEEEtran}` + `\bibliography{references}` (IEEE/CS convention), or
  `\usepackage[style=apa,backend=biber]{biblatex}` + `\addbibresource{references.bib}` +
  `\printbibliography` (APA).
- **Quarto (`.qmd`)**: citeproc is the default citation engine — cite with `@citekey` (or
  `[@citekey]` for parenthetical) directly in prose, no `\cite{}` macro. Point the document's YAML
  frontmatter `bibliography:` field at the same `.bib` file; set `csl:` to a style file for
  non-default citation formatting (Harvard/author-date, numeric, etc.) instead of hand-rolling style
  rules. Quarto's citeproc handles the LaTeX-backend styles above transparently when rendering to PDF
  via a LaTeX target — the `.bib` file itself doesn't need to change between LaTeX and Quarto projects.

## Verification checklist (before submitting/finalizing)
- Every citation key used in the text (`\cite{...}` or `@citekey`) resolves in the `.bib` file — run
  `scripts/check-citations.js <project-dir>`.
- Every `.bib` entry has correct author list, year, title, venue, pages.
- DOIs resolve to the work actually being cited, not a different paper with a similar title — run
  `scripts/validate-bib.js <path/to/references.bib>`.
- No placeholder entries remain (`10.0000/...` or `example` in a DOI field is a placeholder, not a
  real one).
- Preprints are clearly labeled as such — not described as peer-reviewed.

## Claim-evidence map
Use this while drafting to prevent overclaiming; keep a trimmed version as an internal QA artifact,
not published output.

| Claim | Strength | Evidence | Conditions | Caveats | Citation(s) |
|---|---|---|---|---|---|
| "Method X improves accuracy" | Strong/Medium/Weak | Table 2 vs baselines | Dataset A, metric M | limited to in-domain | key1, key2 |

Rules:
- Every non-trivial claim maps to a concrete artifact: a cited external source, an own result table/
  figure, or a formal statement.
- Comparative claims ("better", "faster", "more robust") must name the comparison set explicitly —
  which baselines, which settings.
- Generalization claims must state their evaluation scope (in-domain, cross-domain, out-of-
  distribution, single-case/qualitative — see `genre-dsr-qualitative.md` for single-case scoping
  language).

**Red flags** (feed these into `critic-briefs.md`'s bibliography-auditor and claim-evidence skeptic in
`adversarial-redteam.md`): a claim supported by a citation that doesn't actually contain the stated
result; a claim depending on unstated assumptions (preprocessing, tuning budget, compute, sample
selection); a claim using unquantified intensifiers ("significant", "substantial") with no number or
test behind them.

## Citation-style quick reference
- **APA (author-date)**: in-text `(Smith, 2023)` or `Smith (2023)`; reference list alphabetical by
  author surname.
- **IEEE (numeric)**: in-text `[1]`; reference list in citation order, not alphabetical.
- **ACM (numeric or author-year, venue-dependent)**: check the specific ACM template — some ACM
  venues use numeric, others author-date; don't assume.
- **Harvard/author-date (many European institutions, incl. via a CSL file)**: same shape as APA
  in-text, but check the specific institution's CSL for exact punctuation/ordering — these vary more
  than the "Harvard" label suggests.
Pick the style your target venue/institution mandates — this skill doesn't have an opinion on which is
"better"; it has an opinion on not mixing two within one document.
