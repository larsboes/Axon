# AI-cadence tells — the academic handling

`house-style.md` B2/B7/B8 cover *word-level* AI tells (banned words, intensifiers). This file covers
*rhythm-level* tells — patterns that read as machine-written even when every individual word is fine.
Authored from real, repeatedly-observed patterns in actual academic drafting sessions (generalized from
project-specific usage, not derived from any of the third-party source material this skill was
originally built from).

## Ownership, and why three of these are one line

**`human-writing` owns the tell catalog.** Its `references/tells.md` defines every tell — what it is,
the evidence behind it, and the general fix — with the data behind the ranking (a 600-post audited
sample out of an 89,239-post pull) and the linter that implements it. That is the only place a tell is
defined.

This file states the **academic handling**: the fix convention that applies when the text is a thesis
chapter or a paper. It does not restate what a tell is.

It used to restate three of them, and the two texts had **already drifted**: the catalog flags *any*
em dash ("the rule is simply not to ship one"), while pattern 1 below described only the
interruption shape. Two definitions of one tell, one of them stricter than the other, is exactly the
failure this section exists to prevent. A reader applying the weaker one had no way to know which was
current.

So patterns **1, 5 and 6** name the catalog entry that owns them and state the academic handling only.
Patterns **2, 3, 4 and 7** are academic-only: no catalog entry covers them.

## The seven patterns

1. **Em-dash interruptions.** Owned by the catalog, **entry 1**. Academic handling: open the list
   with a colon and close with a full stop instead of resuming the sentence, or split into two
   sentences outright. Apply the catalog's rule, not a narrower one — it is the stricter of the two
   and it is the one with data behind it.

2. **Semicolon splices.** *Academic-only.* A semicolon joining two full independent clauses. Fix:
   split into two sentences, or use a comma if one clause is genuinely subordinate. (Exception:
   semicolons inside structured lists or ID references — e.g. `D21; D23` — carry real meaning and
   aren't a tell. ASD-STE100 bans the mark outright; this file does not, because an academic document
   is read by people, not parsed by a machine. See `asd-ste100` for that reader.)

3. **Comma before "and" in a 3-item list** (the Oxford-comma question). *Academic-only.* Not wrong,
   but `X, Y, and Z` reads more mechanical than `X, Y and Z` in flowing academic prose — pick one
   convention per document and hold it, but default to the non-Oxford form when no house style
   dictates otherwise.

4. **Accretive multi-clause run-ons.** *Academic-only.* A sentence that keeps adding "and X and Y and
   it also Z" across several clauses. Fix: break into two or three sentences. If a punctuation rule
   (1–3 above) forces an awkward split, that's a signal the sentence was too long to begin with —
   restructure, don't just re-punctuate.

5. **Triads for rhythm, not content.** Owned by the catalog, **entry 18** ("the rule of three").
   Academic handling: keep the triad only when every item is independently load-bearing; otherwise
   cut to the ones that actually matter, even if that leaves one or two.

6. **Negation-elevation framing** ("does not only X. It actually Y."). Owned by the catalog,
   **entry 2** (the antithesis cadence). Academic handling: delete the negation sentence and keep the
   direct assertion; it's stronger alone. If a contrast is genuinely load-bearing, fold it into one
   clause instead of a standalone elevation sentence — the standalone form answers an objection the
   reader was never given, and the negation weakly implies the lesser reading was partly true.

7. **False-hierarchy modifiers** ("primary", "main", "core" where no secondary/other rank actually
   exists in the text). *Academic-only.* These words imply a ranked set. Test: can you name the other
   ranked item(s) the text is implicitly contrasting against? If not, the modifier is empty — delete
   it. (Legitimate use: "its primary contribution over the prior iteration" when a genuine secondary
   contribution is also named nearby.)

## How to apply these

- Treat 1–4 as close to mechanical — they're punctuation-pattern matches a script could flag (see
  `scripts/scan-ai-tells.js`, which greps for the em-dash and semicolon patterns; 5–7 need the
  agent/reviewer's judgment, not a regex, since they depend on whether the surrounding items are
  genuinely load-bearing).
- **The scanner is narrower than the rule, deliberately, and this is the one place that matters.**
  `scan-ai-tells.js` matches the em-dash *interruption* shape only, while the catalog's rule is *any*
  em dash. The narrow regex keeps a dash inside a table row or an enumeration from reading as a
  finding. A human reviewing against the catalog applies the stricter rule; the script is a
  first-pass gate, not the standard.
- These are **review heuristics, not license to silently rewrite** — per the no-ghostwriting posture in
  `SKILL.md`, surface the pattern with a before/after example and let the author choose, exactly as
  `Draft`'s Step 3 already requires for any critique.
- Content-conservation caveat: "shorten this" is a flow instruction, not a delete instruction. When
  compressing a section to fix run-ons (pattern 4) or cut a triad (pattern 5), classify each removed
  fragment as rehomed / redundant / lost before cutting it, and recover anything that would otherwise
  disappear with no trace elsewhere in the document.
