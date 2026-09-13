"""Palette to renderer theme.

One palette is the source. A deck already carries one in `theme.json` — the
role-based palette deckkit uses — and a diagram that is embedded in that deck
must not be able to drift onto different colours. So the Mermaid theme is
*derived*, never hand-written: change `accent` in `theme.json` and the diagrams
change with the slides on the next render.

The derivation is a name mapping, not a colour computation. Everything Mermaid
needs exists in a deckkit palette already, with one exception: the node fill. A
deckkit palette is built for panels on white, and its tints are pale; a diagram
node wants a surface with enough body to read as a shape at 3 metres. That one
value is declared in the theme's optional `diagram` block:

    "diagram": { "node_fill": "A4C5C4", "font_size": 18, "wrapping_width": 520 }

Everything else in that block is a layout override with a default here, so a
theme states a value only when it wants a different one. See
`references/mermaid.md` for the full table and the reasoning behind each default.
"""

from __future__ import annotations

import json
from pathlib import Path

from .errors import DiagramError

#: The palette roles a deckkit theme must define for the mapping to work. A
#: subset of deckkit's required roles: a diagram needs no caution tint, but it
#: does need a paper tone to sit a cluster on.
NEEDED = ("ink", "white", "paper", "accent", "accent_light", "secondary",
          "secondary_light", "caution")

#: Mermaid themeVariables, and where each value comes from. `diagram.<key>` is
#: read from the theme's optional block; a bare name is a palette role. Mermaid
#: has no variable for stroke width or for the wrapping width, so those travel
#: as `themeCSS` and as `flowchart` config.
VARIABLES = {
    "primaryColor": "diagram.node_fill",
    "primaryTextColor": "ink",
    "primaryBorderColor": "accent",
    "secondaryColor": "secondary_light",
    "secondaryTextColor": "ink",
    "secondaryBorderColor": "secondary",
    "tertiaryColor": "paper",
    "tertiaryTextColor": "ink",
    "tertiaryBorderColor": "diagram.cluster_edge",
    "lineColor": "accent",
    "clusterBkg": "paper",
    "clusterBorder": "diagram.cluster_edge",
    "edgeLabelBackground": "white",
    "background": "white",
    "mainBkg": "diagram.node_fill",
    "nodeBorder": "accent",
    "textColor": "ink",
}

#: What a `diagram.*` slot falls back to when the theme does not declare it. A
#: theme without the block still renders: a washed-out node is a look to fix,
#: not a broken build.
DIAGRAM_FALLBACK = {
    "node_fill": "accent_light",
    "cluster_edge": "caution",
}

#: Layout defaults, overridable from the theme's `diagram` block.
LAYOUT = {
    "font_size": 18,
    "curve": "linear",
    "node_spacing": 34,
    "rank_spacing": 38,
    #: The one default that is a finding rather than a preference. Mermaid wraps
    #: a node label at ~200px by default, which turns a three-column fan-out into
    #: three narrow towers and a nearly square diagram (measured: 3136x3032, an
    #: aspect of 1.03 that will not sit on a 16:9 slide). At 520 the same tree
    #: renders 3136x1348, aspect 2.33.
    "wrapping_width": 520,
    "stroke": "1.5px",
}


def load_palette(path: str | Path) -> dict:
    path = Path(path)
    if not path.exists():
        raise DiagramError(
            f"palette not found: {path}\n"
            f"  Pass --theme <theme.json>, or put the theme.json beside the deck."
        )
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise DiagramError(f"palette {path} is not valid JSON: {exc}") from exc
    palette = raw.get("palette")
    if not isinstance(palette, dict):
        raise DiagramError(
            f"{path} has no `palette` object.\n"
            f"  This verb reads a deckkit theme. For a paper that keeps a "
            f"visual-palette.json instead, map it to the same role names first."
        )
    missing = [role for role in NEEDED if role not in palette]
    if missing:
        raise DiagramError(
            f"{path} is missing palette roles: {', '.join(missing)}\n"
            f"  A diagram theme is derived from those roles; see references/mermaid.md."
        )
    return raw


def _hex(value: str) -> str:
    """Mermaid wants `#` on a colour literal; a deckkit palette stores it bare.

    Without the prefix the renderer fails inside the browser with an unhelpful
    puppeteer stack, so the normalisation belongs here rather than in the palette.
    """
    if len(value) == 6 and not value.startswith("#"):
        try:
            int(value, 16)
        except ValueError:
            return value
        return f"#{value}"
    return value


def build(raw: dict) -> dict:
    """The Mermaid config, from a loaded theme."""
    palette = raw["palette"]
    diagram = dict(LAYOUT)
    diagram.update(raw.get("diagram") or {})
    type_scale = raw.get("type") or {}
    font = type_scale.get("font") or "Arial"

    def value(source: str) -> str:
        if source.startswith("diagram."):
            key = source.split(".", 1)[1]
            fallback = palette[DIAGRAM_FALLBACK.get(key, "accent_light")]
            return _hex(str(diagram.get(key) or fallback))
        return _hex(str(palette[source]))

    variables = {name: value(source) for name, source in VARIABLES.items()}
    variables["fontSize"] = f"{diagram['font_size']}px"
    variables["fontFamily"] = f"{font}, sans-serif"

    stroke = diagram["stroke"]
    theme_css = " ".join([
        f".node rect, .node polygon, .node circle, .node path {{ stroke-width: {stroke}; }}",
        f".flowchart-link, .edgePath .path {{ stroke-width: {stroke}; }}",
        f".marker {{ stroke-width: {stroke}; }}",
        f".edgeLabel {{ background-color: {_hex(palette['white'])}; }}",
    ])

    return {
        "theme": "base",
        "themeVariables": variables,
        "themeCSS": theme_css,
        "flowchart": {
            "curve": diagram["curve"],
            "htmlLabels": True,
            "nodeSpacing": diagram["node_spacing"],
            "rankSpacing": diagram["rank_spacing"],
            "wrappingWidth": diagram["wrapping_width"],
        },
    }


def write(theme_path: str | Path, out: str | Path | None = None) -> Path:
    raw = load_palette(theme_path)
    config = build(raw)
    target = Path(out) if out else Path(theme_path).parent / "mermaid-theme.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    return target
