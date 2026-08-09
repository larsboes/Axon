//! Bank-export staging and reviewed journal writes.
//!
//! CSV is an edge format. It is parsed into candidates, immediately discarded,
//! and never becomes canonical. Only an explicitly confirmed candidate is rendered
//! into the plaintext journal; rejection writes nothing.

use csv::StringRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path;

pub type ImportResult<T> = Result<T, ImportError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvMapping {
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    #[serde(default = "default_decimal_separator")]
    pub decimal_separator: char,
    pub date_column: String,
    pub amount_column: String,
    pub description_column: String,
    #[serde(default)]
    pub reference_column: Option<String>,
    #[serde(default)]
    pub currency_column: Option<String>,
    #[serde(default = "default_currency")]
    pub default_currency: String,
    pub source_account: String,
    #[serde(default)]
    pub amount_sign: AmountSign,
    #[serde(default = "default_date_formats")]
    pub date_formats: Vec<CsvDateFormat>,
    #[serde(default)]
    pub row_policy: CsvRowPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountSign {
    #[default]
    AsProvided,
    Invert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvDateFormat {
    IsoYearMonthDay,
    DayMonthYearDots,
    DayMonthYearSlashes,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvRowPolicy {
    #[default]
    Strict,
    RequiredFields,
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

fn default_date_formats() -> Vec<CsvDateFormat> {
    vec![
        CsvDateFormat::IsoYearMonthDay,
        CsvDateFormat::DayMonthYearDots,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Pending,
    Confirmed,
    Rejected,
}

impl CandidateState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "confirmed" => Some(Self::Confirmed),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionCandidate {
    pub id: String,
    pub fingerprint: String,
    pub booked_at: String,
    pub description: String,
    pub amount_cents: i64,
    pub currency: String,
    pub source_account: String,
    pub source_reference: Option<String>,
    pub proposed_account: String,
    pub confidence_basis_points: u16,
    pub state: CandidateState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CsvImportPreview {
    pub preview_id: String,
    pub candidate_count: usize,
    pub duplicate_rows: usize,
    pub ignored_non_transaction_rows: usize,
    pub outflow_count: usize,
    pub inflow_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCsv {
    pub preview: CsvImportPreview,
    pub candidates: Vec<TransactionCandidate>,
}

pub fn parse_csv(bytes: &[u8], mapping: &CsvMapping) -> ImportResult<Vec<TransactionCandidate>> {
    Ok(prepare_csv(bytes, mapping)?.candidates)
}

pub fn preview_csv(bytes: &[u8], mapping: &CsvMapping) -> ImportResult<CsvImportPreview> {
    Ok(prepare_csv(bytes, mapping)?.preview)
}

pub fn prepare_csv(bytes: &[u8], mapping: &CsvMapping) -> ImportResult<PreparedCsv> {
    validate_mapping(mapping)?;
    if !mapping.delimiter.is_ascii() {
        return Err(ImportError("delimiter must be one ASCII character".into()));
    }
    if !matches!(mapping.decimal_separator, ',' | '.') {
        return Err(ImportError(
            "decimal separator must be comma or period".into(),
        ));
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(mapping.delimiter as u8)
        .flexible(mapping.row_policy == CsvRowPolicy::RequiredFields)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| ImportError("CSV header could not be read".into()))?
        .clone();
    let date = column(&headers, &mapping.date_column)?;
    let amount = column(&headers, &mapping.amount_column)?;
    let description = column(&headers, &mapping.description_column)?;
    let reference = optional_column(&headers, mapping.reference_column.as_deref())?;
    let currency = optional_column(&headers, mapping.currency_column.as_deref())?;

    let mut candidates = Vec::new();
    let mut fingerprints = HashSet::new();
    let mut duplicate_rows = 0;
    let mut ignored_non_transaction_rows = 0;
    for (index, record) in reader.records().enumerate() {
        let row = index + 2;
        let record =
            record.map_err(|_| ImportError(format!("CSV row {row} does not match its header")))?;
        let date_value = record.get(date).unwrap_or_default();
        let amount_value = record.get(amount).unwrap_or_default();
        let description_value = record.get(description).unwrap_or_default();
        if mapping.row_policy == CsvRowPolicy::RequiredFields
            && amount_value.trim().is_empty()
            && normalize_date_with_formats(date_value, &mapping.date_formats).is_err()
        {
            ignored_non_transaction_rows += 1;
            continue;
        }
        let booked_at = normalize_date_with_formats(date_value, &mapping.date_formats)
            .map_err(|error| row_error(row, error))?;
        let mut amount_cents = parse_decimal_cents(amount_value, mapping.decimal_separator)
            .map_err(|error| row_error(row, error))?;
        if mapping.amount_sign == AmountSign::Invert {
            amount_cents = amount_cents.checked_neg().ok_or_else(|| {
                ImportError(format!(
                    "CSV row {row}: amount is outside the supported range"
                ))
            })?;
        }
        let description = normalize_text(description_value);
        if description.is_empty() {
            return Err(ImportError(format!(
                "CSV row {row}: description must not be blank"
            )));
        }
        let source_reference = reference
            .and_then(|index| record.get(index))
            .map(normalize_text)
            .filter(|value| !value.is_empty());
        let currency = currency
            .and_then(|index| record.get(index))
            .map(normalize_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| mapping.default_currency.clone())
            .to_ascii_uppercase();
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ImportError(format!(
                "CSV row {row}: currency must be a three-letter code"
            )));
        }
        let amount_text = amount_cents.to_string();
        let fingerprint = fingerprint(&[
            &booked_at,
            &amount_text,
            &currency,
            &description,
            source_reference.as_deref().unwrap_or(""),
            &mapping.source_account,
        ]);
        if !fingerprints.insert(fingerprint.clone()) {
            duplicate_rows += 1;
            continue;
        }
        candidates.push(TransactionCandidate {
            id: format!("candidate_{fingerprint}"),
            fingerprint,
            booked_at,
            description,
            amount_cents,
            currency,
            source_account: mapping.source_account.clone(),
            source_reference,
            proposed_account: if amount_cents < 0 {
                "expenses:uncategorized".into()
            } else {
                "income:uncategorized".into()
            },
            confidence_basis_points: 0,
            state: CandidateState::Pending,
        });
    }
    let outflow_count = candidates
        .iter()
        .filter(|candidate| candidate.amount_cents < 0)
        .count();
    let inflow_count = candidates
        .iter()
        .filter(|candidate| candidate.amount_cents > 0)
        .count();
    let preview = CsvImportPreview {
        preview_id: preview_id(&candidates, duplicate_rows, ignored_non_transaction_rows),
        candidate_count: candidates.len(),
        duplicate_rows,
        ignored_non_transaction_rows,
        outflow_count,
        inflow_count,
    };
    Ok(PreparedCsv {
        preview,
        candidates,
    })
}

pub fn render_journal_entry(
    candidate: &TransactionCandidate,
    account: &str,
) -> ImportResult<String> {
    validate_account(&candidate.source_account)?;
    validate_account(account)?;
    if candidate.state == CandidateState::Rejected {
        return Err(ImportError(
            "a rejected candidate cannot be confirmed".into(),
        ));
    }
    if candidate.amount_cents == 0 {
        return Err(ImportError(
            "zero-value candidates are not journal entries".into(),
        ));
    }
    let balance_transfer = is_balance_account(&candidate.source_account)
        && is_balance_account(account)
        && candidate.source_account != account;
    if candidate.amount_cents < 0 && !account.starts_with("expenses:") && !balance_transfer {
        return Err(ImportError(
            "outflows must be reviewed into an expenses account or a different balance account"
                .into(),
        ));
    }
    if candidate.amount_cents > 0 && !account.starts_with("income:") && !balance_transfer {
        return Err(ImportError(
            "inflows must be reviewed into an income account or a different balance account".into(),
        ));
    }
    let description = sanitize_description(&candidate.description);
    Ok(format!(
        "\n{} * {}\n    {}  {} {}\n    {}\n    ; source-id: {}\n",
        candidate.booked_at,
        description,
        candidate.source_account,
        decimal(candidate.amount_cents),
        candidate.currency,
        account,
        candidate.fingerprint,
    ))
}

/// Append at most once. A crash after the file write but before the database state
/// update is recoverable: the retry sees the marker and does not duplicate money.
pub fn append_confirmed(
    journal: &Path,
    candidate: &TransactionCandidate,
    account: &str,
) -> ImportResult<bool> {
    let existing = std::fs::read_to_string(journal)
        .map_err(|error| ImportError(format!("journal could not be read: {error}")))?;
    let marker = format!("source-id: {}", candidate.fingerprint);
    if existing.contains(&marker) {
        return Ok(false);
    }
    let entry = render_journal_entry(candidate, account)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal)
        .map_err(|error| ImportError(format!("journal could not be opened: {error}")))?;
    file.write_all(entry.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| ImportError(format!("journal could not be updated: {error}")))?;
    Ok(true)
}

fn validate_mapping(mapping: &CsvMapping) -> ImportResult<()> {
    validate_account(&mapping.source_account)?;
    if mapping.default_currency.len() != 3 {
        return Err(ImportError(
            "default currency must be a three-letter code".into(),
        ));
    }
    if mapping.date_formats.is_empty() {
        return Err(ImportError(
            "at least one explicit date format is required".into(),
        ));
    }
    Ok(())
}

fn is_balance_account(account: &str) -> bool {
    account.starts_with("assets:") || account.starts_with("liabilities:")
}

pub fn validate_account(account: &str) -> ImportResult<()> {
    let valid = !account.is_empty()
        && account.split(':').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        });
    if valid {
        Ok(())
    } else {
        Err(ImportError(
            "account must be a symbolic colon-separated name".into(),
        ))
    }
}

fn column(headers: &StringRecord, name: &str) -> ImportResult<usize> {
    headers
        .iter()
        .position(|header| header.trim().trim_start_matches('\u{feff}') == name)
        .ok_or_else(|| ImportError(format!("configured column {name:?} is absent")))
}

fn optional_column(headers: &StringRecord, name: Option<&str>) -> ImportResult<Option<usize>> {
    name.map(|name| column(headers, name)).transpose()
}

pub(crate) fn normalize_date(value: &str) -> ImportResult<String> {
    normalize_date_with_formats(value, &default_date_formats())
}

fn normalize_date_with_formats(value: &str, formats: &[CsvDateFormat]) -> ImportResult<String> {
    let value = value.trim();
    for format in formats {
        let parts = match format {
            CsvDateFormat::IsoYearMonthDay => date_parts(value, b'-', true),
            CsvDateFormat::DayMonthYearDots => date_parts(value, b'.', false),
            CsvDateFormat::DayMonthYearSlashes => date_parts(value, b'/', false),
        };
        if let Some((year, month, day)) = parts.filter(|parts| valid_date(*parts)) {
            return Ok(format!("{year:04}-{month:02}-{day:02}"));
        }
    }
    Err(ImportError(
        "date does not match the configured formats or is not a calendar date".into(),
    ))
}

fn date_parts(value: &str, separator: u8, year_first: bool) -> Option<(u32, u32, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[2 + usize::from(year_first) * 2] != separator
        || bytes[5 + usize::from(year_first) * 2] != separator
    {
        return None;
    }
    let digits = |start: usize, end: usize| value.get(start..end)?.parse::<u32>().ok();
    if year_first {
        Some((digits(0, 4)?, digits(5, 7)?, digits(8, 10)?))
    } else {
        Some((digits(6, 10)?, digits(3, 5)?, digits(0, 2)?))
    }
}

fn valid_date((year, month, day): (u32, u32, u32)) -> bool {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day > 0 && day <= days
}

fn row_error(row: usize, error: ImportError) -> ImportError {
    ImportError(format!("CSV row {row}: {}", error.0))
}

fn parse_decimal_cents(value: &str, decimal_separator: char) -> ImportResult<i64> {
    let mut value = value.trim().replace(['\u{a0}', ' '], "");
    if value.ends_with('-') {
        value.pop();
        value.insert(0, '-');
    }
    let grouping_separator = if decimal_separator == ',' { '.' } else { ',' };
    value = value.replace(grouping_separator, "");
    if value.matches(decimal_separator).count() > 1 {
        return Err(ImportError("amount has more than one decimal mark".into()));
    }
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value.as_str());
    if unsigned.is_empty()
        || !unsigned
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == decimal_separator as u8)
    {
        return Err(ImportError("amount is not a decimal number".into()));
    }
    let decimal = value.rfind(decimal_separator);
    let negative = value.starts_with('-');
    let digits: String = value
        .bytes()
        .filter(|byte| byte.is_ascii_digit())
        .map(char::from)
        .collect();
    if digits.is_empty() {
        return Err(ImportError("amount is not a decimal number".into()));
    }
    let fraction_digits = decimal.map_or(0, |index| {
        value[index + 1..]
            .bytes()
            .filter(|byte| byte.is_ascii_digit())
            .count()
    });
    if fraction_digits > 2 {
        return Err(ImportError(
            "amount has more than two decimal places".into(),
        ));
    }
    let mut cents = digits
        .parse::<i64>()
        .map_err(|_| ImportError("amount is outside the supported range".into()))?;
    if fraction_digits == 0 {
        cents = cents
            .checked_mul(100)
            .ok_or_else(|| ImportError("amount is outside the supported range".into()))?;
    } else if fraction_digits == 1 {
        cents = cents
            .checked_mul(10)
            .ok_or_else(|| ImportError("amount is outside the supported range".into()))?;
    }
    Ok(if negative { -cents } else { cents })
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_description(value: &str) -> String {
    normalize_text(value).replace([';', '\n', '\r'], " ")
}

fn decimal(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let absolute = cents.unsigned_abs();
    format!("{sign}{}.{:02}", absolute / 100, absolute % 100)
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

fn preview_id(
    candidates: &[TransactionCandidate],
    duplicate_rows: usize,
    ignored_non_transaction_rows: usize,
) -> String {
    let mut fingerprints: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.fingerprint.as_str())
        .collect();
    fingerprints.sort_unstable();
    let duplicate_rows = duplicate_rows.to_string();
    let ignored_non_transaction_rows = ignored_non_transaction_rows.to_string();
    fingerprints.push(&duplicate_rows);
    fingerprints.push(&ignored_non_transaction_rows);
    fingerprint(&fingerprints)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> CsvMapping {
        CsvMapping {
            delimiter: ';',
            decimal_separator: ',',
            date_column: "Buchung".into(),
            amount_column: "Betrag".into(),
            description_column: "Text".into(),
            reference_column: Some("Referenz".into()),
            currency_column: Some("Waehrung".into()),
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            amount_sign: AmountSign::AsProvided,
            date_formats: default_date_formats(),
            row_policy: CsvRowPolicy::Strict,
        }
    }

    #[test]
    fn a_configurable_german_csv_becomes_review_candidates() {
        let csv = b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n03.08.2026;1.234,50;Employer;row-2;EUR\n";
        let rows = parse_csv(csv, &mapping()).unwrap();
        assert_eq!(rows[0].booked_at, "2026-08-02");
        assert_eq!(rows[0].amount_cents, -1234);
        assert_eq!(rows[0].proposed_account, "expenses:uncategorized");
        assert_eq!(rows[1].amount_cents, 123450);
        assert_eq!(rows[1].proposed_account, "income:uncategorized");
        assert!(rows.iter().all(|row| row.state == CandidateState::Pending));
    }

    #[test]
    fn fingerprints_make_reimport_deterministic() {
        let csv =
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n";
        let first = parse_csv(csv, &mapping()).unwrap();
        let second = parse_csv(csv, &mapping()).unwrap();
        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn credit_card_shape_is_explicitly_normalized_before_preview() {
        let mut mapping = mapping();
        mapping.delimiter = ',';
        mapping.date_column = "Date".into();
        mapping.amount_column = "Amount".into();
        mapping.description_column = "Description".into();
        mapping.reference_column = None;
        mapping.currency_column = None;
        mapping.amount_sign = AmountSign::Invert;
        mapping.date_formats = vec![CsvDateFormat::DayMonthYearSlashes];
        mapping.row_policy = CsvRowPolicy::RequiredFields;
        mapping.source_account = "liabilities:card:review".into();
        let csv = b"\xef\xbb\xbfDate,Amount,Description,Optional\n09/08/2026,\"12,34\",Synthetic service\nSummary\n09/08/2026,\"12,34\",Synthetic service\n";

        let prepared = prepare_csv(csv, &mapping).unwrap();

        assert_eq!(prepared.preview.candidate_count, 1);
        assert_eq!(prepared.preview.outflow_count, 1);
        assert_eq!(prepared.preview.inflow_count, 0);
        assert_eq!(prepared.preview.duplicate_rows, 1);
        assert_eq!(prepared.preview.ignored_non_transaction_rows, 1);
        assert_eq!(prepared.candidates[0].booked_at, "2026-08-09");
        assert_eq!(prepared.candidates[0].amount_cents, -1234);
        assert_eq!(
            prepared.candidates[0].proposed_account,
            "expenses:uncategorized"
        );
    }

    #[test]
    fn rows_with_amounts_fail_closed_when_dates_are_unsupported() {
        let mut mapping = mapping();
        mapping.delimiter = ',';
        mapping.date_column = "Date".into();
        mapping.amount_column = "Amount".into();
        mapping.description_column = "Description".into();
        mapping.reference_column = None;
        mapping.currency_column = None;
        mapping.date_formats = vec![CsvDateFormat::DayMonthYearSlashes];
        mapping.row_policy = CsvRowPolicy::RequiredFields;
        let error = preview_csv(
            b"Date,Amount,Description\n09-08-2026,12.34,Synthetic service\n",
            &mapping,
        )
        .unwrap_err();

        assert!(error.0.contains("CSV row 2"));
        assert!(error.0.contains("configured formats"));
    }

    #[test]
    fn amounts_with_unconfigured_text_fail_closed() {
        let error = parse_decimal_cents("EUR 12,34", ',').unwrap_err();
        assert_eq!(error.0, "amount is not a decimal number");
        assert_eq!(parse_decimal_cents("+12,34", ',').unwrap(), 1234);
        assert_eq!(parse_decimal_cents("12,34-", ',').unwrap(), -1234);
    }

    #[test]
    fn balance_account_transfers_render_without_weakening_direction_checks() {
        let mut candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Synthetic settlement;row-1;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);

        assert!(render_journal_entry(&candidate, "liabilities:card:review").is_ok());
        assert!(render_journal_entry(&candidate, "income:salary").is_err());

        candidate.source_account = "liabilities:card:review".into();
        candidate.amount_cents = 1234;
        assert!(render_journal_entry(&candidate, "assets:bank:checking").is_ok());
        assert!(render_journal_entry(&candidate, "expenses:food").is_err());
    }

    #[test]
    fn only_a_reviewed_direction_can_render() {
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        assert!(render_journal_entry(&candidate, "income:salary").is_err());
        let entry = render_journal_entry(&candidate, "expenses:food").unwrap();
        assert!(entry.contains("assets:bank:checking  -12.34 EUR"));
        assert!(entry.contains("source-id:"));
    }

    #[test]
    fn symbolic_accounts_cannot_carry_bank_identifiers_or_journal_syntax() {
        assert!(validate_account("assets:bank:checking").is_ok());
        assert!(validate_account("assets:bank:account with space").is_err());
        assert!(validate_account("expenses:food\n2026-01-01 bad").is_err());
    }

    #[test]
    fn confirmation_appends_once_and_remains_a_valid_journal() {
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        let path = std::env::temp_dir().join(format!(
            "axon-finance-import-{}-{}.journal",
            std::process::id(),
            candidate.fingerprint
        ));
        std::fs::write(&path, "decimal-mark .\ncommodity 1,000.00 EUR\n").unwrap();
        assert!(append_confirmed(&path, &candidate, "expenses:food").unwrap());
        assert!(!append_confirmed(&path, &candidate, "expenses:food").unwrap());
        let journal = std::fs::read_to_string(&path).unwrap();
        assert_eq!(journal.matches("source-id:").count(), 1);
        if std::process::Command::new("hledger")
            .arg("--version")
            .output()
            .is_ok()
        {
            use crate::accounting::AccountingEngine;
            crate::accounting::HledgerEngine::new(&path)
                .check()
                .unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }
}
