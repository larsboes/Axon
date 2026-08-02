# The reader's job

Pick the type by what the reader is trying to **do** with the text, not by the container it
ships in. Email, blog, doc, and Slack are containers. The reader's job is what dictates
rhythm, structure, and what "good" means here. A landing page and a fundraising letter share
the persuasive job and need the same spine. A tutorial and an API reference share the
container "docs" and need opposite craft.

The "characteristic AI failure" column is the tell specific to that job, on top of the
universal catalog in [tells.md](tells.md), which applies to every type.

## How to use this

1. Name the job first. A piece that does two jobs still has a primary one. Write to that, and
   do not let the secondary blur the spine.
2. Apply the row's craft rules on top of the five structural moves and the register profile.
3. Run the register-consistency check at the bottom last. It catches the failure the
   per-type rows miss.

## The taxonomy

| Type | Reader's job | Craft rules | Characteristic AI failure |
|---|---|---|---|
| **Narrative** | Follow a sequence of events | Control scene against summary deliberately. Concrete sensory and causal detail. Show rather than tell. | Flattening: cliché density, uniform affect, a predictable setup-to-resolution arc, telling where it should show. |
| **Expository** | Understand why something is true | Order known to unknown. One governing analogy, not five. Answer "why", not only "what". | Over-explains the obvious, restates instead of clarifying, hedges to avoid being wrong. |
| **Persuasive** | Decide what to believe | Claim up front. Calibrate evidence to how skeptical the audience is. Address the strongest counterargument, not a strawman. | Confident tone substituting for evidence. Assertion without proof. |
| **Marketing** | Decide to act, fast | Benefit first. Specificity beats superlative. One emotional hook tied to one concrete proof point. | Buzzword density ("unlock", "seamless", "game-changer"), vague superlatives, zero specifics. |
| **Expressive** | Encounter a particular human voice | Hold one voice, or split past and present self on purpose. Idiosyncratic detail. Tolerate unresolved ambiguity. | Institutional voice, ambiguity resolved into a tidy lesson, moralizing instead of showing. |
| **Announcement** | Know what changed and what it means for them | Lead with the change and its consequence. Versions, dates, names. Say what to do next. | Corporate throat-clearing before the point, "we're excited to" with no substance, the actual change buried. |
| **Instructional** | Do a task and succeed | Imperative mood. One verifiable action per step. Cover the error states, not only the happy path. | Steps that assume everything works and are not grounded in the reader's actual system. |
| **Reference** | Look up one fact fast | Neutral tone. Structurally parallel entries. No narrative framing. | Terminology drifting between entries; reads complete while silently omitting cases. |

## Scope boundary

This skill edits prose humans read, which is the first six types. **Instructional** and
**Reference** shade into technical documentation: when the artifact is an API reference,
docstrings, or code comments, leave it alone or follow that project's own conventions. Their
rows stay here because a prose piece often carries one instructional or reference passage.
Apply the row to that passage, not to a whole API doc.

## Register consistency

One failure cuts across every type: the register drifts mid-piece. Promotional language
bleeds into a memo, marketing hype surfaces in a status update, a casual aside lands inside a
formal argument. A human writer holds a register by instinct; a model slips between the ones
it has seen most, which makes this a reliable tell.

After drafting, read for register breaks. Does any sentence belong to a different type than
the one chosen? Pull it back. Hold one register unless a shift is deliberate, and note that
this is a different thing from the *shift register* move in the skill body, which is about
tonal range **inside** one register rather than sliding between genres.

---

Paraphrased from the reader-job taxonomy in
[ryanthedev/oberskills](https://github.com/ryanthedev/oberskills) `write` (MIT declared in
`.claude-plugin/plugin.json`), which in turn draws on Diátaxis, Kinneavy's aims of discourse,
and Aristotle's appeals. Rewritten rather than copied, given that repo carries no root
LICENSE file.
