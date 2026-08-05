//! The seam between bytes off the wire and the text an item stores.
//!
//! Fetching stays per source and is not extraction: the GitHub API, Reddit's
//! JSON, yt-dlp and a plain GET speak four different protocols. What they all
//! end up holding is a document plus what kind of document it is, and turning
//! that into text is one job — one implementation per input class, behind this
//! trait (#77).
//!
//! Extraction stops at "faithful bytes to text". `normalize::normalize` owns
//! "clean" (#86). An extractor that stripped page furniture would be making,
//! silently, the judgement that stage exists to make inspectably.
//!
//! Two implementations, and which owns which class is policy. `Builtin` reads
//! HTML and plain text; `Xberg` reads PDF, the class nothing here could read
//! before (#77). HTML deliberately did not move to xberg: see `Xberg`'s own
//! note for what its PDF output actually looks like, which is also why that
//! rung sits last in `media::fetch_arxiv`. Whichever route ran is recorded per
//! item as `transcript_source` and `producer` (#78).

use crate::{CommsError, Result};

/// What a document is, which is what decides who can read it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputClass {
    Html,
    Pdf,
    PlainText,
}

impl InputClass {
    pub fn as_str(self) -> &'static str {
        match self {
            InputClass::Html => "html",
            InputClass::Pdf => "pdf",
            InputClass::PlainText => "text",
        }
    }
}

/// Bytes plus what they are. Borrowed, because an extractor reads a document
/// and never owns one — the fetch that produced it owns its buffer.
pub struct Document<'a> {
    pub class: InputClass,
    pub bytes: &'a [u8],
}

impl<'a> Document<'a> {
    pub fn html(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::Html,
            bytes,
        }
    }

    pub fn pdf(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::Pdf,
            bytes,
        }
    }

    pub fn text(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::PlainText,
            bytes,
        }
    }
}

/// What an extractor produced, and which one produced it.
///
/// `producer` is recorded rather than inferred: once there are two
/// implementations for the same class, "which one read this item" is the first
/// question a surprising result raises.
#[derive(Debug, Clone, PartialEq)]
pub struct Extraction {
    pub title: Option<String>,
    pub text: String,
    pub producer: &'static str,
}

/// What the stored text is, relative to the document it came from.
///
/// Not a length judgement — `content_status` already answers that, and the two
/// are independent: a long abstract is `full`/`Abstract`, a one-line article is
/// `thin`/`FullText`.
///
/// Two variants, because two have producers. A third for "the source offered a
/// card instead of the thing" would be inventing a taxonomy ahead of a caller
/// that sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptSource {
    /// The document itself: the article body, the README, the paper.
    FullText,
    /// A stand-in the source offered in place of the document.
    Abstract,
}

impl TranscriptSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TranscriptSource::FullText => "full-text",
            TranscriptSource::Abstract => "abstract",
        }
    }
}

pub trait Extractor: Sync {
    fn name(&self) -> &'static str;
    fn handles(&self, class: InputClass) -> bool;
    fn extract(&self, doc: &Document<'_>) -> Result<Extraction>;
}

/// The extractor for an input class, or `None` when nothing here reads it.
///
/// `None` is an answer, not a failure: a caller that can degrade (arXiv to its
/// abstract) checks this and records what it did. A caller that cannot says so
/// with `require`.
///
/// First match wins, so this order is policy rather than convenience. `Builtin`
/// keeps HTML: it is measured by the corpus gate and produces the line
/// structure `normalize` needs, and xberg is an extractor rather than a
/// readability cleaner (#77). xberg owns what nothing here could read.
pub fn for_class(class: InputClass) -> Option<&'static dyn Extractor> {
    const REGISTERED: &[&dyn Extractor] = &[&Builtin, &Xberg];
    REGISTERED
        .iter()
        .copied()
        .find(|extractor| extractor.handles(class))
}

/// `for_class`, for a caller with no fallback to offer.
pub fn require(class: InputClass) -> Result<&'static dyn Extractor> {
    for_class(class).ok_or_else(|| {
        CommsError::Other(format!(
            "no extractor is registered for {} input",
            class.as_str()
        ))
    })
}

// ── xberg ───────────────────────────────────────────────────────────────────

/// PDF, through [xberg](https://github.com/xberg-io/xberg) (`=1.0.5`).
///
/// Registered for `Pdf` only. xberg reads HTML too and returns more text than
/// `Builtin` on every URL in the recorded benchmark, but it is an extractor and
/// not a readability cleaner: its output keeps navigation and share widgets,
/// and the short-line share runs 0.03-1.00. Swapping the HTML path to it is a
/// separate change with a scorecard already in place, the corpus gate's `html`
/// class, rather than a side effect of wanting PDFs.
///
/// **Its PDF output is last-resort quality, measured rather than assumed.** On
/// arXiv 2608.02599 (2026, two-column) the text is correct but woven column
/// against column, line by line, so prose order is lost. On arXiv 0704.0001
/// (2007, Type1 fonts) every `c` is dropped: "Abstrat", "Mihigan", "quantum
/// hromo dynamis". Both took 2.4s and 11.7s respectively, against ~0.3s for
/// the same papers as HTML.
///
/// That is why the PDF rung sits below both HTML hosts in `media::fetch_arxiv`
/// rather than replacing them, and why `producer` is stored per item: an
/// embedding built from scrambled columns is not the same evidence as one
/// built from LaTeXML output, and nothing downstream can tell them apart
/// without being told.
pub struct Xberg;

impl Extractor for Xberg {
    fn name(&self) -> &'static str {
        "xberg-1.0.5"
    }

    fn handles(&self, class: InputClass) -> bool {
        matches!(class, InputClass::Pdf)
    }

    fn extract(&self, doc: &Document<'_>) -> Result<Extraction> {
        if !self.handles(doc.class) {
            return Err(CommsError::Other(format!(
                "xberg is registered for pdf, not {}",
                doc.class.as_str()
            )));
        }
        let result = extract_blocking(doc.bytes.to_vec(), "application/pdf")?;
        let document = result.results.into_iter().next().ok_or_else(|| {
            let why = result
                .errors
                .first()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "no document and no error".into());
            CommsError::Other(format!("xberg returned nothing: {why}"))
        })?;

        // OCR is folded into the producer rather than given a field of its own.
        // "which extractor produced this" is the question `producer` answers,
        // and text recovered from a scan is a different answer from text read
        // out of the file, for anything that later doubts the content.
        let producer: &'static str = match document.extraction_method {
            Some(xberg::ExtractionMethod::Ocr) => "xberg-1.0.5/ocr",
            Some(xberg::ExtractionMethod::Mixed) => "xberg-1.0.5/mixed",
            _ => self.name(),
        };

        Ok(Extraction {
            title: document.metadata.title.filter(|t| !t.trim().is_empty()),
            text: cap(document.content),
            producer,
        })
    }
}

/// Run xberg's async API from this capability's synchronous ingest path.
///
/// xberg is async; comms is not. The CLI carries no runtime at all by design
/// (see this capability's README), and comms-server is already inside one, so
/// neither `Runtime::block_on` nor a bare `Handle` works for both: building a
/// runtime inside a runtime panics.
///
/// A thread that owns its own runtime is correct from either caller without
/// either having to know which it is. The cost is one thread per document,
/// which an ingest path can afford, and the alternative was making the trait
/// async and colouring every caller up to the CLI.
fn extract_blocking(bytes: Vec<u8>, mime: &'static str) -> Result<xberg::ExtractionResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CommsError::Other(format!("xberg runtime: {e}")))
            .and_then(|runtime| {
                runtime.block_on(async {
                    let input = xberg::ExtractInput::from_bytes(bytes, mime, None);
                    xberg::extract(input, &xberg::ExtractionConfig::default())
                        .await
                        .map_err(|e| CommsError::Other(format!("xberg: {e}")))
                })
            });
        let _ = tx.send(outcome);
    });
    rx.recv()
        .map_err(|_| CommsError::Other("xberg extraction thread died".into()))?
}

// ── The built-in extractor ──────────────────────────────────────────────────

/// HTML and plain text, hand-rolled, no dependency.
///
/// It stays after xberg lands rather than being deleted: it is the fallback
/// when a document does not parse, and the reference the corpus gate scores a
/// replacement against.
pub struct Builtin;

/// Max characters of extracted body kept. Extraction caps; it does not clean.
pub const TEXT_CAP: usize = 20_000;

impl Extractor for Builtin {
    fn name(&self) -> &'static str {
        "builtin"
    }

    fn handles(&self, class: InputClass) -> bool {
        matches!(class, InputClass::Html | InputClass::PlainText)
    }

    fn extract(&self, doc: &Document<'_>) -> Result<Extraction> {
        let body = String::from_utf8_lossy(doc.bytes);
        match doc.class {
            InputClass::Html => Ok(Extraction {
                title: html_title(&body),
                text: html_to_lines(&body),
                producer: self.name(),
            }),
            InputClass::PlainText => Ok(Extraction {
                title: None,
                text: cap(body.trim().to_string()),
                producer: self.name(),
            }),
            InputClass::Pdf => Err(CommsError::Other(
                "the built-in extractor does not read PDFs".into(),
            )),
        }
    }
}

/// Cap a body at `TEXT_CAP` characters (not bytes — the cap is about what a
/// summarizer can read, and a multibyte split would corrupt the text).
pub fn cap(s: String) -> String {
    if s.chars().count() <= TEXT_CAP {
        s
    } else {
        s.chars().take(TEXT_CAP).collect()
    }
}

/// HTML to text, keeping the line structure the normalizer needs.
///
/// What a tag becomes is the whole point. Running every tag through a blanket
/// stripper and then collapsing all whitespace welded the document into one
/// line, which broke both stages downstream: `normalize` is a table of line
/// predicates guarded by a max line length, so a single long line made every
/// rule unreachable and stored consent walls verbatim as article text, and a
/// tag boundary that emitted nothing welded `Home</a><a>Jobs` into one token
/// the embedder then scored on.
pub fn html_to_lines(html: &str) -> String {
    let without_blocks = remove_blocks(html, &["script", "style", "noscript", "head"]);
    let text = tags_to_breaks(&without_blocks);
    let collapsed = collapse_lines(&decode_basic_entities(&text));
    collapsed.chars().take(TEXT_CAP).collect()
}

pub fn html_title(html: &str) -> Option<String> {
    let low = html.to_lowercase();
    let start = low.find("<title")?;
    let gt = low[start..].find('>')? + start + 1;
    let end = low[gt..].find("</title>")? + gt;
    let title = decode_basic_entities(&strip_all_tags(html[gt..end].trim()));
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Does this look like markup rather than the text a client already extracted?
pub fn looks_like_html(raw: &str) -> bool {
    let low = raw.to_lowercase();
    low.contains("<html")
        || low.contains("<body")
        || low.contains("<div")
        || low.contains("<p>")
        || low.contains("<span")
        || low.contains("<article")
}

/// HTML elements that end a line of text. Everything not listed is inline and
/// becomes a single space, so two adjacent text nodes stay two words.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "button",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "option",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Replace every tag with the separator its element implies: a newline for a
/// block element, a space for an inline one. An unterminated `<` is markup the
/// page never closed, so the remainder goes.
fn tags_to_breaks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = match after.find('>') {
            Some(i) => i,
            None => return out,
        };
        out.push(if is_block_tag(&after[..close]) {
            '\n'
        } else {
            ' '
        });
        rest = &after[close + 1..];
    }

    out.push_str(rest);
    out
}

fn is_block_tag(inner: &str) -> bool {
    let name = inner
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or_default();
    BLOCK_TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(name))
}

/// Collapse whitespace inside each line, keeping the breaks the tag walk
/// produced. Collapsing across newlines is exactly the structure loss this
/// path exists to avoid.
fn collapse_lines(s: &str) -> String {
    s.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drop a whole element, opening tag to closing tag.
///
/// The tag NAME has to match, not a prefix of it. `<head` also starts
/// `<header`, and once the real `</head>` had been consumed the search for a
/// second one failed and took the `None` arm below — which truncates. Every
/// page carrying a site `<header>` therefore lost everything from that element
/// onward, silently, as an empty or stub transcript. A real arXiv paper
/// extracted to zero characters, which is how this surfaced.
fn remove_blocks(html: &str, tags: &[&str]) -> String {
    let mut out = html.to_string();
    for tag in tags {
        loop {
            let low = out.to_lowercase();
            let open = match find_open_tag(&low, tag) {
                Some(i) => i,
                None => break,
            };
            let close_tag = format!("</{tag}>");
            let close = match low[open..].find(&close_tag) {
                Some(i) => open + i + close_tag.len(),
                // Unclosed: the rest of the document is inside an element we
                // are removing, so there is nothing after it to keep.
                None => {
                    out.truncate(open);
                    break;
                }
            };
            out.replace_range(open..close, " ");
        }
    }
    out
}

/// Offset of `<tag` where the next character actually ends the tag name.
fn find_open_tag(haystack: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{tag}");
    let mut from = 0usize;
    while let Some(hit) = haystack[from..].find(&needle) {
        let at = from + hit;
        let after = haystack[at + needle.len()..].chars().next();
        match after {
            None | Some('>') | Some('/') => return Some(at),
            Some(c) if c.is_whitespace() => return Some(at),
            // `<header` when looking for `<head`: keep searching.
            _ => from = at + needle.len(),
        }
    }
    None
}

/// Depth-counted tag removal, for a fragment where no separator is wanted --
/// a `<title>` is one line by definition.
fn strip_all_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whitespace collapsed across the whole string, newlines included.
///
/// The opposite of what `collapse_lines` does, and correct for the callers
/// that have it: a `<title>`, or one field of machine-generated XML, is one
/// line by definition. Never use it on a document — that is the bug that made
/// every normalization rule unreachable.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The entities Axon's sources actually emit.
///
/// `&amp;` is decoded **last**, and that ordering is the whole correctness
/// argument: decoding it first turns `&amp;lt;` into `&lt;`, which the next
/// replacement then turns into a real `<`. A page that escaped its markup for
/// display would have it silently reconstituted as markup, one stage before
/// the tag walk that eats it.
pub fn decode_basic_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = s.to_string();
    for (entity, replacement) in [
        ("&nbsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&#39;", "'"),
        ("&#x2F;", "/"),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&amp;", "&"),
    ] {
        if out.contains(entity) {
            out = out.replace(entity, replacement);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_extraction_hands_the_normalizer_line_structure() {
        // The normalizer is a line predicate table, so extraction owes it
        // lines. Collapsing a page to one paragraph made every rule
        // unreachable and shipped LinkedIn's cookie banner as article text.
        let html = "<html><head><title>T</title></head><body>\
            <nav><a href=\"/a\">Home</a><a href=\"/b\">Jobs</a></nav>\
            <div id=\"consent\"><p>Accept all cookies</p>\
            <p>Reject non-essential cookies</p></div>\
            <article><p>Transit data should remain inspectable, which is the \
            single claim this article exists to make.</p></article>\
            </body></html>";

        let out = Builtin.extract(&Document::html(html.as_bytes())).unwrap();
        let clean = crate::normalize::normalize(&out.text);

        assert!(
            !clean.text.contains("Accept all cookies"),
            "consent banner survived normalization: {}",
            clean.text
        );
        assert!(
            clean
                .text
                .contains("Transit data should remain inspectable"),
            "the article body must survive: {}",
            clean.text
        );
        assert_eq!(out.title.as_deref(), Some("T"));
        assert_eq!(out.producer, "builtin");
    }

    #[test]
    fn a_site_header_does_not_take_the_rest_of_the_page_with_it() {
        // `<head` is a prefix of `<header`. With prefix matching, the second
        // pass over the document found `<header`, failed to find a closing
        // `</head>` (the real one was already consumed), and truncated
        // everything from there on. A real arXiv paper extracted to zero
        // characters; every page with a site header lost its body silently.
        let html = "<html><head><title>T</title></head><body>\
            <header><p>Site chrome</p></header>\
            <article><p>The body that must survive.</p></article>\
            </body></html>";

        let out = Builtin.extract(&Document::html(html.as_bytes())).unwrap();
        assert!(
            out.text.contains("The body that must survive."),
            "the page was truncated at <header>: {:?}",
            out.text
        );
        assert!(
            out.text.contains("Site chrome"),
            "<header> is page furniture for the normalizer to judge, not for \
             extraction to delete: {:?}",
            out.text
        );
        assert_eq!(out.title.as_deref(), Some("T"));
    }

    #[test]
    fn an_unclosed_removed_element_still_takes_the_rest() {
        // The truncating branch is correct on its own terms: everything after
        // an unclosed <script> is inside it. Kept as a decision, not an
        // accident, now that exact matching stops it firing on `<header>`.
        let out = Builtin
            .extract(&Document::html(
                b"<body><p>Kept.</p><script>var x = 1;<p>Gone.</p></body>",
            ))
            .unwrap();
        assert!(out.text.contains("Kept."));
        assert!(!out.text.contains("Gone."));
    }

    #[test]
    fn script_and_style_bodies_never_reach_the_text() {
        let html = "<html><head><title>My &amp; Page</title><style>.x{}</style></head>\
            <body><script>bad()</script><p>Hello  world</p></body></html>";
        let out = Builtin.extract(&Document::html(html.as_bytes())).unwrap();
        assert_eq!(out.title.as_deref(), Some("My & Page"));
        assert!(out.text.contains("Hello world"), "got: {}", out.text);
        assert!(!out.text.contains("bad()"), "script body must be dropped");
        assert!(!out.text.contains(".x{"), "style body must be dropped");
    }

    #[test]
    fn a_tag_boundary_separates_words_rather_than_welding_them() {
        let out = Builtin
            .extract(&Document::html(b"<a>Home</a><a>Jobs</a>"))
            .unwrap();
        assert_eq!(out.text, "Home Jobs");
    }

    #[test]
    fn plain_text_is_returned_as_it_arrived_apart_from_the_cap() {
        let out = Builtin
            .extract(&Document::text(b"  already extracted  "))
            .unwrap();
        assert_eq!(out.text, "already extracted");
        assert_eq!(out.title, None);
    }

    #[test]
    fn every_class_has_exactly_the_reader_the_registry_says_it_does() {
        // Which extractor owns which class is policy, not an accident of
        // ordering, so it is asserted by name. `Builtin` keeping HTML is the
        // part most likely to be changed by someone who assumes registering
        // xberg meant handing it everything.
        assert_eq!(for_class(InputClass::Html).unwrap().name(), "builtin");
        assert_eq!(for_class(InputClass::PlainText).unwrap().name(), "builtin");
        assert_eq!(for_class(InputClass::Pdf).unwrap().name(), "xberg-1.0.5");
        assert!(require(InputClass::Pdf).is_ok());
    }

    #[test]
    fn each_extractor_refuses_a_class_it_does_not_own() {
        // Neither returns an empty string for a document it cannot read. An
        // empty body is indistinguishable from "this page had nothing", which
        // is what `content_status` means, and would be a lie here.
        assert!(Builtin.extract(&Document::pdf(b"%PDF-1.7")).is_err());
        assert!(Xberg.extract(&Document::html(b"<p>hi</p>")).is_err());
    }

    #[test]
    fn xberg_reads_a_real_pdf_and_says_it_produced_the_text() {
        // A genuine one-page PDF, built here rather than committed: the point
        // is that bytes in this shape come back as their text, and a binary
        // fixture would hide what is being asserted.
        let pdf = minimal_pdf("Transit data should remain inspectable.");
        let out = Xberg
            .extract(&Document::pdf(&pdf))
            .expect("a well-formed PDF must extract");

        assert!(
            out.text.contains("Transit data should remain inspectable"),
            "got: {:?}",
            out.text
        );
        assert!(
            out.producer.starts_with("xberg-1.0.5"),
            "producer must name the extractor: {}",
            out.producer
        );
        assert!(out.text.chars().count() <= TEXT_CAP);
    }

    #[test]
    fn a_pdf_that_is_not_a_pdf_is_an_error_rather_than_empty_text() {
        assert!(Xberg.extract(&Document::pdf(b"not a pdf at all")).is_err());
    }

    /// The smallest PDF that carries one line of extractable text.
    fn minimal_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        let objects = [
            "1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n".to_string(),
            "2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n".to_string(),
            "3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
             /Resources<</Font<</F1 4 0 R>>>>/Contents 5 0 R>>endobj\n"
                .to_string(),
            "4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n".to_string(),
            format!(
                "5 0 obj<</Length {}>>stream\n{}\nendstream endobj\n",
                stream.len(),
                stream
            ),
        ];
        for object in &objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_at = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for offset in &offsets {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer<</Size {}/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_at
        ));
        pdf.into_bytes()
    }

    #[test]
    fn malformed_input_yields_text_or_nothing_but_never_an_error() {
        // The judgement here is that a broken page is not an exceptional
        // condition: pages are broken constantly, and a caller that had to
        // handle an Err per malformed document would end up swallowing it.
        // Emptiness is the signal instead, and `content_status` carries it.
        for (name, bytes) in [
            (
                "unterminated tag",
                &b"<html><body><p>Text<div class=\"x"[..],
            ),
            ("stray closing tag", &b"</p></div>Loose text</span>"[..]),
            (
                "angle brackets in prose",
                &b"<p>Use a &lt; b and c > d</p>"[..],
            ),
            ("nothing at all", &b""[..]),
            ("not html", &b"\x00\x01\x02 binary"[..]),
            ("unclosed comment", &b"<p>Before<!-- never closed"[..]),
        ] {
            let out = Builtin
                .extract(&Document::html(bytes))
                .unwrap_or_else(|e| panic!("{name} returned an error: {e}"));
            assert!(
                out.text.chars().count() <= TEXT_CAP,
                "{name} exceeded the cap"
            );
        }

        // Invalid UTF-8 is lossy-decoded rather than rejected: a page in the
        // wrong declared encoding is still mostly readable text.
        let out = Builtin
            .extract(&Document::html(b"<p>caf\xFF\xFE latte</p>"))
            .unwrap();
        assert!(out.text.contains("latte"), "got: {:?}", out.text);
    }

    #[test]
    fn escaped_markup_does_not_come_back_as_markup() {
        // Decoding `&amp;` first turns `&amp;lt;` into `&lt;`, which the next
        // replacement turns into a real `<`. A page that escaped its markup
        // for display would have it reconstituted one stage before the tag
        // walk eats it. Ordering is the fix, so ordering is what is tested.
        assert_eq!(
            decode_basic_entities("&amp;lt;script&amp;gt;"),
            "&lt;script&gt;"
        );
        assert_eq!(decode_basic_entities("Rust &amp; Zig"), "Rust & Zig");
        assert_eq!(decode_basic_entities("a &lt; b"), "a < b");
        assert_eq!(decode_basic_entities("path&#x2F;to"), "path/to");
        // Untouched when there is nothing to decode.
        assert_eq!(decode_basic_entities("plain text"), "plain text");
    }

    #[test]
    fn a_title_survives_entity_decoding_and_nested_markup() {
        let html = "<title>My &amp; <b>Page</b></title>";
        assert_eq!(html_title(html).as_deref(), Some("My & Page"));
    }

    #[test]
    fn transcript_source_names_what_was_read_not_how_long_it_was() {
        assert_eq!(TranscriptSource::FullText.as_str(), "full-text");
        assert_eq!(TranscriptSource::Abstract.as_str(), "abstract");
    }
}
