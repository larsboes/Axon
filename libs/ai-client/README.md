# ai-client

Shared LLM-routing library — not a capability. See
`README.md#three-architectural-nouns` for why this lives in `libs/` instead
of `capabilities/`: it owns no domain (no upstream verdict, no external system, no CLI), it's a
crate other capabilities `use`.

## What it is

One `LlmRouter` trait behind two providers, following the intelligence ladder in
`README.md#implementation-languages-and-intelligence` (heuristic → algorithm → classical ML →
**local AI → cloud AI**) — those last two rungs, behind
one interface, not two separate things:

- `providers::local_openai` — an OpenAI-compatible local endpoint (`LOCAL_AI_HOST`, default
  `http://localhost:8000`) for anything the security doctrine's data-classes rule says must
  never leave the machine (`vault`-class content).
- `providers::gemini` — Google's Gemini API, for everything else. Key via `GEMINI_API_KEY`.
- `router::ConfigurableLlmClient` — routes by `Priority` (`Speed`/`Cost` → local, falling back to
  cloud only when priority is `Cost`; `Reasoning` → cloud directly). Config is env-var defaults
  today; `AI_ROUTER_CONFIG_PATH` is opt-in file-based override, unset by default (no config file
  exists in Axon's shape yet — see `router.rs`'s comment).

## Status

Migrated 2026-07-11 from a bulk port off the previous private hub, sitting untracked and
un-Bazel-wired (same pattern as the original `dashboard` port —
`dashboard/README.md`'s backstory). Real `BUILD.bazel`,
own `crate.from_cargo` block (`@crate_index_ai_client`, per
`MODULE.bazel's crate_index block` — no type-sharing with
scouting/transit/axon-status). One rough edge fixed in the move: the config loader pointed at a
hardcoded `infra/config/ai_router.json` that doesn't exist in Axon's shape.

**Still no named consumer.** Nothing in `capabilities/` or `dashboard` calls into this yet
— it moved because its *shape* was wrong (untracked, outside Bazel, contradicting the old
capability placeholder), not because a consumer showed up. Don't wire a real caller to it on the
strength of this README alone; that's a separate call when something actually needs LLM routing.
`GEMINI_API_KEY` is a bare env var for now — goes through `tools/setup-secret.sh` → Vaultwarden
(README.md#secrets) once there's a real consumer provisioning it, not before.
