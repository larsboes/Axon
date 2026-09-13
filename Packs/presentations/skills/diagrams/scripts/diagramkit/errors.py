"""One error type, so the launcher prints one shape of message."""


class DiagramError(Exception):
    """A condition the caller can fix: a missing palette, a missing renderer."""
