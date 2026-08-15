# host-watch

The machine's own vital signs, checked hourly, reported only when something crosses a
line. Most runs print one line and write nothing, and that silence is the product.

## Why it exists

Two host conditions were invisible until they hurt, and both did on 2026-08-15 (Axon#177).

A System Settings Storage pane got stuck at 08:35 and ran until it was killed at 17:52.
`ApplicationsStorageExtension` burned **3h29m of CPU** in that window; the only symptom
anyone experienced was "the Mac feels slow", nine hours in. Nothing here watched
cumulative CPU time, so a process quietly holding a third of a core forever looked
exactly like an idle one.

The disk half is the more instructive failure, because it was **already solved**.
`tools/storage` and `tools/sysmon report` (#135/#136) answered the question in seconds
when someone finally ran them. But both were on-demand only — no schedule, no threshold,
no surface. A tool nobody runs and a tool nobody built are the same tool. That is the
gap this closes, and it is a scheduling gap, not a missing feature.

## What it checks

**Runaway processes.** Cumulative CPU time divided by seconds alive — how much of a core
a process has held for its entire life. Two conditions must both hold: a floor of
accumulated CPU (`min_cpu_seconds`, so a five-minute compile is not a finding) and a
sustained-core ratio (`min_cpu_ratio`, so merely being old is not a finding either).

Ranking by raw CPU time is the obvious rule and it is wrong. On the day this was written
WindowServer had **more** cumulative CPU than the stuck extension — 168 minutes against
148 — and was perfectly healthy. It had simply been alive four times longer. That pair is
frozen as a fixture in `tools/host-watch.test.ts`, because it is the one comparison that
tells a working implementation from a plausible one.

**Free space.** Delegated whole to `tools/storage --json`; this reads its verdict and
adds no thresholds of its own. Deliberately fires on the volume state alone and **not**
on a class being large: `class_flag_gb` legitimately fires today, on a 28 GB cargo target
dir, on a machine with 130 GB free and nothing wrong with it. Alerting on that would mean
the watcher's first ever run produced a task nobody needed, which is how a watcher gets
muted. The class breakdown stays what it already was — what `sysmon storage` tells you
once you are already looking.

Memory and thermals are not checked. Both measured healthy at the incident (pressure
normal, zero swap, no thermal warning recorded in 26h) and adding a check for a condition
that has never fired manufactures alerts rather than information.

## How it reports

It writes a `tasks` record and nothing else. No new notification machinery: core Axon has
never had a notifier and does not grow one here — the precedent is stated in
`tools/sparpreis-watch.ts`'s own header, and `capabilities/tasks` states the other half
("other capabilities notice things and hand them here"). `requires = ["tasks"]` because
that hand-off *is* the alert surface.

**One task per run of a condition.** tasks' partial unique index on `(source_capability,
source_id)` collapses repeats, so a breach that persists for a week is one record whose
note is refreshed, not seven records. The `source_id` additionally carries a generation
(`cpu:Foo#1`), and a new one is minted only once every prior task for that condition is
closed — otherwise an operator marking a task done would silence that condition forever,
which is the failure mode a watcher notices last.

## Commands

```
tools/host-watch              check, write findings to tasks
tools/host-watch --dry-run    check and print; write nothing
tools/host-watch --json       machine-readable findings
```

## Why it is a manifest and not a LaunchAgent

Same reason as macmon, learned the same way (Axon#65): a hand-written unit versions its
interval nowhere and needs a doctor exemption that removes the only check that would
notice it is gone. `schedule = "1h"` renders launchd or systemd through
`tools/service-runner.sh install-persistence` like everything else.

Hourly is measured, not assumed. The expensive half is the storage scan it shells out to
— 6.6s wall, almost entirely I/O wait, 0.2% of an hour. A watcher that is itself a
resource hog would be a poor joke given what it watches for.

## Machine facts live in the overlay

Every threshold and allowed process name is in
`<overlay>/config/host-watch-policy.toml`; this capability's code contains no process
name and no number (README.md#generic-in-axon-specific-in-the-overlay). Shape:
`schemas/host-watch-policy.toml.example`.

The allowlist is meant to stay short. Every name on it is a process this watcher can
never warn about again, so an allowlist grown to silence noise is how the check quietly
stops working. Raising `min_cpu_ratio` is nearly always the better answer.

## Considered and declined

- **Killing the runaway.** Detection only. A scheduled job with kill authority over
  arbitrary host processes is a blast radius nobody asked for, and the diagnosis of
  "stuck" versus "working hard" is exactly the judgement a threshold cannot make.
- **Running `storage --apply` on a schedule.** Reclaiming stays a deliberate human
  command. Deleting a build cache mid-compile is a cure worse than a full disk.
- **A dashboard panel.** tasks already has a surface and a count endpoint. A second place
  to look is the failure this exists to prevent.
