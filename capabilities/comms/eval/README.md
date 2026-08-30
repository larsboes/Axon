# Comms quality evaluation

## Extraction and normalization

`extraction-corpus.json` is the frozen, offline gate for the text passed from each
extractor class into normalization. Its synthetic stored-page snapshots cover an
article, repository, paper, client-rendered page, captured page and PDF text. Each
fixture names exact text that must survive and exact boilerplate that must not.

Run it from `capabilities/comms`:

```sh
cargo run --bin comms-extraction-eval -- eval/extraction-corpus.json
```

The runner reports raw and normalized character counts, total retention, useful
retention and boilerplate leakage per fixture. The fixed gate requires 100% of
the judged useful text and 0% of the judged boilerplate; a miss exits non-zero.
It also requires every declared input class and every inspectable normalization
rule to have fixture coverage. A host adapter change refreshes or adds its stored
snapshot before implementation; a normalization rule change adds its expectation
to a fixture. Change judgements before examining the new result, never to make a
failed implementation pass. Append every accepted baseline under `results/`.

The snapshots begin at the extractor/normalizer seam, not at the network. They
therefore remain deterministic and measure the canonical text contract while the
live fetchers keep their separate HTTP tests. The PDF fixture is stored extracted
text because xberg adoption and byte-level PDF extraction remain separately
blocked by their dependency cooldown.

## Relevance

This directory owns the small public quality baseline for semantic Feed–TELOS ranking. It is
separate from unit tests: the runner calls the real local oMLX server, while the committed
corpus contains only synthetic text and explicit human judgements.

Run it from the Axon root:

```sh
bun capabilities/comms/eval/run-relevance.ts
```

The cross-encoder candidate is gated against the same corpus and unchanged judgements:

```sh
bun capabilities/comms/eval/run-reranking.ts
```

That runner calls the loopback `/v1/rerank` route once per lens and validates the returned index
and closed `0..=1` score contract. `OMLX_RERANKING_MODEL` selects another installed model. The
embedding baseline remains the candidate-retrieval gate; the reranking run deliberately isolates
the second stage without rewriting the corpus around a model's scores. A disposable, loopback-only
test server with authentication disabled may set `OMLX_NO_AUTH=1`; the normal path still requires
the configured key reference.

The runner reads `.auth.api_key` from `~/.omlx/settings.json`, sends one batch to the loopback
`/v1/embeddings` endpoint and never prints the key or vectors. `OMLX_SETTINGS_PATH`,
`OMLX_BASE_URL` and `OMLX_EMBEDDING_MODEL` override machine-specific details; E5-base is the
default. A non-loopback endpoint is rejected because this evaluation is intentionally local. A
loopback server that enforces no key of its own may set `OMLX_NO_AUTH=1`, the same spelling
`run-reranking.ts` uses; the normal path still requires the configured key reference.

An explicit first argument selects another schema-compatible corpus. Private real-world corpora
and their results stay in the private overlay and are never copied into this directory:

```sh
bun capabilities/comms/eval/run-relevance.ts "$AXON_PERSONAL_ROOT/config/private-corpus.json"
```

A query may provide named `text_variants`. Setting
`RELEVANCE_EVAL_QUERY_VARIANT=<name>` evaluates that representation while keeping candidates,
judgements and acceptance thresholds unchanged. This exists for controlled input-shape
experiments; a missing variant is an error, and variants must never be used to rewrite
judgements after seeing model scores. Candidate IDs need to be unique within a query, not across
queries, so one real Feed snapshot can be judged independently against multiple TELOS lenses.

Apple's built-in sentence embeddings run against the same corpus:

```sh
xcrun swift capabilities/comms/eval/run-apple-nlembedding.swift
```

For every query, the runner chooses Apple's German or English sentence model and asks that
single model to rank both German and English candidates. This makes cross-language behavior an
observed result rather than assuming that separate language-specific vector spaces align. It
preflights both languages and records each system model's revision and dimensions because macOS
may update those assets. On this machine the German revision-1 model has 640 dimensions and the
English revision-1 model has 512. Both run, but neither preserves the corpus's intended
cross-language ordering.

Apple's shared Latin contextual model is the executable native comparison:

```sh
xcrun swift capabilities/comms/eval/run-apple-contextual.swift
```

It declares [both German and English in one Latin vector space](https://developer.apple.com/documentation/naturallanguage/nlcontextualembedding/languages).
The runner mean-pools Apple's
[subword vectors](https://developer.apple.com/documentation/naturallanguage/nlcontextualembeddingresult/enumeratetokenvectors%28in%3Ausing%3A%29)
into one vector per text, passes each text's actual language and applies the unchanged acceptance
gate. Missing system assets are never downloaded implicitly; an intentional first run may ask
macOS for them:

```sh
xcrun swift capabilities/comms/eval/run-apple-contextual.swift --request-assets
```

If Command Line Tools and the active Swift SDK do not match, select the installed full Xcode
for either command with
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`. The native comparison is
experimental: Apple's contextual API supports pooling, but the sentence-embedding API is its
purpose-built semantic-similarity surface.

## Why this shape: judgements before scores

Each query has four candidates rated from 0 (unrelated) to 3 (direct match), with a written
rationale. The cases balance German and English and make the strongest match cross-language.
Acceptance requires every top-ranked candidate to be useful, at least 75% correctly ordered
unequal pairs and mean nDCG of at least 0.90.

Add or revise a judgement from its meaning and rationale before looking at a model's score.
Never tune a label merely to turn a failing model green. Private TELOS notes, real Feed items
and their result records belong in the private overlay, not this public baseline.

Results are append-only evidence under [`results/`](results/). The first run,
[`2026-07-30-multilingual-e5-small-mlx.md`](results/2026-07-30-multilingual-e5-small-mlx.md),
fails the fixed acceptance gate and records the two ranking errors instead of weakening the
judgements. The larger
[`multilingual-e5-base-mlx` run](results/2026-07-30-multilingual-e5-base-mlx.md) then passes
the unchanged gate and records the measured resource delta that justified promotion. Both
native Apple variants are retained as tested failures:
[`NLEmbedding`](results/2026-07-30-apple-nlembedding.md) uses the language-specific sentence
models, while the shared [`NLContextualEmbedding`](results/2026-07-30-apple-contextual.md)
runs after an explicit asset request and mean-pools subword vectors. Neither clears the same
corpus.

The second-stage
[`bge-reranker-v2-m3-mlx` run](results/2026-08-04-bge-reranker-v2-m3-mlx.md) uses the same
corpus unchanged and passes at 6/6 useful top-1, `0.912` pairwise accuracy and `0.993` mean nDCG.
Its result is kept separately because cross-encoder scores and embedding cosine scores are not
the same scale.

A second first-stage candidate cleared the unchanged gate on 2026-08-30:
[`bge-m3` served by Ollama](results/2026-08-30-bge-m3-ollama.md), at 6/6 useful top-1, `0.912`
pairwise accuracy and `0.987` mean nDCG. It is recorded as measured and **not** as adopted —
which backend serves the `embedding` role is a deployment fact in the overlay. That record also
carries why it was run: the configured oMLX backend is absent from the host, so the ranking had
been falling back to the deterministic `lexical` control.
