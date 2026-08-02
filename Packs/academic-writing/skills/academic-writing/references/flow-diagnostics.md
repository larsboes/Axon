# Flow diagnostics — does this writing flow?

Five checkable techniques for diagnosing paragraph- and document-level flow. §1–4 are distilled from a
university writing-center handout; §5 (given-new sentence flow) is a genuine addition beyond that
source — a real, more precise diagnostic for exactly the kind of flow problem §1–4 can miss: a
paragraph where every topic sentence traces cleanly to the thesis and every transition word is correct,
but individual sentences still feel disconnected from each other. Use these as the procedure for the
`FlowCheck` workflow in `SKILL.md`.

## 1. Pretend you are the reader
Ask, for each paragraph/section: (a) would someone outside this specific subfield understand the
vocabulary level? (b) does this paragraph connect back to the thesis/central claim, and is that
connection easy to see, or does the reader have to infer it? (c) would someone one level less expert
than the author understand what's being said here?

## 2. Reverse-outlining (the core diagnostic — run this first)
Post-writing procedure, run on a complete draft:
1. Write down the thesis/central claim statement in one sentence.
2. For every paragraph, write down its topic sentence and its main evidence/explanation points.
3. Check every topic sentence traces back to the thesis/central claim.
4. Check every evidence point actually supports its own paragraph's topic (not a different, adjacent
   point).
5. Any topic sentence that doesn't clearly relate to the thesis → revise it or cut the paragraph.

**Diagnostic signal**: if reverse-outlining is *easy* (the topic sentences and evidence points are
obvious on a read-through), the document is well-organized. If it's *hard* — you have to hunt for what
a paragraph is actually claiming — the thesis statement or the topic sentences are unclear, not the
reader.

## 3. Headings as a drafting tool
Add headings/subheadings during drafting and revision even if the final document won't keep them
(some formats require removing them) — headings force the visible "parts" of an argument and how they
connect to become explicit while writing, which is easy to lose track of in continuous prose.

## 4. Transitional words, by function
Use this table to check a paragraph transition is doing the rhetorical job it should — or to find the
right connector when a transition currently reads as abrupt or absent.

| Function | Words |
|---|---|
| Cause/effect | accordingly, consequently, hence, thus, therefore |
| Comparison | likewise, similarly, in the same way |
| Contrast | however, nevertheless, on the contrary, conversely |
| Examples | for instance, indeed, such as, specifically |
| Place/position | furthermore, moreover, next, in addition |
| Time/sequence | meanwhile, subsequently, simultaneously, following this |
| Summary/conclusion | in short, to summarize, as a result, in conclusion |

## 5. Given-new sentence flow and rhythm variance
A sentence generally reads well when it opens with **given** information (something the reader already
has from an earlier sentence) and closes with **new** information (what the sentence actually adds).
When consecutive sentences violate this — each opening with brand-new material instead of picking up
where the last one left off — the paragraph can pass reverse-outlining (§2) and have correct transition
words (§4) and still feel disconnected sentence-to-sentence, because the *information*, not the
*topic*, isn't chaining. Check: for each sentence after the first in a paragraph, can you point to the
specific word or phrase in the *previous* sentence that this sentence's opening picks up? If not, either
the sentence order is wrong or a bridging clause is missing.

Separately, check **sentence-length rhythm**: a paragraph of uniformly long, similarly-structured
sentences (or uniformly short ones) reads as monotone regardless of whether the content is correct —
count approximate word counts sentence-by-sentence within a paragraph; if every sentence is within a
narrow band with no short sentence anywhere, that's a rhythm flag, not just a style preference (a
one-idea-per-sentence discipline that never varies length is a common mechanical-writing tell — see
`ai-cadence-tells.md` for related rhythm patterns at the multi-sentence level).

## Procedure for `FlowCheck`
1. Run reverse-outlining (§2) on the target section/chapter first — it's the highest-signal check and
   will often surface the same issues the other techniques would find individually.
2. For any paragraph whose topic sentence didn't trace cleanly, apply the reader test (§1) to diagnose
   whether the problem is vocabulary, missing connection to the central claim, or genuinely unclear
   writing.
3. For paragraphs that survived §2 but still feel disconnected on a read-through, run the given-new
   check (§5) — this catches sentence-to-sentence flow problems the topic-sentence-level checks miss.
4. Spot-check transitions between paragraphs against the table in §4 — a paragraph that reverse-outlines
   fine and passes given-new can still read as disconnected from its *neighbor* if the transition
   between paragraphs is missing or wrong-function.
5. Run `node scripts/scan-ai-tells.js <file>` for the mechanically-detectable rhythm/filler patterns as
   a supplementary pass — treat its output as leads to verify, not confirmed findings.
6. Report findings as a list of paragraph- or sentence-level issues (the specific location, the
   specific problem, and which of §1–5 caught it) — not a general "flows well" / "doesn't flow" verdict,
   which isn't actionable.
