# Comms relevance evaluation

This directory owns the small public quality baseline for semantic Feed–TELOS ranking. It is
separate from unit tests: the runner calls the real local oMLX server, while the committed
corpus contains only synthetic text and explicit human judgements.

Run it from the Axon root:

```sh
bun capabilities/comms/eval/run-relevance.ts
```

The runner reads `.auth.api_key` from `~/.omlx/settings.json`, sends one batch to the loopback
`/v1/embeddings` endpoint and never prints the key or vectors. `OMLX_SETTINGS_PATH`,
`OMLX_BASE_URL` and `OMLX_EMBEDDING_MODEL` override machine-specific details; E5-base is the
default. A non-loopback endpoint is rejected because this evaluation is intentionally local.

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
