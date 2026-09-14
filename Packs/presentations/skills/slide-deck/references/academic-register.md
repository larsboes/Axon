# Academic register on slides

A talk does not use the register of a thesis. A deck that copies the thesis reads as a
document spoken aloud. This file covers person, tense and register for the spoken genre.

It is not prose house-style. That belongs to `academic-writing`
(`references/house-style.md`). It is not the AI-tell linter either. That belongs to
`human-writing`. This file covers the register of one sentence on a slide.

## The spoken genre

A thesis can stay impersonal. A talk need not, and usually should not. First-person
singular is normal in a defence. The candidate did the work, and saying so is the genre.

The thesis writes *"es wurde gezeigt"* or *"it was demonstrated"*. The talk says
*"I show"*. Do not carry the passive nominal style onto the slide. Nobody says *"A
comparison of nine studies was conducted"* out loud.

## Person

- **I**: what you did, decided, chose or measured. Use it as the default in a defence.
- **we**: the team, the field, or the users of the artifact. Keep it consistent. In a
  single-author thesis, do not use *we* to hide a decision that was yours.
- **one** or **man**: avoid it. In a room with one answerable person, it reads as
  evasion.

**The slide is not the script.** The first person belongs in what you say, not in what
is written. A slide that reads *"Ich zeige an einem realen Enterprise-Fall …"* puts the
speaker into the evidence, where a reader who arrives late or reads the deck afterwards
cannot use it. Write the slide as *"Context Engineering ist als Designgegenstand
evaluierbar"* and say the *ich* out loud. The same applies to the notes' instructions:
they address no one personally, so the deck survives being handed to someone else.

## Tense

| what | tense | example |
|---|---|---|
| the work you did | past | "I ran fifteen runs across four configurations." |
| what holds in general | present | "Behavior preservation concerns test-covered input-output behavior." |
| what the slide shows | present | "This figure shows all thirty-six ratings." |
| the outlook | future, sparingly | "The next step is a review of all six final outputs." |

A count does not become more certain in the present tense.

## Register

- Use shorter sentences than the thesis. A slide bullet is spoken, not parsed.
- Use active verbs. Write "The reviewers rated Module A lower", not "lower ratings were
  observed for Module A".
- Delete marketing adjectives: *novel, powerful, seamless, robust, cutting-edge*. Use the
  measurement that earns the claim. In a defence, *novel* invites the obvious question.
- Delete hedging padding: *somewhat, arguably, to a certain extent, it could be said
  that*. `academic-narrative.md` section The bounding register covers the rule. Claim
  first, bound second, and cut the padding that pretends to be either.
- Write numbers as numerals, and attach the n.

### Two things that read as machine-written

The em dash and the number cascade. Both are tells a reader notices without being able
to name, and both were named by a reader of this deck.

- **No em dash (`—`).** On a slide it becomes the strongest mark on the line and the
  slide starts to look generated. Use a comma, a colon, or a full stop. An en dash
  (`–`) is fine and is not the same character: it belongs in ranges (*0:40–1:35*,
  *2–3*, *60 → 80*).
- **No number cascade.** A pile of counts followed by an instruction reads as a
  machine's summary: *"36 Bewertungen, Mittelwerte 3,75 / 3,83 / 3,50. Einzelitems
  sind ordinal …"*. Give the number that carries the sentence, say what it means, and
  put the rest in the notes. A trajectory (*0/3 → 1/3 → 1/3 → 6/6*) is not a cascade:
  it is one fact per configuration, and the arrow carries the reading.

## German and English

- The deck uses the language of the talk. A German defence of an English thesis uses a
  German deck. Do not mix the two on one slide, and do not leave an English heading in a
  German deck.
- **German**: „…" quotes, capitalised nouns, no Oxford comma, and avoid *man*.
- **English**: "…" quotes, and one variety throughout (*behaviour* or *behavior*). Match
  the venue or the thesis.

## Take the sentence from the source

When the thesis or the paper already states a claim, use its sentence. A slide that
paraphrases the thesis into a snappier construction loses the precision the examiners
already accepted, and it picks up the habits of whatever wrote the paraphrase.

The commonest of those habits is the paired negation. "Executable code is not yet a
correct migration. The source code is not the specification." reads as rhetoric, and it
asserts less than the thesis does. The thesis states the positive form next to the
negative one: the behavioural requirements live in pipeline configuration, tests,
expected outputs and platform documentation. Put that sentence on the slide.

The same trap catches a sentence that sounds cleverer than it is. "The design is not
built once and checked once" describes a process the thesis already describes
precisely: design, demonstration and evaluation run four times, and the first three
evaluations inform the next design increment. Use the precise version.

A slide sentence should be one the source document would survive.

## Headings name the object

A column heading names the object it covers. Use the noun the source document's own
section headings use: *Forschungsstand*, *Forschungsbedarf*, *Untersuchungsfall*,
*Qualitätsanforderungen*, *Stand der Forschung*.

A heading that stages a contrast reads as a pitch slide rather than as an academic one.
*Die offene Frage* is a rhetorical beat. *Der Fall als Setting* borrows an English word
into a role it does not have. *Warum X nicht Y genügt* promises an argument in the
heading instead of naming the object. Each one tells the room that a deck was built
before the argument was settled.

Keep the two jobs apart. A slide title asserts something. A column heading names the
thing below it.

## The test

Read the sentence aloud. If you would not say it to a person, it belongs in the notes.
Keep it off the wall.
