# backup

The scheduled run of every capability's backup contract.

`tools/backup-all.sh` asks the registry which capabilities declare `backup_target` and runs
`tools/backup.sh` for each. The set is derived, never typed: a capability is backed up because its
manifest says it has something to back up.

It runs every contract even when one fails, and exits non-zero if any did. Stopping at the first
failure would let one broken capability cancel the others' backups — the shape of outage that ends
with two weeks of nothing.

## Why this is a capability and not a LaunchAgent

It was a LaunchAgent, hand-written, for weeks. `tools/doctor` reported it as an orphan on every
run — a unit no manifest owned, so nothing versioned its schedule and a rebuilt machine would have
come back without it, silently.

Doctor's advice was `remove-persistence`, which would have deleted the only scheduled backup on
the machine. **The check was right about the shape and wrong about the remedy.** An orphan unit is
a declaration gap, and a gap can be closed from either end; here the missing end was the manifest.

That unit also named one capability, `backup.sh store`, while three declare a contract — so the
vault and finance were outside the schedule that was supposed to protect everything.
