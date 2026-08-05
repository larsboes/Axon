//! `libs/summarize` — one adaptive digest engine for every kind of observed content.
//!
//! A digest is what the local model wrote about a thing. It is deliberately a
//! different noun from `summary`: calendar's `summary` is what the *source* said
//! it is, and overwriting that with a generated paragraph would destroy the
//! only verbatim description an entry has. See `libs/content-item/src/lib.rs`.
//!
//! ## Why length decides the shape
//!
//! A summariser with one fixed prompt produces a three-bullet digest for a
//! forty-word mail and the same three bullets for a forty-page paper. The first
//! is longer than its source and the second throws away most of it. The shape
//! is therefore derived from how much source there actually is, and below a
//! floor no digest is produced at all — the source *is* the summary, and
//! storing a paraphrase of it is a second thing to read, not a shorter one.
//!
//! ## Why "more detailed" is a rung, not an adjective
//!
//! Asking a model to "be more detailed" gets you a longer version of the same
//! guess. [`Depth::Detailed`] instead moves the shape exactly one step up the
//! same ladder, which changes both the requested structure and the token
//! ceiling. That is inspectable after the fact — a stored digest records the
//! rung it was produced at — and an operator pressing refine twice does not get
//! two different unrelated documents.
//!
//! ## Dependency rule
//!
//! Compiled into consumers by `#[path]` include, like `libs/inference` and
//! `libs/content-item`, so it may only use crates every consumer already has:
//! `serde_json` and blocking `reqwest`. It deliberately does **not** name
//! `libs/inference`'s types: a consumer builds a [`Target`] from whatever it
//! resolved, so a capability that has no inference module still compiles.

// Non-inline submodule: rustc resolves it against this file's own directory,
// which holds under the `#[path]` include too. Bazel globs `src/**/*.rs`, but a
// consumer naming this lib's sources by label has to name both files.
#[path = "chart.rs"]
pub mod chart;

use std::time::Duration;

/// Bump with any change to [`digest_prompt`] or the shape ladder. It is part of
/// the stored producer string, so a change here is what makes an existing
/// digest legibly stale rather than silently mixed with new ones.
pub const DIGEST_PROMPT_REVISION: &str = "content-digest-v1-adaptive";

/// Bump with any change to [`diagram_prompt`] or the accepted diagram headers.
pub const DIAGRAM_PROMPT_REVISION: &str = "content-diagram-v1-mermaid";

/// Max characters of source text handed to the model. A one-hour transcript is
/// 100k+ characters and the local model's window is finite; the full text stays
/// stored unchanged either way.
pub const INPUT_CAP: usize = 15_000;

/// Below this, the source is already shorter than any honest digest of it.
///
/// Roughly a hundred words. A "digest" of a two-line mail is the same two lines
/// with different words in front of them, and it costs a model call to produce
/// something nobody gains by reading. The automatic pass skips these; an
/// explicit [`Depth::Detailed`] press overrides the skip, because the operator
/// looking at the item knows something the character count does not.
pub const SHORT_SOURCE_FLOOR: usize = 600;

/// How much structure the digest is asked for. Derived from source length,
/// never chosen by the caller directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Nothing worth digesting. Not a failure — a verdict.
    None,
    /// A few bullets, no framing sentence.
    Brief,
    /// Bullets plus one sentence of context. What every feed item got before
    /// this ladder existed.
    Standard,
    /// Grouped bullets under short headings, plus context. For papers and
    /// long transcripts, where a flat bullet list loses the argument's shape.
    Sectioned,
}

impl Shape {
    /// The rung a source of this length earns on its own.
    pub fn for_length(chars: usize) -> Self {
        match chars {
            0..=599 => Shape::None,
            600..=2_499 => Shape::Brief,
            2_500..=8_999 => Shape::Standard,
            _ => Shape::Sectioned,
        }
    }

    /// One rung up, saturating. This is the whole implementation of "more
    /// detailed, please".
    pub fn next(self) -> Self {
        match self {
            Shape::None => Shape::Brief,
            Shape::Brief => Shape::Standard,
            Shape::Standard | Shape::Sectioned => Shape::Sectioned,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Shape::None => "none",
            Shape::Brief => "brief",
            Shape::Standard => "standard",
            Shape::Sectioned => "sectioned",
        }
    }

    /// The ceiling that goes with the rung. A bigger structure needs room to
    /// finish; a smaller one must not be allowed to sprawl into the larger
    /// one's territory, or the ladder stops meaning anything.
    pub fn max_tokens(self) -> u32 {
        match self {
            Shape::None => 0,
            Shape::Brief => 200,
            Shape::Standard => 500,
            Shape::Sectioned => 1_000,
        }
    }

    /// What the model is actually asked to produce at this rung.
    fn instruction(self) -> &'static str {
        match self {
            // Unreachable through `digest`, which returns SkippedShort first.
            Shape::None => "",
            Shape::Brief => {
                "Write two to three short bullet points covering only what the source \
                 actually establishes. Do not add a closing sentence."
            }
            Shape::Standard => {
                "Write the key points as short bullet points, then add exactly one sentence \
                 of context."
            }
            Shape::Sectioned => {
                "Group the key points under at most four short headings that follow the \
                 source's own structure, as bullet points under each. Then add exactly one \
                 sentence of context. Keep concrete numbers, names and results where the \
                 source gives them."
            }
        }
    }
}

/// Whether this run is the automatic pass or an operator asking for more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    #[default]
    Standard,
    Detailed,
}

impl Depth {
    pub fn as_str(self) -> &'static str {
        match self {
            Depth::Standard => "standard",
            Depth::Detailed => "detailed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Depth::Standard),
            "detailed" => Some(Depth::Detailed),
            _ => None,
        }
    }
}

/// At most this many focus terms, each at most this long. The operator types
/// these, so the bound is about keeping the prompt's shape predictable rather
/// than about trust — but a prompt whose instruction section can grow without
/// limit is one where the source text gets squeezed out by the request for it.
pub const MAX_FOCUS_TERMS: usize = 8;
pub const MAX_FOCUS_TERM_CHARS: usize = 40;

/// What the operator asked for, on top of what the length already decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directive {
    pub depth: Depth,
    pub focus: Vec<String>,
}

impl Directive {
    /// Trim, drop empties and control characters, de-duplicate case-insensitively,
    /// and bound both the count and each term.
    pub fn new(depth: Depth, focus: impl IntoIterator<Item = String>) -> Self {
        let mut clean: Vec<String> = Vec::new();
        for term in focus {
            let term: String = term
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_FOCUS_TERM_CHARS)
                .collect();
            let term = term.trim().to_string();
            if term.is_empty() {
                continue;
            }
            if clean
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&term))
            {
                continue;
            }
            clean.push(term);
            if clean.len() == MAX_FOCUS_TERMS {
                break;
            }
        }
        Self {
            depth,
            focus: clean,
        }
    }

    /// The stored form. Comma-joined because it is display state read back into
    /// a text field, not something anything queries by.
    pub fn focus_text(&self) -> String {
        self.focus.join(", ")
    }

    /// Split a stored `focus` column back into terms.
    pub fn parse_focus(stored: &str) -> Vec<String> {
        stored
            .split(',')
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The rung this directive lands on for a source of this length.
    pub fn shape_for(&self, source_chars: usize) -> Shape {
        let base = Shape::for_length(source_chars);
        match self.depth {
            Depth::Standard => base,
            Depth::Detailed => base.next(),
        }
    }
}

/// Where to send the request. Built by the caller from whatever role it
/// resolved, so this lib never names `libs/inference`'s types.
#[derive(Debug, Clone)]
pub struct Target {
    /// A full chat-completions URL.
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    /// Whether the endpoint is loopback. Mail digests must refuse anything else
    /// — see [`digest`].
    pub loopback: bool,
}

/// Typed outcome, so a caller can tell "nothing to do" from "server down" from
/// "the model answered with nothing" and record the class for bounded retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok(String),
    /// The source is below the floor and no one forced it. Terminal and correct.
    SkippedShort,
    /// The item is Personal or Private and the resolved target is not loopback.
    /// A policy refusal, never retried into success by waiting.
    RemoteRefused,
    Unconfigured,
    HttpError(String),
    ModelError(String),
    EmptyResponse,
    Timeout,
}

impl Outcome {
    /// Short, loggable state for the digest row.
    pub fn state(&self) -> &'static str {
        match self {
            Outcome::Ok(_) => "generated",
            Outcome::SkippedShort => "skipped_short",
            Outcome::RemoteRefused => "remote_refused",
            Outcome::Unconfigured => "unconfigured",
            Outcome::HttpError(_) => "http_error",
            Outcome::ModelError(_) => "model_error",
            Outcome::EmptyResponse => "empty_response",
            Outcome::Timeout => "timeout",
        }
    }

    /// Whether a later run could plausibly do better. `skipped_short` and
    /// `remote_refused` are verdicts, not transient failures.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Outcome::HttpError(_)
                | Outcome::ModelError(_)
                | Outcome::EmptyResponse
                | Outcome::Timeout
        )
    }

    /// The message worth showing a reader, if any.
    pub fn error_detail(&self) -> Option<&str> {
        match self {
            Outcome::HttpError(detail) | Outcome::ModelError(detail) => Some(detail),
            _ => None,
        }
    }
}

/// The stored producer string: which backend, which model, which prompt.
/// Changing any of the three makes existing rows legibly stale.
pub fn producer(target_cache_key: &str, prompt_revision: &str) -> String {
    format!("{target_cache_key}:{prompt_revision}")
}

/// Cap the text handed to the model, marking the cut so a reader can tell a
/// digest of the whole thing from a digest of its first half.
pub fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        text.to_string()
    } else {
        let head: String = text.chars().take(cap).collect();
        format!("{head}…[truncated]")
    }
}

/// The prompt. English output regardless of source language, no preamble, and
/// the focus terms named explicitly rather than folded into the instruction —
/// the operator should be able to read the stored `focus` and recognise what
/// they asked for.
pub fn digest_prompt(input: &str, shape: Shape, directive: &Directive) -> String {
    let mut prompt = String::from("Summarize the following content as a compact digest. ");
    prompt.push_str(shape.instruction());
    prompt.push_str(
        " Write in English, even when the source is in another language. Do not add a preamble, \
         and do not describe what you are about to do.",
    );
    if !directive.focus.is_empty() {
        prompt.push_str(
            "\n\nThe reader has asked you to pay particular attention to these topics. Cover what \
             the source says about each one; if the source says nothing about a topic, say so in \
             one short line rather than inventing material:\n",
        );
        for term in &directive.focus {
            prompt.push_str("- ");
            prompt.push_str(term);
            prompt.push('\n');
        }
    }
    prompt.push_str("\n\nContent:\n");
    prompt.push_str(input);
    prompt
}

/// Produce a digest of `text`.
///
/// `allow_remote` is the caller's data-class verdict: Personal and Private
/// content passes `false`, and a non-loopback target is then refused outright
/// rather than quietly downgraded. That check lives here, at the one place that
/// makes the request, because a policy enforced by each caller separately is a
/// policy with as many holes as there are callers.
pub fn digest(
    target: Option<&Target>,
    text: &str,
    directive: &Directive,
    allow_remote: bool,
) -> Outcome {
    // Empty is not "short" — there is nothing for the operator to have seen
    // that the count missed, so the Detailed override does not apply. Asking a
    // model to summarize an empty string produces confident invention.
    if text.trim().is_empty() {
        return Outcome::SkippedShort;
    }
    let source_chars = text.chars().count();
    let shape = directive.shape_for(source_chars);
    if shape == Shape::None {
        return Outcome::SkippedShort;
    }
    let Some(target) = target else {
        return Outcome::Unconfigured;
    };
    if !allow_remote && !target.loopback {
        return Outcome::RemoteRefused;
    }
    let input = truncate(text, INPUT_CAP);
    let prompt = digest_prompt(&input, shape, directive);
    complete(target, &prompt, shape.max_tokens())
}

/// The diagram headers a renderer can actually draw. A model asked for "a
/// diagram" will happily answer with prose, and prose stored in a diagram
/// column is a render error at the reader rather than a failure here.
pub const MERMAID_HEADERS: [&str; 12] = [
    "flowchart",
    "graph",
    "sequenceDiagram",
    "classDiagram",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "mindmap",
    "timeline",
];

pub fn diagram_prompt(input: &str) -> String {
    format!(
        "Draw the structure of the following content as a single Mermaid diagram. Choose the \
         diagram type that fits what the content actually is: `flowchart` for a process or an \
         architecture, `sequenceDiagram` for an exchange between parties, `timeline` for dated \
         events, `mindmap` for a topic breakdown. Use at most fifteen nodes and keep every label \
         under six words. Answer with one fenced ```mermaid block and nothing else — no \
         explanation before or after it. Label text must not contain parentheses, quotes or \
         semicolons.\n\nContent:\n{input}"
    )
}

/// Pull the diagram out of a model answer and refuse anything a renderer would
/// choke on. Accepts a fenced block or a bare diagram, because models produce
/// both and the fence is presentation, not content.
pub fn extract_mermaid(answer: &str) -> Result<String, String> {
    let body = match answer.split_once("```mermaid") {
        Some((_, rest)) => rest
            .split_once("```")
            .map(|(inside, _)| inside)
            .ok_or_else(|| "unterminated ```mermaid block".to_string())?,
        // A bare answer may still be a diagram; a *differently* fenced one is a
        // model that ignored the format, and guessing which fence it meant is
        // how prose ends up in the column.
        None if answer.contains("```") => {
            return Err("answer used a fence that was not ```mermaid".into())
        }
        None => answer,
    };
    let body = body.trim();
    if body.is_empty() {
        return Err("empty diagram".into());
    }
    let header = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
        .unwrap_or_default();
    let keyword = header.split_whitespace().next().unwrap_or_default();
    if !MERMAID_HEADERS.contains(&keyword) {
        return Err(format!("not a Mermaid diagram: starts with {keyword:?}"));
    }
    Ok(body.to_string())
}

/// Produce a Mermaid diagram of `text`. Same remote refusal as [`digest`]: a
/// diagram of a private mail is still that mail.
pub fn diagram(target: Option<&Target>, text: &str, allow_remote: bool) -> Outcome {
    let Some(target) = target else {
        return Outcome::Unconfigured;
    };
    if !allow_remote && !target.loopback {
        return Outcome::RemoteRefused;
    }
    if text.trim().is_empty() {
        return Outcome::SkippedShort;
    }
    let prompt = diagram_prompt(&truncate(text, INPUT_CAP));
    match complete(target, &prompt, 700) {
        Outcome::Ok(answer) => match extract_mermaid(&answer) {
            Ok(diagram) => Outcome::Ok(diagram),
            Err(reason) => Outcome::ModelError(reason),
        },
        other => other,
    }
}

/// One OpenAI-compatible chat completion. The only place in this lib that
/// speaks HTTP.
pub(crate) fn complete(target: &Target, prompt: &str, max_tokens: u32) -> Outcome {
    let http = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
    {
        Ok(client) => client,
        Err(error) => return Outcome::HttpError(error.to_string()),
    };
    let mut request = http.post(&target.endpoint).json(&serde_json::json!({
        "model": target.model,
        "messages": [{ "role": "user", "content": prompt }],
        "max_tokens": max_tokens,
        "stream": false,
    }));
    if let Some(key) = &target.api_key {
        request = request.bearer_auth(key);
    }
    let response = match request.send() {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return Outcome::Timeout,
        Err(error) => return Outcome::HttpError(error.to_string()),
    };
    if !response.status().is_success() {
        return Outcome::ModelError(format!("status {}", response.status()));
    }
    let body: serde_json::Value = match response.json() {
        Ok(body) => body,
        Err(error) => return Outcome::ModelError(error.to_string()),
    };
    let answer = body
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(str::trim)
        .unwrap_or_default();
    if answer.is_empty() {
        Outcome::EmptyResponse
    } else {
        Outcome::Ok(answer.to_string())
    }
}

// Gated on the standalone-tests feature rather than bare cfg(test), matching
// libs/content-item and libs/inference: this file is compiled into every
// consumer by `#[path]` include, and a lib's own suite has no business running
// inside each consumer's test binary. //libs/summarize:summarize_test sets the
// feature and runs them.
#[cfg(all(test, feature = "standalone-tests"))]
mod tests {
    use super::*;

    #[test]
    fn the_shape_ladder_has_no_gaps_and_no_overlaps() {
        assert_eq!(Shape::for_length(0), Shape::None);
        assert_eq!(Shape::for_length(SHORT_SOURCE_FLOOR - 1), Shape::None);
        assert_eq!(Shape::for_length(SHORT_SOURCE_FLOOR), Shape::Brief);
        assert_eq!(Shape::for_length(2_499), Shape::Brief);
        assert_eq!(Shape::for_length(2_500), Shape::Standard);
        assert_eq!(Shape::for_length(8_999), Shape::Standard);
        assert_eq!(Shape::for_length(9_000), Shape::Sectioned);
        assert_eq!(Shape::for_length(400_000), Shape::Sectioned);
    }

    /// The ceiling has to rise with the rung, or a Sectioned digest gets cut
    /// off mid-section and reads as a worse Standard one.
    #[test]
    fn every_rung_raises_the_token_ceiling() {
        assert!(Shape::Brief.max_tokens() < Shape::Standard.max_tokens());
        assert!(Shape::Standard.max_tokens() < Shape::Sectioned.max_tokens());
        assert_eq!(Shape::None.max_tokens(), 0);
    }

    #[test]
    fn detailed_moves_exactly_one_rung_and_saturates() {
        let detailed = Directive::new(Depth::Detailed, []);
        let standard = Directive::new(Depth::Standard, []);
        assert_eq!(standard.shape_for(3_000), Shape::Standard);
        assert_eq!(detailed.shape_for(3_000), Shape::Sectioned);
        assert_eq!(detailed.shape_for(50_000), Shape::Sectioned);
    }

    /// The automatic pass skips a short source; an explicit press does not.
    /// That is the whole affordance — the operator can see something the
    /// character count cannot.
    #[test]
    fn an_explicit_detailed_press_overrides_the_short_skip() {
        let text = "Two lines. Nothing more.";
        assert_eq!(
            digest(None, text, &Directive::default(), true),
            Outcome::SkippedShort
        );
        // Past the floor check, an absent target is what stops it — proving the
        // skip is no longer what returned.
        assert_eq!(
            digest(None, text, &Directive::new(Depth::Detailed, []), true),
            Outcome::Unconfigured
        );
    }

    /// The override is for a source the operator can see and the count
    /// under-rates. Nothing is not that: found live on a feed item whose
    /// transcript is zero characters, where forcing a digest would have asked
    /// the model to summarize an empty string and invent the answer.
    #[test]
    fn nothing_at_all_is_skipped_even_when_the_operator_insists() {
        for empty in ["", "   ", "\n\t "] {
            assert_eq!(
                digest(None, empty, &Directive::new(Depth::Detailed, []), true),
                Outcome::SkippedShort,
                "{empty:?} should stay skipped"
            );
        }
    }

    #[test]
    fn focus_terms_are_trimmed_deduplicated_and_bounded() {
        let directive = Directive::new(
            Depth::Standard,
            [
                "  benchmark ".into(),
                "BENCHMARK".into(),
                String::new(),
                "  ".into(),
                "a".repeat(200),
            ],
        );
        assert_eq!(directive.focus.len(), 2);
        assert_eq!(directive.focus[0], "benchmark");
        assert_eq!(directive.focus[1].chars().count(), MAX_FOCUS_TERM_CHARS);

        let many: Vec<String> = (0..50).map(|n| format!("term{n}")).collect();
        assert_eq!(
            Directive::new(Depth::Standard, many).focus.len(),
            MAX_FOCUS_TERMS
        );
    }

    #[test]
    fn focus_text_round_trips_through_storage() {
        let directive = Directive::new(Depth::Detailed, ["cost".into(), "latency".into()]);
        let stored = directive.focus_text();
        assert_eq!(stored, "cost, latency");
        assert_eq!(Directive::parse_focus(&stored), vec!["cost", "latency"]);
        assert!(Directive::parse_focus("").is_empty());
        assert!(Directive::parse_focus(" , ,").is_empty());
    }

    #[test]
    fn the_prompt_carries_the_rung_the_language_and_the_focus() {
        let directive = Directive::new(Depth::Standard, ["evaluation".into()]);
        let prompt = digest_prompt("Ein deutschsprachiger Quelltext.", Shape::Sectioned, &directive);
        assert!(prompt.contains("at most four short headings"));
        assert!(prompt.contains("Write in English"));
        assert!(prompt.contains("- evaluation"));
        assert!(prompt.contains("Ein deutschsprachiger Quelltext."));

        let plain = digest_prompt("x", Shape::Brief, &Directive::default());
        assert!(plain.contains("two to three short bullet points"));
        assert!(
            !plain.contains("particular attention"),
            "no focus section when nothing was asked for"
        );
    }

    /// Personal and Private content passes `allow_remote: false`. A cloud
    /// target must then be refused outright — not truncated, not downgraded.
    #[test]
    fn a_non_loopback_target_is_refused_for_restricted_content() {
        let cloud = Target {
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: false,
        };
        let text = "x".repeat(1_000);
        assert_eq!(
            digest(Some(&cloud), &text, &Directive::default(), false),
            Outcome::RemoteRefused
        );
        assert_eq!(
            diagram(Some(&cloud), &text, false),
            Outcome::RemoteRefused
        );
    }

    #[test]
    fn a_refusal_or_a_skip_is_never_retried() {
        assert!(!Outcome::SkippedShort.retryable());
        assert!(!Outcome::RemoteRefused.retryable());
        assert!(!Outcome::Unconfigured.retryable());
        assert!(Outcome::Timeout.retryable());
        assert!(Outcome::EmptyResponse.retryable());
        assert!(Outcome::ModelError("boom".into()).retryable());
    }

    #[test]
    fn truncate_marks_only_when_it_cuts() {
        assert_eq!(truncate("short", 15_000), "short");
        let long = "x".repeat(20_000);
        let out = truncate(&long, 15_000);
        assert!(out.ends_with("…[truncated]"));
        assert_eq!(out.chars().count(), 15_000 + "…[truncated]".chars().count());
    }

    #[test]
    fn a_fenced_diagram_is_unwrapped() {
        let answer = "Here you go:\n```mermaid\nflowchart TD\n  A --> B\n```\nHope that helps.";
        assert_eq!(
            extract_mermaid(answer).unwrap(),
            "flowchart TD\n  A --> B"
        );
    }

    #[test]
    fn a_bare_diagram_is_accepted() {
        assert_eq!(
            extract_mermaid("sequenceDiagram\n  A->>B: hi").unwrap(),
            "sequenceDiagram\n  A->>B: hi"
        );
        assert!(extract_mermaid("%% a comment\nmindmap\n  root").is_ok());
    }

    /// The gate that matters: prose stored in a diagram column is a render
    /// error at the reader, which is exactly where it is hardest to diagnose.
    #[test]
    fn prose_and_foreign_fences_are_rejected_rather_than_stored() {
        for answer in [
            "I'd be happy to help! Here is an overview of the paper.",
            "```python\nprint('hi')\n```",
            "```mermaid\nflowchart TD\n  A --> B",
            "```mermaid\n\n```",
            "",
        ] {
            assert!(
                extract_mermaid(answer).is_err(),
                "should have been rejected: {answer:?}"
            );
        }
    }

    #[test]
    fn the_producer_string_names_backend_model_and_prompt() {
        assert_eq!(
            producer("openai|http://127.0.0.1:8080|gemma", DIGEST_PROMPT_REVISION),
            "openai|http://127.0.0.1:8080|gemma:content-digest-v1-adaptive"
        );
    }

    #[test]
    fn depth_parses_only_the_two_stored_values() {
        assert_eq!(Depth::parse("standard"), Some(Depth::Standard));
        assert_eq!(Depth::parse("detailed"), Some(Depth::Detailed));
        assert_eq!(Depth::parse("deeper"), None);
        assert_eq!(Depth::parse(""), None);
    }
}
