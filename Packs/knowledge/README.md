# llm-wiki pack

**Status: WIP.** Not linked yet; finish the TODO below first.

- **`llm-wiki`** — a pattern + runbook for a persistent, LLM-maintained wiki inside an Obsidian vault: ingest a source, query across pages, lint for stale/contradictory/orphan content. The LLM writes pages; the human curates sources and directs.

Vault-coupled: reads the vault root from the overlay (`$AXON_PERSONAL_ROOT/config/knowledge.toml`, `vault_root`), so nothing personal is baked into the skill.

## Open TODO (before linking)
- [ ] Validate the migrated SKILL.md against the `writing` pack's `writing-skills` (frontmatter, triggers, structure) — it was converted from opencode format by hand.
- [ ] Map `<domain>` to the real vault structure — confirm `Projects/LifeOS/LLM Wiki.md` exists (or update the schema-source path) and which `Knowledge/<domain>/` roots this runs against.
- [ ] Wire a real vault backup step before any destructive ingest/lint run (currently a generic "back up affected pages" line).
- [ ] Test one ingest + one query + one lint end-to-end against the vault.
- [ ] Then activate with `"$AXON_ROOT/tools/packs.sh" link llm-wiki` for Claude Code or
  `"$AXON_ROOT/tools/packs-codex" deploy llm-wiki` for Codex.

## Could grow into
A broader `knowledge` pack if the other vault skills (gardener, inbox, daily-briefing, review-workflow) get added later — for now it's llm-wiki alone.

## Attribution
Original work (Lars). License: MIT.
