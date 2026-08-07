# comms

General observed-information intake for Axon, plus reviewed mail triage. Its `feed_items`
store is intentionally source-agnostic. Security advisories and updates to systems or
packages belong here. So do watched-repository changes, news, useful articles and opportunity
signals such as scholarships, hackathons, calls for papers or events. The extractors
implemented today are the current ingestion set, not the boundary of that contract.

The capability currently has two ingestion paths:

1. **Gmail-as-router (read-only sweep, explicit reviewed actions).** Sweeps your inbox threads, classifies
   each into a stream (`aktiv`, `issue`, `feed`, `werbung`, `belege`, `steuern`,
   `sonstiges`) with a one-sentence rationale, and proposes triage. A sweep never
   changes Gmail. Authenticated dashboard actions may archive one stored thread
   or move it to Gmail Trash after a separate confirmation; permanent deletion,
   sending and arbitrary label changes do not exist.
2. **Share-link media ingest.** Turns a pasted URL into a feed item — metadata,
   an optional transcript, and an optional local-LLM summary — stored for a
   dashboard to read. Per-source extractors for YouTube/Instagram/podcast
   (yt-dlp), GitHub repositories, arXiv papers and Reddit posts; anything else
   falls through to a generic fetch-and-strip.
3. **Bounded awareness sources.** A source scan imports public GitHub Trending
   repositories or the newest results for configured arXiv queries. The public,
   non-personal defaults are daily GitHub Trending plus recent `cs.AI`, `cs.LG`
   and `cs.CL`; an explicit `feed_sources: []` disables them. Personal arXiv
   queries belong in the private overlay.

Mirrors `capabilities/scouting`'s shape: a `comms` library plus two binaries
(`comms` CLI, `comms-server` HTTP API), sync Postgres client, config resolved
from the private overlay at runtime.

### Mail classification today

Mail triage is deterministic and local. It does not call an LLM, embedding
model, or cloud AI service, and it does not produce an importance score. The
classifier considers only the sender header, subject header, and whether a
`List-Unsubscribe` header exists. The fetched snippet, Gmail labels, internal
date, message body, and attachments do not affect the category.

Rules use first-match-wins order: personal rules from the private overlay,
then generic public heuristics, then the conservative `aktiv` fallback. Every
proposal stores the rationale, method, and classifier revision. A category
changed in the dashboard becomes a `human` override and later sweeps preserve
it. Category ordering in the dashboard is an attention aid, not a hidden score.

Every shared content item also carries one inspectable trust class. `public`
(shown as **Public**) may use local processing and is eligible for configured
cloud processing. `personal` (shown as **Personal**) stays local unless an
explicitly reviewed, pseudonymized derivative is created. `vault` (shown as
**Private**) keeps the original content local-only. Public Feed sources default
to `public`; mail defaults to `personal`, while deterministic metadata rules
elevate likely financial, health, authentication, or recovery mail to `vault`.
The rules use sender, subject, and category only—never body or attachment
content—and a dashboard override is stored as `human` and preserved by refreshes.

For `vault` mail the metadata *is* the payload: a one-time code arrives in the
subject line, so storing that subject verbatim would publish it to a log, an API
response and a dashboard at once. The sweep therefore redacts subject and
snippet before the row is written, using the same local deterministic detector
the cloud preview uses (`deterministic-entity-redaction-v2` — links, addresses,
IBANs, phone numbers, long numbers, token-like secrets). A redacted subject
reads `Your verification code is [number]`. The sender is kept: it is what makes
a proposal reviewable when its subject cannot be read. Both sweep entry points —
the CLI and the HTTP API — go through one intake path, because a gate only one
of them uses is a gate over half the traffic. `POST /triage/redact` applies the
same pass to rows stored before this existed; it is idempotent and reports what
kind of thing it removed, never the value.

### Collecting on a schedule

`inbox_sweep_minutes` lets comms-server pull proposals unattended. It ships as
`0`, disabled — a background job that reads a mailbox is opt-in per machine, not
something a fresh clone starts doing. The manual paths are unaffected either
way: `comms sweep --dry-run` previews without storing, and the board's own sweep
button still pages on demand.

Each scheduled pass takes the newest `inbox_sweep_max_threads` threads and stops
— a bound, not a cursor. A cursor advancing every run would walk backwards
through the whole mailbox over days, which is the unbounded rescan the schedule
is meant to avoid; re-reading the newest page instead costs nothing, because
proposals upsert on Gmail thread id and preserve human decisions. Paging deeper
stays a manual, cursor-carrying call. Both the schedule and the manual route run
the same sweep function, for the same reason both go through the intake gate.

`inbox_sweep_quiet_hours` (`"22-7"`, wrapping midnight) suppresses only the
unattended pass. Failures are counted, not narrated: the run records an error
*class* — `auth`, `quota`, `network`, `unknown` — and each consecutive failure
doubles the number of ticks skipped, up to 32, so an expired token backs off
instead of retrying every fifteen minutes all night. Any success clears the
streak. `GET /triage/sweep/status` exposes last run, last *success* (kept
separate, because a failing run still ran), last failure, the error class and
the counts from the last pass; the mail board renders it, and reads visibly
different when the schedule is failing.

Category and TELOS relevance are deliberately separate axes. An explicit
relevance refresh compares the stored sender, subject and Gmail snippet with
the configured TELOS lenses through the existing embedding/reranking pipeline.
Only loopback inference endpoints are accepted for mail; if the local model is
unavailable, the result is truthfully labelled `lexical`. Mail bodies and
attachments are not fetched or scored. Raw relevance values are ranking
signals, not calibrated probabilities, and neither scoring nor a human mail
correction changes a TELOS source note.

The dashboard presents proposed mail as a horizontally scrolling category
board. Each column has a real stored-item count, its own select-all control and
cards ranked by their strongest TELOS match when one exists. Inbox collection
is cursor-paged in batches of at most 100 instead of being capped at the CLI's
25-item default. The cursor is intentionally browser-session state: rescanning
from the newest page is idempotent because proposal upserts preserve human
decisions.

The board is an index, not a second reader. Opening either a normal Feed entry
or a mail proposal resolves the versioned
[`content-item-v1`](../../schemas/content-item.schema.json) contract and uses
the same dashboard reader. The contract owns canonical title, author, optional
summary, source content, relevance, evaluation and provenance. Its nullable
`mail` extension adds category and classification evidence; Gmail mutations
remain separate authenticated actions. This adapter boundary lets collection
tables evolve independently without duplicating content presentation.

## Digests: what the local model wrote, sized to the source

A **digest** is a different noun from a summary. `summary` on `content-item-v1` is what the
*source* said it is — calendar reads it from the entry's own description — so a generated
paragraph written over it would destroy the only verbatim text an entry has. The digest is
stored beside it in `content_digests`, one table for every source, because a digest has none
of the per-item invariants that keep the item tables apart.

The shape follows the length, and nothing else chooses it: under 600 characters no digest is
produced at all, then brief, standard and sectioned, each raising both the structure asked for
and the token ceiling. The full ladder and its reasoning live in
[`libs/summarize`](../../libs/summarize/README.md).

**Improving one is a press, not a re-run of the same thing.** `POST /content/:source/:id/digest`
takes `{"depth":"detailed","focus":["…"]}`. `detailed` moves the shape exactly one rung up that
same ladder — asking a model to "be more detailed" only gets a longer version of the same
guess — and the focus terms are named individually in the prompt, with an instruction to say so
rather than invent when the source is silent on one. Both are stored with the result, so a
differently-shaped digest is explained rather than mysterious, and the automatic pass skips any
row whose `depth` is `detailed`: a model upgrade must not quietly throw away a decision an
operator made. A digest replaces its predecessor; there is no revision history.

That ladder also gives the short floor its escape hatch. The automatic pass skips a 400-character
item as `skipped_short`; an explicit press produces one anyway, because the operator looking at it
can see something the character count cannot. An *empty* source is not that case and stays
skipped either way — a model asked to summarize nothing invents the answer confidently.

### Mail reads a body, and does not keep it

The sweep is unchanged: `format=metadata`, read-only, no body. A **digest** fetches
`format=full` for that one thread, walks the MIME tree to `text/plain` (or strips `text/html`
through the same extractor the article path uses), hands it to the model and drops it. Nothing
writes it. The digest row is the only thing that survives the call, which is the whole point:
mail is distilled into an outcome, never retained as a local copy of the message.

Two gates follow the data class rather than the caller. Only `public` content may use a
non-loopback target, and mail is never `public` by construction — so a mail digest is
loopback-only under the same rule that already governs mail relevance, and a cloud endpoint is
refused outright rather than downgraded. For `vault` content the produced digest passes the same
deterministic detector the sweep runs on subject and snippet **before** it is stored, and the
count is recorded: a model asked to summarize a one-time-code mail will quote the code, and the
digest must not be where it gets published.

`POST /content/digests/refresh {"source":"mail","limit":25}` is the bounded automatic pass. It
is explicit rather than timer-driven for the same reason the inbox sweep ships disabled: a
background job that quietly pulls every body out of a mailbox is not something a fresh clone
should start doing.

### Diagrams

`POST /content/:source/:id/diagram` draws the item as one Mermaid diagram, from the digest when
there is one — a diagram of a long paper drawn from its first 15,000 characters is a diagram of
its introduction. The answer is validated before it is stored: a model asked for a diagram will
cheerfully answer with prose, and prose in a diagram column fails at the reader, which is the
hardest place to work out what went wrong. Same remote refusal as the digest, because a diagram
of a private mail is still that mail.

### Charts, and why a number has to be findable

`POST /content/:source/:id/chart` pulls one set of comparable numbers out of the item's source
and stores it as a table. Most content has none, and that comes back as `skipped_short` rather
than an error: a reader that showed a failure for every ordinary essay would train you to stop
reading the state.

**The blocker was never rendering.** A feed item is prose, so "chart this" is an extraction
problem, and extraction has one failure mode that matters: a model asked for numbers produces
plausible ones. Every value therefore has to appear **verbatim in the source** before it is
allowed into a figure. That gate is deterministic, not a second model grading the first, and it
runs against the same capped text the model was shown — claiming a number is present because it
sits in text the model never saw would make the check a formality. Rows that fail are dropped;
if too few survive, the whole extraction is refused with a count of what could not be found.

**One measure, one series.** The figure palette is the low-chroma print palette the operator's
papers use. Run it through a categorical-separation check and even two of its hues fail: `warm_dark`
against `teal_dark` is ΔE 9.7 for *normal* vision, under the 15 floor, before colour-vision
deficiency is considered. It cannot carry identity, so a chart drawn from it must not need to.
That is also what prose actually yields.

**The form is derived, not chosen.** No second model call picks a chart type: an ordered run of
three or more categories gets a line, everything else bars. One less model output to validate,
and the same answer every time.

What is stored is a **table, not a chart specification**. The dashboard compiles it into
Vega-Lite, so transforms, data URLs and the palette stay out of a model's reach; the model
chooses the numbers and, indirectly, the mark, and nothing else. The figure ships with its
caption and a disclosure holding the extracted values, because an extracted number is a claim
about a source and the table is where you check it.

Calendar entries are digested through the same routes. Comms reads one entry over Calendar's own
`content-item-v1` contract — the bounded cross-capability read it already does against Trips —
rather than opening a second capability's database schema.

## Relationship to scouting

Feed and Scouting are connected but not interchangeable. Feed answers “what changed or may be
worth noticing?” Scouting answers “which opportunities fit the operator's current profile,
and why?”

A feed entry may later be promoted to Scouting when it represents a scholarship, hackathon,
event, call for papers or another opportunity that needs scoring. Most kinds should not take
that path. Security updates, release notes, repository changes and general articles remain
useful observations without becoming opportunities. Conversely, a Scouting run may publish a typed
observation into the feed so the result appears in the daily stream. The cross-capability
link must preserve the source item ID rather than copy its content into an unrelated record.

TELOS relevance is an annotation on this general feed, not a filter that turns it into
Scouting. Explicitly configured `TELOS/Focus/*.md` notes become independent lenses. The
optional single-line `relevance_query` frontmatter owns the semantic search intent; when it is
present, only that value is embedded, keeping file names, UI labels, wiki links and note
scaffolding out of the vector. Notes without it retain the original bounded full-note
representation. The item and every lens are embedded in one batch through the same backend; if
that backend is unavailable, both sides use the same deterministic lexical vector space and the
stored match is labelled `lexical`. Scores are raw ranking signals, not calibrated probabilities.

The Feed's displayed rank is a separate deterministic evaluation, not an LLM judgment.
`feed-evaluator-v2-travel` combines the strongest TELOS match (45%), an explicit upcoming-trip
match (25%), age since first ingest (20%) and the stored content basis — title, author, summary
and source text (10%). Travel matching compares item text with destination names and the plan's
declared interests; the winning factor carries the Trip ID, label, dates and matched terms so
the UI never has to reverse-engineer a prose explanation.

Trips remains the owner. Comms reads `GET /api/plans`, retains a bounded snapshot containing
only upcoming plan identity, title, destinations, date window, interests and revision, and
uses the last successful snapshot while Trips is temporarily unavailable. It does not copy
traveler names or itinerary items. A plan change alters the evaluation context revision;
normal refreshes therefore evaluate changed context once and skip it thereafter. Changing a
Feed status alone still does not invalidate the evaluation.

Obsidian has two bounded roles:

- The HTTP scanner reads only exact Markdown files declared in private configuration. A
  source may require an exact heading; when that heading is missing it returns zero candidates
  and never falls back to the full note. Frontmatter, fenced code, inline-code URLs, image
  targets, private-network hosts and credential-looking URLs are excluded. Scanning does not
  fetch anything. Each candidate still needs an explicit import.
- The existing CLI keeper export can write a distilled reading note for an explicitly kept
  item when `keeper_export_dir` is configured. The dashboard's **Behalten** action currently
  changes status only and must not claim it wrote a Vault note.
- `comms keep <id>` resolves a MAIL id as well. Until it did, a mail had no way out of the
  Inbox except staying in it, so the Information lane was declared rather than real. The note
  carries subject, sender, date, a Gmail permalink, the stream and the data class. It never
  carries the snippet or the body, which are the raw mail this lane exists to avoid copying.
  Every field comes from the stored row, so a Private mail exports the redacted form the intake
  gate produced. No Gmail write happens: archiving is a mutation the doctrine permits only on
  explicit approval, and folding it in here would archive as a side effect of filing.

Neither path is the Trips importer, edits TELOS, or scans Scouting's opportunity notes.

## Why this shape: one local inference server, then native Apple where it earns the seam

Comms does not need both oMLX and Ollama as permanent runtimes. The split was historical:
summaries already used oMLX's OpenAI-compatible chat endpoint, while relevance was first
implemented directly against Ollama's `/api/embed`. Current oMLX releases expose
`/v1/chat/completions`, `/v1/embeddings`, `/v1/rerank` and `/v1/models`, support embedding and
sequence-classification models alongside
LLMs, and manage them through LRU eviction, per-model idle TTLs and a process memory guard.
The intended local-AI rung is therefore one authenticated oMLX process with one generation
model, one much smaller multilingual embedding model and one bounded reranker. See the
[oMLX API and model-management documentation](https://github.com/jundot/omlx#api-compatibility).
The selected model is
[`mlx-community/multilingual-e5-base-mlx` at the audited commit](https://huggingface.co/mlx-community/multilingual-e5-base-mlx/tree/576fdf3eab52a419f6d126a0c4d7c59b3882ffde):
573 MB, 94 languages, XLM-RoBERTa, MIT-licensed, and represented by tokenizer/config data plus
one safetensors weight shard. It replaced the 253 MB E5-small baseline only after the base
model passed the same frozen DE/EN corpus that small failed; both decisions and exact pins are
recorded in `upstreams.toml`. The reranking stage uses the Apache-2.0
[`soichisumi/bge-reranker-v2-m3-mlx` pinned tree](https://huggingface.co/soichisumi/bge-reranker-v2-m3-mlx/tree/b4577f49e18adb53ed9e557192094f69f3dc2c1c),
an XLM-RoBERTa sequence-classification conversion whose inspected tree contains only model,
tokenizer and config artifacts.

The shared `inference.json` makes that protocol choice explicit. Comms asks for
the `embedding`, `reranking` and `summarization` roles; backend URLs, model IDs, role prefixes
and key-file references are declared once there rather than repeated in
`comms.json`:

- `openai` is the preferred path. It posts one revision-cached batch to
  `{base_url}/embeddings`, restores the response by its declared indexes and may use the same
  backend key reference as the summarizer. The role's `query_prefix` and `document_prefix` express
  retrieval-model roles without coupling Comms to E5; the documented candidate uses `query: `
  for TELOS lenses and `passage: ` for feed items.
- `ollama` remains a migration and compatibility path for installations that already have an
  embedding model there. Its request sets `keep_alive: 0`, because a cached background
  enrichment pass benefits more from returning unified memory immediately than from keeping a
  model warm for Ollama's default idle period. See Ollama's
  [keep-alive documentation](https://docs.ollama.com/faq#how-can-i-preload-a-model-into-ollama-to-get-faster-response-times).
  It has no native reranking endpoint, so Comms truthfully keeps semantic scores on that backend.
- An unknown, unavailable or incomplete provider fails to the labelled lexical path. It never
  silently presents a heuristic score as semantic.

Feed relevance is two-stage. One mixed embedding batch selects at most three candidate lenses
per item. The reranker then evaluates those `(lens, item)` pairs jointly in batches of 32. If a
reranking request fails, the whole refresh keeps the embedding scores rather than mixing score
spaces; if embedding fails, the deterministic lexical control remains labelled `lexical`.

The resolved embedding and reranking roles' cache keys are part of the evaluation context
revision. Changing either producing backend or model therefore invalidates the right ledger rows
on the next normal refresh. Relevance input is capped at 1,800 characters, and a stored summary
replaces rather than duplicates the raw transcript; this fits the selected model's 512-token
window without spending local compute on text it would truncate.

On this memory-constrained interactive Mac, oMLX now uses its aggressive memory guard, at most
eight concurrent requests, an embedding batch size of 32, a 60-second idle TTL for both E5
models and 180 seconds for Gemma. Those are machine settings in oMLX, not tracked Axon defaults.
They should be tuned from measured peak memory and latency rather than copied as a universal
configuration.

Apple's system model is the next summarizer experiment, not the evaluator and not an embedding
replacement. The
[Foundation Models framework](https://developer.apple.com/documentation/foundationmodels/)
and Apple's official
[Foundation Models SDK for Python](https://apple.github.io/python-apple-fm-sdk/)
provide local, OS-managed summarization on Apple Intelligence-capable Macs. The experiment
must account for the system model's
[4,096-token session limit](https://developer.apple.com/documentation/foundationmodels/managing-the-context-window):
Comms currently permits 15,000 input characters, so a native provider needs bounded
chunk-and-reduce summarization rather than simple truncation. It must also record the system
model/OS revision because Apple updates the model with the OS.

For relevance, Apple's
[`NLEmbedding`](https://developer.apple.com/documentation/naturallanguage/nlembedding) was the
first native candidate because it is the purpose-built semantic-similarity API. The current
Mac exposes revision-1 German (640-dimensional) and English (512-dimensional) sentence models.
Using each query's language model to rank both German and English candidates failed at 3/6
useful top-1, `0.500` pairwise accuracy and `0.699` mean nDCG. Apple's
[`NLContextualEmbedding`](https://developer.apple.com/documentation/naturallanguage/nlcontextualembedding)
does expose one shared Latin model for both languages. Mean-pooling its documented subword
vectors produced a fast, low-process-memory comparison, but failed the unchanged corpus at
4/6 useful top-1, `0.618` pairwise accuracy and `0.776` mean nDCG. Five of six top results
matched the query language, including four weaker same-language candidates ahead of the
intended cross-language match. It is therefore not a Comms replacement. The deterministic
evaluator and lexical fallback remain the control in every run.

### First embedding smoke test

On 2026-07-29 the exact registered E5 snapshot was loaded through oMLX 0.5.0 and called through
the authenticated `/v1/embeddings` endpoint. It returned four 384-dimensional vectors for
synthetic German/English inputs. One German local-AI query scored `0.876` against the matching
English passage and `0.845` against the matching German passage, versus `0.753` for an
unrelated ceramics passage. This proves protocol/model compatibility and gives a first
cross-language separation signal; it is deliberately not presented as a quality benchmark.
It therefore did not decide adoption; that required the fixed, manually judged item–TELOS set
described above. With this initial E5-small setup, the first normal 30-day Comms refresh
evaluated 35 stored items semantically against five configured lenses with zero lexical
fallbacks. Repeating the same non-forced refresh evaluated zero items and skipped all 35 as
revision-current, confirming that the normal cache path does not spend a second model pass.

The repeatable public baseline lives in [`eval/`](eval/README.md): six manually judged,
synthetic DE/EN rankings, explicit 0–3 rationales, fixed acceptance thresholds and a runner
that calls only the loopback oMLX endpoint. It complements rather than exports the private
TELOS/Feed evaluation. E5-small failed at 4/6 useful top-1, `0.765` pairwise accuracy and
`0.865` mean nDCG. E5-base passed without changing the corpus at 6/6, `0.882` and `0.986`,
respectively. Its observed 30-vector run took 1.14 seconds; the local model uses 547 MiB on
disk and raised oMLX RSS to about 1.16 GB immediately after the batch, bounded by a 60-second
per-model idle TTL. The compiled Apple contextual runner took 0.75 seconds and peaked near
85 MiB RSS for the same 30 vectors; `NLEmbedding` took 0.67 seconds and peaked near 73 MiB.
Both resource wins are outweighed by their large quality losses.
After E5-base became the configured model, the first normal refresh semantically evaluated all
35 stored items; the immediate second non-forced refresh evaluated zero and skipped all 35 as
revision-current.

## Gmail mutation boundary

Every sweep is **strictly read-only**. The Google endpoints used by the sweep are:

- OAuth token refresh — `POST https://oauth2.googleapis.com/token`
- list inbox threads — `GET .../gmail/v1/users/me/threads?q=in:inbox`
- thread metadata — `GET .../gmail/v1/users/me/threads/{id}` (`format=metadata`)

`comms sweep --dry-run` and `comms sweep` are equally read-only against Gmail;
`--dry-run` only controls whether proposals are written to the local store.

Two writes exist only behind the authenticated Comms mutation router and only
for a thread already present in the local proposal store:

- Archive removes the `INBOX` label through `POST .../threads/{id}/modify`.
- Move to Trash calls `POST .../threads/{id}/trash`; it does not permanently delete.

The dashboard requires a second confirmation for either Gmail write. Dismissing
a proposal in Axon changes only local state and is labelled accordingly.

## Commands

```
comms sweep [--limit N=25] [--dry-run]   # read-only inbox triage proposals
comms ingest <url>                       # ingest any supported URL (see Extractors)
comms feed [--stream news|media] [--days N=7] [--include-dismissed]
comms keep <id> | dismiss <id>           # feed item: set status (+ export if configured)
                                         # mail: write the distilled note; no Gmail write
comms summarize --pending                # retry summaries for items that lack one
comms --help
```

`comms keep <id>` additionally writes a distilled markdown note (title, URL,
date, summary — never the raw transcript) when `keeper_export_dir` is
configured, refusing to overwrite an existing file.

Media ingest shells out to `yt-dlp` (metadata + subtitles) via argument arrays
only, downloads subtitles into a temp dir that is always removed, and never
persists raw audio/video. Summaries come from the shared `summarization`
inference role through an OpenAI-compatible chat-completions endpoint; if the
role is absent or unreachable the ingest still succeeds with `summary` left null (fill it later
with `comms summarize --pending`).

## Extractors

A URL is routed by host and path. Only http(s) is accepted, checked before any
extractor runs: `file://` would otherwise make yt-dlp and the article fetcher
read the local disk, and this path is reachable over HTTP.

| Source | Matches | Title / author | Transcript |
|---|---|---|---|
| `youtube` | youtube.com, youtu.be | yt-dlp metadata | subtitles (de/en), VTT stripped |
| `instagram`, `podcast` | instagram.com; `.mp3`/`.m4a`/"podcast" | yt-dlp metadata | subtitles when present |
| `github` | github.com/`<owner>`/`<repo>` only | repo description; owner | stars/language/license/topics + raw README |
| `arxiv` | arxiv.org `/abs/` or `/pdf/` | paper title; authors (3 + et al.) | the paper if readable, else the abstract |
| `reddit` | a `/comments/<id>` permalink | post title; `u/<author>` | selftext + top-level comments |
| `article` | everything else | `<title>`; none | visible text, tags stripped |

A deeper GitHub path (an issue, a blob, a profile) is an `article` on purpose:
the generic path already renders those, while the repo API answers what no HTML
strip can. arXiv keeps the version suffix, because v1 and v2 are different
papers to a reader.

### Fetching is not extraction

Routing and protocol live in `media.rs`; turning bytes into text lives behind
`extraction::Extractor`, one implementation per input class (`html`, `pdf`,
`text`). The two were the same function until #77, which is how the same HTML
stripper ended up written twice with two different bugs.

**An arXiv item stores the paper, not its abstract.** arXiv renders LaTeX
submissions to HTML at `/html/<id>`, backfilled classics included, so
`fetch_arxiv` reads the paper through the HTML implementation already here —
no PDF conversion and no new dependency. HTML beats a PDF for this on its own
merits: LaTeXML keeps document structure, while PDF extraction has to
reconstruct reading order from a two-column layout and mangles maths doing it.

A 404 there is a clean signal rather than a judgement about a bad conversion,
and it falls through to [ar5iv](https://ar5iv.labs.arxiv.org), the older
LaTeXML pipeline with far wider backfill. Measured 2026-08-04 over the 24
newest `cs.AI`/`cs.LG`/`cs.CL` papers: arxiv.org served 21, ar5iv answered all
three misses plus a 2007 paper, with real body text. It is labs-grade, so it is
the fallback and never the first request.

What is left after both is papers with no LaTeX source at all, scans and
PDF-only submissions, mostly old. Those fall through to the PDF, read by
[xberg](https://github.com/xberg-io/xberg) `=1.0.5` behind the same trait
(#77). Failing all three, the abstract is stored. No raw PDF is persisted.

**The PDF rung is last for a measured reason.** On arXiv 2608.02599 (2026,
two-column) xberg returns correct text in the wrong reading order, columns
interleaved line by line. On arXiv 0704.0001 (2007, Type1 fonts) it drops
every `c`: "Abstrat", "Mihigan", "quantum hromo dynamis", in 11.7s against
0.3s for the same paper as HTML. Degraded full text still beats an abstract
for retrieval, and it is worse evidence than LaTeXML output, so the item
records which extractor produced it rather than leaving the two
indistinguishable.

Which route ran is recorded per item as `transcript_source` (`full-text` |
`abstract` | `unknown` for rows predating the distinction).

`transcript_source` is a separate axis from `content_status`, which measures how
much text there is: a long abstract is `full` + `abstract`, a one-line article
is `thin` + `full-text`. It is a column rather than an `Abstract:` prefix inside
the body because a prefix has to be parsed back out by every reader, and the
embedder would score it as if the paper had said it (#78).

**Reddit needs credentials and does not have them.** Verified 2026-07-28: the
`.json` view answers 403 to every unauthenticated caller, on `www` and `old`,
with a descriptive UA, an API-format UA and a browser UA alike. The parser is
written against the shape that endpoint still returns for an authorized caller;
reaching it needs a registered Reddit app and an OAuth token against
`oauth.reddit.com`, which is a secret the operator provisions (README.md#secrets). Until then a Reddit paste fails loudly instead of storing an empty item.

**A site that publishes its own text is a rung nobody has built.**
[llms.txt](https://llmstxt.org) proposes that a site expose a curated index of
its content beside `robots.txt`, with `llms-full.txt` carrying the text itself.
Were it adopted here it would sit ahead of the HTML rung for the sites that
publish one, on the same argument the arXiv HTML rung already makes against the
PDF: reading what an author curated beats reconstructing what a renderer emitted.

It is an idea, not a plan. There is no `upstreams.toml` verdict, nothing is built
on it, and the proposal's adoption in the wild is not measured here. It is
recorded beside the routing it would change rather than in a standalone link
file, because a link nobody re-reads is a link nobody acts on.

## Feed source collectors

`feed_sources[]` is the general-awareness source registry. `github-trending`
reads the bounded daily/weekly/monthly Trending page, extracts repository
identity, description and visible momentum metadata, and preserves the Trending
page as provenance. `arxiv` calls the official Atom query API with a configured
`search_query`, newest submissions first. Both clamp a run to at most 30 items.

A scan upserts by the canonical target URL, preserving `keeper`/`dismissed`
state, then records `feed_origins` and `source_state`. Re-seeing an existing item
is reported as known rather than new. Background enrichment compares the
item/context/evaluator revisions before embedding or evaluating, so an unchanged
second scan does not spend another model pass.

## comms-server

Axum HTTP API (dashboard-origin-only CORS), port from `AXON_PORT`
(runner-exported) → config → default **8083**. JSON contract consumed by a
dashboard panel.

`src/server/main.rs` is the composition root: configuration, route assembly,
loopback bind, and shutdown. Its sibling modules own one reason to change each:
`auth.rs` owns the shared-secret boundary; `contracts.rs` owns response
projections; `error.rs` owns the stable JSON error shape; `feed.rs`,
`content.rs`, `cloud.rs`, `source_handlers.rs`, `vault.rs`, and `triage.rs` own
their existing HTTP contracts; and `background.rs` owns the periodic task
lifecycle. `BackgroundServices` retains every task handle and aborts its tasks
when the server lifecycle ends. Blocking storage, inference, and network work
continues to cross `spawn_blocking` inside the owning workflow.

Routes:

- `GET /feed?stream=&days=&include_dismissed=` → feed items (no `transcript`)
- `GET /feed/:id` → one reader item incl. `transcript`, every stored TELOS relevance match,
  the factorized evaluation and Vault provenance
- `GET /content/:source/:id` where source is `feed` or `mail` → the shared versioned
  content reader contract, including the stored `digest` when there is one. Reads only: a GET
  that quietly runs a local model turns opening an item into a two-minute wait. Mail supplies
  Gmail's bounded snippet as `content` and leaves `summary` null; a preview is not a generated
  summary, and the digest beside it says which is which.
- `POST /content/:source/:id/digest` `{"depth":"standard"|"detailed","focus":["..."]}` →
  generate or refine the local digest and replace it in place. Both fields are optional; an
  unknown `depth`, or a body that is not readable JSON, is a `400` rather than a quiet fall back
  to the default. Synchronous, unlike `POST /ingest`: this is a button the operator is watching.
- `POST /content/:source/:id/diagram` → one validated Mermaid diagram, drawn from the digest
  when there is one. A model answer that is not a diagram is a typed rejection, not a stored
  string.
- `POST /content/:source/:id/chart` → extract one set of comparable numbers as a table, every
  value verified verbatim against the source. `skipped_short` when the content holds none, which
  is the answer for most prose.
- `POST /content/digests/refresh` `{"source":"mail"|"feed","limit":25}` → the bounded automatic
  pass over items with no digest, a stale producer, or a retryable failure with attempts left.
  It never touches a row an operator refined.
- `POST /content/:source/:id/cloud-preview` → builds a bounded local preview. Public content
  is copied as-is; Personal and Private content receives local deterministic entity redaction
  for recognized people after salutations, addresses, links, phone/account numbers and
  token-like secrets. The response lists the recognized entity types, names the limitations,
  and always reports zero provider calls. A Private preview is inspectable local evidence only;
  it cannot be approved, queued or dispatched to cloud processing.
- `POST /content/:source/:id/cloud-approval` `{"preview_hash":"..."}` → regenerates the
  current preview, rejects a stale hash, and stages the exact reviewed derivative locally.
  It does not select or contact a cloud provider.
- `GET /content/cloud-providers` → lists only explicit `cloud_*` inference roles backed by
  non-loopback HTTPS endpoints with a reviewed provider name, data tier and billing boundary.
  It exposes safe role/model/policy metadata, daily usage/remaining calls and an availability
  reason, never the endpoint, account ID, key-file path or key value.
- `POST /content/:source/:id/cloud-queue`
  `{"preview_hash":"...","provider_role":"cloud_summarization"}` → validates the exact
  derivative against the role's reviewed data tier and materialized credential, then idempotently
  records a queue job. Queueing is local and performs zero provider calls.
- `POST /content/cloud-jobs/:job_id/run` → explicitly sends only the staged, hash-approved
  derivative to the selected role first, then only to configured same-tier fallbacks in stable
  priority order. It runs the fixed
  `content-analysis-v1` task, validates and bounds the structured response, and persists the
  result or a safe retryable error. Credentials, credit expiry, input-token upper bound and the
  UTC daily request ceiling are revalidated immediately before every attempt. A policy-disabled
  role causes zero provider requests. Every actual request records role, model, exact approved
  derivative hash and bounded error/result provenance. Jobs never run automatically and stop
  after five total provider calls. The original content is not loaded into the dispatch path.
  A completed result still performs no Calendar write: each resolved date or dated action has a
  separate reader action that creates or refreshes one non-blocking Calendar proposal through
  Calendar's external-entry contract.
- `GET /feed/evaluation/status` → configured local model names, cheap endpoint reachability,
  TELOS profile count, active semantic/lexical mode and persisted ledger counts; no secret or
  API-key value is returned
- `POST /feed/relevance/refresh` `{"days":90,"force":false}` → inspect at most 200 stored
  items and evaluate only missing or revision-stale rows against the configured TELOS lenses.
  The response separates `considered`, `evaluated` and `skipped_current`. `force:true` is an
  explicit maintenance override, not the dashboard default.
- `POST /feed/:id/status` `{"status":"keeper"|"dismissed"|"new"}` → sets a feed
  item's status (validated)
- `POST /ingest` `{"url":"..."}` → `201` with the stored item, or `400` with a
  reader-facing `error`. Answers as soon as the item is stored and summarizes
  behind the response, so a fresh item legitimately comes back with
  `summary: null`; it appears on the next feed read. A paste box that blocks for
  the summarizer's two minutes is a paste box nobody uses.
- `POST /vault-links/scan` → metadata-only candidates from declared Vault sources
- `POST /vault-links/import` `{"source_id":"...","url":"..."}` → re-validates the
  candidate against the current source, fetches it, stores provenance and then enriches it
- `GET /sources` → enabled/configured general Feed collectors and last-run state; personal
  query text is not returned
- `POST /sources/scan` `{"source_id":"github-trending-daily"}` → scan one enabled
  collector; `{"source_id":null}` scans all. Returns per-source fetched/new/known counts and
  enriches only revision-stale items behind the response.
- `GET /triage?status=proposed` → triage items
- `POST /triage/sweep` `{"limit":100,"cursor":null}` → fetches one read-only inbox page,
  stores new proposals, and returns the opaque cursor for the next page
- `POST /triage/relevance/refresh` `{"limit":200}` → scores stored pending mail against
  TELOS through loopback-only local inference or the labelled lexical fallback; TELOS is read-only
- `POST /triage/data-class/refresh` `{"limit":500}` → locally classifies stored pending mail
  from sender, subject and category only; no provider is called and human overrides are preserved
- `POST /triage/bulk` applies one reviewed action to at most 100 stored proposals and reports
  per-item failures. `categorize` uses `stream`, `set-data-class` uses `data_class`, and
  `dismiss`, `archive`, or `trash` need only the selected `ids`.
- `POST /triage/:id/status` `{"status":"proposed"|"approved"|"dismissed"}`
  → local proposal state only; Gmail lifecycle states cannot be forged through this route
- `POST /triage/:id/stream` `{"stream":"aktiv"|"issue"|"feed"|"werbung"|"belege"|"steuern"|"sonstiges"}`
  → records a persistent human category override without resolving the proposal
- `POST /triage/:id/data-class` `{"data_class":"public"|"personal"|"vault"}`
  → records a persistent human trust-class override; the dashboard labels `vault` as **Private**
- `POST /triage/:id/gmail` `{"action":"archive"|"trash"|"restore"}` → performs the
  explicit action through a durable local intent. Replays first check Gmail's metadata labels,
  so a process failure after Gmail success can finish locally without duplicating the mutation.
  Trash retains its local copy for 30 days; restore returns either state to Inbox.
- `POST /triage/:id/gmail-job` `{"decision":"retry"|"cancel"}` → explicitly reopens the
  bounded retry window or cancels an action after five automatic failures. Queued work cannot be
  canceled while it may be in flight.
- `POST /triage/reconcile` → retries due Gmail intents and compares up to 200 stored threads with
  Gmail metadata labels. It updates Axon Archive/Trash/Inbox state without fetching bodies or
  attachments. Gmail 404/410 becomes an explicit Missing state that retains Axon's local record;
  a Trash cleanup deadline remains active. The server runs the same bounded maintenance on its
  configured interval.
- `GET /health` → liveness. Answers from the process alone, so a start completes without a
  database and a Postgres outage does not read as a crash.
- `GET /ready` → readiness: liveness plus a reachable database, `503` when it is not. This is
  what `axon-status` judges availability on, and what the dashboard reports.

Binds `127.0.0.1`, not `0.0.0.0`: `/ingest` makes the server fetch a URL on
request, so anything that can reach the port can use it to reach whatever the
host can. Remote access belongs in front of the process (Tailscale), not in an
open bind.

## Configuration

Resolved in order (mirrors scouting):

1. `$AXON_COMMS_CONFIG` (explicit path to a JSON file)
2. `$AXON_PERSONAL_ROOT/config/comms.json` (the private overlay)
3. `capabilities/comms/comms.config.json` (gitignored dev fallback)

Every field is optional; the tool runs zero-config against the shared local
Postgres with only the built-in classification heuristics. Fields:
`database_url` (unset → built from `axon-overlay/config/postgres.env`),
`google_env_path` (default `$AXON_PERSONAL_ROOT/config/comms.env`), `port`,
`gmail_maintenance_minutes` (default `15`; `0` disables the automatic pass),
`relevance {profile_paths}`. Model roles come from the overlay's shared
`inference.json`; see `libs/inference/inference.config.example.json`. The active producer revision
includes both embedding and reranking roles, so a model change invalidates stale evaluations. A
profile note may define a single-line `relevance_query` in
its frontmatter; this is the inspectable embedding input and changes that profile's revision,
while `summary`, `current_focus` and `category_affinity` remain reader-facing metadata,
`vault_link_sources[] {id, path, heading?, enabled}`,
`feed_sources[] {id, adapter, enabled, query?, language?, since?, limit}`,
`rules[]`, `keeper_export_dir`. See
`comms.config.example.json`. Nothing personal lives in this repo — sender
addresses and personal rules belong in the overlay.

Mutating HTTP routes require the shared token referenced by `api_secret_file`.
The local dashboard never puts that token in its browser bundle: its Vite proxy
resolves the same config at startup, rejects cross-origin mutations, and adds the
bearer header on the server side. Restart both `comms` and `dashboard` after a
token rotation. Browser-extension and direct HTTP clients must supply the token
themselves.

Google credentials (`GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`,
`GOOGLE_REFRESH_TOKEN`) live in the overlay `comms.env`; mint the refresh token
once with `auth/get-refresh-token.ts` (bun). Token values are never logged, and
the DB connection string is redacted before any display.

## Schema (`comms` schema, shared instance)

`src/store.rs` is the stable `Store` facade and owns the public persistence
types. Its implementation lives in workflow modules under `src/store/`:
migrations and pool access; triage and Gmail actions; reviewed cloud jobs and
digests; feed ingest and relevance; evaluation and travel context; capture
origins; collector state; and database-row mapping. SQL stays beside the
workflow that owns it, while callers continue to depend only on `Store`.

- `triage_items` — one row per inbox thread: `id` (gmail thread id) PK,
  `from_addr`, `subject`, `snippet`, `internal_date`, `stream` (CHECK), `rationale`,
  `status` (`proposed`/`approved`/`archived`/`trashed`/`missing`/`dismissed`; `executed` is retained
  for legacy rows), `gmail_action`, `gmail_action_at`, `purge_after`, `first_seen`, `last_seen`,
  observed Gmail location/time and durable sync state/error, plus trust-class value,
  rationale, method and classifier revision. Human trust-class
  overrides survive later rule refreshes.
- `gmail_action_jobs` — content-free durable intent and bounded retry ledger for Archive, Trash,
  and Restore. Only one queued action may exist per thread; five failed attempts move it to the
  explicit attention state. An operator can retry that exact action or cancel it; canceled history
  remains auditable without blocking a later action.
- `triage_relevance` — additive per-thread/per-lens ranking rows containing the raw score,
  rationale, truthful scoring mode, profile revision and scoring time. Replacing these rows
  never changes the proposal category/status or its TELOS source.
- `content_cloud_derivatives` — the latest explicitly approved bounded derivative per source
  item, including source revision, preview hash, original/derivative class, transformation,
  redaction count and approval time. It is local staging state.
- `content_cloud_jobs` — idempotent provider intent and execution ledger for an exact approved
  derivative. Jobs move through queued/running/succeeded/failed only after an explicit request,
  count at most five provider calls, and retain a bounded structured result or safe error. No
  background worker sends queued content automatically.
- `content_cloud_attempts` — one immutable-identity row per actual provider request, including
  sequence, role, model and approved derivative hash plus bounded result/error provenance. It is
  also the local UTC-day usage ledger; candidates rejected by credentials, expiry or ceilings
  create no row because they made no provider request.
- `feed_items` — one row per ingested URL: `id` (sha256 of canonical URL) PK,
  `stream` (`news`/`media`), `kind` (`youtube`/`instagram`/`podcast`/`article`/
  `mail`/`github`/`arxiv`/`reddit`), `title`, `url`, `author`, `summary`,
  `transcript`, `day`, `created_at`, `status` (`new`/`keeper`/`dismissed`).
  Widening that `kind` set needs a `DROP CONSTRAINT` + re-`ADD` migration, not
  an edit to the `CREATE TABLE`: `IF NOT EXISTS` never touches an installed
  table's constraints, so the edit alone would work on a fresh database and
  silently reject the new kinds on every existing one.
- `content_digests` — one row per `(source, item_id)`: the generated text, its state, the rung
  and depth that produced it, the operator's focus terms, the producer, how much source was
  measured, the redaction count, the retry ledger, the Mermaid columns and the extracted chart
  table. One table rather than
  a column on each item table: a digest is derived data with the same axes everywhere, so three
  migrations would buy only drift. **No raw source is kept here** — a mail body is fetched,
  digested and dropped inside one call, and this row is all that survives it.
- `source_state` — per-source run bookkeeping.
- `feed_relevance` — additive per-item/per-lens match rows: raw score, rationale,
  truthful scoring mode and a profile revision fingerprint. Refresh replaces only relevance;
  it never changes the feed item's triage status.
- `feed_evaluations` — one revisioned, inspectable overall evaluation per item: normalized
  score, deterministic explanation, scoring mode and the item/context/evaluator cache key.
- `feed_evaluation_factors` — ordered normalized factors with label, value, weight and
  rationale. It is normalized rather than JSON so later trip/deadline factors remain queryable
  and individually inspectable.
- `feed_quality_flags` — the latest deterministic review signals per item. Each row stores the
  signal name, human-readable reason, inspectable evidence and derivation time. An explicit
  refresh replaces an item's complete set; the read route only returns stored rows.
- Stage provenance stays beside the value it describes: extraction tier/revision in
  `feed_raw_content`, normalization and summary tier/revision in `feed_items`, and ranking tier
  beside the existing revision tuple in `feed_evaluations`. Tiers are ordered
  `legacy < deterministic < model < human`; equal-tier reruns replace idempotently and a lower
  tier cannot downgrade the stored value.
- `feed_origins` — one row per exact Vault source/reference that introduced an item. An item
  may retain multiple origins without duplicating its canonical feed row.

Both item tables use a **status-preserving upsert**: `status` is set only on
first insert and is deliberately absent from the `ON CONFLICT DO UPDATE`, so a
human's triage/keeper decision survives the same item being re-swept or
re-ingested (`upsert_preserves_status_across_refetch_*` tests prove it).
The reader shows the human keeper/dismiss verdict above every processing
producer; migrated values whose producer cannot be recovered are labelled
`legacy-unknown` rather than assigned a guessed model or revision.

## Computed quality review

`POST /feed/quality/refresh` examines at most 500 stored items and replaces their review flags;
`GET /feed/quality` reads the resulting queue. The dashboard exposes both under **Feed → Review**.
The computation reads only stored content status, capture/extraction provenance, raw and canonical
text, summary retry state, and whether a ranking row exists. It never calls an inference provider,
never treats a model's self-confidence as evidence, and never changes the human Feed status.

The public defaults in `comms.config.example.json` come from the frozen extraction corpus rather
than an intuition: its passing input classes retain 39.7–89.6% of total raw text, preserve
100% of judged useful text, and leak 0% of judged boilerplate. Operational items do not have
per-item human judgements, so the review signal deliberately uses the rounded 39–90% observed
total-retention envelope and a 0% residual-normalizer-rule threshold as suggestions for inspection,
not correctness verdicts. Two failed summary attempts warn before the existing three-attempt cap.

**One fixture class starts before extraction.** Every other one stores the text an extractor
already produced, which is why the gate could not see the defect it exists to catch: the real HTML
path emitted a single line, every normalization rule is guarded by a max line length, and the
corpus reported 0% leakage while production stored consent walls verbatim as article bodies. The
`html` class holds a page (`eval/fixtures/page.html`) and runs extraction then normalization, so a
change to either is scored end to end — and a replacement extractor (#77) can be measured against
the built-in one on the same pages.

## Tests

`cargo test -- --list` counts them; the hand-written number here was wrong twice, so it is
gone (README.md#documentation-stays-owned-and-current). The store tests need the shared local Postgres
(`capabilities/postgres`) reachable; they isolate into a per-pid schema and take
the connection from `COMMS_TEST_DATABASE_URL`, falling back to the same
`Config::load()` the binaries use, so a rotated password can't leave them behind.

They resolve it exactly once, and the config test that clears
`AXON_PERSONAL_ROOT` restores it on drop. Both are load-bearing: Rust runs a
crate's tests as threads of one process, and an unrestored `remove_var` left
eight store tests failing against a healthy Postgres, reading as a credential
problem that did not exist. The same fix applies in `scouting` and `transit`.
