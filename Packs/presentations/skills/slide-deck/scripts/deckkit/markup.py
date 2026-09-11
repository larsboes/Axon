"""Text with a two-token markup, converted to real PowerPoint runs.

A deck's content file is Python, so rich text has to survive as a plain string.
Rather than import every run and font, the content writes:

    "Die Verhaltensanforderungen stehen **nicht im Quellcode**. Sie liegen ~verteilt~."

and this module turns it into runs. Two tokens only, because a deck needs two:
emphasis, and one accented phrase per block.

    **bold**
    ~accent~    bold, and coloured by the surface it lands on

The accent colour is a parameter, not a constant, because the same string is read
on a white panel and on a filled accent bar. Callers pass `Theme.emphasis_on(fill)`.
"""

from __future__ import annotations

import re

TAG = re.compile(r"(\*\*.+?\*\*|~[^~]+~)", re.S)

BULLET = "\u25cf   "


def segments(text: str) -> list[tuple[str, str]]:
    """Split marked-up text into `(kind, content)` where kind is plain/bold/accent."""
    out: list[tuple[str, str]] = []
    for chunk in TAG.split(text):
        if not chunk:
            continue
        if chunk.startswith("**") and chunk.endswith("**"):
            out.append(("bold", chunk[2:-2]))
        elif chunk.startswith("~") and chunk.endswith("~"):
            out.append(("accent", chunk[1:-1]))
        else:
            out.append(("plain", chunk))
    return out


def residue(text: str) -> str | None:
    """Return the first stray marker in `text`, or None.

    `check` uses this: an unpaired `*` or `~` reaches the slide as a literal
    character, and the only place it is obvious is the rendered output. Catching it
    in the source is cheaper than catching it by eye on slide 24.
    """
    stripped = TAG.sub("", text)
    for marker in ("*", "~"):
        if marker in stripped:
            return marker
    return None


def write_runs(paragraph, text, *, size, colour, font, bold=False, italic=False,
               accent=None, bullet=False, bullet_size=None, bullet_colour=None):
    """Append `text` to a python-pptx paragraph as styled runs.

    A newline inside a run becomes a real line break. This matters: setting
    `run.text = "a\\nb"` writes a literal newline inside `<a:t>`, which PowerPoint
    treats as whitespace and LibreOffice happens to render as a break. The two
    renderers disagree, so the portable form is an explicit break element.
    """
    if bullet:
        mark = paragraph.add_run()
        mark.text = BULLET
        mark.font.name = font
        mark.font.size = _pt(bullet_size if bullet_size is not None else size * 0.6)
        mark.font.bold = True
        if bullet_colour is not None:
            mark.font.color.rgb = bullet_colour

    for kind, content in segments(text):
        for index, line in enumerate(content.split("\n")):
            if index:
                paragraph.add_line_break()
            if not line:
                continue
            run = paragraph.add_run()
            run.text = line
            run.font.name = font
            run.font.size = _pt(size)
            run.font.italic = italic
            run.font.bold = bold or kind in ("bold", "accent")
            run.font.color.rgb = accent if kind == "accent" and accent is not None else colour
    return paragraph


def _pt(value):
    from pptx.util import Pt

    return Pt(value)
