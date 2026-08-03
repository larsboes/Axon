# TEMPORARY CODE SCANNING PROBE — DELETE IN THE NEXT COMMIT ON THIS BRANCH.
#
# This file exists for one reason: Axon#13 asks that a finding be SHOWN to surface as an alert
# rather than assumed to, and CodeQL found nothing in the existing tree, so there was nothing to
# show. It is deliberately vulnerable. It is not imported, not executed, not referenced by
# anything, and it never reaches main — the PR squash-merges, so main's history has no commit
# containing it, and the branch is deleted.
#
# If you are reading this on main, something went wrong: delete it.

import sys


def run_untrusted(argv=None):
    """py/code-injection: an argv-controlled string reaches eval()."""
    payload = (argv or sys.argv)[1]
    return eval(payload)  # noqa: S307 - deliberate, see the header
