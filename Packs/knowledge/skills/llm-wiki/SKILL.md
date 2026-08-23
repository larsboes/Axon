---
name: llm-wiki
description: Builds and maintains a persistent, LLM-maintained wiki inside an Obsidian vault — the LLM writes/maintains all pages, the human curates sources and asks questions. Ingest a source, query across pages, or lint for stale/contradictory/orphan content. Vault root comes from the overlay (config/knowledge.toml). Use when ingesting an article/paper/transcript into the vault, synthesizing across notes, health-checking the wiki, or building a knowledge structure for a domain. Do not use for code documentation, one-off notes, or non-vault knowledge.
allowed-tools: Read, Write, Edit, Bash, WebFetch
---

# LLM Wiki

A pattern for a persistent, LLM-maintained wiki inside an Obsidian vault. The LLM writes and maintains all wiki pages; the human curates sources, asks questions, and directs the analysis.

**Vault is not hardcoded.** The vault root lives in the overlay — resolve it first, never assume a path:
```bash
VAULT=$(uv run --python 3 python -c 'import tomllib,os;p=os.path.expanduser(os.environ["AXON_PERSONAL_ROOT"])+"/config/knowledge.toml";print(os.path.expanduser(tomllib.load(open(p,"rb"))["vault_root"]))')
```
All `$VAULT/...` paths below resolve against that.

## When to use
- Ingest a new source (article, paper, book chapter, transcript) into the knowledge base
- Answer a question that needs synthesis across multiple wiki pages
- Health-check / lint the wiki for stale, contradictory, or orphaned content
- Build out a knowledge structure for a new domain

## What this skill does
1. **Reads the LLM Wiki project note** at `$VAULT/Projects/LifeOS/LLM Wiki.md` for current schema conventions, page formats, and domain config.
2. **Reads `index.md`** in the relevant Knowledge subdirectory to see what pages exist.
3. **Ingest:** reads the source, extracts key info, updates all affected pages (summaries, entities, concepts, cross-refs, index, log).
4. **Query:** searches the wiki, reads relevant pages, synthesizes an answer with citations, optionally files it back as a page.
5. **Lint:** scans for contradictions, stale claims, orphan pages, missing cross-references, data gaps.

## Wiki structure
Under `$VAULT/Knowledge/<domain>/llm-wiki/`:
```
index.md              -- catalog of all pages with links + one-line summaries
log.md                -- append-only chronological record of all operations
sources/source-<title>.md
entities/entity-<name>.md
concepts/concept-<name>.md
comparisons/comparison-<topic>.md
queries/query-<topic>.md
```
A flat form (`index.md`, `log.md`, `<topic>.md`) is valid when the domain is narrow.

## Schema conventions
Every page gets YAML frontmatter following vault conventions:
```yaml
---
type: note
maturity: seedling | growing | evergreen
summary: "One-line description of what this page covers"
categories:
  - "[[Category Link]]"
related:
  - "[[Related Page]]"
sources:
  - "Source title or URL"
---
```
**Maturity:** `seedling` = new, single source, likely incomplete · `growing` = multiple sources synthesized, cross-refs in place · `evergreen` = stable, well-cross-referenced, contradictions resolved.

## Operations

### Ingest a source
1. Ask the user to place the source in `$VAULT/raw/<domain>/` (or accept a URL to fetch → save to `$VAULT/raw/<domain>/<title>.md`)
2. Read the source; discuss key takeaways with the user
3. Determine pages to create/update: entity pages, concept pages, the source summary, cross-ref updates, index + log
4. **Present the planned changes before writing**, then write/update all pages and append to `log.md`

### Answer a question (query)
1. Read `index.md` to find relevant pages → read them
2. Synthesize an answer with citations to specific wiki pages
3. If it adds lasting value, file it as `queries/query-<topic>.md` and update the index

### Lint the wiki
1. Walk all wiki pages; check for contradictions, stale claims, orphan pages, concepts lacking a page, missing cross-refs
2. Surface results with suggested fixes; apply approved fixes

## Tips
- **Mermaid** for architecture/flow/comparison diagrams (Obsidian-native)
- **Dataview** over YAML frontmatter for dynamic tables
- **MOCs** link pages into broader domains via `categories`
- **One source at a time** for ingest — keeps the human-in-the-loop quality
- Take a backup/dump of affected pages before a schema change
