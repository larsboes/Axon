# foundation-models

Apple's on-device model, served over the OpenAI chat-completions shape on loopback.

## Why this exists

Axon's digest ladder picks a shape from how much source there is: a few bullets for a
short article, grouped sections for a long transcript. Until now every rung ran on the
same 26B model through oMLX, which meant a two-paragraph digest of a short page competed
for the same GPU memory as a forty-page paper.

On 2026-08-05 that competition produced a real failure. Four concurrent prefills pushed
oMLX past its hard memory watermark and it aborted all four, two of them feed digests.
Apple's model does not sit in that budget at all. It runs in the OS, answers a short
digest in about two seconds against twelve to twenty for the 26B, and taking the cheap
rungs off the shared GPU is worth more than the quality difference at those rungs.

## The 4,096-token window is the feature

`SystemLanguageModel.contextSize` is 4096, shared between prompt and reply. That is not
a limitation to work around; it is what makes this a rung-selector rather than a
downgrade. Roughly:

| Rung | Source | Fits? |
|---|---|---|
| Brief | under ~2,500 chars | yes |
| Standard | ~2,500 to 9,000 chars | yes |
| Sectioned | above ~9,000 chars | no |

The digest that started this work needed 3,568 input tokens plus 1,000 of reply. It
cannot run here, and `libs/summarize::fits_context` knows that before the request is
made, so the strong model takes it.

Over-window requests are refused, never truncated. A silently shortened prompt produces
a digest of the first half of a document and says nothing about it, which is worse than
no digest because the reader cannot tell.

## Wiring it up

Add a second summarization role to `inference.json` in the overlay:

```json
"summarization_light": {
  "backend": "foundation-models",
  "model": "apple-on-device",
  "max_input_tokens": 4096
}
```

with the matching backend:

```json
"foundation-models": { "api": "openai", "base_url": "http://127.0.0.1:8091/v1" }
```

`max_input_tokens` is required. Without it comms cannot tell whether a source fits and
skips the role entirely, because guessing produces a context error instead of a digest.

Nothing else changes. `comms` picks the light role when the source demonstrably fits and
falls back to the strong one otherwise, and a machine with no light role configured
behaves exactly as it did before.

## Requirements

macOS 26+, Apple Silicon, Apple Intelligence enabled. Without them the binary refuses to
start, instead of binding a port and failing every request:

```
$ afm-server --check
foundation-models: available, context 4096 tokens
```

The Pi never enables this capability. There is no manifest field for "Mac only", so the
check lives in the binary where it cannot be forgotten.

## Design notes

**No package dependencies.** `FoundationModels` and `Network` both ship in the SDK. The
whole surface is two routes on loopback, and a server framework would be more code to
audit than the thing it serves.

**Errors are a 200 with an error envelope**, matching what oMLX does when its memory
guard fires. Axon reads that envelope before it reads `choices`, so a refusal arrives as
a typed cause rather than as an empty answer. Returning a 4xx would be more correct in
the abstract and less useful in practice: the one consumer reads any non-2xx as "server
down" rather than "this request will not fit".

**Loopback only, not configurable.** The endpoint has no authentication because it needs
none on 127.0.0.1. The moment it listened on a routable address that would stop being
true, so it cannot.

## Sources

- [Foundation Models framework](https://developer.apple.com/documentation/foundationmodels)
  — `SystemLanguageModel`, `LanguageModelSession`, `GenerationOptions`. The authority for
  `contextSize` and `exceededContextWindowSize`. Note the documentation site is a JS
  application and does not fetch as text; the `.swiftinterface` in the installed SDK is
  the more reliable read.
- [apple/foundation-models-utilities](https://github.com/apple/foundation-models-utilities)
  — emerging patterns. Its `ChatCompletionsLanguageModel` runs the opposite direction to
  this shim (a Swift session talking *to* an OpenAI server), which is useful for the wire
  shape but is not the mechanism here. Its history modifiers — `summarizeHistory()`,
  `rollingWindow()`, `droppingCompletedToolCalls()` — are the thing to reach for if this
  ever holds a conversation rather than answering one-shot prompts against 4,096 tokens.
- [rudrankriyam/Foundation-Models-Framework-Lab](https://github.com/rudrankriyam/Foundation-Models-Framework-Lab)
  — a workbench of runnable examples: `@Generable`/`@Guide` structured output, tool
  calling, streaming, availability gating. Worth reading before adding structured output
  here, which is the obvious next step and deliberately not in this version.
