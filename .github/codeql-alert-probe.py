# TEMPORARY CODE SCANNING PROBE — DELETE IN THE NEXT COMMIT ON THIS BRANCH.
#
# This file exists for one reason: Axon#13 asks that a finding be SHOWN to surface as an alert
# rather than assumed to, and CodeQL found nothing in the existing tree, so there was nothing to
# show. It is deliberately wrong. It is not imported, not executed, not referenced by anything,
# and it never reaches main — the PR squash-merges, so main's history has no commit containing
# it, and the branch is deleted.
#
# The first attempt used `eval(sys.argv[1])` and produced no alert: CodeQL's default Python suite
# does not treat a command-line argument as a remote source, so the taint query never fired. The
# sinks below are syntactic instead — they need no source-to-sink path, only the call shape — so
# whether an alert appears is a question about the pipeline rather than about taint modelling.
#
# If you are reading this on main, something went wrong: delete it.

import hashlib
import socket

import requests


def fetch_without_cert_check(url):
    """py/request-without-cert-validation — verify=False disables TLS verification."""
    return requests.get(url, verify=False, timeout=5)


def hash_a_password(password):
    """py/weak-sensitive-data-hashing — MD5 over something named like a credential."""
    return hashlib.md5(password.encode()).hexdigest()


def listen_everywhere(port=8080):
    """py/bind-socket-all-network-interfaces — binds every interface, not loopback."""
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.bind(("0.0.0.0", port))
    return server
