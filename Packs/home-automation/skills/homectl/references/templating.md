# Automation templating contract

homectl materializes templates owned by the active private overlay. Template names and their
aggregate set are private because they describe a residence even when all entity IDs are tokens.

## Tokens

- %%NAME%% inserts one scalar overlay value.
- %%LIST:NAME%% expands a whole YAML value into a sequence.
- %%JSON:NAME%% emits a compact JSON literal for Jinja consumption.

Home Assistant Jinja expressions are left unchanged.

## Rules

- Real entity IDs, hosts, devices, coordinates, and secrets stay in the overlay.
- Templates and component selections stay in the overlay.
- Generated YAML stays under the overlay build or configuration root.
- Missing variables, orphan variables, unresolved tokens, and invalid YAML fail materialization.
- Runtime thresholds belong in overlay-owned Home Assistant helpers where practical.
- Component vendoring uses reviewed, pinned overlay entries and never fetches at runtime.
