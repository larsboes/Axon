"""A slide bound to its theme, its header and its footer.

`Deck.open()` returns one of these already carrying the header, the accent rule and
the footer. `.top` is where content may start, derived from how many lines the
title took — a two-line title pushes the rule down, which is what the reference
deck does and what makes a two-line title look deliberate instead of cramped.

The archetypes (`three_columns`, `statement`, `figure_beside`, ...) are the house
layouts. They exist so a content file reads as a sequence of decisions rather than
a sequence of coordinates. When a slide needs something none of them do, the
primitives from `layout` are on the object and take the same arguments without the
slide, which is the intended escape hatch — not a failure.
"""

from __future__ import annotations

from pptx.enum.text import MSO_ANCHOR, PP_ALIGN

from . import layout as L
from .theme import Theme

#: Height of a statement bar, and the gap the layout leaves above it. Exported so a
#: content file can pass `reserve=s.statement_reserve` instead of guessing.
STATEMENT_H = 0.88
STATEMENT_GAP = 0.16


class Slide:
    def __init__(self, shape_slide, theme: Theme, top: float, number, footer_label: str,
                 author: str, notes: str = ""):
        self.shape = shape_slide
        self.theme = theme
        self.top = top
        self.number = number
        self.footer_label = footer_label
        self.author = author
        self._notes = notes
        self._footer_shapes: list = []

    # ── geometry shortcuts ──────────────────────────────────────────────
    @property
    def x(self) -> float:
        return self.theme.metric("margin")

    @property
    def w(self) -> float:
        return self.theme.content_width

    @property
    def bottom(self) -> float:
        return self.theme.metric("body_bottom")

    @property
    def height(self) -> float:
        """Usable vertical space between the header rule and the footer."""
        return self.bottom - self.top

    def columns(self, n: int, gap: float = 0.32) -> list[float]:
        return self.theme.columns(n, gap)

    def span(self, n: int, gap: float = 0.32) -> float:
        return self.theme.span(n, gap)

    # ── primitives, with the slide bound ────────────────────────────────
    def rect(self, *args, **kwargs):
        return L.rect(self.shape, self.theme, *args, **kwargs)

    def panel(self, *args, **kwargs):
        return L.panel(self.shape, self.theme, *args, **kwargs)

    def write(self, *args, **kwargs):
        return L.write(self.shape, self.theme, *args, **kwargs)

    def rule(self, *args, **kwargs):
        return L.rule(self.shape, self.theme, *args, **kwargs)

    def label(self, *args, **kwargs):
        return L.label(self.shape, self.theme, *args, **kwargs)

    def table(self, cols, rows, *, row_h=None, head_h=0.40, reserve=0.0, y=None,
              **kwargs):
        """A table sized to the space it has.

        Passing `reserve` makes `row_h` automatic, which removes the arithmetic that
        otherwise has to agree between the table and whatever sits below it. That
        arithmetic silently disagreeing is how a table ends up under a statement bar.
        """
        y = y if y is not None else self.top
        if row_h is None:
            available = self.bottom - reserve - y - head_h
            row_h = max(0.30, available / max(len(rows), 1))
        return L.table(self.shape, self.theme, self.x, y, cols, rows,
                       row_h=row_h, head_h=head_h, **kwargs)

    def picture(self, *args, **kwargs):
        return L.picture(self.shape, self.theme, *args, **kwargs)

    def kpi(self, *args, **kwargs):
        return L.kpi(self.shape, self.theme, *args, **kwargs)

    def card(self, *args, **kwargs):
        return L.card(self.shape, self.theme, *args, **kwargs)

    def bullets(self, x, y, w, items, *, size=None, gap=12, voice="accent", role="body"):
        """A bullet rail with no panel — for the column beside a figure."""
        theme = self.theme
        size = size if size is not None else theme.size(role)
        return self.write(x, y, w, 6.0, items, size=size, gap=gap,
                          line_spacing=1.04, bullet=True)

    # ── archetypes ──────────────────────────────────────────────────────
    # `reserve` is the vertical space an archetype must leave free at the bottom
    # for whatever is drawn under it — usually a statement bar. Without it the
    # columns run to the body floor and the statement lands on top of them, which
    # `check` reports as an overlap and a reader sees as a broken slide.

    @property
    def statement_reserve(self) -> float:
        """Pass this as `reserve=` when a statement bar follows the content."""
        return STATEMENT_H + STATEMENT_GAP

    def body(self, *, reserve: float = 0.0, top_pad: float = 0.0) -> float:
        """Usable height for content that must leave `reserve` inches below it."""
        return self.height - top_pad - 0.30 - reserve

    def three_columns(self, cards, *, height=None, size=None, gap=0.32, top_pad=0.06,
                      reserve=0.0):
        """`cards` is [(title, voice, [items]), ...] — the default content layout."""
        height = height if height is not None else self.body(reserve=reserve,
                                                             top_pad=top_pad)
        width = self.span(len(cards), gap)
        out = []
        for index, (title, voice, items) in enumerate(cards):
            out.append(self.card(self.x + index * (width + gap), self.top + top_pad,
                                 width, height, title, items, voice=voice, size=size))
        return out

    def two_columns(self, left, right, *, label_height=0.36, height=None, size=None,
                    gap=0.45, bullet=True, reserve=0.0):
        """`left`/`right` are (label, [items]) pairs shown as labelled panels."""
        height = height if height is not None else \
            self.body(reserve=reserve) - label_height + 0.06
        width = (self.w - gap) / 2
        for index, (label, items) in enumerate((left, right)):
            x = self.x + index * (width + gap)
            self.label(x, self.top, width, label)
            self.panel(x, self.top + label_height, width, height, items,
                       size=size, bullet=bullet)
        return height

    def statement(self, text, *, height=None, size=None, y=None, width=None, x=None,
                  voice="accent", fill=None, align=PP_ALIGN.LEFT):
        """A filled bar carrying the one sentence the slide exists to deliver."""
        height = height if height is not None else STATEMENT_H
        fill = fill if fill is not None else self.theme.hex(voice)
        y = y if y is not None else self.bottom - height
        return self.panel(x if x is not None else self.x, y,
                          width if width is not None else self.w, height, text,
                          fill=fill, size=size or self.theme.size("statement"),
                          pad=0.30, anchor=MSO_ANCHOR.MIDDLE, align=align,
                          line_spacing=1.05)

    def figure_beside(self, path, items, *, ratio=0.55, size=None, note=None,
                      reserve=0.0, **picture):
        """Figure left, bullet rail right — the reading order for a result slide."""
        width = self.w * ratio - 0.25
        rail_x = self.x + self.w * ratio + 0.25
        rail_w = self.w - self.w * ratio - 0.25
        box_h = self.body(reserve=reserve) - (0.50 if note else 0.0)
        self.picture(path, self.x, self.top, width, box_h, **picture)
        self.bullets(rail_x, self.top + 0.02, rail_w, items,
                     size=size or self.theme.size("body"))
        if note:
            self.write(self.x, self.bottom - reserve - 0.44, width, 0.44, note,
                       size=self.theme.size("small"), colour=self.theme.hex("accent_mid"),
                       line_spacing=1.0)
        return rail_x, rail_w

    def headline(self, text, *, height=1.85, size=None, fill=None, voice="accent",
                 align=PP_ALIGN.CENTER, reserve=0.0):
        """The oversized pull-quote: a research question, a contribution, a verdict."""
        y = self.top if not reserve else self.top
        return self.statement(text, height=height,
                              size=size or self.theme.size("subtitle"),
                              y=y, voice=voice, fill=fill, align=align)

    def kpi_row(self, cards, *, height=1.42, gap=0.30, reserve=0.0):
        """`cards` is [(value, caption, trail, voice), ...]."""
        width = self.span(len(cards), gap)
        return [self.kpi(self.x + i * (width + gap), self.top, width, height,
                         value, caption, trail=trail, voice=voice)
                for i, (value, caption, trail, voice) in enumerate(cards)]

    # ── notes and footer ────────────────────────────────────────────────
    def notes(self, text: str):
        self._notes = text
        self.shape.notes_slide.notes_text_frame.text = text.strip()
        return self

    def footer(self, note: str | None = None):
        self._footer(note)
        return self

    def _footer(self, note=None):
        """Draw the footer. Idempotent: a second call replaces the first.

        `Deck.open()` draws the footer up front, and a slide that needs a footnote
        calls `footer(note)` afterwards. Without the replace, that call draws a
        *second* badge and a second page number exactly on top of the first — which
        renders as slightly bolder text and is invisible until something measures it.
        """
        for shape in self._footer_shapes:
            shape._element.getparent().remove(shape._element)
        self._footer_shapes = []

        theme = self.theme
        y = theme.metric("footer_y")
        self._footer_shapes.append(L.badge(self.shape, theme, self.x, y, self.author))
        if note:
            L.write(self.shape, theme, self.x + 1.98, y + 0.01, 5.9, 0.5,
                    note.split("\n"), size=theme.size("tiny"),
                    colour=theme.hex("accent_mid"), line_spacing=0.95, gap=1)
        L.write(self.shape, theme, theme.metric("width") - self.x - 2.15, y + 0.06,
                1.55, 0.30, self.footer_label, size=theme.size("footer"),
                colour=theme.hex("accent_mid"), align=PP_ALIGN.RIGHT)
        L.write(self.shape, theme, theme.metric("width") - self.x - 0.42, y + 0.06,
                0.42, 0.30, str(self.number), size=theme.size("footer"),
                colour=theme.hex("accent_mid"), align=PP_ALIGN.RIGHT)

    def __repr__(self) -> str:
        return f"<Slide {self.number} at top={self.top:.2f}>"
