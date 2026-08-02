"""Runs a bun package's build script as a Bazel action and captures its output.

Pairs with deps.bzl: dependencies are installed at fetch time, so this action
needs no network -- it gets a lockfile-pinned `node_modules` handed to it as a
declared input, like any other input.

The build runs in a staging directory assembled from the declared sources
rather than in the package itself. That is deliberate: building in place would
mean the action's correctness depended on the sandbox strategy, and a local
(unsandboxed) run would write `.svelte-kit/` and `dist/` into the source tree.
Staging costs a copy of the sources -- kilobytes -- and makes the action behave
the same either way. `node_modules` is symlinked rather than copied, because
that is the part that is thousands of files.
"""

load(":toolchain.bzl", "BunToolchainInfo")  # buildifier: disable=unused-load

_NODE_MODULES_MARKER = "/node_modules/"

def _node_modules_root(files):
    """The directory holding the installed tree, derived from any file in it."""
    for f in files:
        index = f.path.find(_NODE_MODULES_MARKER)
        if index != -1:
            return f.path[:index + len(_NODE_MODULES_MARKER) - 1]
    fail("bun_vite_build: node_modules attribute contained no files under node_modules/")

def _package_relative(ctx, file):
    prefix = ctx.label.package + "/"
    path = file.short_path
    if path.startswith(prefix):
        return path[len(prefix):]
    return path

def _bun_vite_build_impl(ctx):
    info = ctx.toolchains["//tools/bazel/bun:toolchain_type"].bun_info

    # Named after the target, not after built_dir: a package that has been built
    # locally already has a `dist/` directory in the source tree, and a target
    # sharing that name collides with it.
    out = ctx.actions.declare_directory(ctx.label.name)

    manifest = ctx.actions.declare_file(ctx.label.name + ".stage.tsv")
    ctx.actions.write(
        manifest,
        "".join([
            "%s\t%s\n" % (f.path, _package_relative(ctx, f))
            for f in ctx.files.srcs
        ]),
    )

    command = """
set -euo pipefail

EXECROOT="$(pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

while IFS="$(printf '\\t')" read -r src dst; do
  mkdir -p "$WORK/$(dirname "$dst")"
  cp "$EXECROOT/$src" "$WORK/$dst"
done < "{manifest}"

# node_modules is a real, writable directory whose entries are symlinks into the
# read-only input tree. A single symlink to the tree itself is not enough: vite
# writes its config-loading temp file into node_modules/.vite-temp/, and that
# would land in the sandbox's read-only inputs. Linking per entry keeps the copy
# at 45 symlinks instead of thousands of files.
mkdir -p "$WORK/node_modules"
for entry in "$EXECROOT/{node_modules}"/* "$EXECROOT/{node_modules}"/.[!.]*; do
  if [ -e "$entry" ]; then
    ln -s "$entry" "$WORK/node_modules/$(basename "$entry")"
  fi
done

chmod +x "{bun}"
export HOME="$WORK/.home"
mkdir -p "$HOME"

# The .bin shims carry `#!/usr/bin/env node` shebangs, and there is no node in
# the sandbox by design (README.md#language-tooling: bun, never npm). A `node` on PATH that IS the
# pinned bun makes every such shebang resolve to it. `bun run --bun` is supposed
# to do this too, but only for what bun executes directly -- the shims are
# spawned by the script's own shell, past that point.
mkdir -p "$WORK/.shim"
ln -s "$EXECROOT/{bun}" "$WORK/.shim/node"
export PATH="$WORK/.shim:$PATH"

cd "$WORK"
"$EXECROOT/{bun}" run {script} >/dev/null

cp -R "{built}/." "$EXECROOT/{out}/"
""".format(
        manifest = manifest.path,
        node_modules = _node_modules_root(ctx.files.node_modules),
        bun = info.bun_binary.path,
        script = ctx.attr.script,
        built = ctx.attr.built_dir,
        out = out.path,
    )

    ctx.actions.run_shell(
        outputs = [out],
        inputs = depset(
            direct = ctx.files.srcs + ctx.files.node_modules + [manifest, info.bun_binary],
        ),
        command = command,
        mnemonic = "BunViteBuild",
        progress_message = "Building %s with hermetic bun %s" % (ctx.label, info.version),
    )

    return [DefaultInfo(files = depset([out]))]

bun_vite_build = rule(
    implementation = _bun_vite_build_impl,
    doc = "Builds a bun package into a directory output, using the pinned toolchain.",
    attrs = {
        "node_modules": attr.label(
            mandatory = True,
            doc = "The matching //tools/bazel/bun:deps.bzl repo's :node_modules filegroup.",
        ),
        "built_dir": attr.string(
            default = "dist",
            doc = "Directory the build script writes, relative to the package root.",
        ),
        "script": attr.string(
            default = "build",
            doc = "package.json script to run.",
        ),
        "srcs": attr.label_list(
            allow_files = True,
            mandatory = True,
            doc = "Everything the build reads: sources, configs, the manifest.",
        ),
    },
    toolchains = ["//tools/bazel/bun:toolchain_type"],
)
