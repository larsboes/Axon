# libs/inference

One home for **which model answers which job on this machine**.

A shared library, not a capability: no domain of its own, no upstream verdict, no CLI
(README.md#three-architectural-nouns). Consumers compile it in with a `#[path]` include rather than a cargo
path dependency, for the same reason `libs/axon-config` does.

## Why it exists

The same fact had four homes: `comms`' `SummarizerConfig` and `RelevanceConfig`,
`scouting`'s `EmbedConfig`, `libs/ai-client`'s `RouterConfig`, and `tools/graphify.sh`'s
`GRAPHIFY_BACKEND`. Each knew a base URL and a model name, and moving a machine between
runtimes meant editing all of them.

`systems.toml` already stated the rule, in the oMLX entry:

> Referenced by id, not by URL, because host/port/model differ per machine.

This implements it.

## The shape

Two levels. Callers only ever touch the second.

- A **backend** is a server: an API shape (`openai` or `ollama`), a base URL, optionally a
  file to read a bearer key out of. Declared once.
- A **role** is a job: `embedding`, `summarization`. It names a backend, the model on it,
  and that model's input conventions.

```rust
let role = InferenceConfig::load(overlay_config).role("embedding");
let vectors = role.embed(&texts, TextRole::Query)?;
```

A capability asks for a role and never learns whether it just talked to oMLX or Ollama.
That is the point: oMLX needs Metal and cannot exist on the family Pi, Ollama runs
anywhere, and moving between them is a config edit rather than a code change.

## Implementation

`src/lib.rs` serves the Rust capabilities that need model roles (scouting today). It reads
`inference.json`, honours the backend override, and resolves bearer keys from the referenced
private file. Consumers receive a resolved role without hardcoding a URL or model.

## Config

`<overlay>/config/inference.json`, or `AXON_INFERENCE_CONFIG` to point somewhere else.
Field docs live in `inference.config.example.json` beside this file.

A missing config is not an error. Every consumer is expected to degrade to something that
still works offline — scouting falls back to hash embedding — so a machine with no
inference set up keeps running instead of failing at startup.

## Machine override

`AXON_INFERENCE_BACKEND` replaces the backend for every role. `service-runner.sh` exports
it from `machine.toml`'s `[inference] backend`, the same path `[capability.<name>] port`
already takes to reach a process. A Linux or Raspberry Pi machine says so once, in the file
that already holds machine-local facts, and no capability config changes.

## Two things portability actually requires

**Models are not interchangeable.** `multilingual-e5-*` wants `query: ` and `passage: `
role prefixes; `nomic-embed-text` wants `search_query: ` and `search_document: `. Sending
the wrong ones costs retrieval quality and raises no error, so the prefixes belong to the
role, beside the model, and travel with it.

**Cached vectors belong to the model that produced them.** A cache keyed on the input alone
will serve e5 vectors to a nomic run after a backend switch: every score wrong, nothing
logged. `ResolvedRole::cache_key()` returns `backend:model` so a cache can name its
producer and refuse a mismatch. `capabilities/scouting/src/embed.rs` is the worked example.
