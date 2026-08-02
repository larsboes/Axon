"""The bun toolchain type + rule, and a smoke-test rule that proves a build
action can actually resolve and execute the hermetic binary (not just that
the toolchain declares cleanly)."""

BunToolchainInfo = provider(
    doc = "Points at the pinned, hermetic bun binary for the current platform.",
    fields = ["bun_binary", "version"],
)

def _bun_toolchain_impl(ctx):
    return [
        platform_common.ToolchainInfo(
            bun_info = BunToolchainInfo(
                bun_binary = ctx.file.bun_binary,
                version = ctx.attr.version,
            ),
        ),
    ]

bun_toolchain = rule(
    implementation = _bun_toolchain_impl,
    attrs = {
        "bun_binary": attr.label(
            allow_single_file = True,
            mandatory = True,
            doc = "The downloaded bun executable for this platform.",
        ),
        "version": attr.string(mandatory = True),
    },
)

def _bun_smoke_check_impl(ctx):
    info = ctx.toolchains["//tools/bazel/bun:toolchain_type"].bun_info
    out = ctx.actions.declare_file(ctx.label.name + ".version.txt")

    # chmod +x defensively: zip's Unix-permission preservation through
    # Bazel's http_archive has historically been inconsistent across
    # versions/platforms, so don't rely on the archive's stored mode bit.
    ctx.actions.run_shell(
        outputs = [out],
        inputs = [info.bun_binary],
        command = "chmod +x {bun} && {bun} --version > {out}".format(
            bun = info.bun_binary.path,
            out = out.path,
        ),
        mnemonic = "BunSmokeTest",
        progress_message = "Running hermetic bun --version (pinned %s)" % info.version,
    )
    return [DefaultInfo(files = depset([out]))]

bun_smoke_check = rule(
    implementation = _bun_smoke_check_impl,
    toolchains = ["//tools/bazel/bun:toolchain_type"],
)
