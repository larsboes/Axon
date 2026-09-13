"""Shape primitives, all in inches on the theme's grid.

Every function takes the slide as its first argument and returns the shape, so a
content file can compose freely and then reach into the shape's text frame if it
needs to restyle one paragraph (the archetypes in `slide.py` do exactly that).

Two behaviours here are not obvious and are not optional:

1. `_flat()` removes each autoshape's `<p:style>` element. python-pptx gives every
   inserted shape a theme preset style carrying `effectRef idx="2"` — a drop
   shadow. `shape.shadow.inherit = False` inserts an empty `<a:effectLst/>` in
   `spPr`, and LibreOffice draws the shadow anyway; PowerPoint honours it. Removing
   the style element is what actually makes a deck look designed rather than like
   default Office output.

2. Alignment is always set explicitly. An autoshape's default text frame carries
   `<a:pPr algn="ctr"/>`, so any paragraph that does not set its own alignment is
   silently centred — which is wrong for every panel in this system and is
   invisible until you render.
"""

from __future__ import annotations

from pathlib import Path

from pptx.enum.shapes import MSO_SHAPE
from pptx.enum.text import MSO_ANCHOR, PP_ALIGN
from pptx.oxml.ns import qn
from pptx.util import Inches, Pt

from .errors import DeckError
from . import markup
from .markup import write_runs
from .theme import Theme

#: String forms accepted anywhere `align` or `anchor` is taken, so a content file
#: does not have to import python-pptx to say "centre this".
ALIGN = {"left": PP_ALIGN.LEFT, "center": PP_ALIGN.CENTER, "right": PP_ALIGN.RIGHT}
ANCHOR = {"top": MSO_ANCHOR.TOP, "middle": MSO_ANCHOR.MIDDLE,
          "bottom": MSO_ANCHOR.BOTTOM}


def as_align(value):
    return ALIGN.get(value, value) if isinstance(value, str) else value


def as_anchor(value):
    return ANCHOR.get(value, value) if isinstance(value, str) else value

def _flat(shape):
    """Strip the theme preset style; keep the shape's explicit fill and line."""
    element = shape._element
    style = element.find(qn("p:style"))
    if style is not None:
        element.remove(style)
    shape.shadow.inherit = False
    return shape

def textbox(slide, x, y, w, h, anchor=MSO_ANCHOR.TOP):
    box = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    frame = box.text_frame
    frame.word_wrap = True
    frame.margin_left = frame.margin_right = 0
    frame.margin_top = frame.margin_bottom = 0
    frame.vertical_anchor = as_anchor(anchor)
    return frame

def paragraph(frame, first=False, before=0.0, after=0.0, line=None, align=PP_ALIGN.LEFT):
    p = frame.paragraphs[0] if first else frame.add_paragraph()
    p.space_before = Pt(before)
    p.space_after = Pt(after)
    if line is not None:
        p.line_spacing = line
    p.alignment = as_align(align)
    return p

def write(slide, theme: Theme, x, y, w, h, text, *, size=None, role="body",
          colour=None, fill=None, line_spacing=1.0, gap=7, bold=False,
          italic=False, align=PP_ALIGN.LEFT, anchor=MSO_ANCHOR.TOP, bullet=False):
    """Free-standing text with no panel. `text` may be a list of paragraphs."""
    items = [text] if isinstance(text, str) else list(text)
    if not items:
        return None
    size = size if size is not None else theme.size(role)
    colour = colour if colour is not None else theme.ink_on(fill)
    accent = theme.emphasis_on(fill)
    frame = textbox(slide, x, y, w, h, anchor=anchor)
    for index, item in enumerate(items):
        p = paragraph(frame, first=(index == 0), before=0 if index == 0 else gap,
                      line=line_spacing, align=align)
        write_runs(p, item, size=size, colour=_rgb(colour), font=theme.type["font"],
                   bold=bold, italic=italic, accent=_rgb(accent), bullet=bullet,
                   bullet_colour=_rgb(accent))
    return frame

def _rgb(value):
    from pptx.dml.color import RGBColor

    return value if isinstance(value, RGBColor) else RGBColor.from_string(value)

def rect(slide, theme: Theme, x, y, w, h, *, fill=None, line=None, line_width=1.0,
         shape=MSO_SHAPE.RECTANGLE, adjust=None, deco=False, radius=None, alpha=None):
    """A rectangle. `deco=True` marks it as decoration, not a content block.

    The marker is a name prefix, and `check` skips those shapes when it looks for
    blocks that collide. Hairlines and table outlines are meant to sit on an edge,
    so counting them as collision candidates would bury the real finding.

    `radius` rounds the corners: pass inches and the shape is built as a rounded
    rectangle whose adjustment is derived from the shorter side (PowerPoint's
    `adj` is a fraction, not a length, so the same radius looks different on a
    statement bar and on a near-square panel unless it is converted here).

    `alpha` sets the fill's opacity, 0..1. python-pptx exposes no opacity API, so
    the `<a:alpha>` element is written directly; a low-alpha fill is how a motif
    stays behind the content instead of becoming a block.
    """
    if radius and radius > 0 and shape == MSO_SHAPE.RECTANGLE and adjust is None:
        shape = MSO_SHAPE.ROUNDED_RECTANGLE
        adjust = max(0.0, min(0.5, radius / max(min(w, h), 0.01)))
    sh = slide.shapes.add_shape(shape, Inches(x), Inches(y), Inches(w), Inches(h))
    _flat(sh)
    if deco:
        sh.name = f"deco {sh.name}"
    if fill:
        sh.fill.solid()
        sh.fill.fore_color.rgb = _rgb(fill)
        if alpha is not None:
            _fill_alpha(sh, alpha)
    else:
        sh.fill.background()
    if line:
        sh.line.color.rgb = _rgb(line)
        sh.line.width = Pt(line_width)
    else:
        sh.line.fill.background()
    if adjust is not None:
        sh.adjustments[0] = adjust
    sh.text_frame.word_wrap = True
    return sh


def _fill_alpha(shape, alpha: float):
    """Write `<a:alpha>` into a solid fill. 0 is transparent, 1 is opaque."""
    sp_pr = shape._element.spPr
    solid = sp_pr.find(qn("a:solidFill"))
    if solid is None:
        return
    colour = solid.find(qn("a:srgbClr"))
    if colour is None:
        return
    value = max(0, min(100000, int(round(alpha * 100000))))
    element = colour.makeelement(qn("a:alpha"), {"val": str(value)})
    colour.append(element)
    return element


def flourish(slide, theme: Theme, *, rotation=-32.0, opacity=1.0):
    """The optional background motif: one broad diagonal band and its gradations.

    Geometry is fixed in inches on the 16:9 grid and lives in the lower right, so
    it reads as a corner of the canvas rather than as a block of content. Every
    part is `deco=True`, which keeps `check` from reporting the motif where the
    motif is meant to sit: behind the statement bar and the footer.

    The bars stay inside the canvas unrotated. PowerPoint rotates about the shape
    centre, so the rendered band bleeds a little past the corner, which is the
    point; `check` measures the unrotated box and therefore passes.

    `opacity` scales every alpha, so a deck can make the motif quieter without
    touching the geometry.
    """
    accent = theme.hex("accent")
    # (centre x, centre y, length, thickness, alpha) — the band first, then the
    # progressively shorter rules that step away from it.
    parts = [
        (10.60, 5.55, 4.60, 0.34, 0.07),
        (10.78, 5.79, 3.85, 0.075, 0.10),
        (10.96, 6.03, 3.05, 0.06, 0.13),
        (11.13, 6.27, 2.25, 0.05, 0.16),
        (11.31, 6.51, 1.45, 0.04, 0.20),
    ]
    for cx, cy, length, thickness, alpha in parts:
        bar = rect(slide, theme, cx - length / 2, cy - thickness / 2, length,
                   thickness, fill=accent, alpha=alpha * opacity, deco=True,
                   shape=MSO_SHAPE.ROUNDED_RECTANGLE, adjust=0.5)
        bar.rotation = rotation

def _auto_name(shape, items):
    """Name a block after its first line of text.

    Purely so `check` can say which block collided. `'Columns' and 'Konsequenz'
    overlap` is actionable; `'Rectangle 9' and 'Rectangle 16'` is a scavenger hunt.
    """
    first = items[0] if isinstance(items, (list, tuple)) and items else str(items)
    text = "".join(content for _, content in markup.segments(str(first)))
    text = " ".join(text.split())
    if text:
        shape.name = text[:40]
    return shape


def panel(slide, theme: Theme, x, y, w, h, items, *, voice=None, fill=None,
          outline=None, outline_width=1.25, size=None, role="body",
          colour=None, pad=0.20, bullet=False, gap=6, line_spacing=1.02,
          anchor=MSO_ANCHOR.TOP, bold=False, align=PP_ALIGN.LEFT, mono=False):
    """A bordered or filled block of paragraphs.

    `voice` selects a palette family ('accent', 'secondary', 'caution') and drives
    both the outline colour and the emphasis colour. Passing `fill` explicitly
    overrides it, which is how a filled statement bar is made from the same call.
    """
    if isinstance(items, str):
        items = [items]
    if fill is None and voice is not None:
        fill = None
    line_colour = outline if outline is not None else (
        theme.hex(voice) if voice else theme.hex("hairline"))
    sh = rect(slide, theme, x, y, w, h,
              fill=fill, line=line_colour if fill is None else None,
              line_width=outline_width, radius=theme.corners)
    frame = sh.text_frame
    frame.margin_left = frame.margin_right = Inches(pad)
    # Vertical padding does not scale with horizontal padding: a statement bar is
    # only 0.88in tall, and 0.30in of top and bottom inset leaves it no room for a
    # second line. Horizontal breathing and vertical breathing are different needs.
    v_pad = min(pad * 0.8, 0.12)
    frame.margin_top = frame.margin_bottom = Inches(v_pad)
    frame.vertical_anchor = as_anchor(anchor)

    size = size if size is not None else theme.size(role)
    colour = colour if colour is not None else theme.ink_on(fill)
    accent = theme.emphasis_on(fill)
    for index, item in enumerate(items):
        p = paragraph(frame, first=(index == 0), before=0 if index == 0 else gap,
                      line=line_spacing, align=align)
        write_runs(p, item, size=size, colour=_rgb(colour), font=theme.type["font"],
                   bold=bold, accent=_rgb(accent), bullet=bullet,
                   bullet_colour=_rgb(accent))
    return _auto_name(sh, items)

def rule(slide, theme: Theme, x, y, w, *, colour=None, thickness=None):
    """A horizontal hairline. The header rule and the section rule are this."""
    return rect(slide, theme, x, y, w,
                thickness if thickness is not None else theme.metric("accent_rule"),
                fill=colour or theme.hex("accent"), deco=True)

def label(slide, theme: Theme, x, y, w, text, *, size=None, colour=None):
    return write(slide, theme, x, y, w, 0.30, text,
                 size=size if size is not None else theme.size("body") + 0.5,
                 colour=colour or theme.hex("ink"), bold=True)

def table(slide, theme: Theme, x, y, cols, rows, *, row_h=0.38, head_h=0.40,
          size=None, voice="accent", zebra=True, aligns=None, bold_cols=(),
          first_col_bold=False, highlight_rows=(), highlight_fill=None):
    """A table drawn from rectangles.

    python-pptx's own table object carries PowerPoint's theme styling, which fights
    the deck's palette and cannot be re-coloured without XML surgery. Composing
    rectangles costs a few lines and gets the deck's own type and rules.

    `cols` is [(header, width_inches), ...]; `aligns` accepts 'left'/'center'/'right'.
    `highlight_rows` fills those rows with the voice tint and bolds them, which is
    how a matrix names the row the argument turns on.
    """
    size = size if size is not None else theme.size("small")
    align_map = {"left": PP_ALIGN.LEFT, "center": PP_ALIGN.CENTER,
                 "right": PP_ALIGN.RIGHT}
    aligns = [align_map[a] for a in (aligns or ["left"] * len(cols))]
    head_fill = theme.hex(voice)
    stripes = (theme.hex("white"), theme.hex("muted_neutral"))
    highlight_rows = set(highlight_rows)
    highlight_fill = highlight_fill or theme.tint_for(voice)

    cx = x
    for index, (header, width) in enumerate(cols):
        sh = rect(slide, theme, cx, y, width, head_h, fill=head_fill)
        _cell_text(sh, theme, header, size, theme.hex("white"), aligns[index],
                   head_h, pad=0.11, accent=theme.hex("emphasis_on_dark"), bold=True)
        cx += width

    ry = y + head_h
    for row_index, row in enumerate(rows):
        marked = row_index in highlight_rows
        fill = highlight_fill if marked else (
            stripes[row_index % 2] if zebra else stripes[0])
        cx = x
        for col_index, cell in enumerate(row):
            width = cols[col_index][1]
            sh = rect(slide, theme, cx, ry, width, row_h, fill=fill)
            _cell_text(sh, theme, cell, size, theme.hex("ink"), aligns[col_index],
                       row_h, pad=0.11, accent=theme.hex("accent"),
                       bold=marked or (col_index in bold_cols)
                            or (first_col_bold and col_index == 0))
            cx += width
        rule(slide, theme, x, ry, sum(w for _, w in cols),
             colour=theme.hex("hairline"), thickness=0.008)
        ry += row_h

    rect(slide, theme, x, y, sum(w for _, w in cols), ry - y,
         line=theme.hex("hairline"), line_width=0.75, deco=True)
    return ry

def _cell_text(shape, theme, text, size, colour, align, height, *, pad, accent, bold):
    frame = shape.text_frame
    frame.margin_left = frame.margin_right = Inches(pad)
    frame.margin_top = frame.margin_bottom = 0
    frame.vertical_anchor = MSO_ANCHOR.MIDDLE
    p = paragraph(frame, first=True, align=align, line=1.0)
    write_runs(p, text, size=size, colour=_rgb(colour), font=theme.type["font"],
               bold=bold, accent=_rgb(accent))

def picture(slide, theme: Theme, path, x, y, w, h, *, align="center",
            valign="middle", frame=False, alt=None, credit=None):
    """Fit an image inside a box, preserving aspect ratio. Never distorts.

    `alt` is the accessibility description and `credit` the provenance; both are
    written to the shape's `descr`, so PowerPoint reads the one and a reader who opens
    the file finds the other. A figure with neither is a `check` warning: an academic
    deck that reproduces a figure owes its source a line, and a deck that is read on a
    projector or by a screen reader owes it a description.
    """
    path = Path(path)
    if not path.exists():
        raise DeckError(
            f"figure not found: {path}\n"
            f"  Figures are resolved relative to the deck file that names them."
        )
    from PIL import Image

    with Image.open(path) as image:
        iw, ih = image.size
    aspect = iw / ih
    box_w, box_h = w, h
    if box_w / box_h > aspect:
        box_w = box_h * aspect
    else:
        box_h = box_w / aspect
    px = {"left": x, "center": x + (w - box_w) / 2, "right": x + w - box_w}[align]
    py = {"top": y, "middle": y + (h - box_h) / 2, "bottom": y + h - box_h}[valign]
    pic = slide.shapes.add_picture(str(path), Inches(px), Inches(py),
                                   Inches(box_w), Inches(box_h))
    _flat(pic)
    # Always written, empty when nothing was given: python-pptx may default `descr` to
    # the filename, and `check` must be able to tell "described" from "not described".
    descr = " | ".join(part for part in (alt, credit) if part)
    nv = pic._element.find(qn("p:nvPicPr"))
    if nv is not None:
        c_nv = nv.find(qn("p:cNvPr"))
        if c_nv is not None:
            c_nv.set("descr", descr)
    if frame:
        pic.line.color.rgb = _rgb(theme.hex("hairline"))
        pic.line.width = Pt(0.75)
    return pic

def kpi(slide, theme: Theme, x, y, w, h, value, caption, *, trail=None, note=None,
        voice="accent", fill=None, size=None):
    """A number card: one value, its label, and optionally a path and a note.

    The value is restyled after the panel is built, because a panel's paragraphs
    share one size and this block deliberately does not.
    """
    fill = fill if fill is not None else theme.tint_for(voice)
    items = [value, caption]
    if trail:
        items.append(trail)
    if note:
        items.append(note)
    sh = panel(slide, theme, x, y, w, h, items, fill=fill, size=theme.size("small"),
               gap=2, pad=0.18, anchor=MSO_ANCHOR.MIDDLE, align=PP_ALIGN.CENTER)
    sh.name = f"kpi {value}"[:40]
    paragraphs = sh.text_frame.paragraphs
    _restyle(paragraphs[0], size=size or theme.size("kpi"), bold=True,
             colour=theme.hex(voice))
    _restyle(paragraphs[1], size=theme.size("body"), bold=True)
    if trail:
        _restyle(paragraphs[2], size=theme.size("small"), colour=theme.hex("accent_mid"))
    if note:
        _restyle(paragraphs[3], size=theme.size("tiny"), colour=theme.hex("accent_mid"))
    return sh

def card(slide, theme: Theme, x, y, w, h, title, items, *, voice="accent",
         size=None, gap=11, fill=None, outline=None):
    """An accent header rule, a coloured title, and a bordered body under it.

    The house shape for a column of content: three of these across the content
    width is the most-used layout in the system.
    """
    rule(slide, theme, x, y, w, colour=theme.hex(voice), thickness=theme.metric("header_rule"))
    write(slide, theme, x, y + 0.18, w, 0.32, title,
          size=size if size is not None else theme.size("body") + 1.5,
          colour=theme.hex(voice), bold=True)
    return panel(slide, theme, x, y + 0.56, w, h - 0.56, items, voice=voice,
                 outline=outline or theme.hex("hairline"), outline_width=1.0,
                 size=size, gap=gap, bullet=True, pad=0.22, fill=fill)

def badge(slide, theme: Theme, x, y, text, *, height=0.36, min_width=1.20,
          pad=0.30, size=None):
    """The name badge, sized to its text and never wrapping.

    A fixed-width badge is the wrong default: text longer than the box wraps to a
    second line and spills out of the rounded rectangle, which reads as a rendering
    fault rather than as a long name. Width is computed from the text, and wrapping
    is off so a miscalculation overflows sideways by a hair instead of visibly
    downwards.
    """
    size = size if size is not None else theme.size("body")
    width = max(min_width, len(text) * size * 0.62 / 72 + pad)
    box = rect(slide, theme, x, y, width, height, fill=theme.hex("accent"),
               shape=MSO_SHAPE.ROUNDED_RECTANGLE, adjust=0.26)
    frame = box.text_frame
    frame.word_wrap = False
    frame.margin_left = frame.margin_right = 0
    frame.margin_top = frame.margin_bottom = 0
    frame.vertical_anchor = MSO_ANCHOR.MIDDLE
    p = paragraph(frame, first=True, align=PP_ALIGN.CENTER)
    write_runs(p, text, size=size, colour=theme.rgb("white"), font=theme.type["font"])
    return box


def _restyle(paragraph, *, size=None, bold=None, colour=None):
    from pptx.dml.color import RGBColor

    for run in paragraph.runs:
        if size is not None:
            run.font.size = Pt(size)
        if bold is not None:
            run.font.bold = bold
        if colour is not None:
            run.font.color.rgb = colour if isinstance(colour, RGBColor) \
                else RGBColor.from_string(colour)
    return paragraph
