# lifeos

The harness Axon runs on. `~/.claude` **is** LifeOS — installed, not cloned: no
`.git`, no remote, no way to `git diff` your own machine against upstream.

This capability owns Axon's delta over a stock install. Upstream verdict:
`upstreams.toml [lifeos]`, `overlay`, pinned at `v7.1.1`.

```
overlay/settings.hooks.json      hook wiring LifeOS ships files for but never registers
overlay/hooks/ProseGate.hook.ts  Axon-authored hook, not shipped by LifeOS
```

Instance config — which sinks and modules are on for *this* machine — lives in
the overlay at `axon-overlay/config/lifeos/PULSE.toml`, not here (README.md#public-core-and-private-overlays:
machine-local facts live in exactly one private overlay).

Deploy and check with `tools/lifeos-sync`.

## Why this exists

Not the reason you would guess. LifeOS' installer is **non-destructive
throughout**: `DeployCore` and `DeployComponents` go through `copyMissing`,
which never overwrites a populated target, and `InstallSettings` adds only
*absent* top-level and env keys, backing up before every write. An update does
not clobber local edits.

The problem is the mirror image of that. Because the installer never touches an
existing value:

- New upstream defaults never reach a file you have edited. You silently keep
  the old shape forever.
- Nothing records which parts of `~/.claude` are yours. A year of small edits
  becomes indistinguishable from what shipped.
- A reinstall, or a second machine, starts from stock and every edit is gone.

So Axon owns the delta and projects it down. `tools/lifeos-sync status` answers
"what do I differ in", which a stock install cannot answer at all.

## Why only the SYSTEM zone

LifeOS splits its tree into four zones (its own
`DOCUMENTATION/SystemUserBoundary.md`). The USER zone — identity, TELOS,
MEMORY — already lives outside the harness at `~/.config/LIFEOS/USER`, survives
updates on its own, and has its own backup path in
`tools/lifeos-user-sync.sh`. Duplicating it here would be a second home for the
same fact (README.md#one-manifest-per-concern). This capability covers only what sits *inside* the
system tree and would otherwise be invisible and unrecoverable.

What the USER zone does get is visibility. `tools/doctor` runs
`tools/mirror-lifeos-user.sh` in dry-run and reports the divergence count
against the overlay's recovery mirror — read-only, warning not failure, count
without paths. Drift between refreshes is normal; the mirror silently winning
over the original is not, which is why nothing on this path ever writes.

## Why two mechanisms

`tools/lifeos-sync` picks per target, and the choice is not stylistic:

- **symlink** for `ProseGate.hook.ts` and `PULSE.toml`. Drift becomes
  structurally impossible rather than merely detectable, and the installer's
  `existsSync` guard skips a symlink forever. Only safe because neither file has
  relative imports — a hook that did `import "./lib/..."` would resolve against
  its *real* path in Axon and fail to find LifeOS' `hooks/lib/`. Check that
  before adding a third link.
- **merge** for `settings.json`. Claude Code writes into it too (permissions,
  its own keys), so it cannot be a symlink without losing those writes. The
  hook delta is merged in and matched on command string, which makes a re-run a
  no-op even if a timeout was retuned by hand.

## What was wrong when this was built (2026-07-28)

Three hooks LifeOS ships but wires to nothing — `Doctor.ts --reconcile` reports
them itself. `PostToolObserver` is the documented host that imports
`LoopDetector` (exact-repeat / oscillation / hammering detection) and
`AlgorithmNudge`; nothing registered it, so loop detection had never run.
`DriftReminder` had zero imports anywhere. `AlgorithmNudge` was registered on
`PostToolUseFailure` alone though its own header declares three events.

`PULSE.toml` shipped with six modules enabled that had nothing behind them on
this install — sinks whose env vars were never set, integrations for hardware
that is not present, a harness that is not installed. Two of them received the
`error` and `security` notification routes, so both routes were half-failing.
Which ones and why is instance-specific and lives inline in the overlay's own
`PULSE.toml`; the pattern worth carrying forward is that **Pulse defaults a
module to enabled and fails soft**, so an unconfigured sink looks identical to a
working one until you read the logs.

One of them, `telegram`, failed hard instead: it bailed on every start and the
supervisor respawned it every 10 seconds for weeks. The trap is worth naming
because the obvious fix does not work — `modules/telegram.ts:761` uses `??`, not
`||`, so an *empty but set* `TELEGRAM_ALLOWED_USERS` is not `undefined`, and the
fallback to `TELEGRAM_PRINCIPAL_CHAT_ID` that the config comments promise never
fires. Leave the var unset, not blank.

The hot-layer memory read 0 entries while holding 52 curated ones. `MemoryWriter`
brackets entries with `<!-- BEGIN ENTRIES -->` / `<!-- END ENTRIES -->`, but the
scaffolded file ships only a prose comment where BEGIN belongs. `parseFile` then
takes its marker-recovery branch, which leaves every prior END marker sitting in
the body, and `serializeFile` appends one more — so each curation run added an
END marker and wrote the entries *after* it. `LoadMemory`'s reader stops at
`indexOf(END)`, the first one, three lines into the file. Nothing was lost, and
nothing was ever read either: 47 write cycles produced 47 markers. Repairing the
file is enough — with a BEGIN marker present the writer's normal path takes over
and stays correct. Worth checking on any install whose memory panel reads empty
while the file is visibly not.

## Considered and declined

**Mirroring all of `~/.claude` into Axon.** Full reproducibility, but every
LifeOS update becomes a manual merge across thousands of files Axon has no
opinion about, and the delta — the only interesting part — disappears into the
noise. The delta is what is worth versioning.

**Fixing the `claude` shell wrapper instead of deleting it.** It appended
`PAI_SYSTEM_PROMPT.md`, renamed to `LIFEOS_SYSTEM_PROMPT.md` in 7.x, so its own
`[[ -f ]]` guard had made it a silent no-op since the upgrade. Pointing it at
the real file would load the constitutional layer into *every* `claude`
invocation, including subagent and cmux spawns. Upstream's current doctrine is
the split it was fighting (LifeOS `INSTALL.md:96-102`): plain `claude` stays
vanilla, `lifeos` opts in. Deleted; the `lifeos` alias already exists and works.

**Bazel.** No dependency graph to buy anything, and the tool writes into `$HOME`
rather than a build output, so sandboxing is pure cost (README.md#argue-bazel-per-case, same call as
`README.md#argue-bazel-per-case`).
