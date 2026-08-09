//! Replaceable accounting-engine boundary.
//!
//! Axon owns these DTOs and invokes an engine as a separate process. The default
//! adapter speaks to hledger, but no caller has to know its command line or JSON
//! shape. That keeps the journal portable and leaves a later native engine free to
//! implement the same contract without changing importers or the dashboard.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub type AccountingResult<T> = Result<T, AccountingError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountingError {
    pub operation: &'static str,
    pub message: String,
}

impl std::fmt::Display for AccountingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for AccountingError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Amount {
    pub commodity: String,
    pub mantissa: i64,
    pub scale: u32,
}

impl Amount {
    pub fn minor_units(&self, scale: u32) -> Option<i64> {
        if self.scale > scale {
            let divisor = 10_i64.checked_pow(self.scale - scale)?;
            (self.mantissa % divisor == 0).then_some(self.mantissa / divisor)
        } else {
            self.mantissa
                .checked_mul(10_i64.checked_pow(scale - self.scale)?)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub account: String,
    pub amounts: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalTransaction {
    pub index: u64,
    pub date: String,
    pub description: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    pub postings: Vec<Posting>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRow {
    pub date: String,
    pub description: String,
    pub account: String,
    pub change: Vec<Amount>,
    pub running_total: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceRow {
    pub account: String,
    pub amounts: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashFlowReport {
    pub rows: Vec<BalanceRow>,
    pub total: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetRow {
    pub account: String,
    pub actual: Vec<Amount>,
    pub budget: Vec<Amount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoiPeriod {
    pub begin: String,
    pub end: String,
    pub value_begin: String,
    pub cash_flow: String,
    pub value_end: String,
    pub pnl: String,
    pub irr_percent: f64,
    pub twr_period_percent: f64,
    pub twr_annual_percent: f64,
}

pub trait AccountingEngine: Send + Sync {
    fn check(&self) -> AccountingResult<()>;
    fn transactions(&self) -> AccountingResult<Vec<JournalTransaction>>;
    fn register(&self) -> AccountingResult<Vec<RegisterRow>>;
    fn balances(&self) -> AccountingResult<Vec<BalanceRow>>;
    fn cash_flow(&self) -> AccountingResult<CashFlowReport>;
    fn budget(&self) -> AccountingResult<Vec<BudgetRow>>;
    fn roi(
        &self,
        investment_query: &str,
        pnl_query: &str,
        today: &str,
    ) -> AccountingResult<Vec<RoiPeriod>>;
}

#[derive(Debug, Clone)]
pub struct HledgerEngine {
    executable: PathBuf,
    journal: PathBuf,
}

impl HledgerEngine {
    pub fn new(journal: impl Into<PathBuf>) -> Self {
        Self {
            executable: PathBuf::from("hledger"),
            journal: journal.into(),
        }
    }

    pub fn with_executable(journal: impl Into<PathBuf>, executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            journal: journal.into(),
        }
    }

    pub fn journal(&self) -> &Path {
        &self.journal
    }

    fn output(&self, operation: &'static str, args: &[&str]) -> AccountingResult<String> {
        let output = Command::new(&self.executable)
            .arg("--no-conf")
            .arg("-f")
            .arg(&self.journal)
            .args(args)
            .output()
            .map_err(|error| AccountingError {
                operation,
                message: format!("could not start accounting engine: {error}"),
            })?;
        if !output.status.success() {
            return Err(AccountingError {
                operation,
                // Engine diagnostics can contain source rows. Keep the public error
                // bounded; the operator can run the same check directly on the journal.
                message: format!("accounting engine exited with {}", output.status),
            });
        }
        String::from_utf8(output.stdout).map_err(|_| AccountingError {
            operation,
            message: "accounting engine returned non-UTF-8 output".into(),
        })
    }

    fn json(&self, operation: &'static str, args: &[&str]) -> AccountingResult<Value> {
        serde_json::from_str(&self.output(operation, args)?).map_err(|_| AccountingError {
            operation,
            message: "accounting engine returned an unsupported report shape".into(),
        })
    }
}

impl AccountingEngine for HledgerEngine {
    fn check(&self) -> AccountingResult<()> {
        self.output("check", &["check"]).map(|_| ())
    }

    fn transactions(&self) -> AccountingResult<Vec<JournalTransaction>> {
        parse_transactions(self.json("transactions", &["print", "-O", "json"])?)
    }

    fn register(&self) -> AccountingResult<Vec<RegisterRow>> {
        parse_register(self.json("register", &["register", "-O", "json"])?)
    }

    fn balances(&self) -> AccountingResult<Vec<BalanceRow>> {
        let value = self.json("balances", &["balance", "-O", "json"])?;
        parse_balance_rows(value.get(0).cloned().unwrap_or(Value::Null), "balances")
    }

    fn cash_flow(&self) -> AccountingResult<CashFlowReport> {
        let value = self.json("cash_flow", &["cashflow", "-O", "json"])?;
        let report = value
            .pointer("/cbrSubreports/0/1")
            .ok_or_else(|| shape("cash_flow"))?;
        Ok(CashFlowReport {
            rows: parse_periodic_rows(report.get("prRows"), "cash_flow")?,
            total: parse_amounts(report.pointer("/prTotals/prrTotal")),
        })
    }

    fn budget(&self) -> AccountingResult<Vec<BudgetRow>> {
        let value = self.json("budget", &["balance", "--budget", "-O", "json"])?;
        let rows = value
            .get("prRows")
            .and_then(Value::as_array)
            .ok_or_else(|| shape("budget"))?;
        Ok(rows
            .iter()
            .map(|row| BudgetRow {
                account: row
                    .get("prrName")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                actual: parse_nested_amounts(row.pointer("/prrTotal/0")),
                budget: parse_nested_amounts(row.pointer("/prrTotal/1")),
            })
            .collect())
    }

    fn roi(
        &self,
        investment_query: &str,
        pnl_query: &str,
        today: &str,
    ) -> AccountingResult<Vec<RoiPeriod>> {
        let output = self.output(
            "roi",
            &[
                "roi",
                "--inv",
                investment_query,
                "--pnl",
                pnl_query,
                "--value=end,EUR",
                "--today",
                today,
                "--color=n",
                "--pretty=n",
            ],
        )?;
        parse_roi(&output)
    }
}

fn shape(operation: &'static str) -> AccountingError {
    AccountingError {
        operation,
        message: "accounting engine returned an unsupported report shape".into(),
    }
}

fn parse_amount(value: &Value) -> Option<Amount> {
    let quantity = value.get("aquantity")?;
    Some(Amount {
        commodity: value.get("acommodity")?.as_str()?.to_string(),
        mantissa: quantity.get("decimalMantissa")?.as_i64()?,
        scale: quantity.get("decimalPlaces")?.as_u64()?.try_into().ok()?,
    })
}

fn parse_amounts(value: Option<&Value>) -> Vec<Amount> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_amount)
        .collect()
}

fn parse_nested_amounts(value: Option<&Value>) -> Vec<Amount> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .flat_map(|item| {
                if item.get("acommodity").is_some() {
                    parse_amount(item).into_iter().collect()
                } else {
                    parse_nested_amounts(Some(item))
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_posting(value: &Value) -> Option<Posting> {
    Some(Posting {
        account: value.get("paccount")?.as_str()?.to_string(),
        amounts: parse_amounts(value.get("pamount")),
    })
}

fn parse_tags(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| {
            let pair = tag.as_array()?;
            Some((
                pair.first()?.as_str()?.to_string(),
                pair.get(1)?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn parse_transactions(value: Value) -> AccountingResult<Vec<JournalTransaction>> {
    let rows = value.as_array().ok_or_else(|| shape("transactions"))?;
    rows.iter()
        .map(|row| {
            let tags = parse_tags(row.get("ttags"));
            let source_id = tags.get("source-id").cloned().or_else(|| {
                row.get("tpostings")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find_map(|posting| parse_tags(posting.get("ptags")).get("source-id").cloned())
            });
            Ok(JournalTransaction {
                index: row
                    .get("tindex")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| shape("transactions"))?,
                date: row
                    .get("tdate")
                    .and_then(Value::as_str)
                    .ok_or_else(|| shape("transactions"))?
                    .to_string(),
                description: row
                    .get("tdescription")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                source_id,
                tags,
                postings: row
                    .get("tpostings")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(parse_posting)
                    .collect(),
            })
        })
        .collect()
}

fn parse_register(value: Value) -> AccountingResult<Vec<RegisterRow>> {
    let rows = value.as_array().ok_or_else(|| shape("register"))?;
    rows.iter()
        .map(|row| {
            let columns = row.as_array().ok_or_else(|| shape("register"))?;
            let posting = columns.get(3).ok_or_else(|| shape("register"))?;
            Ok(RegisterRow {
                date: columns
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                description: columns
                    .get(2)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                account: posting
                    .get("paccount")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                change: parse_amounts(posting.get("pamount")),
                running_total: parse_amounts(columns.get(4)),
            })
        })
        .collect()
}

fn parse_balance_rows(value: Value, operation: &'static str) -> AccountingResult<Vec<BalanceRow>> {
    let rows = value.as_array().ok_or_else(|| shape(operation))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let columns = row.as_array()?;
            Some(BalanceRow {
                account: columns.first()?.as_str()?.to_string(),
                amounts: parse_amounts(columns.get(3)),
            })
        })
        .collect())
}

fn parse_periodic_rows(
    value: Option<&Value>,
    operation: &'static str,
) -> AccountingResult<Vec<BalanceRow>> {
    let rows = value
        .and_then(Value::as_array)
        .ok_or_else(|| shape(operation))?;
    Ok(rows
        .iter()
        .map(|row| BalanceRow {
            account: row
                .get("prrName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            amounts: parse_amounts(row.get("prrTotal")),
        })
        .collect())
}

fn parse_roi(output: &str) -> AccountingResult<Vec<RoiPeriod>> {
    let mut periods = Vec::new();
    for line in output.lines() {
        let columns: Vec<&str> = if line.trim_start().starts_with('|') {
            line.split('|')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .skip(1)
                .collect()
        } else {
            line.split("  ")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect()
        };
        if columns.len() != 9 || !columns[0].starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let percent = |value: &str| value.trim_end_matches('%').parse::<f64>().ok();
        let Some(irr_percent) = percent(columns[6]) else {
            continue;
        };
        let Some(twr_period_percent) = percent(columns[7]) else {
            continue;
        };
        let Some(twr_annual_percent) = percent(columns[8]) else {
            continue;
        };
        periods.push(RoiPeriod {
            begin: columns[0].into(),
            end: columns[1].into(),
            value_begin: columns[2].into(),
            cash_flow: columns[3].into(),
            value_end: columns[4].into(),
            pnl: columns[5].into(),
            irr_percent,
            twr_period_percent,
            twr_annual_percent,
        });
    }
    if periods.is_empty() {
        Err(shape("roi"))
    } else {
        Ok(periods)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/finance-journal.example")
    }

    #[test]
    fn decimal_amounts_convert_without_floating_point() {
        assert_eq!(
            Amount {
                commodity: "EUR".into(),
                mantissa: 1234,
                scale: 2
            }
            .minor_units(2),
            Some(1234)
        );
        assert_eq!(
            Amount {
                commodity: "EUR".into(),
                mantissa: 12,
                scale: 0
            }
            .minor_units(2),
            Some(1200)
        );
        assert_eq!(
            Amount {
                commodity: "EUR".into(),
                mantissa: 123,
                scale: 3
            }
            .minor_units(2),
            None
        );
    }

    #[test]
    fn synthetic_journal_round_trips_through_the_real_engine() {
        if Command::new("hledger").arg("--version").output().is_err() {
            return;
        }
        let engine = HledgerEngine::new(fixture());
        engine.check().unwrap();
        let transactions = engine.transactions().unwrap();
        assert!(!transactions.is_empty());
        assert!(transactions.iter().any(|transaction| transaction
            .postings
            .iter()
            .any(|posting| posting.account == "income:salary")));
        assert!(!engine.register().unwrap().is_empty());
        assert!(!engine.balances().unwrap().is_empty());
        assert!(!engine.cash_flow().unwrap().rows.is_empty());
        assert!(!engine.budget().unwrap().is_empty());
        assert!(!engine
            .roi("assets:investments", "income:capital-gains", "2026-08-08")
            .unwrap()
            .is_empty());

        let projection = crate::analytics::project(&transactions, "EUR");
        let dashboard = crate::analytics::dashboard(
            &projection,
            &[],
            &crate::analytics::AnalyticsFilter::default(),
        );
        assert_eq!(dashboard.summary.income_cents, 120_000);
        assert_eq!(dashboard.summary.expense_cents, 6_120);
        assert_eq!(dashboard.summary.net_cash_flow_cents, 113_880);
    }

    #[test]
    fn roi_text_is_normalized_at_the_boundary() {
        let report = "begin        end          value begin  cashflow  value end  PnL       IRR     TWR/period  TWR/year\n2026-01-01  2026-02-01  0 EUR        100 EUR   110 EUR    10 EUR    12.50%  10.00%      20.00%\n";
        let rows = parse_roi(report).unwrap();
        assert_eq!(rows[0].irr_percent, 12.5);
        assert_eq!(rows[0].pnl, "10 EUR");
    }
}
