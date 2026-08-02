# host-audit

Inventory of everything installed on this machine (apt/brew), diffed against Axon's declared
base, snapshotted to the overlay, and CVE-scanned — the "watch what I'm using" tool. Distinct
from `tools/audit`, which scans Axon's own repo and overlay: this scans the whole machine's
package surface, the part that differs from box to box.

## Verdict

**Build (CLI), adopt `trivy` underneath.** The job is small and heuristic: ask the host's
package manager what's installed, set-diff it against `toolchain.toml`, and hand the installed
set to a real CVE scanner. `trivy` (already adopted — `upstreams.toml [trivy]`) does the
vulnerability half; nothing here re-implements a scanner.

## Commands

```
host-audit inventory [--json]    # what's installed, grouped base (in toolchain.toml) vs extra
host-audit snapshot              # write inventory --json to the overlay + update latest.json
host-audit diff [--since <file>] # current vs latest snapshot (default): added/removed/changed
host-audit scan [--sarif <dir>]  # CVE scan of installed OS packages via trivy
```

The CLI is on PATH automatically — `capabilities/shell/init.zsh` sweeps for
`capabilities/<name>/<name>` and adds the dir, so `host-audit` is callable from any shell with
nothing registered by hand. It branches on which package manager is **present**
(`dpkg-query`/`brew`), not on the declared OS, so it works on Linux, macOS, and linuxbrew alike.

Inventories cover the **manually-installed** set (`apt-mark showmanual`, `brew leaves
--installed-on-request`) — the packages you actually chose, not the full dependency closure,
which is the signal that answers "what am I using." Snapshots land in the overlay
(`$AXON_PERSONAL_ROOT/data/host-audit/`): machine-specific data, never the public repo (README.md#one-manifest-per-concern and README.md#public-core-and-private-overlays), and backed up with the overlay repo itself. Registering the directory as a
`[[state_mount]]` in `machine.toml` is optional — the overlay git repo already carries it.

## Why this shape: build & language

Bash, not Rust, and no Bazel target — the same call `cv` makes (see `capabilities/cv/README.md`
and README.md#argue-bazel-per-case). This sits on the lowest rung of README.md#implementation-languages-and-intelligence's intelligence ladder
(heuristic): run package managers, set-diff against a manifest, shell out to trivy. The shape
and idioms are identical to `tools/audit` and `tools/toolchain-check` (both bash) — there are no
Rust types crossing a serde boundary with another crate, no performance need, and no container.
A Bazel target would drag in a toolchain and buy nothing; an interpreted tool with no build step
is invoked directly here by design.

Base-vs-extra classification is exact-name matching against `toolchain.toml`'s tool ids and bins
(read live via `tools/toolchain-check --json`). It is deliberately best-effort: a package whose
apt name differs from its tool id (`docker-ce` vs `docker`) lands in "extra." The valuable
output is the extra list — the machine-specific surface — not a perfect base partition.

## Considered and declined

- **`grype` (Anchore) as the scanner.** Would be a *new* upstream needing its own verdict +
  cooldown (README.md#dependency-verdicts-and-provenance and README.md#pins-and-cooldown). `trivy` is already adopted and its `rootfs` mode covers installed OS
  packages, so a second scanner earns nothing here.
- **Full dependency-closure inventory** (every installed package, not just the manual set).
  Noise: the closure is hundreds of auto-pulled dependencies. `apt-mark showmanual` / `brew
  leaves` is the "what I chose" set the CVE scan and the diff both want.
- **Snapshots as a heavy `[[state_mount]]` with its own restic target.** The overlay is already
  a backed-up git repo; snapshots committed into it are covered without a second sync path.

## Known gap

On macOS, `trivy` has no Homebrew analyzer, so `scan` cannot CVE-cross-reference brew formulae —
it says so out loud and exits rather than silently reporting "clean" (README.md#documentation-stays-owned-and-current, no silent caps).
`inventory`/`diff` work fully on macOS; OS-package CVE scanning is the Linux (`trivy rootfs`)
path. Candidate to close it later: query OSV.dev by formula.

## Done looks like

- `host-audit inventory --json` emits valid JSON on any box with a package manager present.
- `host-audit scan` degrades cleanly (install hint, non-zero, no crash) when `trivy` is absent.
- Snapshots are written under the overlay's `data/host-audit/`, never into this public repo.

## Attribution

- **trivy** — the CVE scanning engine (`scan`). Runtime dependency; verdict, pin and license in
  `upstreams.toml [trivy]`. Host package managers (`apt`/`dpkg`, Homebrew) are OS-provided
  infrastructure, invoked, not vendored.
