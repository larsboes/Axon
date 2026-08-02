"""Downloads the pinned bun release binary for each supported platform.

Hand-rolled: no published Bazel ruleset wraps bun as of this writing --
rules_js/rules_ts are pnpm-lockfile-based, which would contradict README.md#language-tooling ("bun, never npm -- no exceptions"). Per the adopt > contribute >
overlay > fork > build preference order (README.md#upstream-first,
upstreams.toml header), build is the sanctioned last resort when nothing exists to adopt.

Checksums come from oven-sh/bun's published SHASUMS256.txt for the pinned
tag and were cross-checked in-session by downloading bun-darwin-aarch64.zip
directly and hashing it -- not just trusted from the file. See upstreams.toml
[bun] for the pin/cooldown record.
"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

BUN_VERSION = "1.3.14"

# platform key -> (bazel os, bazel cpu, release asset stem, sha256 of the zip)
PLATFORMS = {
    "darwin_aarch64": struct(
        os = "macos",
        cpu = "aarch64",
        asset = "bun-darwin-aarch64",
        sha256 = "d8b96221828ad6f97ac7ac0ab7e95872341af763001e8803e8267652c2652620",
    ),
    "darwin_x64": struct(
        os = "macos",
        cpu = "x86_64",
        asset = "bun-darwin-x64",
        sha256 = "4183df3374623e5bab315c547cfa0974533cd457d86b73b639f7a87974cd6633",
    ),
    "linux_aarch64": struct(
        os = "linux",
        cpu = "aarch64",
        asset = "bun-linux-aarch64",
        sha256 = "a27ffb63a8310375836e0d6f668ae17fa8d8d18b88c37c821c65331973a19a3b",
    ),
    "linux_x64": struct(
        os = "linux",
        cpu = "x86_64",
        asset = "bun-linux-x64",
        sha256 = "951ee2aee855f08595aeec6225226a298d3fea83a3dcd6465c09cbccdf7e848f",
    ),
}

_BUILD_CONTENT = """\
exports_files(["bun"])
"""

def bun_repositories(version = BUN_VERSION):
    """Registers one http_archive repo per supported platform.

    Each repo extracts to a single "bun" executable at its root (the release
    zip's top-level directory matches the asset name, e.g.
    bun-darwin-aarch64/bun -- stripped via strip_prefix so every platform
    repo exposes the same "bun" label regardless of platform).
    """
    for key, p in PLATFORMS.items():
        http_archive(
            name = "bun_" + key,
            url = "https://github.com/oven-sh/bun/releases/download/bun-v{v}/{asset}.zip".format(
                v = version,
                asset = p.asset,
            ),
            sha256 = p.sha256,
            strip_prefix = p.asset,
            build_file_content = _BUILD_CONTENT,
        )
