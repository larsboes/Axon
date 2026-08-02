# AI-cadence tells — sentence-rhythm patterns, not just word choice

`house-style.md` B2/B7/B8 cover *word-level* AI tells (banned words, intensifiers). This file covers
*rhythm-level* tells — patterns that read as machine-written even when every individual word is fine.
Authored from real, repeatedly-observed patterns in actual academic drafting sessions (generalized from
project-specific usage, not derived from any of the third-party source material this skill was
originally built from) — genuinely more precise than a generic "avoid robotic writing" instruction.
Feed these into `FlowCheck` and the `writing-reviewer`-equivalent pass in `CriticReview`.

## The seven patterns

1. **Em-dash interruptions.** `noun — list, of, things — continues`. The single strongest tell. Fix:
   open the list with a colon and close with a full stop instead of resuming the sentence, or split
   into two sentences outright.

2. **Semicolon splices.** A semicolon joining two full independent clauses. Fix: split into two
   sentences, or use a comma if one clause is genuinely subordinate. (Exception: semicolons inside
   structured lists or ID references — e.g. `D21; D23` — carry real meaning and aren't a tell.)

3. **Comma before "and" in a 3-item list** (the Oxford-comma question). Not wrong, but `X, Y, and Z`
   reads more mechanical than `X, Y and Z` in flowing academic prose — pick one convention per document
   and hold it, but default to the non-Oxford form when no house style dictates otherwise.

4. **Accretive multi-clause run-ons.** A sentence that keeps adding "and X and Y and it also Z" across
   several clauses. Fix: break into two or three sentences. If a punctuation rule (1–3 above) forces an
   awkward split, that's a signal the sentence was too long to begin with — restructure, don't just
   re-punctuate.

5. **Triads for rhythm, not content.** A reflexive three-item list ("controllable, traceable and
   interpretable") used because three items sound balanced, not because there are exactly three real
   things. Fix: keep only when every item is independently load-bearing; otherwise cut to the ones that
   actually matter, even if that leaves one or two.

6. **Negation-elevation framing** ("does not only X. It actually Y."). A sentence that first rules out a
   lesser reading, then asserts the intended one, purely for rhetorical emphasis — it answers an
   objection the reader was never given, and the negation weakly implies the lesser reading was still
   partly true. Fix: delete the negation sentence and keep the direct assertion; it's stronger alone. If
   a contrast is genuinely load-bearing, fold it into one clause instead of a standalone elevation
   sentence.

7. **False-hierarchy modifiers** ("primary", "main", "core" where no secondary/other rank actually
   exists in the text). These words imply a ranked set. Test: can you name the other ranked item(s) the
   text is implicitly contrasting against? If not, the modifier is empty — delete it. (Legitimate use:
   "its primary contribution over the prior iteration" when a genuine secondary contribution is also
   named nearby.)

## How to apply these
- Treat 1–4 as close to mechanical — they're punctuation-pattern matches a script could flag (see
  `scripts/scan-ai-tells.js`, which greps for the em-dash and semicolon patterns; 5–7 need the
  agent/reviewer's judgment, not a regex, since they depend on whether the surrounding items are
  genuinely load-bearing).
- These are **review heuristics, not license to silently rewrite** — per the no-ghostwriting posture in
  `SKILL.md`, surface the pattern with a before/after example and let the author choose, exactly as
  `Draft`'s Step 3 already requires for any critique.
- Content-conservation caveat: "shorten this" is a flow instruction, not a delete instruction. When
  compressing a section to fix run-ons (pattern 4) or cut a triad (pattern 5), classify each removed
  fragment as rehomed / redundant / lost before cutting it, and recover anything that would otherwise
  disappear with no trace elsewhere in the document.
