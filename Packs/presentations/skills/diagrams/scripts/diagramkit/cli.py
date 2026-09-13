"""`scripts/diagrams` — three verbs, one per step of the pipeline.

    diagrams theme  <theme.json> [--out <file>]     derive the renderer theme
    diagrams render <mmd-dir> [--theme <file>] [--out <dir>] [--only <name>]
    diagrams doctor                                 what is installed, and what is pinned

`theme` and `render` are separate on purpose. Deriving the theme is a pure
function of the palette and is worth running alone when a colour changed;
rendering is the slow step that shells out to Node. `render` derives the theme
first when one is not supplied, so the common case is one command.

Every failure prints what to do next, because the caller is usually an agent
reading stderr. Nothing here asks a question: an unanswerable one becomes a named
default, printed on the way past.
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

from . import render as R
from . import theme as T
from .errors import DiagramError


def _find_theme(start: Path) -> Path:
    """Walk up from the diagram directory looking for a deck theme."""
    for candidate in (start, *start.parents):
        path = candidate / "theme.json"
        if path.exists():
            return path
    raise DiagramError(
        f"no theme.json above {start}\n"
        f"  Pass --theme <theme.json>. The palette is the source of the diagram "
        f"theme, so the render cannot guess one."
    )


def _default_out(mmd_dir: Path) -> Path:
    """The deck convention: diagrams/ renders into ../Assets/figures/."""
    candidate = mmd_dir.parent / "Assets" / "figures"
    if candidate.is_dir():
        return candidate
    raise DiagramError(
        f"cannot guess an output directory for {mmd_dir}\n"
        f"  Pass --out <dir>. The convention is <deck>/Assets/figures/, which does "
        f"not exist here."
    )


def _theme(args) -> int:
    target = T.write(args.theme, args.out)
    print(f"theme   {target}")
    print(f"        from {Path(args.theme).resolve()}")
    return 0


def _render(args) -> int:
    mmd_dir = Path(args.directory).resolve()
    if not mmd_dir.is_dir():
        raise DiagramError(f"not a directory: {mmd_dir}")
    palette = Path(args.theme).resolve() if args.theme else _find_theme(mmd_dir)
    out_dir = Path(args.out).resolve() if args.out else _default_out(mmd_dir)
    theme_path = Path(args.theme_out).resolve() if args.theme_out \
        else mmd_dir / "mermaid-theme.json"

    T.write(palette, theme_path)
    print(f"theme   {theme_path}")
    print(f"        from {palette}")

    rendered = 0
    for source in R.sources(mmd_dir):
        if args.only and source.stem != args.only:
            continue
        target = out_dir / f"{source.stem}.png"
        width, height = R.render_one(source, target, theme_path)
        print(f"figure  {target.name:34} {width}x{height}  aspect {width / height:.2f}")
        rendered += 1
    if not rendered:
        raise DiagramError(f"nothing matched --only {args.only!r} in {mmd_dir}")
    print(f"\n{rendered} figure(s) in {out_dir}")
    print("        next: place it with s.picture(...) using the aspect above")
    return 0


def _doctor(args) -> int:
    """Report what is installed, because the failure mode is a missing Chromium."""
    print(f"diagrams   mermaid-cli pinned to {R.CLI}")
    print(f"           override with DIAGRAMS_MERMAID_CLI=<version>")

    for tool, hint in (("bunx", "curl -fsSL https://bun.sh/install | bash"),
                       ("node", "brew install node")):
        found = shutil.which(tool)
        print(f"{(tool + ':').ljust(11)}{found or 'MISSING — ' + hint}")

    cache = Path.home() / ".bun" / "install" / "cache" / "@mermaid-js"
    cached = sorted(p.name for p in cache.glob("mermaid-cli@*")) if cache.is_dir() else []
    print(f"cli cache  {', '.join(cached) if cached else 'empty — the first render downloads it'}")

    puppeteer = Path.home() / ".cache" / "puppeteer"
    chromium = list(puppeteer.rglob("chrome-headless-shell")) if puppeteer.is_dir() else []
    print(f"chromium   {puppeteer if chromium else 'not downloaded — the first render fetches it (~150 MB)'}")

    try:
        print(f"palette    {_find_theme(Path.cwd())}")
    except DiagramError as exc:
        print(f"palette    {exc}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="diagrams",
        description="Render Mermaid sources to figures, themed from a deck's palette.",
        epilog=(
            "A diagram is a .mmd file. A figure is the PNG it renders to.\n"
            "The theme is derived, never hand-written: change theme.json and the\n"
            "diagrams follow."
        ),
    )
    sub = parser.add_subparsers(dest="verb", required=True)

    p_theme = sub.add_parser("theme", help="derive the Mermaid theme from a palette")
    p_theme.add_argument("theme", help="the artifact's theme.json")
    p_theme.add_argument("--out", default=None,
                         help="where to write it (default: beside theme.json)")
    p_theme.set_defaults(func=_theme)

    p_render = sub.add_parser("render", help="render every .mmd in a directory")
    p_render.add_argument("directory", help="the directory holding the .mmd sources")
    p_render.add_argument("--theme", default=None,
                          help="the palette theme.json (default: nearest above the sources)")
    p_render.add_argument("--out", default=None,
                          help="output directory (default: <dir>/../Assets/figures)")
    p_render.add_argument("--theme-out", default=None,
                          help="where to write the derived theme (default: <dir>/mermaid-theme.json)")
    p_render.add_argument("--only", default=None,
                          help="render one source, by file stem")
    p_render.set_defaults(func=_render)

    p_doctor = sub.add_parser("doctor", help="report the renderer and the palette")
    p_doctor.set_defaults(func=_doctor)

    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except DiagramError as exc:
        print(f"diagrams: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
