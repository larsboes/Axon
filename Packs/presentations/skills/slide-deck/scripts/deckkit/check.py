"""Structural validation of a built .pptx.

`render` answers "does it look right" and needs eyes. `check` answers "is anything
mechanically broken" and needs none, so it can run in a loop, in CI, or before a
human ever opens the file.

Every check here exists because the corresponding defect actually reached a
rendered deck at least once:

  markup      an unpaired `*` printed its own asterisk
  newline     a literal newline inside `<a:t>` broke differently in PowerPoint
  shadow      every autoshape carried a theme drop shadow
  alignment   paragraphs silently centred inside a panel
  overflow    a statement bar ran into the footer
  overlap     a statement bar was drawn on top of the columns above it
  notes       a slide shipped with no time budget
  page        the footer counter and the physical slide order disagreed
  font        one run kept PowerPoint's default face and rendered in Calibri

Output is one line per finding: `SEVERITY slide N [check] message`. Exit code is
the error count, so `deck check && deck render` is a usable gate.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

from pptx import Presentation
from pptx.oxml.ns import qn
from pptx.util import Emu

from .errors import DeckError
from .markup import residue
from .theme import Theme

#: Rough advance width of a character as a fraction of the font size, for Arial.
#: Used only by the overflow estimate, which is a warning, never a failure.
#: Tuned to over-report: a false warning costs one glance, a missed one costs a slide.
CHAR_RATIO = 0.55

#: Two blocks closer than this on both axes are treated as touching, not overlapping.
#: Adjacent table cells share an edge exactly, and a half-point of rounding must not
#: read as a collision.
OVERLAP_TOLERANCE = 0.04


@dataclass
class Finding:
    severity: str
    slide: int | str
    check: str
    message: str

    def __str__(self) -> str:
        return f"{self.severity.upper():5} slide {self.slide!s:>3} [{self.check}] {self.message}"


def check(path: str | Path, theme: Theme | None = None, *,
          expect_notes: bool = True, max_words: int = 120) -> list[Finding]:
    path = Path(path)
    if not path.exists():
        raise DeckError(
            f"cannot check {path}: not built yet.\n"
            f"  Run `scripts/deck build <deck.py>` first."
        )
    prs = Presentation(str(path))
    findings: list[Finding] = []
    width = Emu(prs.slide_width).inches
    height = Emu(prs.slide_height).inches
    body_bottom = (theme.metric("body_bottom") if theme else 6.68)
    footer_y = (theme.metric("footer_y") if theme else 6.83)

    for index, slide in enumerate(prs.slides, start=1):
        _check_slide(slide, index, findings, width, height, body_bottom, footer_y,
                     expect_notes, max_words)
    return findings


def _check_slide(slide, number, findings, width, height, body_bottom, footer_y,
                 expect_notes, max_words):
    if expect_notes:
        text = (slide.notes_slide.notes_text_frame.text
                if slide.has_notes_slide else "")
        if not text.strip():
            findings.append(Finding("error", number, "notes",
                                    "no speaker notes; every slide carries a time budget"))

    words = 0
    blocks: list[tuple[str, float, float, float, float]] = []
    for shape in slide.shapes:
        _check_style(shape, number, findings)
        if shape.shape_type == 13:  # MSO_SHAPE_TYPE.PICTURE
            _check_figure(shape, number, findings)
        box = _block_box(shape)
        if box is not None:
            blocks.append(box)
        if shape.has_text_frame:
            for paragraph in shape.text_frame.paragraphs:
                _check_paragraph(paragraph, shape, number, findings)
                words += sum(len(run.text.split()) for run in paragraph.runs)
        _check_geometry(shape, number, findings, width, height, body_bottom, footer_y)

    _check_overlap(blocks, number, findings)
    if words > max_words:
        findings.append(Finding("warn", number, "density",
                                f"{words} words on the slide; >{max_words} is a wall of text"))


def _check_style(shape, number, findings):
    if shape._element.find(qn("p:style")) is not None:
        findings.append(Finding(
            "error", number, "shadow",
            f"'{shape.name}' kept its theme preset style (drop shadow); build via deckkit.layout"))
        return True
    return False


def _check_figure(shape, number, findings):
    """Warn on a picture with no alt text and no credit.

    The one forward-looking check rather than the retrospective kind, and the reason it
    is mechanical: an academic deck that reproduces a figure owes the source a line and
    owes a reader a description, and both live in the same `descr` attribute. `layout.
    picture` always writes it, empty when nothing was given, so silence here means
    genuinely undescribed.
    """
    nv = shape._element.find(qn("p:nvPicPr"))
    descr = ""
    if nv is not None:
        c_nv = nv.find(qn("p:cNvPr"))
        if c_nv is not None:
            descr = c_nv.get("descr", "") or ""
    if not descr.strip():
        findings.append(Finding(
            "warn", number, "figure",
            f"'{shape.name}' carries no alt text or provenance; pass alt= (and "
            f"credit=) to s.picture, or note= to s.figure_beside"))


def _is_filled(shape) -> bool:
    """True when the shape paints an outline or a fill — i.e. it is visible as a box.

    Pictures are excluded: they have no fill or line to inspect, and an image is
    never an accidental overlap of the kind this check exists to catch.
    """
    if shape.shape_type == 13:  # MSO_SHAPE_TYPE.PICTURE
        return False
    try:
        return _paints(shape.fill) or _paints(shape.line.fill)
    except (AttributeError, TypeError):
        return False


def _paints(fill) -> bool:
    try:
        return fill.type == 1  # MSO_FILL.SOLID
    except (AttributeError, TypeError):
        return False


def _block_box(shape):
    """The visual footprint of a content block, or None if it is not one.

    Three exclusions, each for a measured reason:

    * decoration (name prefixed `deco` by `layout.rect`): hairlines and table
      outlines are meant to sit on an edge, not to be collided with.
    * a pure textbox: these declare a generous height and use what they need — a
      bullet rail claims 6in and may occupy 2 — so their box is not a footprint.
      `_check_geometry` measures their *text* instead.
    * a hairline by dimension, which catches one drawn before the marker existed.
    """
    if shape.name.startswith("deco "):
        return None
    if shape.has_text_frame and not _is_filled(shape):
        return None
    if not _is_filled(shape):
        return None
    try:
        left, top = Emu(shape.left).inches, Emu(shape.top).inches
        w, h = Emu(shape.width).inches, Emu(shape.height).inches
    except TypeError:
        return None
    if min(w, h) < 0.03:
        return None
    return (shape.name, left, top, w, h)


def _check_overlap(blocks, number, findings):
    """Flag filled blocks drawn on top of each other.

    This is the check that catches a statement bar landing on the columns above it:
    invisible in the source, obvious on screen, and easy to ship if nothing renders.
    """
    for i, (name_a, ax, ay, aw, ah) in enumerate(blocks):
        for name_b, bx, by, bw, bh in blocks[i + 1:]:
            dx = min(ax + aw, bx + bw) - max(ax, bx)
            dy = min(ay + ah, by + bh) - max(ay, by)
            if dx > OVERLAP_TOLERANCE and dy > OVERLAP_TOLERANCE:
                findings.append(Finding(
                    "error", number, "overlap",
                    f"'{name_a}' and '{name_b}' overlap by {dx:.2f}in x {dy:.2f}in"))


def _check_paragraph(paragraph, shape, number, findings):
    text = "".join(run.text for run in paragraph.runs)
    mark = residue(text)
    if mark:
        findings.append(Finding("error", number, "markup",
                                f"stray '{mark}' would print literally: {text[:60]!r}"))
    if "\n" in text or "\r" in text:
        findings.append(Finding(
            "error", number, "newline",
            f"literal newline inside a run (PowerPoint renders it as a space): {text[:60]!r}"))
    if paragraph.alignment is None and len(paragraph.runs) > 1:
        findings.append(Finding(
            "warn", number, "alignment",
            f"paragraph has no explicit alignment; it inherits the autoshape's centring: {text[:50]!r}"))

    for run in paragraph.runs:
        name = run.font.name
        if name is None:
            findings.append(Finding(
                "warn", number, "font",
                f"run carries no font name and falls back to the viewer's default: {run.text[:40]!r}"))


def _check_geometry(shape, number, findings, width, height, body_bottom, footer_y):
    try:
        left = Emu(shape.left).inches
        top = Emu(shape.top).inches
        w = Emu(shape.width).inches
        h = Emu(shape.height).inches
    except TypeError:
        return

    textbox = shape.has_text_frame and not _is_filled(shape)
    # Decoration is exempt from the footer rules below. A background motif is meant
    # to sit behind the statement bar and the footer; reporting it as a collision
    # would train the reader to ignore the finding. The canvas-bounds check above
    # still applies, because the motif's unrotated box must stay on the slide.
    deco = shape.name.startswith("deco ")
    if not textbox and (left < -0.01 or top < -0.01
                        or left + w > width + 0.01 or top + h > height + 0.01):
        findings.append(Finding(
            "error", number, "bounds",
            f"'{shape.name}' leaves the canvas "
            f"(x {left:.2f}..{left + w:.2f}, y {top:.2f}..{top + h:.2f})"))

    if _is_footer(shape, top, footer_y) or deco:
        return

    if textbox:
        # A textbox declares a generous height and uses what it needs, so its own box
        # is not the footprint — the estimated text extent is.
        needed = _estimate_text_height(shape, w)
        if needed and top + needed > body_bottom + 0.02:
            findings.append(Finding(
                "error", number, "overflow",
                f"text on '{shape.name}' reaches y {top + needed:.2f}, past the body floor "
                f"{body_bottom:.2f} — it collides with the footer"))
        return

    if top + h > body_bottom + 0.02 and h < height - 0.5:
        findings.append(Finding(
            "error", number, "overflow",
            f"'{shape.name}' ends at y {top + h:.2f}, past the body floor {body_bottom:.2f} — "
            f"it collides with the footer"))

    _estimate_text_fit(shape, number, findings, w, h)


def _is_footer(shape, top, footer_y) -> bool:
    return top >= footer_y - 0.05


def _estimate_text_height(shape, w: float) -> float:
    """Approximate the height a text frame's content needs, in inches."""
    frame = shape.text_frame
    left_inset = Emu(frame.margin_left).inches if frame.margin_left is not None else 0.1
    right_inset = Emu(frame.margin_right).inches if frame.margin_right is not None else 0.1
    top_inset = Emu(frame.margin_top).inches if frame.margin_top is not None else 0.05
    bottom_inset = Emu(frame.margin_bottom).inches if frame.margin_bottom is not None else 0.05
    usable_w = max(w - left_inset - right_inset, 0.2)

    needed = top_inset + bottom_inset
    for paragraph in frame.paragraphs:
        runs = paragraph.runs
        if not runs:
            continue
        size = max((run.font.size.pt for run in runs if run.font.size), default=12.0)
        chars = sum(len(run.text) for run in runs)
        per_line = max(usable_w * 72 / (size * CHAR_RATIO), 4)
        lines = max(1, math.ceil(chars / per_line))
        spacing = paragraph.line_spacing or 1.0
        needed += lines * (size * 1.2 * spacing) / 72
        needed += (paragraph.space_before.pt if paragraph.space_before else 0) / 72
        needed += (paragraph.space_after.pt if paragraph.space_after else 0) / 72
    return needed


def _estimate_text_fit(shape, number, findings, w, h):
    """Warn when a filled panel is too short for the text it holds.

    Deliberately approximate: word wrap, kerning and per-glyph widths are not
    modelled. It is tuned to over-report rather than under-report, because a false
    warning costs one glance and a missed one costs a slide.
    """
    if not shape.has_text_frame:
        return
    frame = shape.text_frame
    if not frame.text.strip():
        return

    top_inset = Emu(frame.margin_top).inches if frame.margin_top is not None else 0.05
    bottom_inset = Emu(frame.margin_bottom).inches if frame.margin_bottom is not None else 0.05
    usable_h = max(h - top_inset - bottom_inset, 0.05)
    needed = _estimate_text_height(shape, w) - top_inset - bottom_inset

    if needed > usable_h * 1.12:
        findings.append(Finding(
            "warn", number, "textfit",
            f"'{shape.name}' holds ~{needed:.2f}in of text in {usable_h:.2f}in — "
            f"shorten the text or grow the box"))


def summary(findings: list[Finding]) -> str:
    errors = sum(1 for f in findings if f.severity == "error")
    warns = sum(1 for f in findings if f.severity == "warn")
    if not findings:
        return "clean: 0 errors, 0 warnings"
    return f"{errors} error(s), {warns} warning(s)"
