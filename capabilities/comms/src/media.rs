//! Share-link media ingest: a URL becomes a `FeedItem` with metadata, an
//! optional transcript, and an optional summary. External processes (yt-dlp)
//! are invoked via `std::process::Command` with argument arrays only -- never a
//! shell string. Subtitles download into a temp dir that is always removed (via
//! `TmpDir`'s Drop). No raw audio/video is ever written.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::config::Config;
use crate::normalize;
use crate::provenance::StageProvenance;
use crate::store::{FeedItem, Store};
use crate::{CommsError, Result};

/// Max characters of article/transcript text fed to the summarizer prompt.
const SUMMARY_INPUT_CAP: usize = 15_000;
pub const SUMMARY_PROMPT_REVISION: &str = "feed-summary-v2-english";
/// Max characters of extracted article body persisted as transcript.
const ARTICLE_TEXT_CAP: usize = 20_000;
/// Transcript length at or above which `content_status` is `full` (not `thin`).
/// One threshold, read by the classifier here and by the evaluator grading a
/// legacy row whose status predates classification.
pub const CONTENT_FULL_THRESHOLD: usize = 1_000;

/// Typed summarization outcome -- replaces the old `Option<String>` that
/// collapsed every failure into `None`, making them indistinguishable from
/// "not configured" and unretryable.
pub enum SummarizeOutcome {
    Ok(String),
    Unconfigured,
    HttpError(String),
    ModelError(String),
    EmptyResponse,
    Timeout,
}

impl SummarizeOutcome {
    /// Short, loggable error class for the retry ledger.
    pub fn error_class(&self) -> &'static str {
        match self {
            SummarizeOutcome::Ok(_) => "ok",
            SummarizeOutcome::Unconfigured => "unconfigured",
            SummarizeOutcome::HttpError(_) => "http_error",
            SummarizeOutcome::ModelError(_) => "model_error",
            SummarizeOutcome::EmptyResponse => "empty_response",
            SummarizeOutcome::Timeout => "timeout",
        }
    }
}

/// A temp directory removed on drop (covers every early-return / error path).
struct TmpDir(PathBuf);
impl TmpDir {
    fn new(tag: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("comms-ingest-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum GitHubTarget {
    Repo {
        owner: String,
        repo: String,
    },
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
    Blob {
        owner: String,
        repo: String,
        branch: String,
        path: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum HuggingFaceTarget {
    Model { model_id: String },
    Dataset { dataset_id: String },
    Paper { paper_id: String },
}

/// (kind, stream) for a URL. Watch/listen kinds land in `media`; read kinds in
/// `news`. A URL that matches no extractor is an `article`, which is the generic
/// fetch-and-strip path and therefore always a valid fallback.
fn detect(url: &str) -> (&'static str, &'static str) {
    let low = url.to_lowercase();
    if low.contains("youtube.com") || low.contains("youtu.be") {
        ("youtube", "media")
    } else if low.contains("instagram.com") {
        ("instagram", "media")
    } else if parse_github_url(url).is_some() {
        ("github", "news")
    } else if arxiv_id(url).is_some() {
        ("arxiv", "news")
    } else if reddit_permalink(url).is_some() {
        ("reddit", "news")
    } else if parse_huggingface_url(url).is_some() {
        ("huggingface", "news")
    } else if low.ends_with(".mp3") || low.ends_with(".m4a") || low.contains("podcast") {
        ("podcast", "media")
    } else {
        ("article", "news")
    }
}

/// Path segments of a URL, with the scheme, host, query and fragment removed.
/// Empty segments (leading, trailing, doubled slashes) are dropped.
fn path_segments(url: &str) -> Vec<&str> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path = after_scheme.split_once('/').map(|(_, p)| p).unwrap_or("");
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// `(owner, repo)` for a GitHub *repository root* only. Deeper paths (issues,
/// pulls, blobs, a user profile) return None on purpose: the generic article
/// path already renders those readably, while the repo API below answers a
/// question no HTML strip can (description, topics, README).
#[allow(dead_code)]
fn github_repo(url: &str) -> Option<(String, String)> {
    if host_of(url) != "github.com" && host_of(url) != "www.github.com" {
        return None;
    }
    let seg = path_segments(url);
    if seg.len() != 2 {
        return None;
    }
    let repo = seg[1].trim_end_matches(".git");
    if seg[0].is_empty() || repo.is_empty() {
        return None;
    }
    Some((seg[0].to_string(), repo.to_string()))
}

fn parse_github_url(url: &str) -> Option<GitHubTarget> {
    let host = host_of(url);
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    let seg = path_segments(url);
    if seg.len() < 2 {
        return None;
    }
    let owner = seg[0].to_string();
    let repo = seg[1].trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    if seg.len() == 2 {
        return Some(GitHubTarget::Repo { owner, repo });
    }

    if (seg[2] == "issues" || seg[2] == "pull") && seg.len() >= 4 {
        if let Ok(number) = seg[3].parse::<u64>() {
            return Some(GitHubTarget::Issue {
                owner,
                repo,
                number,
            });
        }
    }

    if (seg[2] == "blob" || seg[2] == "raw") && seg.len() >= 5 {
        let branch = seg[3].to_string();
        let path = seg[4..].join("/");
        return Some(GitHubTarget::Blob {
            owner,
            repo,
            branch,
            path,
        });
    }

    Some(GitHubTarget::Repo { owner, repo })
}

fn parse_huggingface_url(url: &str) -> Option<HuggingFaceTarget> {
    let host = host_of(url);
    if !host.ends_with("huggingface.co") {
        return None;
    }
    let seg = path_segments(url);
    if seg.is_empty() {
        return None;
    }

    if seg[0] == "papers" && seg.len() >= 2 {
        return Some(HuggingFaceTarget::Paper {
            paper_id: seg[1].to_string(),
        });
    }

    if seg[0] == "datasets" && seg.len() >= 2 {
        let dataset_id = if seg.len() >= 3 {
            format!("{}/{}", seg[1], seg[2])
        } else {
            seg[1].to_string()
        };
        return Some(HuggingFaceTarget::Dataset { dataset_id });
    }

    if seg.len() >= 2 {
        let model_id = format!("{}/{}", seg[0], seg[1]);
        return Some(HuggingFaceTarget::Model { model_id });
    } else if seg.len() == 1 && !seg[0].contains('/') {
        return Some(HuggingFaceTarget::Model {
            model_id: seg[0].to_string(),
        });
    }

    None
}

/// arXiv identifier from an abs/ or pdf/ URL, version suffix kept (it is part of
/// the identity — v1 and v2 are different papers to a reader). Handles both the
/// modern `2501.12345` form and the legacy `cs/0112017` archive form.
fn arxiv_id(url: &str) -> Option<String> {
    if !host_of(url).ends_with("arxiv.org") {
        return None;
    }
    let seg = path_segments(url);
    let rest = match seg.split_first() {
        Some((first, rest)) if *first == "abs" || *first == "pdf" => rest,
        _ => return None,
    };
    if rest.is_empty() {
        return None;
    }
    let id = rest.join("/");
    let id = id.strip_suffix(".pdf").unwrap_or(&id);
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Canonical `www.reddit.com` comments permalink for a post URL. Only a post
/// (`/r/<sub>/comments/<id>/...`) qualifies — a subreddit listing or a user page
/// has no single piece of content to ingest.
fn reddit_permalink(url: &str) -> Option<String> {
    let host = host_of(url);
    if !(host.ends_with("reddit.com") || host == "redd.it") {
        return None;
    }
    let seg = path_segments(url);
    let idx = seg.iter().position(|s| *s == "comments")?;
    // /r/<sub>/comments/<id> — the id segment must exist.
    let id = seg.get(idx + 1)?;
    let sub = seg.get(idx.checked_sub(1)?)?;
    Some(format!("https://www.reddit.com/r/{sub}/comments/{id}"))
}

#[derive(Deserialize, Default)]
struct YtMeta {
    title: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    uploader_id: Option<String>,
    // duration is parsed but unused today beyond presence; kept for the record.
    #[allow(dead_code)]
    duration: Option<f64>,
}

/// `yt-dlp --dump-json --skip-download <url>` -> parsed metadata.
/// Args for the `--dump-json` metadata call. `impersonate` adds the browser
/// impersonation flags YouTube's anti-bot needs under load.
fn ytdlp_meta_args(url: &str, impersonate: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["--dump-json".into(), "--skip-download".into()];
    if impersonate {
        a.push("--impersonate".into());
        a.push("chrome".into());
    }
    a.push(url.to_string());
    a
}

/// Args for the subtitle fetch. `impersonate` as above.
fn ytdlp_sub_args(url: &str, dir: &Path, impersonate: bool) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--skip-download".into(),
        // Without --ignore-errors, yt-dlp stops after the first missing
        // language. Prefer English, then retain German as a multilingual-input
        // fallback; generated Axon text still defaults to English.
        "--ignore-errors".into(),
        "--write-auto-subs".into(),
        "--write-subs".into(),
        "--sub-langs".into(),
        "en,en-orig,de".into(),
        "--sub-format".into(),
        "vtt/best".into(),
    ];
    if impersonate {
        a.push("--impersonate".into());
        a.push("chrome".into());
    }
    a.push("-P".into());
    a.push(dir.to_string_lossy().into_owned());
    a.push("-o".into());
    a.push("sub".into());
    a.push(url.to_string());
    a
}

/// Run yt-dlp trying browser impersonation first (YouTube 429s the subtitle/
/// metadata endpoints otherwise). If that run fails *because the impersonate
/// target is unavailable on this machine* (stderr mentions "impersonate"),
/// retry once without the flag. Any other failure returns None -- the caller
/// keeps its existing degrade behavior. `make_args(impersonate)` builds the arg
/// list for each attempt.
fn run_ytdlp_with_fallback(
    make_args: impl Fn(bool) -> Vec<String>,
) -> Option<std::process::Output> {
    let first = Command::new("yt-dlp").args(make_args(true)).output().ok()?;
    if first.status.success() {
        return Some(first);
    }
    if String::from_utf8_lossy(&first.stderr).contains("impersonate") {
        let second = Command::new("yt-dlp")
            .args(make_args(false))
            .output()
            .ok()?;
        if second.status.success() {
            return Some(second);
        }
    }
    None
}

fn ytdlp_meta(url: &str) -> Result<YtMeta> {
    let out = run_ytdlp_with_fallback(|imp| ytdlp_meta_args(url, imp)).ok_or_else(|| {
        CommsError::Other("yt-dlp metadata failed (not runnable or fetch error)".into())
    })?;
    // --dump-json emits one JSON object per line; take the first.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().unwrap_or("{}");
    Ok(serde_json::from_str(line).unwrap_or_default())
}

fn author_of(m: &YtMeta) -> Option<String> {
    m.uploader
        .clone()
        .or_else(|| m.channel.clone())
        .or_else(|| m.uploader_id.clone())
}

/// Download subtitles into `dir` and return the parsed transcript, if any.
/// Never fails the ingest -- returns None on any subtitle error.
fn ytdlp_transcript(url: &str, dir: &Path) -> Option<String> {
    run_ytdlp_with_fallback(|imp| ytdlp_sub_args(url, dir, imp))?;
    let vtt = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().map(|x| x == "vtt").unwrap_or(false))?;
    let body = std::fs::read_to_string(&vtt).ok()?;
    let text = parse_vtt(&body);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Strip a VTT into plain transcript text: drop the header, cue numbers,
/// timestamp lines, inline `<...>` tags, and consecutive duplicate lines.
fn parse_vtt(body: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line == "WEBVTT"
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
            || line.starts_with("NOTE")
            || line.contains("-->")
            || line.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        let cleaned = strip_tags(line);
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        if lines.last().map(|l| l == cleaned).unwrap_or(false) {
            continue; // dedupe consecutive duplicates (common in auto-subs)
        }
        lines.push(cleaned.to_string());
    }
    lines.join("\n")
}

/// Remove `<...>` spans (VTT/HTML inline tags) from a line.
fn strip_tags(s: &str) -> String {
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

/// The one HTTP client every extractor uses. A descriptive user agent is not
/// politeness here — Reddit and the GitHub API both reject the default one.
fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("AxonComms/0.1")
        .gzip(true)
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// Fetch an article and extract (title, visible-text).
///
/// This used to sit behind a `Command::new("xberg")` shell-out that executed
/// whatever binary named `xberg` came first on PATH, and fell through to here
/// whenever that failed — which was always, xberg not being installed. It was
/// also an undeclared external dependency, which README.md#dependency-verdicts-and-provenance forbids and
/// `tools/upstream-checker` cannot see, because it validates the manifest and
/// not the code. Removed 2026-07-31; xberg returns as a crate behind the
/// extraction trait once its cooldown clears (#77).
fn extract_article(url: &str) -> Result<(Option<String>, String)> {
    let http = http_client()?;
    let resp = http.get(url).send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "article fetch HTTP {}",
            resp.status()
        )));
    }
    let html = resp.text()?;
    let title = extract_title(&html);
    let text = extract_visible_text(&html);
    Ok((title, text))
}

fn extract_title(html: &str) -> Option<String> {
    let low = html.to_lowercase();
    let start = low.find("<title")?;
    let gt = low[start..].find('>')? + start + 1;
    let end = low[gt..].find("</title>")? + gt;
    let title = decode_basic_entities(&strip_tags(html[gt..end].trim()));
    let title = collapse_ws(title.trim());
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Drop <script>/<style> blocks, turn the remaining markup into text, cap
/// length. Deliberately simple -- no HTML crate dependency.
///
/// What a tag becomes is the whole point. This used to run every tag through
/// `strip_tags` and then `collapse_ws`, which welded the document into one
/// line. That broke both stages downstream of it: `normalize` is a table of
/// line predicates, so a single line longer than `BOILERPLATE_LINE_MAX` made
/// every rule unreachable and stored consent walls verbatim as article text,
/// and a tag boundary that emitted nothing welded `Home</a><a>Jobs` into one
/// token the embedder then scored on.
fn extract_visible_text(html: &str) -> String {
    let without_blocks = remove_blocks(html, &["script", "style", "noscript", "head"]);
    let text = tags_to_breaks(&without_blocks);
    let collapsed = collapse_lines(&decode_basic_entities(&text));
    collapsed.chars().take(ARTICLE_TEXT_CAP).collect()
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
/// page never closed, so the remainder goes -- same call `strip_tags` makes.
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
/// produced. `collapse_ws` cannot be reused: it joins across newlines, which
/// is exactly the structure loss this path exists to avoid.
fn collapse_lines(s: &str) -> String {
    s.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn remove_blocks(html: &str, tags: &[&str]) -> String {
    let mut out = html.to_string();
    for tag in tags {
        loop {
            let low = out.to_lowercase();
            let open = match low.find(&format!("<{tag}")) {
                Some(i) => i,
                None => break,
            };
            let close_tag = format!("</{tag}>");
            let close = match low[open..].find(&close_tag) {
                Some(i) => open + i + close_tag.len(),
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

fn decode_basic_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cap a body at `ARTICLE_TEXT_CAP` characters (not bytes — the caps are about
/// what a summarizer can read, and a multibyte split would corrupt the text).
///
/// Capping is the last thing extraction does. Cleaning is not extraction's job:
/// `normalize::normalize` owns that, in its own stage, over the raw this
/// returns (#86).
fn cap_text(s: String) -> String {
    if s.chars().count() <= ARTICLE_TEXT_CAP {
        s
    } else {
        s.chars().take(ARTICLE_TEXT_CAP).collect()
    }
}

/// Raw extraction output to canonical body plus its status, in one place, so
/// an ingest and a later re-normalization pass cannot disagree about what an
/// item says. The status is derived from the *normalized* text: a page that is
/// 90% cookie banner is `thin`, whatever its raw length said.
fn normalized_body(raw: &str) -> (Option<String>, String) {
    let text = normalize::normalize(raw).text;
    let text = Some(text).filter(|t| !t.trim().is_empty());
    let status = match &text {
        None => "none",
        Some(t) if t.chars().count() >= CONTENT_FULL_THRESHOLD => "full",
        Some(_) => "thin",
    };
    (text, status.to_string())
}

/// The seam between the two stages. Extraction hands over its raw output; this
/// keeps it and stores the normalized form beside it. An extractor that came
/// back with nothing stores nothing, rather than an empty string the summarizer
/// would then be handed.
fn finish_extraction(item: &mut FeedItem, raw: Option<String>) {
    let raw = raw.filter(|r| !r.trim().is_empty());
    let (text, status) = match &raw {
        Some(r) => normalized_body(r),
        None => (None, "none".to_string()),
    };
    item.raw_content = raw;
    item.transcript = text;
    item.content_status = status;
}

fn get_json(
    http: &reqwest::blocking::Client,
    url: &str,
    accept: &str,
) -> Result<serde_json::Value> {
    let resp = http.get(url).header("Accept", accept).send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "{url} -> HTTP {}",
            resp.status()
        )));
    }
    Ok(resp.json()?)
}

/// A GitHub repository: description, topics and README. The README is the
/// transcript because it is the part worth summarizing; the metadata line above
/// it is what the repo page shows and a README often omits.
fn fetch_github(owner: &str, repo: &str) -> Result<(Option<String>, Option<String>, String)> {
    let http = http_client()?;
    let meta = get_json(
        &http,
        &format!("https://api.github.com/repos/{owner}/{repo}"),
        "application/vnd.github+json",
    )?;

    let full_name = meta
        .get("full_name")
        .and_then(|v| v.as_str())
        .unwrap_or(repo);
    let description = meta
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let title = Some(if description.is_empty() {
        full_name.to_string()
    } else {
        format!("{full_name} — {description}")
    });

    let mut head = String::new();
    if !description.is_empty() {
        head.push_str(description);
        head.push('\n');
    }
    let stars = meta
        .get("stargazers_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let language = meta.get("language").and_then(|v| v.as_str()).unwrap_or("—");
    let license = meta
        .get("license")
        .and_then(|l| l.get("spdx_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("—");
    head.push_str(&format!(
        "Stars: {stars} · Language: {language} · License: {license}\n"
    ));
    if let Some(topics) = meta.get("topics").and_then(|v| v.as_array()) {
        let list: Vec<&str> = topics.iter().filter_map(|t| t.as_str()).collect();
        if !list.is_empty() {
            head.push_str(&format!("Topics: {}\n", list.join(", ")));
        }
    }

    // The raw README, when there is one. A repo without one still ingests.
    let readme = http
        .get(format!(
            "https://api.github.com/repos/{owner}/{repo}/readme"
        ))
        .header("Accept", "application/vnd.github.raw")
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.text().ok())
        .unwrap_or_default();

    let author = meta
        .get("owner")
        .and_then(|o| o.get("login"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok((
        title,
        author,
        cap_text(format!("{head}\n{readme}").trim().to_string()),
    ))
}

fn fetch_github_target(
    target: &GitHubTarget,
    url: &str,
) -> Result<(Option<String>, Option<String>, String)> {
    match target {
        GitHubTarget::Repo { owner, repo } => fetch_github(owner, repo),
        GitHubTarget::Issue {
            owner,
            repo,
            number,
        } => {
            let http = http_client()?;
            let issue_res = get_json(
                &http,
                &format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}"),
                "application/vnd.github+json",
            );
            if let Ok(issue) = issue_res {
                let title_text = issue.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let body_text = issue.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let author = issue
                    .get("user")
                    .and_then(|u| u.get("login"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let title = Some(format!("{owner}/{repo}#{number}: {title_text}"));
                let mut text = format!("# {title_text}\n\n{body_text}");

                if let Ok(comments) = get_json(
                    &http,
                    &format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments?per_page=10"),
                    "application/vnd.github+json",
                ) {
                    if let Some(arr) = comments.as_array() {
                        for c in arr {
                            let user = c.get("user").and_then(|u| u.get("login")).and_then(|v| v.as_str()).unwrap_or("?");
                            let comment_body = c.get("body").and_then(|v| v.as_str()).unwrap_or("").trim();
                            if !comment_body.is_empty() {
                                text.push_str(&format!("\n\n---\n**@{user}**:\n{comment_body}"));
                            }
                        }
                    }
                }
                return Ok((title, author, cap_text(text)));
            }
            extract_article(url).map(|(t, text)| (t, Some(owner.clone()), text))
        }
        GitHubTarget::Blob {
            owner,
            repo,
            branch,
            path,
        } => {
            let http = http_client()?;
            let raw_url =
                format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}");
            let resp = http.get(&raw_url).send();
            if let Ok(r) = resp {
                if r.status().is_success() {
                    if let Ok(text) = r.text() {
                        let title = Some(format!("{path} ({owner}/{repo})"));
                        return Ok((title, Some(owner.clone()), cap_text(text)));
                    }
                }
            }
            extract_article(url).map(|(t, text)| (t, Some(owner.clone()), text))
        }
    }
}

fn fetch_huggingface(
    target: &HuggingFaceTarget,
    url: &str,
) -> Result<(Option<String>, Option<String>, String)> {
    match target {
        HuggingFaceTarget::Paper { paper_id } => fetch_arxiv(paper_id),
        HuggingFaceTarget::Model { model_id } => {
            let http = http_client()?;
            let meta_res = get_json(
                &http,
                &format!("https://huggingface.co/api/models/{model_id}"),
                "application/json",
            );
            let raw_readme = http
                .get(format!(
                    "https://huggingface.co/{model_id}/raw/main/README.md"
                ))
                .send()
                .ok()
                .filter(|r| r.status().is_success())
                .and_then(|r| r.text().ok())
                .unwrap_or_default();

            let author = model_id.split_once('/').map(|(a, _)| a.to_string());
            if let Ok(meta) = meta_res {
                let pipeline = meta
                    .get("pipeline_tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("model");
                let downloads = meta.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
                let likes = meta.get("likes").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = Some(format!("{model_id} ({pipeline})"));

                let header = format!("Model: {model_id} · Pipeline: {pipeline} · Downloads: {downloads} · Likes: {likes}");
                let text = format!("{header}\n\n---\n\n{raw_readme}");
                return Ok((title, author, cap_text(text)));
            }

            extract_article(url).map(|(t, text)| (t, author, text))
        }
        HuggingFaceTarget::Dataset { dataset_id } => {
            let http = http_client()?;
            let meta_res = get_json(
                &http,
                &format!("https://huggingface.co/api/datasets/{dataset_id}"),
                "application/json",
            );
            let raw_readme = http
                .get(format!(
                    "https://huggingface.co/datasets/{dataset_id}/raw/main/README.md"
                ))
                .send()
                .ok()
                .filter(|r| r.status().is_success())
                .and_then(|r| r.text().ok())
                .unwrap_or_default();

            let author = dataset_id.split_once('/').map(|(a, _)| a.to_string());
            if let Ok(meta) = meta_res {
                let downloads = meta.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
                let likes = meta.get("likes").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = Some(format!("Dataset: {dataset_id}"));

                let header =
                    format!("Dataset: {dataset_id} · Downloads: {downloads} · Likes: {likes}");
                let text = format!("{header}\n\n---\n\n{raw_readme}");
                return Ok((title, author, cap_text(text)));
            }

            extract_article(url).map(|(t, text)| (t, author, text))
        }
    }
}

/// First `<tag>…</tag>` payload inside `body`, entity-decoded and whitespace-
/// collapsed. Enough for arXiv's Atom, which is machine-generated and flat — a
/// real XML parser would be a dependency bought for one endpoint.
fn xml_field(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    let text = collapse_ws(&decode_basic_entities(body[start..end].trim()));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// An arXiv paper via the Atom export API: title, authors and abstract.
///
/// The abstract is all of it for now, which is what #78 is about. The PDF branch
/// that used to sit here called the `xberg` CLI, which is not installed, so it
/// never once ran — an inert branch that made the capability read as if it
/// pulled full text. It comes back when there is a PDF extractor that actually
/// works (#77), and not before.
fn fetch_arxiv(id: &str) -> Result<(Option<String>, Option<String>, String)> {
    let http = http_client()?;
    let resp = http
        .get(format!(
            "https://export.arxiv.org/api/query?id_list={id}&max_results=1"
        ))
        .send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "arXiv API HTTP {}",
            resp.status()
        )));
    }
    let body = resp.text()?;
    // The feed carries its own <title>/<id> before the first <entry>; slice past
    // it so the fields below can't come from the envelope.
    let entry = body
        .find("<entry>")
        .map(|i| &body[i..])
        .ok_or_else(|| CommsError::Other(format!("arXiv: no entry for {id}")))?;

    let title = xml_field(entry, "title");
    let abstract_text = xml_field(entry, "summary").unwrap_or_default();

    let mut authors: Vec<String> = Vec::new();
    let mut rest = entry;
    while let Some(i) = rest.find("<author>") {
        rest = &rest[i + "<author>".len()..];
        if let Some(name) = xml_field(rest, "name") {
            authors.push(name);
        }
    }
    let author = match authors.len() {
        0 => None,
        1..=3 => Some(authors.join(", ")),
        _ => Some(format!("{} et al.", authors[..3].join(", "))),
    };

    Ok((title, author, cap_text(abstract_text)))
}

/// A Reddit post via the `.json` view of its permalink: the selftext plus the
/// top-level comments, which is where the substance usually is. Link posts have
/// an empty selftext and still ingest — the comments carry them.
///
/// Verified 2026-07-28: Reddit answers 403 to this endpoint for every
/// unauthenticated caller — www and old, descriptive UA, API-format UA and a
/// browser UA alike. The parsing below is against the shape the endpoint still
/// returns for an authorized caller; making it reachable needs a registered
/// Reddit app and an OAuth token against `oauth.reddit.com`, which is a secret
/// only the operator provisions (README.md#secrets). Until then a Reddit paste
/// fails with the message below rather than silently landing as an empty item.
fn fetch_reddit(permalink: &str) -> Result<(Option<String>, Option<String>, String)> {
    let http = http_client()?;
    let body = get_json(&http, &format!("{permalink}.json?raw_json=1&limit=30"), "application/json")
        .map_err(|e| {
            CommsError::Other(format!(
                "{e} — Reddit blockt unauthentifizierte Zugriffe; braucht eine registrierte App + OAuth-Token"
            ))
        })?;

    // [0] is the post listing, [1] the comment listing.
    let post = body
        .get(0)
        .and_then(|l| l.get("data"))
        .and_then(|d| d.get("children"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("data"))
        .ok_or_else(|| CommsError::Other("reddit: no post in listing".into()))?;

    let title = post
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let author = post
        .get("author")
        .and_then(|v| v.as_str())
        .map(|a| format!("u/{a}"));

    let mut text = String::new();
    if let Some(selftext) = post.get("selftext").and_then(|v| v.as_str()) {
        text.push_str(selftext.trim());
    }
    // A link post points somewhere; the target URL is part of the content.
    if let Some(link) = post.get("url_overridden_by_dest").and_then(|v| v.as_str()) {
        text.push_str(&format!("\n\nVerlinkt: {link}"));
    }

    let comments = body
        .get(1)
        .and_then(|l| l.get("data"))
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for c in comments {
        let d = match c.get("data") {
            Some(d) => d,
            None => continue,
        };
        let author = d.get("author").and_then(|v| v.as_str()).unwrap_or("?");
        let comment = d.get("body").and_then(|v| v.as_str()).unwrap_or("").trim();
        if comment.is_empty() {
            continue;
        }
        let score = d.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
        text.push_str(&format!("\n\n--- u/{author} ({score}) ---\n{comment}"));
    }

    Ok((title, author, cap_text(text.trim().to_string())))
}

/// Reject anything that is not plain http(s) before a URL reaches an extractor.
/// `file://` would make yt-dlp and the article fetcher read the local disk, and
/// this runs behind an HTTP endpoint — the check belongs here, at the one door
/// every caller goes through, not at each call site.
fn check_scheme(url: &str) -> Result<()> {
    let low = url.trim().to_lowercase();
    if low.starts_with("http://") || low.starts_with("https://") {
        Ok(())
    } else {
        Err(CommsError::Other(
            "only http(s) URLs can be ingested".into(),
        ))
    }
}

/// Build a `FeedItem` for a URL: metadata + transcript, no summary. Does NOT
/// persist -- the caller upserts. Never leaves temp files behind.
///
/// Split from `ingest` because summarizing is the slow half (a local model, up
/// to two minutes) while this half is what the caller is waiting to see. The
/// server returns after this and summarizes behind the response; the CLI, which
/// prints the summary, calls `ingest` and waits for both.
pub fn fetch(url: &str) -> Result<FeedItem> {
    check_scheme(url)?;
    let (kind, stream) = detect(url);
    let mut item = FeedItem::new(url, stream, kind);

    let raw = match kind {
        "youtube" | "instagram" | "podcast" => {
            let meta = ytdlp_meta(url)?;
            item.title = meta.title.clone();
            item.author = author_of(&meta);
            let tmp = TmpDir::new("subs")?;
            ytdlp_transcript(url, tmp.path())
            // tmp dropped here -> subtitles removed.
        }
        "github" => {
            let target = parse_github_url(url).expect("detect() matched parse_github_url");
            let (title, author, text) = fetch_github_target(&target, url)?;
            item.title = title;
            item.author = author;
            Some(text)
        }
        "huggingface" => {
            let target =
                parse_huggingface_url(url).expect("detect() matched parse_huggingface_url");
            let (title, author, text) = fetch_huggingface(&target, url)?;
            item.title = title;
            item.author = author;
            Some(text)
        }
        "arxiv" => {
            let id = arxiv_id(url).expect("detect() matched arxiv_id");
            let (title, author, text) = fetch_arxiv(&id)?;
            item.title = title;
            item.author = author;
            Some(text)
        }
        "reddit" => {
            let permalink = reddit_permalink(url).expect("detect() matched reddit_permalink");
            let (title, author, text) = fetch_reddit(&permalink)?;
            item.title = title;
            item.author = author;
            Some(text)
        }
        _ => {
            // article
            let (title, text) = extract_article(url)?;
            item.title = title;
            Some(text)
        }
    };

    // An extractor that came back with nothing stores nothing, rather than an
    // empty string the summarizer would then be handed.
    finish_extraction(&mut item, raw);

    Ok(item)
}

fn is_html(raw: &str) -> bool {
    let low = raw.to_lowercase();
    low.contains("<html")
        || low.contains("<body")
        || low.contains("<div")
        || low.contains("<p>")
        || low.contains("<span")
        || low.contains("<article")
}

/// Build a `FeedItem` from a URL and optional client-supplied content, title and
/// author. When content is supplied the server-side fetch is bypassed entirely,
/// which is the whole point: a page behind a login is one the operator can hand
/// over and the server must never go fetch itself.
///
/// `client` names who handed it over and is stored as the item's capture
/// provenance (#81). `None` there means this content was fetched, not captured.
pub fn fetch_with_content(
    url: &str,
    content: Option<&str>,
    title: Option<&str>,
    author: Option<&str>,
    client: Option<&str>,
) -> Result<FeedItem> {
    if content.is_none() && title.is_none() && author.is_none() {
        return fetch(url);
    }
    check_scheme(url)?;
    let (kind, stream) = detect(url);
    let mut item = FeedItem::new(url, stream, kind);

    let extracted = content.map(|raw| {
        if is_html(raw) {
            if item.title.is_none() {
                item.title = extract_title(raw);
            }
            extract_visible_text(raw)
        } else {
            cap_text(raw.trim().to_string())
        }
    });

    if let Some(t) = title {
        if !t.trim().is_empty() {
            item.title = Some(t.trim().to_string());
        }
    }

    if let Some(a) = author {
        if !a.trim().is_empty() {
            item.author = Some(a.trim().to_string());
        }
    }

    finish_extraction(&mut item, extracted);

    // Only a body that actually came from the client is that client's capture.
    // A call that supplied a title and nothing else fetched nothing and
    // captured nothing.
    if content.is_some() {
        item.captured_via = client
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| c.chars().take(64).collect());
    }

    Ok(item)
}

/// `fetch` plus a summary. The CLI path.
pub fn ingest(url: &str, cfg: &Config) -> Result<FeedItem> {
    let mut item = fetch(url)?;
    let summary_producer = summary_producer_revision(cfg);
    if let Some(text) = &item.transcript {
        if let SummarizeOutcome::Ok(summary) = summarize(text, cfg) {
            item.summary = Some(summary);
            item.summary_provenance = summary_producer.map(StageProvenance::model);
        }
    }
    Ok(item)
}

/// Cap the text handed to the summarizer (a 1h transcript is 100k+ chars; the
/// local model's context is finite). Appends a `…[truncated]` marker when cut.
/// The full transcript is still stored in the DB unchanged.
fn truncate_for_summary(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        text.to_string()
    } else {
        let head: String = text.chars().take(cap).collect();
        format!("{head}…[truncated]")
    }
}

fn summary_prompt(input: &str) -> String {
    format!(
        "Summarize the following content as a compact digest. Start with the key points as short \
         bullet points, then add exactly one sentence of context. Write in English, even when the \
         source is in another language. Do not add a preamble.\n\nContent:\n{input}"
    )
}

pub fn summary_producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| format!("{}:{SUMMARY_PROMPT_REVISION}", role.cache_key()))
}

/// Summarize text into a compact English digest via an OpenAI-compatible
/// chat-completions endpoint. Returns a typed outcome so the caller can
/// distinguish "not configured" from "server down" from "empty response" and
/// record the failure class for bounded retry.
pub fn summarize(text: &str, cfg: &Config) -> SummarizeOutcome {
    let role = match cfg.summarization_role() {
        Some(role) => role,
        None => return SummarizeOutcome::Unconfigured,
    };
    let input = truncate_for_summary(text, SUMMARY_INPUT_CAP);
    let prompt = summary_prompt(&input);

    let http = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(e) => return SummarizeOutcome::HttpError(e.to_string()),
    };
    let mut req = http
        .post(role.chat_completions_endpoint())
        .json(&serde_json::json!({
            "model": role.model,
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 800,
            "stream": false,
        }));
    if let Some(key) = role.bearer_key() {
        req = req.bearer_auth(key);
    }
    let resp = match req.send() {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return SummarizeOutcome::Timeout,
        Err(e) => return SummarizeOutcome::HttpError(e.to_string()),
    };
    if !resp.status().is_success() {
        return SummarizeOutcome::ModelError(format!("status {}", resp.status()));
    }
    let body: serde_json::Value = match resp.json() {
        Ok(b) => b,
        Err(e) => return SummarizeOutcome::ModelError(e.to_string()),
    };
    let out = body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::trim)
        .unwrap_or("");
    if out.is_empty() {
        SummarizeOutcome::EmptyResponse
    } else {
        SummarizeOutcome::Ok(out.to_string())
    }
}

/// Cheap readiness probe for the configured OpenAI-compatible summarizer.
/// `/models` does not trigger a generation or load a model into memory.
pub fn summarizer_reachable(cfg: &Config) -> bool {
    cfg.summarization_role()
        .as_ref()
        .is_some_and(crate::inference::ResolvedRole::model_reachable)
}

/// Summarize one stored item, if it has a transcript and no summary yet.
/// Returns whether a summary was written. Records attempt count and error
/// class on failure so the retry ledger is honest. Used by the server after a
/// `POST /ingest` has already answered — deliberately scoped to the single id
/// rather than reusing `summarize_pending`, so two concurrent ingests don't
/// each pick up the other's row.
pub fn summarize_item(store: &Store, cfg: &Config, id: &str) -> Result<bool> {
    let item = match store
        .get_feed(id)
        .map_err(|e| CommsError::Other(e.to_string()))?
    {
        Some(i) => i,
        None => return Ok(false),
    };
    let Some(producer_revision) = summary_producer_revision(cfg) else {
        return Ok(false);
    };
    if !store
        .feed_summary_needs_revision(id, &producer_revision)
        .map_err(|e| CommsError::Other(e.to_string()))?
    {
        return Ok(false);
    }
    let text = match &item.transcript {
        Some(t) => t,
        None => return Ok(false),
    };
    match summarize(text, cfg) {
        SummarizeOutcome::Ok(summary) => {
            store
                .update_feed_summary(id, &summary, &producer_revision)
                .map_err(|e| CommsError::Other(e.to_string()))?;
            Ok(true)
        }
        SummarizeOutcome::Unconfigured => Ok(false),
        outcome => {
            let _ = store.record_summary_attempt(id, outcome.error_class(), &producer_revision);
            Ok(false)
        }
    }
}

/// Retry summarization for eligible feed items (bounded by attempt cap and
/// exponential backoff). Returns how many were newly summarized.
pub fn summarize_pending(store: &Store, cfg: &Config) -> Result<usize> {
    let Some(producer_revision) = summary_producer_revision(cfg) else {
        return Ok(0);
    };
    let pending = store
        .feed_pending_summaries(Some(&producer_revision))
        .map_err(|e| CommsError::Other(e.to_string()))?;
    let mut done = 0;
    for item in pending {
        if let Some(text) = &item.transcript {
            match summarize(text, cfg) {
                SummarizeOutcome::Ok(summary) => {
                    store
                        .update_feed_summary(&item.id, &summary, &producer_revision)
                        .map_err(|e| CommsError::Other(e.to_string()))?;
                    done += 1;
                }
                SummarizeOutcome::Unconfigured => break, // no point continuing
                outcome => {
                    let _ = store.record_summary_attempt(
                        &item.id,
                        outcome.error_class(),
                        &producer_revision,
                    );
                }
            }
        }
    }
    Ok(done)
}

/// The payoff for retaining raw content: re-run normalization over everything
/// stored, without re-fetching a single page. Returns how many items were
/// rewritten and how many carry no raw content to work from (pre-#86 items,
/// which only a re-fetch can fix).
pub fn renormalize_all(store: &Store) -> Result<RenormalizeReport> {
    let ids = store
        .feed_ids_with_raw_content()
        .map_err(|e| CommsError::Other(e.to_string()))?;

    let mut report = RenormalizeReport::default();
    for id in ids {
        let raw = match store
            .get_raw_content(&id)
            .map_err(|e| CommsError::Other(e.to_string()))?
        {
            Some(raw) => raw,
            None => {
                report.skipped += 1;
                continue;
            }
        };

        let (text, status) = normalized_body(&raw);
        store
            .set_normalized(&id, text.as_deref(), &status)
            .map_err(|e| CommsError::Other(e.to_string()))?;
        report.updated += 1;
    }
    Ok(report)
}

/// What a re-normalization pass did. `skipped` is items with no retained raw.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RenormalizeReport {
    pub updated: usize,
    pub skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_kinds() {
        assert_eq!(detect("https://www.youtube.com/watch?v=x").0, "youtube");
        assert_eq!(detect("https://youtu.be/x").0, "youtube");
        assert_eq!(detect("https://www.instagram.com/reel/x").0, "instagram");
        assert_eq!(detect("https://cdn.example.com/ep12.mp3").0, "podcast");
        assert_eq!(
            detect("https://example.com/some-podcast-episode").0,
            "podcast"
        );
        assert_eq!(detect("https://blog.example.com/post").0, "article");
        assert_eq!(detect("https://blog.example.com/post").1, "news");
        assert_eq!(detect("https://youtu.be/x").1, "media");
    }

    #[test]
    fn detect_share_link_kinds() {
        assert_eq!(detect("https://github.com/larsboes/Axon").0, "github");
        assert_eq!(detect("https://github.com/larsboes/Axon").1, "news");
        assert_eq!(detect("https://arxiv.org/abs/2501.12345").0, "arxiv");
        assert_eq!(
            detect("https://www.reddit.com/r/rust/comments/abc123/some_title/").0,
            "reddit"
        );
        // Deeper GitHub paths (issues, PRs, files) are github target extractions.
        assert_eq!(
            detect("https://github.com/larsboes/Axon/issues/7").0,
            "github"
        );
        assert_eq!(detect("https://github.com/larsboes").0, "article");
        // A subreddit listing has no single post to ingest.
        assert_eq!(detect("https://www.reddit.com/r/rust/").0, "article");
    }

    #[test]
    fn github_repo_parses_root_paths_only() {
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon"),
            Some(("larsboes".into(), "Axon".into()))
        );
        // Trailing slash, query and .git suffix are all the same repo.
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon.git?tab=readme"),
            Some(("larsboes".into(), "Axon".into()))
        );
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon/"),
            Some(("larsboes".into(), "Axon".into()))
        );
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon/blob/main/README.md"),
            None
        );
        assert_eq!(github_repo("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn arxiv_id_covers_abs_pdf_and_legacy_ids() {
        assert_eq!(
            arxiv_id("https://arxiv.org/abs/2501.12345").as_deref(),
            Some("2501.12345")
        );
        // The version is part of the identity and survives.
        assert_eq!(
            arxiv_id("https://arxiv.org/pdf/2501.12345v2").as_deref(),
            Some("2501.12345v2")
        );
        assert_eq!(
            arxiv_id("https://arxiv.org/pdf/2501.12345v2.pdf").as_deref(),
            Some("2501.12345v2")
        );
        // Legacy archive-prefixed ids keep their slash.
        assert_eq!(
            arxiv_id("https://arxiv.org/abs/cs/0112017").as_deref(),
            Some("cs/0112017")
        );
        assert_eq!(arxiv_id("https://arxiv.org/list/cs.AI/recent"), None);
        assert_eq!(arxiv_id("https://example.com/abs/2501.12345"), None);
    }

    #[test]
    fn reddit_permalink_canonicalizes_post_urls() {
        assert_eq!(
            reddit_permalink("https://www.reddit.com/r/rust/comments/abc123/some_title/")
                .as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc123")
        );
        // old.reddit.com and a slugless permalink resolve to the same canonical form.
        assert_eq!(
            reddit_permalink("https://old.reddit.com/r/rust/comments/abc123").as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc123")
        );
        assert_eq!(reddit_permalink("https://www.reddit.com/r/rust/"), None);
        assert_eq!(
            reddit_permalink("https://www.reddit.com/user/someone"),
            None
        );
    }

    #[test]
    fn check_scheme_rejects_non_http() {
        assert!(check_scheme("https://example.com").is_ok());
        assert!(check_scheme("http://example.com").is_ok());
        assert!(check_scheme("file:///etc/passwd").is_err());
        assert!(check_scheme("ftp://example.com/x").is_err());
        assert!(check_scheme("example.com").is_err());
    }

    #[test]
    fn xml_field_reads_first_match_decoded() {
        let entry =
            "<entry><title>A &amp; B\n  study</title><summary>  abstract text </summary></entry>";
        assert_eq!(xml_field(entry, "title").as_deref(), Some("A & B study"));
        assert_eq!(
            xml_field(entry, "summary").as_deref(),
            Some("abstract text")
        );
        assert_eq!(xml_field(entry, "author"), None);
    }

    #[test]
    fn ytdlp_args_carry_impersonate_when_requested() {
        let meta = ytdlp_meta_args("https://youtu.be/x", true);
        assert!(
            meta.windows(2).any(|w| w == ["--impersonate", "chrome"]),
            "meta args must include --impersonate chrome: {meta:?}"
        );
        assert_eq!(meta.last().unwrap(), "https://youtu.be/x", "url is last");

        let subs = ytdlp_sub_args("https://youtu.be/x", std::path::Path::new("/tmp/d"), true);
        assert!(subs.windows(2).any(|w| w == ["--impersonate", "chrome"]));
        assert!(subs
            .windows(2)
            .any(|w| w == ["--sub-langs", "en,en-orig,de"]));
        assert_eq!(subs.last().unwrap(), "https://youtu.be/x");
    }

    #[test]
    fn ytdlp_args_omit_impersonate_on_fallback() {
        let meta = ytdlp_meta_args("https://youtu.be/x", false);
        assert!(
            !meta.iter().any(|a| a == "--impersonate"),
            "fallback drops the flag"
        );
        let subs = ytdlp_sub_args("https://youtu.be/x", std::path::Path::new("/tmp/d"), false);
        assert!(!subs.iter().any(|a| a == "--impersonate"));
    }

    #[test]
    fn truncate_for_summary_marks_only_when_cut() {
        assert_eq!(truncate_for_summary("short", 15_000), "short");
        let long = "a".repeat(20_000);
        let out = truncate_for_summary(&long, 15_000);
        assert!(out.ends_with("…[truncated]"), "cut text carries the marker");
        assert_eq!(out.chars().count(), 15_000 + "…[truncated]".chars().count());
    }

    #[test]
    fn summary_prompt_requires_english_output() {
        let prompt = summary_prompt("Ein deutschsprachiger Quelltext.");
        assert!(prompt.contains("Write in English"));
        assert!(prompt.contains("Content:\nEin deutschsprachiger Quelltext."));
    }

    #[test]
    fn parse_vtt_strips_and_dedupes() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n1\n00:00:00.000 --> 00:00:02.000\nHello <c>world</c>\n\n2\n00:00:02.000 --> 00:00:04.000\nHello world\nHello world\nsecond line\n";
        let out = parse_vtt(vtt);
        assert_eq!(out, "Hello world\nsecond line");
    }

    #[test]
    fn strip_tags_removes_spans() {
        assert_eq!(strip_tags("a <b>bold</b> c"), "a bold c");
        assert_eq!(strip_tags("<00:00:01.000><c> hi </c>"), " hi ");
    }

    #[test]
    fn extract_title_and_text() {
        let html = "<html><head><title>My &amp; Page</title><style>.x{}</style></head><body><script>bad()</script><p>Hello  world</p></body></html>";
        assert_eq!(extract_title(html).as_deref(), Some("My & Page"));
        let text = extract_visible_text(html);
        assert!(text.contains("Hello world"), "got: {text}");
        assert!(!text.contains("bad()"), "script body must be dropped");
    }

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

        let raw = extract_visible_text(html);
        let clean = normalize::normalize(&raw);

        assert!(
            !clean.text.contains("Accept all cookies"),
            "consent banner survived normalization: {}",
            clean.text
        );
        assert!(
            clean.text.contains("Transit data should remain inspectable"),
            "the article body must survive: {}",
            clean.text
        );
    }

    #[test]
    fn fetch_with_content_bypasses_fetch_and_sets_status() {
        let url = "https://example.com/protected";
        let markdown = "# Protected Document\n\nThis is supplied content bypass.";
        let item = fetch_with_content(
            url,
            Some(markdown),
            Some("Supplied Title"),
            Some("Supplied Author"),
            Some("axon-clip"),
        )
        .unwrap();

        assert_eq!(item.url, url);
        assert_eq!(item.title.as_deref(), Some("Supplied Title"));
        assert_eq!(item.author.as_deref(), Some("Supplied Author"));
        assert_eq!(item.transcript.as_deref(), Some(markdown));
        assert_eq!(item.content_status, "thin");

        // Long content gets 'full' status
        let long_md = "a".repeat(1200);
        let item_full = fetch_with_content(url, Some(&long_md), None, None, None).unwrap();
        assert_eq!(item_full.content_status, "full");
    }

    #[test]
    fn a_captured_item_says_which_client_handed_it_over() {
        let url = "https://example.com/behind-a-login";
        let body = "# Members only\n\nThe part the server could never fetch.";

        let captured = fetch_with_content(url, Some(body), None, None, Some("axon-clip")).unwrap();
        assert_eq!(captured.captured_via.as_deref(), Some("axon-clip"));

        // Handed over by something that did not name itself: still a capture,
        // but there is nothing truthful to record about who.
        let anonymous = fetch_with_content(url, Some(body), None, None, None).unwrap();
        assert_eq!(anonymous.captured_via, None);

        // A client name on a call that supplied no body fetched nothing and
        // captured nothing, so it must not be labelled a capture.
        let title_only =
            fetch_with_content(url, None, Some("Just a title"), None, Some("axon-clip")).unwrap();
        assert_eq!(title_only.captured_via, None);
    }

    #[test]
    fn arxiv_id_extraction_and_pdf_url_mapping() {
        let abs_url = "https://arxiv.org/abs/2501.12345v1";
        let id = arxiv_id(abs_url).unwrap();
        assert_eq!(id, "2501.12345v1");

        let pdf_url = format!("https://arxiv.org/pdf/{id}.pdf");
        assert_eq!(pdf_url, "https://arxiv.org/pdf/2501.12345v1.pdf");
    }

    #[test]
    fn parse_github_deep_paths() {
        let repo_url = "https://github.com/larsboes/Axon";
        assert_eq!(
            parse_github_url(repo_url),
            Some(GitHubTarget::Repo {
                owner: "larsboes".into(),
                repo: "Axon".into()
            })
        );

        let issue_url = "https://github.com/larsboes/Axon/issues/80";
        assert_eq!(
            parse_github_url(issue_url),
            Some(GitHubTarget::Issue {
                owner: "larsboes".into(),
                repo: "Axon".into(),
                number: 80
            })
        );

        let blob_url = "https://github.com/larsboes/Axon/blob/main/capabilities/comms/src/media.rs";
        assert_eq!(
            parse_github_url(blob_url),
            Some(GitHubTarget::Blob {
                owner: "larsboes".into(),
                repo: "Axon".into(),
                branch: "main".into(),
                path: "capabilities/comms/src/media.rs".into()
            })
        );
    }

    #[test]
    fn parse_huggingface_urls() {
        let model_url = "https://huggingface.co/mlx-community/multilingual-e5-base-mlx";
        assert_eq!(
            parse_huggingface_url(model_url),
            Some(HuggingFaceTarget::Model {
                model_id: "mlx-community/multilingual-e5-base-mlx".into()
            })
        );

        let dataset_url = "https://huggingface.co/datasets/glue";
        assert_eq!(
            parse_huggingface_url(dataset_url),
            Some(HuggingFaceTarget::Dataset {
                dataset_id: "glue".into()
            })
        );

        let paper_url = "https://huggingface.co/papers/2501.12345";
        assert_eq!(
            parse_huggingface_url(paper_url),
            Some(HuggingFaceTarget::Paper {
                paper_id: "2501.12345".into()
            })
        );
    }

    #[test]
    fn extraction_and_normalization_keep_separate_outputs() {
        let raw = "Menu\nWe use cookies. Accept all\n\nThe actual article body.\n";
        let mut item = FeedItem::new("https://example.com/a", "news", "article");
        finish_extraction(&mut item, Some(raw.to_string()));

        assert_eq!(
            item.raw_content.as_deref(),
            Some(raw),
            "the extractor's output is retained verbatim"
        );
        assert_eq!(item.transcript.as_deref(), Some("The actual article body."));
    }

    #[test]
    fn content_status_follows_the_normalized_text_not_the_raw_bytes() {
        // Raw clears the 1k threshold; almost all of it is boilerplate, so the
        // normalized body does not. Deriving from raw would call this `full`.
        let boilerplate = "Share on Twitter\n".repeat(80);
        let raw = format!("{boilerplate}Two short sentences of actual content.");
        assert!(raw.chars().count() >= CONTENT_FULL_THRESHOLD);

        let mut item = FeedItem::new("https://example.com/b", "news", "article");
        finish_extraction(&mut item, Some(raw));
        assert_eq!(item.content_status, "thin");
    }

    #[test]
    fn an_item_that_is_all_boilerplate_stores_no_transcript() {
        let mut item = FeedItem::new("https://example.com/c", "news", "article");
        finish_extraction(&mut item, Some("Menu\nHome\nCopy link\n".to_string()));
        assert_eq!(item.transcript, None);
        assert_eq!(item.content_status, "none");
        assert!(
            item.raw_content.is_some(),
            "raw is kept even when all of it is dropped"
        );
    }
}
