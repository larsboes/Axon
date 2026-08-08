//! Public, zero-personal-data config for the finance store.
//!
//! Nothing here names a vault, a bank, an institution or a figure. The overlay
//! supplies all of it, which is what keeps this capability publishable while the
//! data it operates on is the most private in the system.
//!
//! Resolution, in order:
//!   1. `$AXON_FINANCE_DATABASE_URL`
//!   2. values from `$AXON_PERSONAL_ROOT/config/postgres.env`
//!   3. a localhost development fallback

use crate::analytics::BudgetTarget;
use crate::import::CsvMapping;
use crate::investment::InvestmentCsvMapping;
use axon_config::{expand_tilde, postgres_conn_from_shared_env, resolve_port};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where subscription notes live. Journal and budget configuration are separate
/// because neither requires an Obsidian vault.
#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub root: PathBuf,
    pub subscriptions_dir: PathBuf,
}

/// A reusable import shape whose values come only from the private overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvMappingProfile {
    pub label: String,
    pub mapping: CsvMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestmentCsvMappingProfile {
    pub label: String,
    pub mapping: InvestmentCsvMapping,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub obsidian: Option<ObsidianConfig>,
    pub journal: Option<PathBuf>,
    pub investment_snapshot: Option<PathBuf>,
    pub budgets: Vec<BudgetTarget>,
    pub csv_mappings: Vec<CsvMappingProfile>,
    pub investment_csv_mappings: Vec<InvestmentCsvMappingProfile>,
}

#[derive(Debug, Deserialize)]
struct FinanceFileConfig {
    obsidian: Option<FinanceFileObsidian>,
    journal: Option<String>,
    investment_snapshot: Option<String>,
    #[serde(default)]
    budgets: Vec<BudgetTarget>,
    #[serde(default)]
    csv_mappings: Vec<CsvMappingProfile>,
    #[serde(default)]
    investment_csv_mappings: Vec<InvestmentCsvMappingProfile>,
}

#[derive(Debug, Deserialize)]
struct FinanceFileObsidian {
    root: String,
    #[serde(default = "default_subscriptions_dir")]
    subscriptions_dir: String,
}

fn default_subscriptions_dir() -> String {
    "Atlas/Finance/Subscriptions".into()
}

fn file_config() -> Option<FinanceFileConfig> {
    let overlay = std::env::var("AXON_PERSONAL_ROOT").ok()?;
    let path = expand_tilde(&overlay).join("config").join("finance.json");
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

impl Config {
    pub fn load() -> Self {
        let database_url = std::env::var("AXON_FINANCE_DATABASE_URL")
            .ok()
            .or_else(postgres_conn_from_shared_env)
            .unwrap_or_else(|| {
                "host=127.0.0.1 port=5432 user=axon password=axon dbname=axon".into()
            });
        let port = resolve_port(None, None, 8090);
        let personal = file_config();
        let obsidian = match std::env::var("AXON_FINANCE_OBSIDIAN_ROOT") {
            Ok(root) => Some(ObsidianConfig {
                root: expand_tilde(&root),
                subscriptions_dir: PathBuf::from(
                    std::env::var("AXON_FINANCE_OBSIDIAN_DIR")
                        .unwrap_or_else(|_| default_subscriptions_dir()),
                ),
            }),
            Err(_) => personal
                .as_ref()
                .and_then(|config| config.obsidian.as_ref())
                .map(|obsidian| ObsidianConfig {
                    root: expand_tilde(&obsidian.root),
                    subscriptions_dir: PathBuf::from(&obsidian.subscriptions_dir),
                }),
        };
        let budgets = personal
            .as_ref()
            .map(|config| config.budgets.clone())
            .unwrap_or_default();
        let csv_mappings = personal
            .as_ref()
            .map(|config| config.csv_mappings.clone())
            .unwrap_or_default();
        let investment_csv_mappings = personal
            .as_ref()
            .map(|config| config.investment_csv_mappings.clone())
            .unwrap_or_default();
        let journal = std::env::var("AXON_FINANCE_JOURNAL")
            .ok()
            .map(|path| expand_tilde(&path))
            .or_else(|| {
                personal
                    .as_ref()
                    .and_then(|config| config.journal.as_ref())
                    .map(|path| expand_tilde(path))
            });
        let investment_snapshot = std::env::var("AXON_FINANCE_INVESTMENT_SNAPSHOT")
            .ok()
            .map(|path| expand_tilde(&path))
            .or_else(|| {
                personal
                    .as_ref()
                    .and_then(|config| config.investment_snapshot.as_ref())
                    .map(|path| expand_tilde(path))
            });
        Self {
            database_url,
            port,
            obsidian,
            journal,
            investment_snapshot,
            budgets,
            csv_mappings,
            investment_csv_mappings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_directory_matches_where_the_notes_already_live() {
        assert_eq!(default_subscriptions_dir(), "Atlas/Finance/Subscriptions");
    }

    #[test]
    fn private_csv_mapping_profiles_parse_as_typed_config() {
        let config: FinanceFileConfig = serde_json::from_value(serde_json::json!({
            "csv_mappings": [{
                "label": "Synthetic semicolon export",
                "mapping": {
                    "delimiter": ";",
                    "decimal_separator": ",",
                    "date_column": "Date",
                    "amount_column": "Amount",
                    "description_column": "Description",
                    "reference_column": "Reference",
                    "currency_column": "Currency",
                    "default_currency": "EUR",
                    "source_account": "assets:bank:checking"
                }
            }]
        }))
        .unwrap();

        assert_eq!(config.csv_mappings.len(), 1);
        assert_eq!(config.csv_mappings[0].label, "Synthetic semicolon export");
        assert_eq!(config.csv_mappings[0].mapping.date_column, "Date");
    }

    #[test]
    fn csv_mapping_profiles_are_optional() {
        let config: FinanceFileConfig = serde_json::from_str("{}").unwrap();
        assert!(config.csv_mappings.is_empty());
        assert!(config.investment_csv_mappings.is_empty());
    }

    #[test]
    fn private_investment_mapping_profiles_parse_as_typed_config() {
        let config: FinanceFileConfig = serde_json::from_value(serde_json::json!({
            "investment_csv_mappings": [{
                "label": "Synthetic activity export",
                "mapping": {
                    "delimiter": ";",
                    "decimal_separator": ",",
                    "date_column": "Date",
                    "instrument_column": "Instrument",
                    "quantity_column": "Quantity",
                    "reference_column": "Reference",
                    "price_column": "Price",
                    "currency_column": "Currency",
                    "default_currency": "EUR",
                    "instrument_aliases": {"source-1": "ACME"}
                }
            }],
            "investment_snapshot": "/private/state/holdings.json"
        }))
        .unwrap();

        assert_eq!(config.investment_csv_mappings.len(), 1);
        assert_eq!(
            config.investment_snapshot.as_deref(),
            Some("/private/state/holdings.json")
        );
        assert_eq!(
            config.investment_csv_mappings[0].mapping.instrument_aliases["source-1"],
            "ACME"
        );
    }
}
