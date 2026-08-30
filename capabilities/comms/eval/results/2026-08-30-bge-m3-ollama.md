# bge-m3 through Ollama — 2026-08-30

- Model: `bge-m3:latest` (BAAI BGE-M3, multilingual)
- Registered digest: `sha256:0c4c9c2a325fb1cdafec606e6809cb745f1cb26a6d919994400d27372303e276`
  (model layer `sha256:daec91ffb5dd0c27…`, 1104 MiB)
- Server: Ollama, OpenAI-compatible `/v1/embeddings` on loopback `11434`
- Shape: 30 vectors in one batch, 1024 dimensions, `query:`/`passage:` roles
- Result: **PASS**

| Metric | Observed | Required | E5-base (adopted) |
|---|---:|---:|---:|
| Useful top-1 | 1.000 (6/6) | 1.000 | 1.000 (6/6) |
| Pairwise accuracy | **0.912** | 0.750 | 0.882 |
| Mean nDCG | **0.987** | 0.900 | 0.986 |

Same corpus, same judgements, same runner, unchanged. Nothing here was relabelled: this run used
`relevance-corpus.json` exactly as `multilingual-e5-base-mlx` left it on 2026-07-30, which is the
only reason the two columns can be read against each other at all.

It ranked the directly relevant cross-language passage first in all six cases and holds both
corrections E5-base made over E5-small — the German typed-boundary passage over the contradictory
English configuration sentence, and the German historical rail-data passage over the unrelated
English local-LLM sentence. Its one ordering slip is the same one E5-base makes: in
`architecture-boundaries-en` the judgement-1 English configuration passage outscores the
judgement-2 German microservice passage.

## Why this was run

The `embedding` role in the deployment's `inference.json` names backend `omlx`, and **oMLX is not
installed on this machine**: no binary on `PATH`, no `~/.omlx/`, no application bundle. So the
role has been resolving to nothing and `relevance.rs` has been falling back to the deterministic
`lexical` control — a `DefaultHasher` projection, not a semantic one.

`tools/toolchain-check` reports `12 checked · 12 ok · 0 missing` on this host, because oMLX is not
declared in `toolchain.toml`. That is the failure `toolchain.toml [macmon]` already records in its
own `why`: "Absent for weeks while the capability was enabled, because a capability's own command
was not a declared tool here and nothing else looked."

## Resource cost, and what it is not

Ollama reported `bge-m3` resident at **0.62 GiB**; the full six-query runner completed in **0.52 s**
warm, and a separate 30-text batch returned in **0.47 s**. The E5-base record measured about
1.16 GB oMLX RSS and 1.14 s.

**These two sets of numbers are not a like-for-like comparison and must not be quoted as one.**
Different server, different day, and Ollama reports a model's resident size where the E5-base
record reported whole-process RSS. What is comparable is the gate above. The resource figures say
only that this candidate is not obviously more expensive; a real cost comparison needs both served
under the same conditions, and that has not been done.

Also unmeasured: throughput on a full Feed batch rather than 30 texts, and Ollama's keep-alive
behaviour against the 60-second per-model idle TTL the E5-base record chose deliberately for
bursty, revision-cached Comms work.

## Reproducing it

```sh
OMLX_NO_AUTH=1 OMLX_BASE_URL=http://127.0.0.1:11434/v1 OMLX_EMBEDDING_MODEL=bge-m3 \
  bun capabilities/comms/eval/run-relevance.ts
```

`OMLX_NO_AUTH=1` was added to this runner in the same change. `run-reranking.ts` already had it;
without it the only way to measure a candidate on a keyless loopback server is a throwaway
settings file holding a key nothing reads, and a run that needs an unrecorded shim is not
evidence.

## Status

**Measured, not adopted.** Which backend serves the `embedding` role is a deployment fact in the
overlay, not a repo decision, and the standing selection in
`results/2026-07-30-multilingual-e5-base-mlx.md` is unchanged by this file alone. What this record
establishes is that a passing local option is already on this disk, and that the reason the Feed
is not semantic is an absent binary rather than a missing model.
