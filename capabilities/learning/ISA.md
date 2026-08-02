---
project: learning
type: isa
effort: E3
phase: observe
progress: 0
mode: reference
---

# ISA · learning service (reference)

Port of LifeOS-mono's learning service ISA. Not yet implemented in Axon.

## Problem

No local-AI rung (oMLX) standing up in Axon. No validated compiled-output
convention. No artifact that turns "self-building learning wiki" from a
vibe into something judgeable.

## Vision

Drop a chapter being studied into `capabilities/learning/<topic>/sources/`, run
one command, and see a structured hands-on lesson guide — frozen,
byte-identical until explicitly regenerated.

## Constraints (from LifeOS-mono spec)

- **C1 · oMLX is the runtime**: OpenAI-compatible API at
  `http://localhost:8000/v1`.
- **C2 · Freeze-on-compile**: guide written to disk under content address
  `(topic, source-set-hash, model, prompt-version)`; reload reads file,
  never regenerates.
- **C3 · Composition-only UI**: dashboard renders + triggers; logic lives
  in the Rust service.
- **C4 · Rust backend**: new logic is Rust.
- **C5 · Schemas are law**: `LearningGuide` type in `schemas/`.
- **C6 · oMLX runs outside repo**: recorded in `upstreams.toml`.

## Goal (stretch)

A Rust service under `capabilities/learning/` (not `services/learning/` — see
`README.md#three-architectural-nouns` on translating
LifeOS-mono directory names through Axon's own placement test) that compiles
one topic's source material via oMLX into a frozen markdown guide, a
`schemas/learning/` `LearningGuide` type, and an `dashboard` route
consuming its HTTP surface once it has one.

## Provenance

Ported from the LifeOS-mono learning service ISA (full 9 ISC criteria,
features, test strategy, and decisions) — this file keeps only the distilled
problem/vision/constraints/goal, not a wholesale copy.
