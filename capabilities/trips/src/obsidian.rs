use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::{PlaceKind, PlaceRef, TransportMode};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObsidianTripCandidate {
    pub reference: String,
    pub title: String,
    pub summary: String,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub destination: Option<PlaceRef>,
    pub status: String,
    pub travelers: Vec<String>,
    pub transport_modes: Vec<TransportMode>,
    pub cover: Option<String>,
    pub issues: Vec<String>,
    #[serde(default)]
    pub imported_plan_id: Option<String>,
}

#[derive(Default)]
struct Frontmatter {
    scalars: HashMap<String, String>,
    lists: HashMap<String, Vec<String>>,
}

pub fn scan_trip_notes(
    root: &Path,
    trips_dir: &Path,
) -> Result<Vec<ObsidianTripCandidate>, Box<dyn std::error::Error>> {
    let canonical_root = root.canonicalize()?;
    let scan_root = canonical_root.join(trips_dir).canonicalize()?;
    if !scan_root.starts_with(&canonical_root) {
        return Err("obsidian trips_dir must stay inside the configured vault".into());
    }

    let mut paths = Vec::new();
    collect_markdown(&scan_root, &mut paths)?;
    paths.sort();
    let mut candidates = Vec::new();
    for path in paths {
        let body = fs::read_to_string(&path)?;
        let Some(candidate) = parse_candidate(&canonical_root, &path, &body)? else {
            continue;
        };
        candidates.push(candidate);
    }
    candidates.sort_by(|a, b| {
        b.date_start
            .as_deref()
            .unwrap_or("")
            .cmp(a.date_start.as_deref().unwrap_or(""))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(candidates)
}

pub fn read_trip_note(
    root: &Path,
    reference: &str,
) -> Result<ObsidianTripCandidate, Box<dyn std::error::Error>> {
    let canonical_root = root.canonicalize()?;
    let path = canonical_root.join(reference).canonicalize()?;
    if !path.starts_with(&canonical_root)
        || path.extension().and_then(|value| value.to_str()) != Some("md")
    {
        return Err(
            "obsidian reference must be a Markdown file inside the configured vault".into(),
        );
    }
    let body = fs::read_to_string(&path)?;
    parse_candidate(&canonical_root, &path, &body)?
        .ok_or_else(|| "the selected note is not marked category: trip".into())
}

fn collect_markdown(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_markdown(&path, paths)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("md")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn parse_candidate(
    root: &Path,
    path: &Path,
    body: &str,
) -> Result<Option<ObsidianTripCandidate>, Box<dyn std::error::Error>> {
    let frontmatter = parse_frontmatter(body);
    if frontmatter.scalars.get("category").map(String::as_str) != Some("trip") {
        return Ok(None);
    }

    let reference = path
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/");
    let title = first_heading(body)
        .or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Unbenannte Reise".into());
    let summary = frontmatter
        .scalars
        .get("summary")
        .cloned()
        .unwrap_or_default();
    let date_start = frontmatter
        .scalars
        .get("start")
        .filter(|value| !value.is_empty())
        .cloned();
    let date_end = frontmatter
        .scalars
        .get("end")
        .filter(|value| !value.is_empty())
        .cloned();
    let location = frontmatter
        .scalars
        .get("location")
        .map(|value| clean_wikilink(value))
        .filter(|value| !value.is_empty());
    let coordinates = frontmatter
        .lists
        .get("coordinates")
        .cloned()
        .unwrap_or_default();
    let latitude = coordinates.first().and_then(|value| value.parse().ok());
    let longitude = coordinates.get(1).and_then(|value| value.parse().ok());
    let destination = location.map(|name| PlaceRef {
        id: format!("obsidian-place:{}", slug(&name)),
        name,
        kind: PlaceKind::City,
        address: None,
        latitude,
        longitude,
    });
    let travelers = frontmatter
        .lists
        .get("people")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|value| clean_wikilink(&value))
        .filter(|value| !value.is_empty())
        .collect();
    let transport_modes = modes_from_value(
        frontmatter
            .scalars
            .get("transport")
            .map(String::as_str)
            .unwrap_or(""),
    );
    let status = frontmatter
        .scalars
        .get("status")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| "planned".into());
    let cover = frontmatter
        .scalars
        .get("cover")
        .filter(|value| !value.is_empty())
        .cloned();

    let mut issues = Vec::new();
    if date_start.is_none() {
        issues.push("Startdatum fehlt".into());
    }
    if date_end.is_none() {
        issues.push("Enddatum fehlt".into());
    }
    if destination.is_none() {
        issues.push("Zielort fehlt".into());
    }

    Ok(Some(ObsidianTripCandidate {
        reference,
        title,
        summary,
        date_start,
        date_end,
        destination,
        status,
        travelers,
        transport_modes,
        cover,
        issues,
        imported_plan_id: None,
    }))
}

fn parse_frontmatter(body: &str) -> Frontmatter {
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Frontmatter::default();
    }
    let mut parsed = Frontmatter::default();
    let mut active_list: Option<String> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("- ") {
            if let Some(key) = active_list.as_ref() {
                parsed
                    .lists
                    .entry(key.clone())
                    .or_default()
                    .push(clean_scalar(value));
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_owned();
        let value = clean_scalar(value);
        if value == "[]" {
            parsed.lists.insert(key.clone(), Vec::new());
            active_list = Some(key);
        } else if value.is_empty() {
            active_list = Some(key.clone());
            parsed.scalars.insert(key, String::new());
        } else {
            active_list = None;
            parsed.scalars.insert(key, value);
        }
    }
    parsed
}

fn clean_scalar(value: &str) -> String {
    let trimmed = value.trim();
    let unquoted = if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    unquoted.trim().to_owned()
}

fn clean_wikilink(value: &str) -> String {
    let trimmed = clean_scalar(value);
    if let Some(inner) = trimmed
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
    {
        return inner
            .rsplit_once('|')
            .map(|(_, label)| label)
            .unwrap_or(inner)
            .trim()
            .to_owned();
    }
    trimmed
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn modes_from_value(value: &str) -> Vec<TransportMode> {
    let normalized = value.to_ascii_lowercase();
    if normalized == "mixed" {
        return vec![
            TransportMode::Train,
            TransportMode::Flight,
            TransportMode::Car,
            TransportMode::Bus,
            TransportMode::Ferry,
        ];
    }
    normalized
        .split([',', '/', '+'])
        .filter_map(|part| match part.trim() {
            "bike" | "bicycle" => Some(TransportMode::Bike),
            "bus" => Some(TransportMode::Bus),
            "car" => Some(TransportMode::Car),
            "ferry" => Some(TransportMode::Ferry),
            "flight" | "plane" => Some(TransportMode::Flight),
            "train" | "rail" => Some(TransportMode::Train),
            "walk" | "walking" => Some(TransportMode::Walk),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_category_trip() {
        let body = r#"---
category: trip
summary: Weekend away
start: 2026-08-01
end: 2026-08-03
location: "[[Copenhagen]]"
coordinates:
  - "55.6761"
  - "12.5683"
people:
  - "[[Ada]]"
transport: train
status: confirmed
---
# Copenhagen weekend
"#;
        let root = Path::new("/vault");
        let path = root.join("Atlas/Events/Copenhagen.md");
        let candidate = parse_candidate(root, &path, body).unwrap().unwrap();
        assert_eq!(candidate.title, "Copenhagen weekend");
        assert_eq!(candidate.destination.unwrap().name, "Copenhagen");
        assert_eq!(candidate.travelers, vec!["Ada"]);
        assert_eq!(candidate.transport_modes, vec![TransportMode::Train]);
        assert!(candidate.issues.is_empty());

        assert!(parse_candidate(root, &path, "---\ncategory: event\n---")
            .unwrap()
            .is_none());
    }

    #[test]
    fn reports_missing_import_fields() {
        let candidate = parse_candidate(
            Path::new("/vault"),
            Path::new("/vault/Atlas/Events/Future.md"),
            "---\ncategory: trip\nstatus: planned\n---",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            candidate.issues,
            vec!["Startdatum fehlt", "Enddatum fehlt", "Zielort fehlt"]
        );
    }
}
