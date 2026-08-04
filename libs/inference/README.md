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
- A **role** is a job: `embedding`, `reranking`, `summarization`, or an explicitly named
  `cloud_*` task. It names a backend, the model on it, and that model's input conventions.

```rust
let role = InferenceConfig::load(overlay_config).role("embedding");
let vectors = role.embed(&texts, TextRole::Query)?;
```

A capability asks for a role and never learns whether it just talked to oMLX or Ollama.
That is the point: oMLX needs Metal and cannot exist on the family Pi, Ollama runs
anywhere, and moving between them is a config edit rather than a code change.

## Implementation

`src/lib.rs` serves the Rust capabilities that need model roles (Scouting and Comms today). It reads
`inference.json`, honours the backend override, and resolves bearer keys from the referenced
private file. Consumers receive a resolved role without hardcoding a URL or model. Comms also
uses the resolved role for model readiness, mixed query/document embedding batches and
OpenAI-compatible chat completion routing. Reranking roles use the Cohere/Jina-compatible
`/v1/rerank` shape, restore sorted results to input order and reject incomplete, duplicate or
out-of-range scores before a consumer can persist them. The Ollama-native API has no equivalent
route, so callers degrade explicitly when a machine overrides that backend.

## Config

`<overlay>/config/inference.json`, or `AXON_INFERENCE_CONFIG` to point somewhere else.
Field docs live in `inference.config.example.json` beside this file.

A missing config is not an error. Every consumer is expected to degrade to something that
still works offline — scouting falls back to hash embedding — so a machine with no
inference set up keeps running instead of failing at startup.

Cloud-capable UI lists only roles whose names start with `cloud_`, resolve to a non-loopback
HTTPS backend, and declare `provider_name`, `cloud_data_tier`, `billing_mode`, a non-zero
`max_requests_per_day` and `max_input_tokens`. Supported
tiers are `public` and `pseudonymized_personal`; supported billing boundaries are `free_only`
and `prepaid_credit`. There is deliberately no unbounded pay-as-you-go mode. The public API may
expose that safe policy, role, model and protocol label, but never the backend URL, account ID,
key-file path or key value. A configured role remains unavailable until its private key file
contains a value. Selecting one records provider intent; a consumer must still implement an
explicit execution boundary before any request is made.

The explicitly selected role runs first. `failover_priority` then orders only roles with the
same exact `cloud_data_tier`; a Public role can never become a pseudonymized-Personal target or
the reverse. `prepaid_credit` additionally requires a valid `credit_expires_on` date and becomes
inert after that UTC day. UTF-8 request bytes plus a fixed prompt allowance form a conservative
provider-independent token upper bound. These local ceilings are hard stops, not claims about a
provider account's billing configuration, which the operator must also keep free-only or prepaid.

Relative `api_key_file` paths resolve beside the private `inference.json`. This lets an overlay
reference a gitignored `runtime-secrets/` file without recording one workstation's absolute
path. `tools/materialize-inference-key` is the human-run bridge from the matching Vaultwarden
Secure Note into that local file; it never prints the value.

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
