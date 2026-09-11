"""Entry point for `python -m deckkit`, which is what `scripts/deck` calls."""

from .cli import main

if __name__ == "__main__":
    raise SystemExit(main())
