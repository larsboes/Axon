"""<deck title> — <what this deck has to do, and to whom>.

Scaffolded by `scripts/deck init`. This file is content and nothing else: the
mechanism is in the slide-deck skill, and a deck should read as a sequence of
decisions. If a layout is missing, compose from the primitives on the Slide
(s.panel, s.picture, s.table, s.kpi) rather than editing the skill.

Copy this checklist and track progress:

    - [ ] Brief fixed: audience, duration, language, and what the talk must achieve
    - [ ] Genre chosen: references/genre-dsr-defence.md, genre-empirical-cs-talk.md or genre-academic-job-talk.md
    - [ ] Theme chosen (from the source artifact if one exists)
    - [ ] Spine written: sections, one assertion per slide, seconds per slide
    - [ ] Content written below
    - [ ] scripts/deck build <this file>      -> clean
    - [ ] scripts/deck check <this file>      -> 0 errors
    - [ ] scripts/deck render <this file>     -> LOOK AT THE CONTACT SHEET
    - [ ] Fix what the sheet shows, rebuild, render again
    - [ ] scripts/deck timing <this file>     -> the budget sums; each script fits its slot
    - [ ] scripts/deck readiness <this file>  -> answer the five (a defence; references/defence-readiness.md)
    - [ ] scripts/deck handout <this file>    -> rehearse from it
    - [ ] Backup slides written for the questions the speaker expects
"""

from pathlib import Path

from deckkit import Deck, Theme

HERE = Path(__file__).resolve().parent

deck = Deck(
    theme=Theme.load(HERE / "Assets/themes/warm-scientific-teal.json"),
    name="my-talk",
    author="<name on the badge>",
    footer_label="<label> / <date>",
    output=HERE / "out",
    # The allotted talk length, for `deck timing`. Counts the notes' ZEIT lines against it.
    minutes=10,
    # Write the PDF beside the .pptx on every build. Needs LibreOffice.
    export_pdf=True,
)

# ── front ───────────────────────────────────────────────────────────────
deck.title(
    kicker="<kicker>",
    title="<full deck title>",
    date="<date line>",
    meta=["<author block>", "<examiner or host block>"],
    tagline="<duration> Minuten Vortrag",
    notes="ZEIT 0:00\u20130:20 (20 s)\n\nSprechtext: \u201e<what to say, not what the slide says>\u201c",
)

deck.agenda(
    ["<Section 1>", "<Section 2>", "<Section 3>", "<Section 4>", "Backup"],
    notes="ZEIT 0:20\u20130:35 (15 s). Name the sections and move.",
)

# ── one section, showing the two shapes you will use most ───────────────
deck.section(1, "<Section title>", "<the one-line claim this section earns>",
             notes="ZEIT 0:35\u20130:40 (5 s). A beat, not a slide.")

s = deck.open("<Assertion, not a topic>", "1 · <Section 1>")
s.three_columns([
    ("<Column heading>", "accent", [
        "First point, ~the phrase that matters~, then the consequence.",
        "Second point.",
    ]),
    ("<Column heading>", "secondary", [
        "First point.",
        "Second point.",
    ]),
    ("<Column heading>", "caution", [
        "The limitation, stated plainly.",
    ]),
], reserve=s.statement_reserve)   # leave room for the bar below
s.statement("The one sentence this slide exists to deliver.")
s.notes("ZEIT 0:40\u20131:30 (50 s).\n\nSprechtext: \u201e<the spoken text, including the pauses>\u201c\n\n"
        "claim -> evidence -> bound. Never the bound alone.")

# `figure_beside` is the reading order for a result slide: figure left, the two
# or three lines that interpret it right. Uncomment it once Assets/figures/ holds
# a real figure — the scaffold ships that folder empty on purpose, so the first
# `build` succeeds rather than failing on a placeholder path.
#
# s = deck.open("<Assertion over a figure>", "1 · <Section 1>")
# s.figure_beside(HERE / "Assets/figures/my-plot.png", [
#     "What the figure shows, in one line.",
#     "The number worth remembering.",
# ], note="Source: <where it came from, and what it does not show>.")
# s.notes("ZEIT 1:30\u20132:10 (40 s).\n\nSprechtext: \u201e<the spoken text>\u201c")

# ── close ───────────────────────────────────────────────────────────────
deck.closing(
    kicker="<the question this deck answers>",
    verdict="<the answer, as a sentence>",
    thanks="Vielen Dank. Ich freue mich auf Ihre Fragen.",
    meta="<author> · <contact>   |   Backup-Folien ab B0",
    notes="ZEIT 2:10\u20132:25 (15 s). Stop after the sentence.",
)

deck.backup_divider(
    claim="Detail slides for the discussion. Not shown in the talk.",
    notes="Open these only when a question asks for one.",
)

s = deck.open("B1 · <question this slide answers>", "Backup")
s.write(s.x, s.top, s.w, 4.0, [
    "The detail a questioner will want.",
])
s.notes("Which question this answers.")
