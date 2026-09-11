"""`scripts/deck` — one CLI with the five verbs a deck needs.

    deck themes                 list built-in themes
    deck init <name> [--dir .]  scaffold a deck module from the template
    deck build <deck.py>        run the module, save the .pptx
    deck check <deck.py>        structural validation of the .pptx (exit = errors)
    deck render <deck.py>       .pptx -> .pdf -> PNGs + contact sheets

Every failure prints what to do next, because the caller is usually an agent
reading stderr. Nothing here asks a question: an unanswerable one becomes a named
default, printed on the way past.
"""

from __future__ import annotations

import argparse
import runpy
import shutil
import sys
from pathlib import Path

from .check import check, summary
from .deck import Deck
from .errors import DeckError
from .render import describe, render
from .theme import Theme

# cli.py lives at <skill>/lib/deckkit/cli.py, so the skill root is three levels up.
# resolve() first: the Pack deployer symlinks the skill into a harness, and the
# assets have to come from the real checkout, not from the link's directory.
SKILL_ROOT = Path(__file__).resolve().parents[2]
THEMES_DIR = SKILL_ROOT / "assets" / "themes"
TEMPLATE = SKILL_ROOT / "assets" / "deck.template.py"


def builtin_themes() -> list[Path]:
    return sorted(THEMES_DIR.glob("*.json"))


def _load_deck(deck_file: Path) -> Deck:
    deck_file = deck_file.resolve()
    if not deck_file.exists():
        raise DeckError(
            f"deck file not found: {deck_file}\n"
            f"  Scaffold one with `scripts/deck init my-deck`."
        )
    if deck_file.suffix != ".py":
        raise DeckError(
            f"{deck_file.name} is not a Python module.\n"
            f"  A deck is a .py file that builds a Deck. See assets/deck.template.py."
        )
    namespace = runpy.run_path(str(deck_file))
    deck = namespace.get("deck")
    if not isinstance(deck, Deck):
        candidates = [k for k, v in namespace.items() if isinstance(v, Deck)]
        if candidates:
            deck = namespace[candidates[0]]
        else:
            raise DeckError(
                f"{deck_file.name} defines no Deck.\n"
                f"  It must assign one to a module-level name, conventionally `deck`."
            )
    if deck.output and not deck.output.is_absolute():
        deck.output = deck_file.parent / deck.output
    return deck


def _build(deck_file: Path, out: Path | None) -> Path:
    deck = _load_deck(deck_file)
    target = deck.save(out)
    print(f"built   {target}")
    print(f"        {len(deck.slides)} slides, theme={deck.theme.name}")
    print("        next: `scripts/deck check` then `scripts/deck render`")
    return target


def _check(deck_file: Path, out: Path | None) -> int:
    deck = _load_deck(deck_file)
    target = out or (deck.output / f"{deck.name}.pptx")
    findings = check(target, deck.theme, max_words=deck.max_words)
    for finding in findings:
        print(finding)
    print(f"\n{summary(findings)}  ({target.name})")
    if not findings:
        print("        next: `scripts/deck render` and look at the contact sheet.")
    return sum(1 for f in findings if f.severity == "error")


def _render(deck_file: Path, out: Path | None, dpi: int) -> None:
    deck = _load_deck(deck_file)
    target = out or (deck.output / f"{deck.name}.pptx")
    result = render(target, dpi=dpi)
    print(describe(result))
    print("\nLook at the contact sheet before believing the build. Bugs a check")
    print("cannot see: shadowing, centring, overlap, imbalance, a dead bottom third.")


def _themes() -> None:
    found = builtin_themes()
    if not found:
        raise DeckError(f"no themes in {THEMES_DIR}; the install is incomplete.")
    print(f"Built-in themes ({THEMES_DIR}):")
    for path in found:
        theme = Theme.load(path)
        print(f"  {path.stem:26} {theme.provenance or theme.name}")
    print("\nCopy one, change the palette, and pass it to Theme.load().")
    print("Deriving a theme from an existing artifact: references/theming.md")


def _init(name: str, directory: Path, theme_name: str | None = None) -> None:
    if not TEMPLATE.exists():
        raise DeckError(f"template missing: {TEMPLATE}")
    available = builtin_themes()
    if not available:
        raise DeckError(f"no themes in {THEMES_DIR}; the install is incomplete.")
    chosen = next((t for t in available if t.stem == theme_name), None) if theme_name else available[0]
    if chosen is None:
        names = ", ".join(t.stem for t in available)
        raise DeckError(f"no theme named '{theme_name}' ({names}).")

    target_dir = directory / name
    deck_file = target_dir / "deck.py"
    if deck_file.exists():
        raise DeckError(f"{deck_file} already exists; not overwriting.")
    target_dir.mkdir(parents=True, exist_ok=True)

    # The template names a theme on purpose, so it can be read as an example.
    # Rewrite that one path to whichever theme was chosen, or the scaffold builds
    # against a file that was never copied and fails on the first run.
    source = TEMPLATE.read_text(encoding="utf-8")
    deck_file.write_text(
        source.replace("Assets/themes/warm-scientific-teal.json",
                       f"Assets/themes/{chosen.name}"),
        encoding="utf-8",
    )

    (target_dir / "Assets" / "themes").mkdir(parents=True, exist_ok=True)
    (target_dir / "Assets" / "figures").mkdir(parents=True, exist_ok=True)
    shutil.copy2(chosen, target_dir / "Assets" / "themes" / chosen.name)
    print(f"created {target_dir}/")
    print(f"  {'deck.py':34}content, one Deck")
    print(f"  {'Assets/themes/' + chosen.name:34}the theme, edit the palette here")
    print(f"  {'Assets/figures/':34}figures the slides embed")
    print(f"\nnext: scripts/deck build {deck_file} && scripts/deck render {deck_file}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="deck",
        description="Build, check and render a slide deck from a Python module.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "A deck is a .py file that builds a deckkit.Deck.\n"
            "Scaffold one:  scripts/deck init my-talk\n"
            "Then:          scripts/deck build my-talk/deck.py\n"
            "               scripts/deck check my-talk/deck.py\n"
            "               scripts/deck render my-talk/deck.py"
        ),
    )
    sub = parser.add_subparsers(dest="verb", required=True)

    p_themes = sub.add_parser("themes", help="list built-in themes")
    p_themes.set_defaults(func=lambda a: _themes() or 0)

    p_init = sub.add_parser("init", help="scaffold a deck directory")
    p_init.add_argument("name")
    p_init.add_argument("--dir", default=".", type=Path)
    p_init.add_argument("--theme", default=None,
                        help="built-in theme to copy (default: the first one)")
    p_init.set_defaults(func=lambda a: _init(a.name, a.dir, a.theme) or 0)

    for verb, help_text, fn in (
        ("build", "compile the deck module into a .pptx", _build),
        ("check", "validate the built .pptx structurally", _check),
    ):
        p = sub.add_parser(verb, help=help_text)
        p.add_argument("deck", type=Path, help="path to the deck .py module")
        p.add_argument("--out", type=Path, default=None,
                       help="explicit output directory (default: the deck's own)")
        p.set_defaults(func=lambda a, fn=fn: fn(a.deck, a.out))

    p_render = sub.add_parser("render", help=".pptx -> .pdf -> PNGs + contact sheets")
    p_render.add_argument("deck", type=Path)
    p_render.add_argument("--out", type=Path, default=None)
    p_render.add_argument("--dpi", type=int, default=110)
    p_render.set_defaults(func=lambda a: _render(a.deck, a.out, a.dpi) or 0)

    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except DeckError as exc:
        print(f"deck: {exc}", file=sys.stderr)
        return 2
    except FileNotFoundError as exc:
        print(f"deck: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
