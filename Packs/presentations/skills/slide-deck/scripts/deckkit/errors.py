"""One exception type, so the CLI can print a clean failure instead of a traceback.

Every raise site names the fix, not just the fault: the caller is usually an
agent that has to self-correct from stderr alone.
"""

from __future__ import annotations


class DeckError(Exception):
    """A deck cannot be built, checked or rendered, and here is why."""
