# travel pack

One skill, **`travel`**, for driving Axon's travel capabilities as a single workflow: calendar,
transit, punctuality, scouting and trips. It carries the ordering between them and the joins that
fail quietly, not their route tables — each capability serves its own manifest at `GET /routes`,
and a copy here would be a copy that goes stale.

## Skills

- `travel` routes a question to the capability that owns it, runs the calendar → transit → trips
  order, and reaches every service through `axon capability call`. Its one reference,
  `references/traps.md`, holds the failures that return an empty or plausible answer instead of
  an error.

## Activate

```sh
axon pack deploy claude travel
axon pack deploy codex travel
```

## Ownership boundary

Axon owns the workflow. The active overlay owns every value that makes it personal: home and
destination stations, the geo policy behind scouting's event routing, the calendar credential and
home timezone, and the shared database file. Nothing in this Pack names a station, a city or
a route.

The capability READMEs stay the contracts. This Pack points at them rather than restating them,
so a change to a capability is a change in one place.
