# libs/content-item

The `content-item-v1` reader contract in Rust. Spine-owned shared code with no
domain of its own — see [Three architectural nouns](../../README.md#three-architectural-nouns).

`schemas/content-item.schema.json` is the normative artifact. This crate exists
so that the capabilities emitting it cannot drift from one another by hand.

## Why a shared contract and not a shared table

A unified **read** contract pays for itself: the dashboard owns one renderer,
and a new source costs an adapter instead of a second reader. A unified
**store** would not. Calendar enforces an exclusive `ends_at` after `starts_at`,
a `(source, external_id)` unique index and a commitment CHECK; mail enforces a
retention window; feed enforces neither. Merged into one table, every one of
those becomes a conditional the database cannot express, and the guarantees are
gone.

So each capability keeps its own tables and projects into this shape on read.

## Ranking is a property of the source

`relevance` and `evaluation` exist because feed is an unbounded inbox that has
to be ranked. Calendar is not — an entry is something the operator already
decided about, and its triage axis is `commitment`, surfaced through `status`.

A source with no ranking leaves those fields empty. It does not synthesise a
score: a `0.0` on a committed event reads as a judgement of the event, which is
how `matched_focus: "Scholarship Profile", score: 0.0` ended up sitting on a real
calendar entry before this contract existed.

## Consumers

`comms` (feed, mail) and `calendar` (entries).

Consumers list `//libs/content-item:src/lib.rs` in their **srcs** and add the
`#[path]` module — not a Bazel `deps` edge, for the reason
[axon-config's README](../axon-config/README.md#consumers) gives.

Two consequences of that include model matter here:

1. **The file is compiled separately into each consumer**, so `ContentItem` in
   comms and `ContentItem` in calendar are different Rust types. That is fine
   and intended: the boundary between them is the serialized JSON, never a
   function call. Do not try to pass one across a capability boundary.
2. **It may only use crates every consumer already has** — `serde` and
   `serde_json`. Anything else silently changes a consumer's dependency
   resolution, or fails to build in whichever consumer lacks it.

## Adding a source

1. Add the value to `source` in the JSON schema and to the `allOf` branch that
   nulls the extensions it does not own.
2. Add an extension struct here if the source has fields nothing else does.
3. Project in the capability that owns the data. Do not reach into another
   capability's store to fill a field.
