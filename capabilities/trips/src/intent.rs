//! A sentence to a plan draft that resolves nothing.
//!
//! There is exactly one way to start a trip today: a form that needs an origin
//! picked from `transit.suggest`, destinations, dates and modes typed field by
//! field. "Somewhere warm in October, under 300 euro, by train, long weekend"
//! has no entry point at all.
//!
//! What this produces is the body of that form, filled in, plus a list of what
//! it could not settle. It is deliberately the smallest possible use of a model:
//!
//! - It persists nothing. The output is a `CreatePlan` nobody has submitted.
//! - It emits no EVA code, no price, no feasibility judgement and no plan id.
//!   Resolving a name to a station is `transit`'s job and stays there.
//! - Every destination comes back as a `place:<slug>` with null coordinates,
//!   which is bit-identical to what the dashboard already mints from typed text.
//!   So the operator still has to pick a real station before anything can be
//!   searched, exactly as before.
//!
//! The model's whole job is turning prose into fields. If it invents a station,
//! the worst case is a slug the operator has to correct in a form, which is the
//! same thing that happens when they mistype one.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A filled-in form, and what it could not settle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntentDraft {
    /// The `CreatePlan` body, ready for a human to review and submit.
    pub draft: Value,
    /// Fields the sentence did not determine, by name. A caller shows these as
    /// blanks rather than letting a default stand in silently.
    pub unresolved: Vec<String>,
    /// What was inferred rather than stated, in plain words, so the operator can
    /// disagree with a guess instead of discovering it later.
    pub assumptions: Vec<String>,
    /// The sentence this came from, so a draft is traceable to its input.
    pub source_text: String,
}

pub const SYSTEM_PROMPT: &str = "\
You turn a travel sentence into JSON for a trip-planning form.

Reply with ONLY a JSON object, no prose and no code fence:
{\"title\": str, \"destinations\": [str], \"date_start\": \"YYYY-MM-DD\"|null,
 \"date_end\": \"YYYY-MM-DD\"|null, \"interests\": str,
 \"transport_modes\": [\"train\"|\"flight\"|\"bus\"|\"car\"|\"ferry\"|\"bike\"|\"walk\"],
 \"travelers\": [str], \"unresolved\": [str], \"assumptions\": [str]}

Rules:
- Destinations are place names as a person would say them. Never a station code.
- Never invent a date. If the sentence gives no dates, use null and add \"dates\" to unresolved.
- Put anything you inferred rather than read into assumptions.
- At most four destinations.";

/// Builds the request body for the chat-completions endpoint.
///
/// Kept separate from the call so the prompt can be tested without a model
/// running, which is most of what is worth testing here.
pub fn request_body(model: &str, sentence: &str) -> Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": sentence }
        ],
        "max_tokens": 700,
        "temperature": 0.2
    })
}

/// Slugifies typed text into a place id, exactly as the dashboard's place field
/// does, so a drafted destination is indistinguishable from a typed one and
/// carries the same null coordinates.
pub fn place_slug(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-').to_string();
    let collapsed = trimmed
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("place:{collapsed}")
}

/// Turns the model's JSON into a `CreatePlan`-shaped body.
///
/// Every field is re-derived here rather than trusted: the model supplies words,
/// this supplies the shape. A model that returns an EVA code, a price or a plan
/// id has those fields dropped on the floor, because nothing reads them.
pub fn draft_from_model_json(sentence: &str, raw: &str) -> Result<IntentDraft, String> {
    draft_from_model_json_on(sentence, raw, &today())
}

/// Today as `YYYY-MM-DD`, from the system clock.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

/// A date the model produced is only kept if it could actually be travelled.
///
/// Measured on 2026-08-11 against the on-device model: asked for "somewhere warm
/// in October" with no year, it returned 2023 both times, and it invented a whole
/// date range for a sentence that named none while simultaneously listing
/// "dates" in its own `unresolved`. A plan silently created for a date three
/// years past is worse than a blank field, so the year is checked here rather
/// than asked for in the prompt. Prompts are not a validation layer.
fn usable_date(value: Option<String>, today: &str) -> Option<String> {
    let value = value?;
    let looks_like_a_date = value.len() == 10
        && value.as_bytes().iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    // String comparison is correct for ISO dates and needs no date library.
    (looks_like_a_date && value.as_str() >= today).then_some(value)
}

pub fn draft_from_model_json_on(
    sentence: &str,
    raw: &str,
    today: &str,
) -> Result<IntentDraft, String> {
    let cleaned = strip_fence(raw);
    let value: Value = serde_json::from_str(&cleaned)
        .map_err(|e| format!("the model did not return JSON ({e}): {}", preview(raw)))?;

    let strings = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let text = |key: &str| -> Option<String> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "null")
            .map(str::to_string)
    };

    let names: Vec<String> = strings("destinations").into_iter().take(4).collect();
    let mut unresolved = strings("unresolved");
    if names.is_empty() && !unresolved.iter().any(|u| u == "destinations") {
        unresolved.push("destinations".into());
    }
    // The check decides, not the model's self-report.
    //
    // It reports "dates" as unresolved almost every time, including when it has
    // just returned correct ones: asked for "Munich the 14th to the 16th of
    // September 2026" it answered 2026-09-14/16 and listed "dates" as unresolved
    // in the same object. Trusting that claim threw away dates the sentence
    // plainly gave. Trusting the dates blindly accepts the 2023 it invents for a
    // sentence with no year. So neither is trusted: the date is kept when it is
    // well-formed and not in the past, and `unresolved` is corrected either way.
    let date_start = usable_date(text("date_start"), today);
    let date_end = usable_date(text("date_end"), today);
    let dates_missing = date_start.is_none() || date_end.is_none();
    unresolved.retain(|u| u != "dates");
    if dates_missing {
        unresolved.push("dates".into());
    }

    let destinations: Vec<Value> = names
        .iter()
        .map(|name| {
            serde_json::json!({
                "id": place_slug(name),
                "name": name,
                "kind": "city",
                "latitude": Value::Null,
                "longitude": Value::Null
            })
        })
        .collect();

    // Modes are checked against the enum rather than passed through: an
    // unknown mode would be rejected by the API later, and a draft that cannot
    // be submitted is worse than one with a blank field.
    let modes: Vec<String> = strings("transport_modes")
        .into_iter()
        .map(|m| m.to_lowercase())
        .filter(|m| {
            ["bike", "bus", "car", "ferry", "flight", "train", "walk"].contains(&m.as_str())
        })
        .collect();

    let draft = serde_json::json!({
        "title": text("title").unwrap_or_else(|| sentence.trim().to_string()),
        // Never guessed. The operator's own starting point is a personal fact
        // and belongs to them, not to a sentence parser.
        "origin": Value::Null,
        "destinations": destinations,
        "date_start": date_start,
        "date_end": date_end,
        "interests": text("interests").unwrap_or_default(),
        "transport_modes": modes,
        "travelers": strings("travelers"),
    });

    if !unresolved.iter().any(|u| u == "origin") {
        unresolved.push("origin".into());
    }

    Ok(IntentDraft {
        draft,
        unresolved,
        assumptions: strings("assumptions"),
        source_text: sentence.to_string(),
    })
}

/// Small models fence their JSON however they feel like. Strip it rather than
/// failing a parse over punctuation.
fn strip_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let without = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    without
        .strip_suffix("```")
        .unwrap_or(without)
        .trim()
        .to_string()
}

fn preview(raw: &str) -> String {
    raw.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_becomes_a_submittable_form_that_resolves_nothing() {
        let raw = r#"{"title":"Warm long weekend","destinations":["Valencia","Lisbon"],
            "date_start":null,"date_end":null,"interests":"warm, under 300 euro",
            "transport_modes":["train"],"travelers":[],
            "unresolved":["dates"],"assumptions":["read 'warm' as southern Europe"]}"#;
        let drafted =
            draft_from_model_json("somewhere warm in October under 300 by train", raw).unwrap();

        let destinations = drafted.draft["destinations"].as_array().unwrap();
        assert_eq!(destinations.len(), 2);
        // A drafted destination is bit-identical to a typed one: a slug and no
        // coordinates, so nothing downstream can mistake it for a resolved place.
        assert_eq!(destinations[0]["id"], "place:valencia");
        assert!(destinations[0]["latitude"].is_null());
        // The origin is never guessed.
        assert!(drafted.draft["origin"].is_null());
        assert!(drafted.unresolved.contains(&"origin".to_string()));
        assert!(drafted.unresolved.contains(&"dates".to_string()));
        assert_eq!(drafted.assumptions.len(), 1);
        assert_eq!(drafted.draft["transport_modes"][0], "train");
    }

    /// The model supplies words; this supplies the shape. Anything it invents
    /// outside the contract has to fall on the floor.
    #[test]
    fn invented_fields_are_dropped_rather_than_carried() {
        let raw = r#"{"title":"Munich","destinations":["Munich"],
            "date_start":"2026-10-02","date_end":"2026-10-05",
            "transport_modes":["train","teleport"],
            "eva":"8000261","price_eur":210,"plan_id":"trip:plan:made-up",
            "unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json("munich in october", raw).unwrap();
        let draft = &drafted.draft;
        for invented in ["eva", "price_eur", "plan_id", "id", "status"] {
            assert!(
                draft.get(invented).is_none(),
                "{invented} must not survive into the draft"
            );
        }
        // An unknown transport mode is dropped, because the API would reject the
        // whole write and a draft that cannot be submitted is worse than a blank.
        let modes = draft["transport_modes"].as_array().unwrap();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0], "train");
    }

    /// The measured failure, pinned. Asked for "somewhere warm in October" with
    /// no year, the on-device model answered 2023 -- twice. A plan quietly
    /// created for a date three years past is worse than a blank field.
    #[test]
    fn a_hallucinated_past_date_is_dropped_rather_than_drafted() {
        let raw = r#"{"destinations":["Valencia"],"date_start":"2023-10-11",
            "date_end":"2023-10-18","unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("warm in october", raw, "2026-08-11").unwrap();
        assert!(drafted.draft["date_start"].is_null());
        assert!(drafted.draft["date_end"].is_null());
        assert!(drafted.unresolved.contains(&"dates".to_string()));

        // A future date the sentence really did give is kept.
        let good = r#"{"destinations":["Munich"],"date_start":"2026-09-14",
            "date_end":"2026-09-16","unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("munich 14-16 sept", good, "2026-08-11").unwrap();
        assert_eq!(drafted.draft["date_start"], "2026-09-14");
        assert!(!drafted.unresolved.contains(&"dates".to_string()));

        // Garbage that is not a date at all is also dropped.
        let bad = r#"{"destinations":["X"],"date_start":"next tuesday","unresolved":[]}"#;
        assert!(draft_from_model_json_on("x", bad, "2026-08-11")
            .unwrap()
            .draft["date_start"]
            .is_null());
    }

    /// The model lists "dates" as unresolved almost every time, including when
    /// it has just returned correct ones. Measured: asked for "Munich the 14th
    /// to the 16th of September 2026" it answered 2026-09-14/16 and called dates
    /// unresolved in the same object. The check decides; the self-report does not.
    #[test]
    fn a_wrong_self_report_does_not_discard_good_dates() {
        let raw = r#"{"destinations":["Munich"],"date_start":"2026-09-14",
            "date_end":"2026-09-16","unresolved":["dates"],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("munich sept 2026", raw, "2026-08-11").unwrap();
        assert_eq!(drafted.draft["date_start"], "2026-09-14");
        assert_eq!(drafted.draft["date_end"], "2026-09-16");
        assert!(
            !drafted.unresolved.contains(&"dates".to_string()),
            "the claim is corrected when the dates are usable"
        );

        // And the reverse: a confident model with a past date still loses it.
        let stale = r#"{"destinations":["Nice"],"date_start":"2023-10-11",
            "date_end":"2023-10-18","unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("warm", stale, "2026-08-11").unwrap();
        assert!(drafted.draft["date_start"].is_null());
        assert!(drafted.unresolved.contains(&"dates".to_string()));
    }

    #[test]
    fn a_fenced_or_broken_reply_is_handled_rather_than_panicking() {
        let fenced = "```json\n{\"destinations\":[\"Porto\"],\"unresolved\":[]}\n```";
        let drafted = draft_from_model_json("porto", fenced).unwrap();
        assert_eq!(drafted.draft["destinations"][0]["name"], "Porto");

        let error = draft_from_model_json("x", "I think you should go to Porto!")
            .expect_err("prose is not a draft");
        assert!(error.contains("did not return JSON"), "got: {error}");
    }

    #[test]
    fn place_slugs_match_what_the_dashboard_mints_from_typed_text() {
        assert_eq!(place_slug("Valencia"), "place:valencia");
        assert_eq!(place_slug("Frankfurt(Main)Hbf"), "place:frankfurt-main-hbf");
        assert_eq!(place_slug("  Bonn  Hbf "), "place:bonn-hbf");
    }

    #[test]
    fn the_prompt_forbids_the_things_the_model_must_not_decide() {
        assert!(SYSTEM_PROMPT.contains("Never invent a date"));
        assert!(SYSTEM_PROMPT.contains("Never a station code"));
        assert!(request_body("apple-on-device", "hi")["messages"][1]["content"] == "hi");
    }
}
