# Voice profiles

A voice profile bends humanized prose toward one specific person. Removing AI tells is table
stakes and runs first; the voice is the point. A piece that is merely "human" but does not
sound like the target person has failed the request.

## Where profiles live

Profiles are private and belong outside this skill, because they are built from someone's
real writing. The skill resolves one in this order:

1. An explicit path or URL given in the request.
2. `$WRITE_VOICES_DIR/<name>.md`, when that variable is set.
3. `~/.claude/writing-voices/<name>.md`, the default private directory. "my voice" resolves
   to the author's own profile there.

If none resolves, say so and offer to build one. Never invent a profile and never guess at
someone's voice from a single sample.

## Building one

Gather real samples of the person writing: messages, email, pull requests, docs. Extract
patterns rather than paraphrases. The most valuable section by a wide margin is the verbatim
sample bank, because a model matches cadence against real sentences far better than against
descriptions of them.

## The skeleton

```markdown
# Voice: <Name>

Built from <sources> (<dates>). Pattern, not paraphrase. The sample bank at the bottom is
verbatim so cadence can be matched directly.

## One-line summary
<What survives from their most casual message to their most formal? One sentence.>

## Lexicon
- Softeners and fillers they actually use
- Abbreviations
- Slang that is specifically theirs
- Openers and sign-offs
- Emoji habits
Note how heavily to salt: one or two markers per message, never a pile. Over-use reads as
caricature, which is its own tell.

## Sentence mechanics
<Fragments? Sentence-initial conjunctions? Trailing ellipses? Comma splices when casual?
Lowercase starts? Contractions? Give a real example of each.>

## How they reason
<Point first or setup first? Concrete, naming repos and versions and tickets, or abstract?
Do they propose a next step with a cost attached? How do they flag uncertainty?>

## Interpersonal tone
<Encouraging? Self-deprecating? How do they own a mistake? Do they leave the other person a
door out? Which values do they actually voice?>

## Register ladder
<The same voice at two or three formality rungs, with a real example at each. Name what
carries across all of them and what only changes at the surface.>

## Structure of a longer message
<Their actual template: opener, the ask, reassurance, close.>

## Anti-voice
<Vocabulary, punctuation, and moves that are definitively not them, including the generic AI
tells they would never produce.>

## Verbatim sample bank
<Grouped by register and mood. Real sentences, quoted exactly. Highest-value section.>
```

## Checks before relying on it

1. Read a draft in the person's voice. Would they actually send this? If it sounds like a
   helpful assistant, redo it.
2. The register matches the medium: casual markers absent from a document, present in a
   message.
3. At least one concrete specific rather than a generic claim.
4. The through-line from the one-line summary is actually present.
5. The mechanical floor still holds: no kill-list vocabulary, contractions where the register
   wants them, varied sentence length.

## A note on privacy

A profile is built from a real person's writing. Keep it in the private directory, never in a
public-safe pack, and never paste someone else's samples into a shared or public location.

---

Paraphrased from the voice-profile template in
[ryanthedev/oberskills](https://github.com/ryanthedev/oberskills) `write` (MIT declared in
`.claude-plugin/plugin.json`). Rewritten rather than copied, given that repo carries no root
LICENSE file.
