//! Pure-Rust PDF/email/regex parsing of German rail ticket confirmations.
//! Ported close to as-is -- this part of the original service was already
//! well-built (10 unit tests, clean error handling) and needed no DB, no
//! network, and no redaction (no personal data in the parsing logic itself).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedLeg {
    pub origin_name: String,
    pub destination_name: String,
    pub train_number: Option<String>,
    pub departure_time: String,
    pub arrival_time: String,
    pub price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTicket {
    pub source_file: String,
    pub text_preview: String,
    pub legs: Vec<ExtractedLeg>,
    pub confirmation_number: Option<String>,
    pub meta: ExtractionMeta,
    /// Whether this parse produced at least one leg to review. Not "the file was
    /// readable": a confirmation whose stations did not parse is readable and
    /// useless, and those are the same value to any caller that only checks a
    /// boolean.
    pub ok: bool,
    /// The fields this parse could not find, by name.
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionMeta {
    pub trains_found: Vec<String>,
    pub stations: StationPair,
    pub has_date: bool,
    pub has_price: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationPair {
    pub origin: Option<String>,
    pub destination: Option<String>,
}

static TRAIN_RE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(ICE\s*\d+)").unwrap(),
        Regex::new(r"(?i)(IC\s*\d+)").unwrap(),
        Regex::new(r"(?i)(EC\s*\d+)").unwrap(),
        Regex::new(r"(?i)(RE\s*\d+)").unwrap(),
        Regex::new(r"(?i)(RB\s*\d+)").unwrap(),
        Regex::new(r"(?i)(IRE\s*\d+)").unwrap(),
        Regex::new(r"(?i)\bS\s*(\d+)").unwrap(),
        Regex::new(r"(?i)(FLX\s*\d+)").unwrap(),
        Regex::new(r"(?i)(NJ\s*\d+)").unwrap(),
        Regex::new(r"(?i)(EN\s*\d+)").unwrap(),
    ]
});

static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{2})\.(\d{2})\.(\d{4})").unwrap());
static ISO_DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap());
static TIME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{2}):(\d{2})").unwrap());

static CONFIRMATION_RE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(?:Buchungsnummer|Buchungscode|Auftragsnummer)\s*[:\-]?\s*([A-Z0-9\-]{6,30})").unwrap(),
        Regex::new(r"(?i)(?:Booking\s*.?code|Booking\s*.?number|Confirmation|Reference)\s*[:\-]?\s*([A-Z0-9\-]{6,30})").unwrap(),
        Regex::new(r"(?i)Ticket\s*[:\-]?\s*([A-Z0-9\-]{6,30})").unwrap(),
    ]
});

static VON_NACH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:von|from|ab)\s+([A-Za-zÄÖÜäöüß\s.\-()]+?)\s+(?:nach|to|an|um)\s+([A-Za-zÄÖÜäöüß\s.\-()]+?)(?:\s+(?:am|um|ab|\d)|$)").unwrap()
});
// `Von` was missing here while `Nach` was already in TO_RE below, so a German
// confirmation laid out as "Von: ... / Nach: ..." parsed its destination and lost
// its origin -- and with one station missing, `parse_ticket_text` builds no legs
// at all. German rail confirmations are this parser's entire subject, so the one
// label it could not read was the one it most needed.
static FROM_TO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:From|Von|Ab|Start|Origin)\s*:\s*(.+)$").unwrap());
static TO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:To|Nach|End|Destination|Ziel)\s*:\s*(.+)$").unwrap());

static PRICE_RE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(?:Preis|Price|Total|Sum|Betrag)\s*[:\-]?\s*(\d+[.,]\d{2})").unwrap(),
        Regex::new(r"(\d+[.,]\d{2})\s*[€€]").unwrap(),
        Regex::new(r"[€€]\s*(\d+[.,]\d{2})").unwrap(),
        Regex::new(r"(\d+[.,]\d{2})\s*EUR").unwrap(),
    ]
});

pub fn extract_from_bytes(bytes: &[u8], file_name: &str) -> Result<ExtractedTicket, String> {
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let text = match ext.as_str() {
        "pdf" => extract_pdf_text(bytes).map_err(|e| format!("PDF extraction failed: {e}"))?,
        "eml" => extract_email_text(bytes).map_err(|e| format!("Email extraction failed: {e}"))?,
        "txt" | "text" | "html" | "htm" => String::from_utf8_lossy(bytes).to_string(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" => {
            return Err(
                "Image files require OCR - not yet supported. Convert to PDF or text first."
                    .to_string(),
            );
        }
        _ => String::from_utf8_lossy(bytes).to_string(),
    };

    Ok(parse_ticket_text(&text, file_name))
}

fn extract_pdf_text(bytes: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let text = pdf_extract::extract_text_from_mem(bytes)?;
    Ok(text)
}

fn extract_email_text(bytes: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let parsed = mailparse::parse_mail(bytes)?;
    let ct = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("content-type"))
        .and_then(|h| {
            let val = h.get_value();
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        });

    let is_html = ct.as_deref().is_some_and(|v| v.contains("text/html"));

    let body = if is_html {
        let html = parsed.get_body()?;
        html2text::from_read(&mut html.as_bytes(), Default::default())
    } else {
        parsed.get_body()?
    };

    let subject = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("subject"))
        .map(|h| h.get_value());

    let from = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("from"))
        .map(|h| h.get_value());

    let mut text = String::new();
    if let Some(s) = &subject {
        text.push_str(&format!("Subject: {s}\n"));
    }
    if let Some(f) = &from {
        text.push_str(&format!("From: {f}\n"));
    }
    text.push_str(&body);
    Ok(text)
}

fn parse_ticket_text(text: &str, file_name: &str) -> ExtractedTicket {
    let text_preview = if text.len() > 500 {
        format!("{}...", &text[..500])
    } else {
        text.to_string()
    };

    let trains = parse_trains(text);
    let stations = parse_stations(text);
    let (departure, arrival) = parse_date_time(text);
    let confirmation = parse_confirmation(text);
    let price = parse_price(text);

    let legs = if !trains.is_empty() && stations.origin.is_some() && stations.destination.is_some()
    {
        let default_time = format!("{}T00:00:00", chrono_now_date());
        let dep = departure.as_deref().unwrap_or(&default_time).to_string();
        let arr = arrival.as_deref().unwrap_or(&dep).to_string();

        trains
            .iter()
            .map(|train| ExtractedLeg {
                origin_name: stations.origin.clone().unwrap_or_default(),
                destination_name: stations.destination.clone().unwrap_or_default(),
                train_number: Some(train.clone()),
                departure_time: dep.clone(),
                arrival_time: arr.clone(),
                price,
            })
            .collect()
    } else {
        vec![]
    };

    // `ok` was the literal `true`, so a parse that found a train number and
    // nothing else reported success with an empty leg list. A realistic DB
    // confirmation does exactly that: on a 2026-08-11 check, the order number and
    // `ICE 1513` parsed while both station names did not, and the reply still
    // said ok. A caller cannot tell that apart from a ticket with no legs.
    //
    // `ok` now means "there is at least one leg here to review". `missing` says
    // what a reviewer has to supply, because "not ok" with no reason sends them
    // back to the PDF to work out which half failed.
    let ok = !legs.is_empty();
    let missing = missing_fields(&trains, &stations, departure.is_some(), price.is_some());

    ExtractedTicket {
        source_file: file_name.to_string(),
        text_preview,
        legs,
        confirmation_number: confirmation,
        meta: ExtractionMeta {
            trains_found: trains,
            stations,
            has_date: departure.is_some(),
            has_price: price.is_some(),
        },
        ok,
        missing,
        error: None,
    }
}

/// What a parse could not find, named for whoever has to fill it in.
fn missing_fields(
    trains: &[String],
    stations: &StationPair,
    has_date: bool,
    has_price: bool,
) -> Vec<String> {
    let mut missing = Vec::new();
    if trains.is_empty() {
        missing.push("trains".to_string());
    }
    if stations.origin.is_none() {
        missing.push("origin".to_string());
    }
    if stations.destination.is_none() {
        missing.push("destination".to_string());
    }
    if !has_date {
        missing.push("date".to_string());
    }
    if !has_price {
        missing.push("price".to_string());
    }
    missing
}

fn parse_trains(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for re in TRAIN_RE.iter() {
        for cap in re.captures_iter(text) {
            let t = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if !t.is_empty() {
                found.push(t.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

fn parse_stations(text: &str) -> StationPair {
    let mut origin: Option<String> = None;
    let mut destination: Option<String> = None;

    if let Some(cap) = VON_NACH_RE.captures(text) {
        if origin.is_none() {
            origin = cap.get(1).map(|m| m.as_str().trim().to_string());
        }
        if destination.is_none() {
            destination = cap.get(2).map(|m| m.as_str().trim().to_string());
        }
    }

    for line in text.lines() {
        if let Some(cap) = FROM_TO_RE.captures(line) {
            if origin.is_none() {
                origin = cap.get(1).map(|m| m.as_str().trim().to_string());
            }
        }
        if let Some(cap) = TO_RE.captures(line) {
            if destination.is_none() {
                destination = cap.get(1).map(|m| m.as_str().trim().to_string());
            }
        }
    }

    StationPair {
        origin,
        destination,
    }
}

fn parse_date_time(text: &str) -> (Option<String>, Option<String>) {
    let dates: Vec<String> = DATE_RE
        .captures_iter(text)
        .map(|c| format!("{}-{}-{}", &c[3], &c[2], &c[1]))
        .chain(
            ISO_DATE_RE
                .captures_iter(text)
                .map(|c| format!("{}-{}-{}", &c[1], &c[2], &c[3])),
        )
        .collect();

    let times: Vec<String> = TIME_RE
        .captures_iter(text)
        .map(|c| format!("{}:{}", &c[1], &c[2]))
        .collect();

    let departure = dates.first().map(|d| {
        let t = times
            .first()
            .map(|t| format!("T{t}:00"))
            .unwrap_or_else(|| "T00:00:00".into());
        format!("{d}{t}")
    });

    let arrival = if dates.len() > 1 && times.len() > 1 {
        Some(format!("{}T{}:00", &dates[1], &times[1]))
    } else if times.len() > 1 {
        dates.first().map(|d| format!("{}T{}:00", d, &times[1]))
    } else {
        None
    };

    (departure, arrival)
}

fn parse_confirmation(text: &str) -> Option<String> {
    for re in CONFIRMATION_RE.iter() {
        if let Some(cap) = re.captures(text) {
            let val = cap.get(1).map(|m| m.as_str().trim().to_string());
            if val.as_ref().is_some_and(|v| !v.is_empty()) {
                return val;
            }
        }
    }
    None
}

fn parse_price(text: &str) -> Option<f64> {
    for re in PRICE_RE.iter() {
        if let Some(cap) = re.captures(text) {
            let raw = cap.get(1).map(|m| m.as_str().replace(',', "."));
            if let Some(Ok(v)) = raw.map(|s| s.parse::<f64>()) {
                if v > 0.0 {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn chrono_now_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let y = 1970 + (days * 400 + 365) / 146097;
    format!("{y}-01-01")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_train_numbers() {
        let text = "ICE 123 von Berlin Hbf nach München Hbf";
        let trains = parse_trains(text);
        assert_eq!(trains, vec!["ICE 123"]);
    }

    #[test]
    fn parse_ic_number() {
        let text = "Your booking: IC 2044";
        let trains = parse_trains(text);
        assert!(trains.contains(&"IC 2044".to_string()));
    }

    #[test]
    fn parse_von_nach() {
        let text = "ICE 123 von Berlin Hbf nach München Hbf";
        let s = parse_stations(text);
        assert_eq!(s.origin.as_deref(), Some("Berlin Hbf"));
        assert_eq!(s.destination.as_deref(), Some("München Hbf"));
    }

    #[test]
    fn parse_from_to_labels() {
        let text = "From: Hamburg Hbf\nTo: Frankfurt(Main)Hbf";
        let s = parse_stations(text);
        assert_eq!(s.origin.as_deref(), Some("Hamburg Hbf"));
        assert_eq!(s.destination.as_deref(), Some("Frankfurt(Main)Hbf"));
    }

    #[test]
    fn parse_date_and_times() {
        let text = "am 15.07.2026 um 08:30, an 14:15";
        let (dep, arr) = parse_date_time(text);
        assert_eq!(dep.as_deref(), Some("2026-07-15T08:30:00"));
        assert_eq!(arr.as_deref(), Some("2026-07-15T14:15:00"));
    }

    #[test]
    fn parse_iso_date() {
        let text = "Date: 2026-07-20";
        let (dep, _) = parse_date_time(text);
        assert!(dep.as_deref() == Some("2026-07-20T00:00:00"));
    }

    /// The exact text that exposed this: a realistic DB confirmation whose order
    /// number, train and price all parse while neither station does. It used to
    /// come back `ok: true` with an empty leg list, which no caller can tell
    /// apart from a ticket that legitimately has no legs.
    #[test]
    fn a_parse_with_no_legs_is_not_ok_and_says_what_is_missing() {
        let text = "Deutsche Bahn - Ihre Buchung\n\
                    Auftragsnummer: XJ4K7Q\n\
                    Hinfahrt am 25.08.2026\n\
                    ICE 1513  Bonn Hbf  ab 10:07\n\
                    Frankfurt(Main)Hbf an 11:49\n\
                    Preis: 68,47 EUR\n";
        let result = parse_ticket_text(text, "ticket.txt");

        assert!(result.legs.is_empty(), "precondition: this text yields no legs");
        assert!(!result.ok, "a parse with no legs must not report ok");
        assert_eq!(result.confirmation_number.as_deref(), Some("XJ4K7Q"));
        assert_eq!(result.meta.trains_found, vec!["ICE 1513"]);
        assert!(result.meta.has_price, "the price does parse");

        // The reviewer is told which half failed rather than being sent back to
        // the PDF to work it out.
        assert!(result.missing.contains(&"origin".to_string()));
        assert!(result.missing.contains(&"destination".to_string()));
        assert!(!result.missing.contains(&"trains".to_string()));
        assert!(!result.missing.contains(&"price".to_string()));
    }

    /// The other direction, and the `Von:` fix at the same time. This exact
    /// layout used to lose its origin, because `From|Ab|Start|Origin` did not
    /// include the German label while `Nach` was already accepted for the
    /// destination. One missing station means no legs at all.
    #[test]
    fn a_german_labelled_ticket_parses_both_stations_and_stays_ok() {
        let text = "Von: Bonn Hbf\nNach: Frankfurt(Main)Hbf\n\
                    ICE 1513\nam 25.08.2026 um 10:07\nPreis: 68,47 EUR\n";
        let result = parse_ticket_text(text, "ticket.txt");
        assert_eq!(result.meta.stations.origin.as_deref(), Some("Bonn Hbf"));
        assert_eq!(
            result.meta.stations.destination.as_deref(),
            Some("Frankfurt(Main)Hbf")
        );
        assert!(!result.legs.is_empty());
        assert!(result.ok);
        assert!(
            result.missing.is_empty(),
            "nothing should be missing, got: {:?}",
            result.missing
        );
    }

    /// `Ziel:` is the other German label a confirmation uses for a destination.
    #[test]
    fn ziel_is_read_as_a_destination_label() {
        let s = parse_stations("Von: Köln Hbf\nZiel: Hamburg Hbf");
        assert_eq!(s.origin.as_deref(), Some("Köln Hbf"));
        assert_eq!(s.destination.as_deref(), Some("Hamburg Hbf"));
    }

    #[test]
    fn parse_confirmation_code() {
        let text = "Buchungsnummer: ABC123DEF";
        assert_eq!(parse_confirmation(text).as_deref(), Some("ABC123DEF"));
    }

    #[test]
    fn parse_booking_code() {
        let text = "Booking code: BKG-7X9K2M";
        assert_eq!(parse_confirmation(text).as_deref(), Some("BKG-7X9K2M"));
    }

    #[test]
    fn parse_price_de() {
        let text = "Preis: 89,90 EUR";
        assert!((parse_price(text).unwrap() - 89.90).abs() < 0.01);
    }

    #[test]
    fn parse_price_en() {
        let text = "Price: 72,50 EUR";
        assert!((parse_price(text).unwrap() - 72.50).abs() < 0.01);
    }

    #[test]
    fn end_to_end_text() {
        let text = "ICE 123 von Berlin Hbf nach München Hbf am 15.07.2026 um 08:30, an 14:15. Preis: 89,90 EUR. Buchungsnummer: ABC123DEF";
        let result = parse_ticket_text(text, "ticket.txt");
        assert!(result.ok);
        assert_eq!(result.legs.len(), 1);
        assert_eq!(result.legs[0].origin_name, "Berlin Hbf");
        assert_eq!(result.legs[0].destination_name, "München Hbf");
        assert_eq!(result.legs[0].train_number.as_deref(), Some("ICE 123"));
        assert_eq!(result.legs[0].departure_time, "2026-07-15T08:30:00");
        assert!(result.confirmation_number.as_deref() == Some("ABC123DEF"));
    }

    #[test]
    fn end_to_end_ic_ticket() {
        let text = "Your booking: IC 2044\nFrom: Hamburg Hbf\nTo: Frankfurt(Main)Hbf\nDate: 2026-07-20\nDeparture: 09:45\nArrival: 14:30\nPrice: 72,50 EUR\nBooking code: BKG-7X9K2M";
        let result = parse_ticket_text(text, "ticket.txt");
        assert!(result.ok);
        assert_eq!(result.legs.len(), 1);
        assert_eq!(result.legs[0].origin_name, "Hamburg Hbf");
        assert!((result.legs[0].price.unwrap_or(0.0) - 72.50).abs() < 0.01);
        assert_eq!(result.confirmation_number.as_deref(), Some("BKG-7X9K2M"));
    }
}
