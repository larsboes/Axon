//! One normalized projection for every finance view.
//!
//! Cards, trends, budgets, the transaction table and Sankey links are derived in
//! this module. Keeping the arithmetic together makes reconciliation testable and
//! prevents the UI from growing a second definition of income or spending.

use crate::accounting::JournalTransaction;
use crate::investment::ReviewedHoldingsSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    Income,
    Expense,
    Transfer,
}

impl TransactionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Income => "income",
            Self::Expense => "expense",
            Self::Transfer => "transfer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "income" => Some(Self::Income),
            "expense" => Some(Self::Expense),
            "transfer" => Some(Self::Transfer),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionRow {
    pub id: String,
    pub date: String,
    pub description: String,
    pub kind: TransactionKind,
    pub account: String,
    pub category: String,
    /// Always positive. Direction lives in `kind` so callers cannot accidentally
    /// invert expenses twice.
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetTarget {
    pub account: String,
    pub monthly_cents: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
}

fn default_currency() -> String {
    "EUR".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct AnalyticsFilter {
    pub start: Option<String>,
    pub end: Option<String>,
    pub account: Option<String>,
    pub category: Option<String>,
    pub currency: Option<String>,
    #[serde(default)]
    pub include_transfers: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub income_cents: i64,
    pub expense_cents: i64,
    pub net_cash_flow_cents: i64,
    pub savings_rate_percent: Option<f64>,
    pub budget_cents: i64,
    pub budget_variance_cents: i64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendPoint {
    pub month: String,
    pub income_cents: i64,
    pub expense_cents: i64,
    pub net_cash_flow_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetRow {
    pub account: String,
    pub budget_cents: i64,
    pub actual_cents: i64,
    pub variance_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SankeyLink {
    pub source: String,
    pub target: String,
    pub amount_cents: i64,
    pub account: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DashboardProjection {
    pub summary: Summary,
    pub trend: Vec<TrendPoint>,
    pub budgets: Vec<BudgetRow>,
    pub transactions: Vec<TransactionRow>,
    pub sankey: Vec<SankeyLink>,
    pub accounts: Vec<String>,
    pub categories: Vec<String>,
    pub investment: Option<ReviewedHoldingsSnapshot>,
}

pub fn project(transactions: &[JournalTransaction], currency: &str) -> Vec<TransactionRow> {
    let mut rows = Vec::new();
    for transaction in transactions {
        let assets: Vec<_> = transaction
            .postings
            .iter()
            .filter(|posting| posting.account.starts_with("assets:"))
            .collect();
        let account = assets
            .iter()
            .find(|posting| amount_cents(posting, currency).unwrap_or(0) != 0)
            .map(|posting| posting.account.as_str())
            .unwrap_or("assets:unknown");
        let mut classified = false;
        for (posting_index, posting) in transaction.postings.iter().enumerate() {
            let Some(signed_cents) = amount_cents(posting, currency) else {
                continue;
            };
            let (kind, amount_cents) =
                if posting.account.starts_with("expenses:") && signed_cents > 0 {
                    (TransactionKind::Expense, signed_cents)
                } else if posting.account.starts_with("income:") && signed_cents < 0 {
                    (TransactionKind::Income, -signed_cents)
                } else {
                    continue;
                };
            classified = true;
            rows.push(TransactionRow {
                id: format!(
                    "transaction_{}_{}_{}",
                    transaction.index,
                    posting_index,
                    currency.to_ascii_lowercase()
                ),
                date: transaction.date.clone(),
                description: transaction.description.clone(),
                kind,
                account: account.to_string(),
                category: posting.account.clone(),
                amount_cents,
                currency: currency.to_string(),
            });
        }
        if !classified && assets.len() >= 2 {
            let source = assets
                .iter()
                .find(|posting| amount_cents(posting, currency).unwrap_or(0) < 0);
            let target = assets
                .iter()
                .find(|posting| amount_cents(posting, currency).unwrap_or(0) > 0);
            if let (Some(source), Some(target)) = (source, target) {
                rows.push(TransactionRow {
                    id: format!(
                        "transaction_{}_transfer_{}",
                        transaction.index,
                        currency.to_ascii_lowercase()
                    ),
                    date: transaction.date.clone(),
                    description: transaction.description.clone(),
                    kind: TransactionKind::Transfer,
                    account: source.account.clone(),
                    category: target.account.clone(),
                    amount_cents: amount_cents(source, currency)
                        .unwrap_or(0)
                        .checked_abs()
                        .unwrap_or(i64::MAX),
                    currency: currency.to_string(),
                });
            }
        }
    }
    rows
}

pub fn dashboard(
    rows: &[TransactionRow],
    budgets: &[BudgetTarget],
    filter: &AnalyticsFilter,
) -> DashboardProjection {
    let currency = filter.currency.as_deref().unwrap_or("EUR");
    let mut transactions: Vec<_> = rows
        .iter()
        .filter(|row| row.currency == currency)
        .filter(|row| filter.start.as_ref().is_none_or(|start| row.date >= *start))
        .filter(|row| filter.end.as_ref().is_none_or(|end| row.date <= *end))
        .filter(|row| {
            filter
                .account
                .as_ref()
                .is_none_or(|account| row.account == *account)
        })
        .filter(|row| {
            filter
                .category
                .as_ref()
                .is_none_or(|category| row.category.starts_with(category))
        })
        .filter(|row| filter.include_transfers || row.kind != TransactionKind::Transfer)
        .cloned()
        .collect();
    transactions.sort_by(|left, right| right.date.cmp(&left.date).then(left.id.cmp(&right.id)));

    let income_cents = transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Income)
        .map(|row| row.amount_cents)
        .sum();
    let expense_cents = transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Expense)
        .map(|row| row.amount_cents)
        .sum();
    let net_cash_flow_cents = income_cents - expense_cents;
    let months = month_count(filter, &transactions);

    let scoped_budgets: Vec<_> = budgets
        .iter()
        .filter(|target| target.currency == currency)
        .collect();
    let mut budget_rows = Vec::new();
    for target in &scoped_budgets {
        let budget_cents = target.monthly_cents.saturating_mul(months as i64);
        let actual_cents = transactions
            .iter()
            .filter(|row| {
                row.kind == TransactionKind::Expense
                    && row.category.starts_with(&target.account)
                    && !scoped_budgets.iter().any(|other| {
                        other.account.len() > target.account.len()
                            && row.category.starts_with(&other.account)
                    })
            })
            .map(|row| row.amount_cents)
            .sum();
        budget_rows.push(BudgetRow {
            account: target.account.clone(),
            budget_cents,
            actual_cents,
            variance_cents: budget_cents - actual_cents,
        });
    }
    budget_rows.sort_by(|left, right| left.account.cmp(&right.account));
    let budget_cents = budget_rows.iter().map(|row| row.budget_cents).sum();
    let budget_actual_cents = budget_rows.iter().map(|row| row.actual_cents).sum::<i64>();

    let mut trend: BTreeMap<String, TrendPoint> = BTreeMap::new();
    for row in &transactions {
        if row.date.len() < 7 {
            continue;
        }
        let point = trend
            .entry(row.date[..7].to_string())
            .or_insert(TrendPoint {
                month: row.date[..7].to_string(),
                income_cents: 0,
                expense_cents: 0,
                net_cash_flow_cents: 0,
            });
        match row.kind {
            TransactionKind::Income => point.income_cents += row.amount_cents,
            TransactionKind::Expense => point.expense_cents += row.amount_cents,
            TransactionKind::Transfer => {}
        }
        point.net_cash_flow_cents = point.income_cents - point.expense_cents;
    }

    let mut sankey: BTreeMap<(String, String, String, String), i64> = BTreeMap::new();
    for row in &transactions {
        let (source, target) = match row.kind {
            TransactionKind::Income => (row.category.clone(), row.account.clone()),
            TransactionKind::Expense => (row.account.clone(), row.category.clone()),
            TransactionKind::Transfer => continue,
        };
        *sankey
            .entry((source, target, row.account.clone(), row.category.clone()))
            .or_default() += row.amount_cents;
    }

    let accounts: BTreeSet<_> = rows.iter().map(|row| row.account.clone()).collect();
    let categories: BTreeSet<_> = rows.iter().map(|row| row.category.clone()).collect();
    DashboardProjection {
        summary: Summary {
            income_cents,
            expense_cents,
            net_cash_flow_cents,
            savings_rate_percent: (income_cents > 0)
                .then_some(net_cash_flow_cents as f64 / income_cents as f64 * 100.0),
            budget_cents,
            budget_variance_cents: budget_cents - budget_actual_cents,
            currency: currency.into(),
        },
        trend: trend.into_values().collect(),
        budgets: budget_rows,
        transactions,
        sankey: sankey
            .into_iter()
            .map(
                |((source, target, account, category), amount_cents)| SankeyLink {
                    source,
                    target,
                    amount_cents,
                    account,
                    category,
                },
            )
            .collect(),
        accounts: accounts.into_iter().collect(),
        categories: categories.into_iter().collect(),
        investment: None,
    }
}

fn amount_cents(posting: &crate::accounting::Posting, currency: &str) -> Option<i64> {
    posting
        .amounts
        .iter()
        .find(|amount| amount.commodity == currency)
        .and_then(|amount| amount.minor_units(2))
}

fn month_count(filter: &AnalyticsFilter, rows: &[TransactionRow]) -> usize {
    let start = filter
        .start
        .as_deref()
        .or_else(|| rows.iter().map(|row| row.date.as_str()).min());
    let end = filter
        .end
        .as_deref()
        .or_else(|| rows.iter().map(|row| row.date.as_str()).max());
    let (Some(start), Some(end)) = (start, end) else {
        return 1;
    };
    let parse = |date: &str| -> Option<i32> {
        Some(date.get(..4)?.parse::<i32>().ok()? * 12 + date.get(5..7)?.parse::<i32>().ok()?)
    };
    match (parse(start), parse(end)) {
        (Some(start), Some(end)) if end >= start => (end - start + 1) as usize,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::{Amount, Posting};

    fn posting(account: &str, cents: i64) -> Posting {
        Posting {
            account: account.into(),
            amounts: vec![Amount {
                commodity: "EUR".into(),
                mantissa: cents,
                scale: 2,
            }],
        }
    }

    fn journal() -> Vec<JournalTransaction> {
        vec![
            JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "salary".into(),
                postings: vec![
                    posting("assets:bank:checking", 200_000),
                    posting("income:salary", -200_000),
                ],
            },
            JournalTransaction {
                index: 2,
                date: "2026-08-02".into(),
                description: "market".into(),
                postings: vec![
                    posting("expenses:food", 25_00),
                    posting("assets:bank:checking", -25_00),
                ],
            },
            JournalTransaction {
                index: 3,
                date: "2026-08-03".into(),
                description: "save".into(),
                postings: vec![
                    posting("assets:bank:savings", 50_000),
                    posting("assets:bank:checking", -50_000),
                ],
            },
        ]
    }

    #[test]
    fn one_projection_drives_reconciling_metrics_and_sankey_links() {
        let rows = project(&journal(), "EUR");
        let view = dashboard(
            &rows,
            &[BudgetTarget {
                account: "expenses:food".into(),
                monthly_cents: 40_00,
                currency: "EUR".into(),
            }],
            &AnalyticsFilter::default(),
        );
        assert_eq!(view.summary.income_cents, 200_000);
        assert_eq!(view.summary.expense_cents, 25_00);
        assert_eq!(view.summary.net_cash_flow_cents, 197_500);
        assert_eq!(view.summary.budget_variance_cents, 15_00);
        assert_eq!(view.transactions.len(), 2);
        assert_eq!(
            view.sankey
                .iter()
                .map(|link| link.amount_cents)
                .sum::<i64>(),
            202_500
        );
    }

    #[test]
    fn transfers_are_excluded_unless_the_filter_asks_for_them() {
        let rows = project(&journal(), "EUR");
        assert_eq!(
            dashboard(&rows, &[], &AnalyticsFilter::default())
                .transactions
                .len(),
            2
        );
        let filter = AnalyticsFilter {
            include_transfers: true,
            ..AnalyticsFilter::default()
        };
        assert_eq!(dashboard(&rows, &[], &filter).transactions.len(), 3);
    }

    #[test]
    fn nested_budget_targets_allocate_actuals_to_the_most_specific_target() {
        let rows = project(&journal(), "EUR");
        let view = dashboard(
            &rows,
            &[
                BudgetTarget {
                    account: "expenses".into(),
                    monthly_cents: 40_00,
                    currency: "EUR".into(),
                },
                BudgetTarget {
                    account: "expenses:food".into(),
                    monthly_cents: 40_00,
                    currency: "EUR".into(),
                },
            ],
            &AnalyticsFilter::default(),
        );

        assert_eq!(view.budgets[0].actual_cents, 0);
        assert_eq!(view.budgets[1].actual_cents, 25_00);
        assert_eq!(view.summary.budget_variance_cents, 55_00);
    }

    #[test]
    fn savings_rate_is_unavailable_without_positive_income() {
        let rows = vec![TransactionRow {
            id: "expense".into(),
            date: "2026-08-02".into(),
            description: "market".into(),
            kind: TransactionKind::Expense,
            account: "assets:bank:checking".into(),
            category: "expenses:food".into(),
            amount_cents: 100,
            currency: "EUR".into(),
        }];
        assert_eq!(
            dashboard(&rows, &[], &AnalyticsFilter::default())
                .summary
                .savings_rate_percent,
            None
        );
    }
}
