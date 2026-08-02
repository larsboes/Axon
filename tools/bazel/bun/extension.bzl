"""bzlmod module extension: registers the per-platform bun download repos.
Fixed platform list (no module_ctx tags needed) -- see repositories.bzl."""

load(":repositories.bzl", "bun_repositories")

def _bun_impl(_module_ctx):
    bun_repositories()

bun = module_extension(implementation = _bun_impl)
