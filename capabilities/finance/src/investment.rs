//! Broker-neutral investment activity preview.
//!
//! This module stops at a deterministic holdings reconstruction. It does not
//! persist source rows or write the journal; that remains a separate reviewed
//! decision, just as it is for cash imports.

use crate::import::{normalize_date, ImportError, ImportResult};
use csv::StringRecord;
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestmentCsvMapping {
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    #[serde(default = "default_decimal_separator")]
    pub decimal_separator: char,
    pub date_column: String,
    pub instrument_column: String,
    pub quantity_column: String,
    #[serde(default)]
    pub reference_column: Option<String>,
    #[serde(default)]
    pub price_column: Option<String>,
    #[serde(default)]
    pub currency_column: Option<String>,
    #[serde(default = "default_currency")]
    pub default_currency: String,
    #[serde(default)]
    pub instrument_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    #[serde(serialize_with = "serialize_i64_as_string")]
    pub mantissa: i64,
    pub scale: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    pub instrument: String,
    pub quantity: Quantity,
    pub latest_unit_price: Option<Quantity>,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestmentPreview {
    pub activity_count: usize,
    pub duplicate_rows: usize,
    pub ignored_non_position_rows: usize,
    pub closed_positions: usize,
    pub holdings: Vec<Holding>,
}

#[derive(Debug)]
struct Activity {
    date: String,
    instrument: String,
    quantity: Quantity,
    unit_price: Option<Quantity>,
    currency: String,
}

#[derive(Debug)]
struct Position {
    quantity: Quantity,
    latest_price: Option<(String, Quantity)>,
    currency: String,
}

pub fn preview_csv(
    bytes: &[u8],
    mapping: &InvestmentCsvMapping,
) -> ImportResult<InvestmentPreview> {
    validate_mapping(mapping)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(mapping.delimiter as u8)
        .flexible(false)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| ImportError("CSV header could not be read".into()))?
        .clone();
    let date = column(&headers, &mapping.date_column)?;
    let instrument = column(&headers, &mapping.instrument_column)?;
    let quantity = column(&headers, &mapping.quantity_column)?;
    let reference = optional_column(&headers, mapping.reference_column.as_deref())?;
    let price = optional_column(&headers, mapping.price_column.as_deref())?;
    let currency = optional_column(&headers, mapping.currency_column.as_deref())?;

    let mut activities = Vec::new();
    let mut seen = HashSet::new();
    let mut duplicate_rows = 0;
    let mut ignored_non_position_rows = 0;
    for record in reader.records() {
        let record = record.map_err(|_| ImportError("CSV row does not match its header".into()))?;
        let raw_instrument = record.get(instrument).unwrap_or_default().trim();
        let raw_quantity = record.get(quantity).unwrap_or_default().trim();
        if raw_quantity.is_empty() {
            ignored_non_position_rows += 1;
            continue;
        }
        let quantity = parse_quantity(raw_quantity, mapping.decimal_separator)?;
        if quantity.mantissa == 0 {
            ignored_non_position_rows += 1;
            continue;
        }
        if raw_instrument.is_empty() {
            return Err(ImportError(
                "a nonzero quantity requires an instrument".into(),
            ));
        }
        let instrument = mapping
            .instrument_aliases
            .get(raw_instrument)
            .map(String::as_str)
            .unwrap_or(raw_instrument)
            .trim()
            .to_string();
        validate_instrument(&instrument)?;
        let date = normalize_date(record.get(date).unwrap_or_default())?;
        let source_reference = reference
            .and_then(|index| record.get(index))
            .map(str::trim)
            .unwrap_or_default();
        let unit_price = price
            .and_then(|index| record.get(index))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| parse_decimal(value, mapping.decimal_separator, "price"))
            .transpose()?;
        let currency = currency
            .and_then(|index| record.get(index))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&mapping.default_currency)
            .to_ascii_uppercase();
        validate_currency(&currency)?;
        let id = fingerprint(&[
            &date,
            &instrument,
            &quantity.mantissa.to_string(),
            &quantity.scale.to_string(),
            source_reference,
            &unit_price
                .as_ref()
                .map(|value| value.mantissa.to_string())
                .unwrap_or_default(),
            &unit_price
                .as_ref()
                .map(|value| value.scale.to_string())
                .unwrap_or_default(),
            &currency,
        ]);
        if !seen.insert(id.clone()) {
            duplicate_rows += 1;
            continue;
        }
        activities.push(Activity {
            date,
            instrument,
            quantity,
            unit_price,
            currency,
        });
    }

    let mut positions: BTreeMap<String, Position> = BTreeMap::new();
    for activity in &activities {
        let position = positions
            .entry(activity.instrument.clone())
            .or_insert_with(|| Position {
                quantity: Quantity {
                    mantissa: 0,
                    scale: activity.quantity.scale,
                },
                latest_price: None,
                currency: activity.currency.clone(),
            });
        if position.currency != activity.currency {
            return Err(ImportError(
                "one instrument cannot use multiple currencies".into(),
            ));
        }
        position.quantity = add(&position.quantity, &activity.quantity)?;
        if let Some(price) = &activity.unit_price {
            let replace = position
                .latest_price
                .as_ref()
                .is_none_or(|(date, _)| activity.date >= *date);
            if replace {
                position.latest_price = Some((activity.date.clone(), price.clone()));
            }
        }
    }

    let closed_positions = positions
        .values()
        .filter(|position| position.quantity.mantissa == 0)
        .count();
    let holdings = positions
        .into_iter()
        .filter(|(_, position)| position.quantity.mantissa != 0)
        .map(|(instrument, position)| Holding {
            instrument,
            quantity: position.quantity,
            latest_unit_price: position.latest_price.map(|(_, price)| price),
            currency: position.currency,
        })
        .collect();
    Ok(InvestmentPreview {
        activity_count: activities.len(),
        duplicate_rows,
        ignored_non_position_rows,
        closed_positions,
        holdings,
    })
}

fn validate_mapping(mapping: &InvestmentCsvMapping) -> ImportResult<()> {
    if !mapping.delimiter.is_ascii() {
        return Err(ImportError("delimiter must be one ASCII character".into()));
    }
    if !matches!(mapping.decimal_separator, ',' | '.') {
        return Err(ImportError(
            "decimal separator must be comma or period".into(),
        ));
    }
    validate_currency(&mapping.default_currency.to_ascii_uppercase())?;
    for alias in mapping.instrument_aliases.values() {
        validate_instrument(alias)?;
    }
    Ok(())
}

fn validate_currency(currency: &str) -> ImportResult<()> {
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(ImportError("currency must be a three-letter code".into()))
    }
}

fn validate_instrument(instrument: &str) -> ImportResult<()> {
    let valid = !instrument.is_empty()
        && instrument.len() <= 64
        && instrument
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ImportError(
            "instrument must be a symbolic ASCII identifier".into(),
        ))
    }
}

fn parse_quantity(value: &str, decimal_separator: char) -> ImportResult<Quantity> {
    parse_decimal(value, decimal_separator, "quantity")
}

fn parse_decimal(value: &str, decimal_separator: char, kind: &str) -> ImportResult<Quantity> {
    let mut value = value.trim().replace(['\u{a0}', ' '], "");
    if value.ends_with('-') {
        value.pop();
        value.insert(0, '-');
    }
    let grouping_separator = if decimal_separator == ',' { '.' } else { ',' };
    value = value.replace(grouping_separator, "");
    if value.matches(decimal_separator).count() > 1 {
        return Err(ImportError(format!(
            "{kind} has more than one decimal mark"
        )));
    }
    let negative = value.starts_with('-');
    let unsigned = value.trim_start_matches(['-', '+']);
    let mut parts = unsigned.split(decimal_separator);
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 12
    {
        return Err(ImportError(format!("{kind} is not a supported decimal")));
    }
    let digits = format!("{whole}{fraction}");
    let mut mantissa = digits
        .parse::<i64>()
        .map_err(|_| ImportError(format!("{kind} is outside the supported range")))?;
    if negative {
        mantissa = -mantissa;
    }
    Ok(Quantity {
        mantissa,
        scale: fraction.len() as u32,
    })
}

fn add(left: &Quantity, right: &Quantity) -> ImportResult<Quantity> {
    let scale = left.scale.max(right.scale);
    let left_factor = 10_i64
        .checked_pow(scale - left.scale)
        .ok_or_else(|| ImportError("quantity is outside the supported range".into()))?;
    let right_factor = 10_i64
        .checked_pow(scale - right.scale)
        .ok_or_else(|| ImportError("quantity is outside the supported range".into()))?;
    let mantissa = left
        .mantissa
        .checked_mul(left_factor)
        .and_then(|value| {
            right
                .mantissa
                .checked_mul(right_factor)
                .and_then(|right| value.checked_add(right))
        })
        .ok_or_else(|| ImportError("quantity is outside the supported range".into()))?;
    Ok(Quantity { mantissa, scale })
}

fn serialize_i64_as_string<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn column(headers: &StringRecord, name: &str) -> ImportResult<usize> {
    headers
        .iter()
        .position(|header| header.trim() == name)
        .ok_or_else(|| ImportError(format!("configured column {name:?} is absent")))
}

fn optional_column(headers: &StringRecord, name: Option<&str>) -> ImportResult<Option<usize>> {
    name.map(|name| column(headers, name)).transpose()
}

fn fingerprint(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0xff]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn default_delimiter() -> char {
    ';'
}

fn default_decimal_separator() -> char {
    ','
}

fn default_currency() -> String {
    "EUR".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> InvestmentCsvMapping {
        InvestmentCsvMapping {
            delimiter: ';',
            decimal_separator: ',',
            date_column: "Date".into(),
            instrument_column: "Instrument".into(),
            quantity_column: "Quantity".into(),
            reference_column: Some("Reference".into()),
            price_column: Some("Price".into()),
            currency_column: Some("Currency".into()),
            default_currency: "EUR".into(),
            instrument_aliases: BTreeMap::new(),
        }
    }

    #[test]
    fn signed_fractional_activity_reconstructs_the_open_holding() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,250;one;10,00;EUR\n2026-02-03;ACME;-0,250;two;12,1234;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.activity_count, 2);
        assert_eq!(preview.holdings.len(), 1);
        assert_eq!(
            preview.holdings[0].quantity,
            Quantity {
                mantissa: 1000,
                scale: 3
            }
        );
        assert_eq!(
            preview.holdings[0].latest_unit_price,
            Some(Quantity {
                mantissa: 121234,
                scale: 4
            })
        );
    }

    #[test]
    fn duplicate_activity_is_counted_once() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,5;same;;EUR\n2026-01-02;ACME;1,5;same;;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.activity_count, 1);
        assert_eq!(preview.duplicate_rows, 1);
        assert_eq!(preview.holdings[0].quantity.mantissa, 15);
    }

    #[test]
    fn distinct_source_references_preserve_identical_trades() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,5;one;;EUR\n2026-01-02;ACME;1,5;two;;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.activity_count, 2);
        assert_eq!(preview.duplicate_rows, 0);
        assert_eq!(preview.holdings[0].quantity.mantissa, 30);
    }

    #[test]
    fn rows_without_quantities_are_reported_but_not_positions() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;;;;EUR\n2026-01-03;ACME;2,0;one;9,00;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.ignored_non_position_rows, 1);
        assert_eq!(preview.activity_count, 1);
    }

    #[test]
    fn a_nonzero_quantity_requires_an_instrument() {
        let csv =
            b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;;1,0;one;10,00;EUR\n";
        let error = preview_csv(csv, &mapping()).unwrap_err();
        assert_eq!(error.0, "a nonzero quantity requires an instrument");
    }

    #[test]
    fn aliases_keep_source_identifiers_out_of_the_holding_name() {
        let mut mapping = mapping();
        mapping
            .instrument_aliases
            .insert("source-1".into(), "ACME".into());
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;source-1;1,0;one;10,00;EUR\n";
        let preview = preview_csv(csv, &mapping).unwrap();
        assert_eq!(preview.holdings[0].instrument, "ACME");
    }
}
