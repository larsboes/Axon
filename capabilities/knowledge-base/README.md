# knowledge-base

The vault, as a thing that gets backed up.

`kind = "data"` — no image, no command, nothing to start. Obsidian owns this directory and Axon
writes to it only inside declared projection regions (`libs/markdown-root/src/projection.rs`).
The manifest exists for the one thing a file tree needs an owner for, which is backup.

## Why it exists

Because the vault had three declared backups and none of them worked.

Measured 2026-08-29, before this capability existed:

| Declared where | What it claimed | What was true |
|---|---|---|
| overlay `machine.toml`, `[[state_mount]]` | `sync = "git"` — *"git is history + backup (bare repo on homepi)"*, `direction = "capture"` | The vault checkout had **no remote and never had one** |
| overlay `systems.local.toml`, `[knowledge-base]` | `url = "https://github.com/larsboes/Knowledge-Base"` | The repository **does not exist** |
| the backup target host | a restic repo, 8.9 GB | **Two snapshots, both 2026-08-02**, and `restic` is not installed on this Mac |

Time Machine had no destination configured either, and no LaunchAgent referenced the vault path.
So the only off-machine copy was iCloud Drive live sync, which propagates a deletion rather than
surviving one.

**A `[[state_mount]]` says where data lives. Only a manifest puts it in the registry that reports
when a backup goes stale** — and that difference is the entire reason for this directory. Every
one of the three failures above was silent, and each would have stayed silent indefinitely,
because nothing was watching a path that no manifest owned.

## What it backs up

The whole vault directory including `.git`, so one archive carries current state *and* complete
history. That is also why `backup_retain` is 7 rather than store's 14: how far back you can go is
not answered by how many tarballs survive.

`.obsidian/plugin-backups/` is excluded. It held 19 symlinks pointing out of the vault at a
since-retired plugin monorepo, so every one dangled — and 4 KB of dead links made the first 704 MB
archive **unrestorable**, because `tools/restore.sh` refuses any archive carrying a link. That
refusal is correct: a symlink inside an archive is how extraction writes outside the directory the
operator chose. `tools/backup.sh` now refuses to *produce* one, so the two tools hold the same
contract instead of disagreeing at restore time.

## Where the path comes from

The member name is tracked here; the root it hangs from is machine-local, in the overlay:

```toml
[capability.knowledge-base]
backup_source_root = "~/Library/.../Documents"
```

An iCloud container path is a fact about one machine, and `backup_paths` must stay relative
because those strings are tar member names as well as sources.

## Rehearsed

2026-08-29, and it is the only reason any of the above is a claim rather than a hope:
704,596,192 bytes shipped, fetched back, restored into an isolated destination, and compared
against the live vault — **2,168 notes, 14,878,214 bytes, zero content differences**.

Eleven filenames differ in Unicode normalisation only (NFD on APFS, NFC through tar). Harmless on
macOS, which is normalisation-insensitive; worth knowing before restoring onto Linux, where those
eleven names would differ byte-wise and Obsidian's wikilinks to them would not resolve.
