# deckkit — the rendering mechanism behind the slide-deck skill.
#
# Three concerns, deliberately separate so a deck's content file can import only
# what it uses:
#
#   markup   text with **bold** / ~accent~ / newlines -> runs and real breaks
#   theme    palette, type scale and grid, loaded from JSON
#   layout   primitives: rect, panel, rule, table, kpi, picture
#   slide    a slide bound to its theme, its header/footer and its primitives
#   deck     the presentation: theme, page counter, archetypes, save
#   check    structural validation of a built .pptx
#   render   .pptx -> .pdf -> PNGs + contact sheets (the verification loop)
#   cli      `deck build|check|render|themes|init`
#
# The package lives under scripts/ rather than a lib/ of its own because the
# skill contract allows exactly scripts, references, assets and evals. scripts/ is
# the executable layer: the bash launcher, and the package it runs.

from pptx.enum.shapes import MSO_SHAPE
from pptx.enum.text import MSO_ANCHOR, PP_ALIGN

from .deck import Deck
from .errors import DeckError
from .slide import Slide
from .theme import Theme

__all__ = ["Deck", "Slide", "Theme", "DeckError",
           "MSO_SHAPE", "MSO_ANCHOR", "PP_ALIGN"]
__version__ = "0.1.0"
