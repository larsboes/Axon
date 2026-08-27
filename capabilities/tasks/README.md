# tasks

The record that something needs doing, and a way back to whatever said so.

Tasks observes nothing. It has no connector, no scanner and no schedule. Other
capabilities notice things and hand them here; this one owns the resulting
record and its lifecycle. That split is the point — the alternative is every
observer growing its own private to-do list, which is how you end up checking
four places to find out what you owe someone.

## Why it exists

The Gmail router doctrine says a mail that needs doing becomes **exactly one
owned action record**, and the mail stops being the thing you track. There was
nowhere for that record to live: calendar owns dated commitments, trips owns
itineraries, comms owns observation. An action that is not a commitment and not
an observation had no home, so in practice it stayed in the inbox, which is the
outcome the doctrine exists to prevent.

"Exactly one" is a unique index here, not a convention:

```sql
CREATE UNIQUE INDEX tasks_one_per_source ON tasks (source_capability, source_id)
    WHERE source_capability IS NOT NULL AND source_id IS NOT NULL;
```

A convention picks up a duplicate the first time a sweep runs twice, and after
that the inbox is authoritative again and nothing was gained. The index is
partial because hand-written tasks share a NULL source, and a plain unique index
would collapse all of them into one row.

Promoting the same mail twice returns the existing task with `created: false`
rather than erroring — pressing a button twice is a reasonable thing to do, and
an error pushes the caller into check-then-create, which races. A re-promote
never overwrites a title the operator has since corrected.

## The record

`title`, `status` (`open` | `done` | `dropped`), optional `due` and `note`, and
the provenance triple `source_capability` / `source_id` / `source_url`.

Provenance is not decoration. Without a way back to the mail, a task is a
sentence someone typed once, and the first question anyone asks about it — *what
did they actually ask for?* — has no answer.

`data_class` is **inherited from the source, never re-derived**. A task promoted
from a Private mail is Private, because the subject line travelled into the
title. The vocabulary is `libs/content-item`'s, so it means the same thing here
as everywhere else.

Completion stamps `completed_at`; reopening clears it. A `done` timestamp left
behind on a reopened task is the kind of small lie that makes a history
untrustworthy.

## Contract

| | |
|---|---|
| `GET /api/tasks?status=` | every task; open first, then by due date |
| `POST /api/tasks` | create, or return the one this source already owns |
| `GET /api/tasks/:id` | one task |
| `PATCH /api/tasks/:id` | title, status, due, note |
| `GET /api/counts` | open and overdue, for a badge |

`PATCH` distinguishes an absent field from a present-null one: absent leaves the
value, `null` clears it. Collapsing the two makes "remove the due date"
unexpressible and forces a sentinel value on the caller.

Runs on `8089`, loopback only. Rows live in the shared SQLite file
(`AXON_DB_PATH`, else `<overlay>/data/axon/axon.db`) under the table prefix
`tasks`, so the one table is `tasks_tasks` — see libs/axon-store/README.md.

## Boundaries

- Tasks never reads Gmail, a calendar or a vault. It is written to.
- It does not schedule, remind or notify. A due date is a field, not a trigger.
- A dated commitment belongs in **calendar**, not here. "Reply to the landlord"
  is a task; "viewing at 14:00 on Thursday" is a calendar entry. A task with a
  due date is still a task — the date bounds it, it does not place it.
