"""Palette, type scale and grid — the three things a deck must not improvise.

A theme is a JSON file. See `assets/themes/` for worked examples and
`references/theming.md` for deriving one from an artifact that already exists
(usually a paper's figures, which is where the colours should come from, since
those figures will sit inside the slides).

The palette is *role-based*, not named by colour. A deck says "this is the
accent" or "this is the caution voice"; it never says teal. That is what lets a
deck be re-themed by swapping one file, and it is why `Theme.emphasis_on()` can
pick the right emphasis colour for a surface without the content knowing which
surface it is.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

from pptx.dml.color import RGBColor

from .errors import DeckError

#: Every palette role a theme must define. `light` variants are panel fills;
#: the base role is the text/border/action colour and a filled surface.
REQUIRED_ROLES = (
    "ink",
    "white",
    "accent",
    "accent_light",
    "secondary",
    "secondary_light",
    "caution",
    "caution_light",
    "emphasis_on_dark",
)

OPTIONAL_ROLES = {
    "accent_mid": "accent",          # de-emphasised accent: muted labels, dividers
    "muted_neutral": "accent_light",  # zebra striping, inert panels
    "hairline": "accent_light",       # table rules and thin dividers
}

#: The type scale. Few sizes on purpose: a deck that uses seven sizes reads as
#: seven decks. Ratios here are ~1.3 between steps.
DEFAULT_TYPE = {
    "font": "Arial",
    "section": 38.0,     # section divider headline
    "title": 24.0,       # slide assertion
    "kpi": 31.0,         # the one big number in a card
    "subtitle": 16.5,    # the section rail under a slide title
    "statement": 15.5,   # a filled statement bar
    "body": 13.0,        # panel text
    "small": 11.0,       # captions, table cells at density
    "tiny": 9.5,         # footnote next to the name badge
    "footer": 10.5,      # footer right
}

#: The grid. Inches on a 16:9 canvas.
DEFAULT_GRID = {
    "width": 13.333,
    "height": 7.5,
    "margin": 0.72,
    "title_y": 0.50,
    "content_y": 0.92,   # where content starts on a slide that carries no title
    "line_h": 0.50,      # per title line
    "sub_h": 0.36,       # subtitle block
    "gap": 0.12,         # vertical breathing under the header rule
    "footer_y": 6.83,
    "body_bottom": 6.68,
    "accent_rule": 0.022,
    "header_rule": 0.075,  # the short accent rule above a card title
    #: The full-width rule under a slide heading. 0 draws none. A deck that carries
    #: its assertion in a claim bar instead of a title wants no heading rule either,
    #: because there is no heading for it to belong to.
    "title_rule": 0.0,
    #: Corner radius of every panel, in inches. 0 = square corners.
    "corner_radius": 0.09,
    #: 1 draws the optional background motif (`layout.flourish`). Off by default:
    #: a motif is a per-deck decision, not a house rule.
    "flourish": 0.0,
}


@dataclass(frozen=True)
class Theme:
    name: str
    palette: dict[str, str]
    type: dict[str, float] = field(default_factory=lambda: dict(DEFAULT_TYPE))
    grid: dict[str, float] = field(default_factory=lambda: dict(DEFAULT_GRID))
    provenance: str = ""

    # ── construction ────────────────────────────────────────────────────
    @classmethod
    def load(cls, path: str | Path) -> "Theme":
        path = Path(path)
        if not path.exists():
            raise DeckError(
                f"theme not found: {path}\n"
                f"  Run `scripts/deck themes` to list the built-in themes, or pass a theme JSON."
            )
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            raise DeckError(f"theme {path} is not valid JSON: {exc}") from exc
        return cls.from_dict(raw, source=path)

    @classmethod
    def from_dict(cls, raw: dict, source: Path | None = None) -> "Theme":
        where = f" ({source})" if source else ""
        palette = dict(raw.get("palette") or {})
        missing = [role for role in REQUIRED_ROLES if role not in palette]
        if missing:
            raise DeckError(
                f"theme{where} is missing palette roles: {', '.join(missing)}\n"
                f"  Required: {', '.join(REQUIRED_ROLES)}\n"
                f"  Optional: {', '.join(OPTIONAL_ROLES)}"
            )
        for role, fallback in OPTIONAL_ROLES.items():
            palette.setdefault(role, palette[fallback])

        bad = [role for role, value in palette.items() if not _is_hex(value)]
        if bad:
            raise DeckError(
                f"theme{where} has non-hex colours: {', '.join(bad)}\n"
                f"  Use 6-digit hex without '#', e.g. \"4A6A69\"."
            )

        type_scale = {**DEFAULT_TYPE, **(raw.get("type") or {})}
        grid = {**DEFAULT_GRID, **(raw.get("grid") or {})}
        return cls(
            name=raw.get("name") or (source.stem if source else "unnamed"),
            palette=palette,
            type=type_scale,
            grid=grid,
            provenance=raw.get("provenance", ""),
        )

    # ── palette ─────────────────────────────────────────────────────────
    def rgb(self, role: str) -> RGBColor:
        try:
            return RGBColor.from_string(self.palette[role])
        except KeyError as exc:
            raise DeckError(
                f"theme '{self.name}' has no palette role '{role}'. "
                f"Available: {', '.join(sorted(self.palette))}"
            ) from exc

    def hex(self, role: str) -> str:
        return self.palette[role]

    def fill_for(self, voice: str | None) -> str | None:
        """The panel fill for a voice: None (plain), 'accent', 'secondary', 'caution'."""
        if voice is None:
            return None
        return self.palette[voice]

    def tint_for(self, voice: str | None) -> str:
        """The light panel tint for a voice."""
        if voice is None:
            return self.palette["white"]
        key = f"{voice}_light"
        if key not in self.palette:
            raise DeckError(
                f"theme '{self.name}' has no light tint for voice '{voice}' (wanted '{key}')."
            )
        return self.palette[key]

    def emphasis_on(self, fill: str | None) -> str:
        """The `~accent~` colour that reads on `fill`.

        On a filled surface, the accent is invisible against itself, so emphasis
        switches to the on-dark colour. This is the single rule that keeps the
        markup portable between a white panel and a filled bar.
        """
        if fill is None or fill in (self.palette["white"], self.palette["muted_neutral"]):
            return self.palette["accent"]
        return self.palette["emphasis_on_dark"]

    def ink_on(self, fill: str | None) -> str:
        """Text colour for a panel fill."""
        if fill is None or fill in (self.palette["white"], self.palette["muted_neutral"]):
            return self.palette["ink"]
        # Every tint role ends in _light and every filled role does not, but rather
        # than parse the name, compare against the tint set the theme actually has.
        tints = {self.palette[f"{v}_light"] for v in ("accent", "secondary", "caution")
                 if f"{v}_light" in self.palette}
        return self.palette["ink"] if fill in tints else self.palette["white"]

    def muted_on(self, fill: str | None) -> str:
        """De-emphasised text colour for a panel fill."""
        return self.palette["accent_mid"] if self.ink_on(fill) == self.palette["ink"] \
            else self.palette["accent_light"]

    # ── type and grid ───────────────────────────────────────────────────
    def size(self, role: str) -> float:
        return self.type[role]

    def metric(self, key: str) -> float:
        return self.grid[key]

    @property
    def corners(self) -> float:
        """Panel corner radius in inches; 0 when the theme wants square corners."""
        return self.grid.get("corner_radius", 0.0)

    @property
    def content_width(self) -> float:
        return self.grid["width"] - 2 * self.grid["margin"]

    def span(self, n: int, gap: float = 0.32) -> float:
        """Column width for `n` equal columns across the content width."""
        return (self.content_width - (n - 1) * gap) / n

    def columns(self, n: int, gap: float = 0.32) -> list[float]:
        """Left x of each of `n` equal columns."""
        width = self.span(n, gap)
        return [self.grid["margin"] + i * (width + gap) for i in range(n)]

    def __str__(self) -> str:
        return f"<Theme {self.name}: {len(self.palette)} roles, {self.type['font']}>"


def _is_hex(value: str) -> bool:
    if not isinstance(value, str) or len(value) != 6:
        return False
    try:
        int(value, 16)
    except ValueError:
        return False
    return True
