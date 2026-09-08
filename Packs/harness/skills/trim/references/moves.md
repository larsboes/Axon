# The three moves

Each one keeps every directive and reduces what is paid for it. They are ordered by risk: MERGE
touches meaning, TIGHTEN touches wording, RELOCATE touches only the address.

## MERGE — two rules become one

**Use when** two rules overlap enough that one sentence carries both without qualification.

**The test before proposing it:** write down every directive in both originals as a separate line.
Write down every directive in the merged version the same way. The two lists match, or the merge
is wrong.

Two rules that cover different cases are not overlapping — "cite the file" and "mark a claim you
cannot cite" look similar and do different work. Merging them loses the second case in the
qualification of the first, which is the characteristic failure of this move.

Never merge across scopes. A rule in the user file and a rule in a project file that say similar
things are not duplicates; they have different reach, and merging them either narrows one or
widens the other.

## TIGHTEN — one rule, fewer words

**Use when** a rule is correct and three times longer than the instruction inside it. Usually:
a preamble explaining why the rule exists, an example that repeats the rule, and a hedge.

**What comes out:** the preamble, the restatement, the second example, and adverbs that change no
behaviour.

**What never comes out:**

- The exception. "Always X, except Y" is two directives and the second is why the first is safe.
- The reason, when the reason decides an edge case. "Prefer A because B" tells the reader what to
  do when B is false. "Prefer A" does not.
- A negation. "Never X" is not implied by "do Y", however obvious it looks.
- The concrete noun. A path, a command or a tool name is what makes a rule executable; the
  general version reads better and cannot be followed.

Never shorten a hedge into a fact. "Usually X" and "X" are different instructions, and the
difference is the whole reason somebody wrote the hedge.

## RELOCATE — the same words, a different address

**Use when** detail is needed rarely and a pointer can stand in its place.

**Targets, in order of preference:**

1. **A skill.** Anything with a procedure. It loads when it triggers and costs nothing otherwise.
2. **A reference file the always-on file points at.** For detail with no procedure: a schema, a
   table, a long list. The pointer says when to read it, not just where it is.
3. **The owning document.** A rule about one repository belongs in that repository's own context
   file, where it is loaded only when working there.

**The rules of a relocation:**

- **Leave a stub that names the trigger.** "For the vault contract, read `<path>`" is a
  relocation. Deleting the section and hoping the model finds the file is a deletion.
- **Verify the destination exists** and contains the text, after moving. A pointer to a file that
  was never written is worse than the paragraph it replaced.
- **Count the saving honestly.** The stub is not free. A three-line stub replacing four lines is
  not a trim.

## Ordering

Take the moves in the order of what they save per unit of attention spent: RELOCATE of a whole
procedure first (largest saving, lowest risk, one decision), then TIGHTEN of the biggest sections,
then MERGE. MERGE is last on purpose — it is the only move that can silently change what a rule
means.
