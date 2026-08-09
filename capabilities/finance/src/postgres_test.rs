use finance::accounting::{Amount, JournalTransaction, Posting};
use finance::analytics::{project, TransactionRow};
use finance::import::{
    parse_csv, AmountSign, CandidateState, CsvDateFormat, CsvMapping, CsvRowPolicy,
};
use finance::investment::{
    Holding, HoldingsCoverage, Quantity, ReviewedHoldingsSnapshot, ReviewedHoldingsSource,
};
use finance::FinanceStore;
use postgres::{Client, NoTls};

fn database_url() -> String {
    std::env::var("FINANCE_TEST_DATABASE_URL")
        .unwrap_or_else(|_| finance::Config::load().database_url)
}

struct TestSchema(String);

impl Drop for TestSchema {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::connect(&database_url(), NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.0));
        }
    }
}

fn store(suffix: &str) -> (FinanceStore, TestSchema) {
    let schema = format!("finance_test_{suffix}_{}", std::process::id());
    let store = FinanceStore::open_in_schema(&database_url(), &schema).unwrap();
    (store, TestSchema(schema))
}

fn synthetic_projection() -> Vec<TransactionRow> {
    let amount = |mantissa| Amount {
        commodity: "EUR".into(),
        mantissa,
        scale: 2,
    };
    project(
        &[JournalTransaction {
            index: 1,
            date: "2026-08-01".into(),
            description: "Synthetic market".into(),
            source_id: None,
            tags: std::collections::BTreeMap::new(),
            postings: vec![
                Posting {
                    account: "expenses:food".into(),
                    amounts: vec![amount(1_000)],
                },
                Posting {
                    account: "assets:bank:checking".into(),
                    amounts: vec![amount(-1_000)],
                },
            ],
        }],
        "EUR",
    )
}

#[test]
fn an_empty_schema_rebuilds_to_the_same_projection_every_time() {
    let rows = synthetic_projection();
    let (store, _schema) = store("rebuild");
    assert!(store.transaction_projection().unwrap().is_empty());
    store.replace_transaction_projection(&rows).unwrap();
    let first = store.transaction_projection().unwrap();
    store.replace_transaction_projection(&rows).unwrap();
    assert_eq!(store.transaction_projection().unwrap(), first);
    assert_eq!(first.len(), rows.len());
}

#[test]
fn candidate_staging_is_idempotent_and_review_is_explicit() {
    let mapping = CsvMapping {
        delimiter: ';',
        decimal_separator: '.',
        date_column: "Date".into(),
        amount_column: "Amount".into(),
        description_column: "Description".into(),
        categorization_columns: Vec::new(),
        reference_column: Some("Reference".into()),
        currency_column: None,
        default_currency: "EUR".into(),
        source_account: "assets:bank:checking".into(),
        default_outflow_account: "expenses:uncategorized".into(),
        default_inflow_account: "income:uncategorized".into(),
        categorization_rules: Vec::new(),
        row_filter: None,
        amount_sign: AmountSign::AsProvided,
        amount_rounding: finance::import::AmountRounding::Reject,
        date_formats: vec![CsvDateFormat::IsoYearMonthDay],
        row_policy: CsvRowPolicy::Strict,
    };
    let first_export = parse_csv(
        b"Date;Amount;Description;Reference\n2026-08-01;-10.00;Synthetic market;one\n2026-08-02;-5.00;Synthetic service;two\n",
        &mapping,
    )
    .unwrap();
    let overlapping_export = parse_csv(
        b"Date;Amount;Description;Reference\n2026-08-02;-5.00;Synthetic service;two\n2026-08-03;20.00;Synthetic refund;three\n",
        &mapping,
    )
    .unwrap();
    let (store, _schema) = store("candidates");
    assert_eq!(
        store.stage_candidates(&first_export, "2026-08-08").unwrap(),
        (2, 0)
    );
    let mut remapped_export = first_export.clone();
    remapped_export[0].proposed_account = "expenses:groceries".into();
    remapped_export[0].confidence_basis_points = 9_000;
    assert_eq!(
        store
            .stage_candidates(&remapped_export, "2026-08-09")
            .unwrap(),
        (0, 2)
    );
    let remapped = store.candidate(&first_export[0].id).unwrap().unwrap();
    assert_eq!(remapped.proposed_account, "expenses:groceries");
    assert_eq!(remapped.confidence_basis_points, 9_000);
    assert_eq!(
        store
            .stage_candidates(&overlapping_export, "2026-08-08")
            .unwrap(),
        (1, 1)
    );
    assert_eq!(
        store
            .candidate(&first_export[0].id)
            .unwrap()
            .unwrap()
            .state,
        CandidateState::Pending
    );
    store
        .review_candidate(
            &first_export[0].id,
            CandidateState::Rejected,
            "expenses:food",
            "2026-08-08",
        )
        .unwrap();
    assert_eq!(
        store
            .candidate(&first_export[0].id)
            .unwrap()
            .unwrap()
            .state,
        CandidateState::Rejected
    );
}

#[test]
fn reference_less_overlap_preserves_multiplicity_without_reimporting_it() {
    let mapping = CsvMapping {
        delimiter: ';',
        decimal_separator: '.',
        date_column: "Date".into(),
        amount_column: "Amount".into(),
        description_column: "Description".into(),
        categorization_columns: Vec::new(),
        reference_column: None,
        currency_column: None,
        default_currency: "EUR".into(),
        source_account: "liabilities:card:review".into(),
        default_outflow_account: "expenses:uncategorized".into(),
        default_inflow_account: "income:uncategorized".into(),
        categorization_rules: Vec::new(),
        row_filter: None,
        amount_sign: AmountSign::AsProvided,
        amount_rounding: finance::import::AmountRounding::Reject,
        date_formats: vec![CsvDateFormat::IsoYearMonthDay],
        row_policy: CsvRowPolicy::Strict,
    };
    let first_export = parse_csv(
        b"Date;Amount;Description\n2026-08-01;-7.13;Synthetic market\n",
        &mapping,
    )
    .unwrap();
    let larger_export = parse_csv(
        b"Date;Amount;Description\n2026-08-01;-7.13;Synthetic market\n2026-08-01;-7.13;Synthetic market\n",
        &mapping,
    )
    .unwrap();
    assert_eq!(first_export[0].id, larger_export[0].id);
    assert_ne!(larger_export[0].id, larger_export[1].id);

    let (store, _schema) = store("candidate_multiplicity");
    assert_eq!(
        store.stage_candidates(&first_export, "2026-08-09").unwrap(),
        (1, 0)
    );
    assert_eq!(
        store
            .stage_candidates(&larger_export, "2026-08-09")
            .unwrap(),
        (1, 1)
    );
    assert_eq!(
        store
            .stage_candidates(&larger_export, "2026-08-09")
            .unwrap(),
        (0, 2)
    );
}

#[test]
fn reconciled_transfer_pair_has_one_canonical_candidate() {
    let mapping = CsvMapping {
        delimiter: ';',
        decimal_separator: '.',
        date_column: "Date".into(),
        amount_column: "Amount".into(),
        description_column: "Description".into(),
        categorization_columns: Vec::new(),
        reference_column: Some("Reference".into()),
        currency_column: None,
        default_currency: "EUR".into(),
        source_account: "assets:bank:checking".into(),
        default_outflow_account: "expenses:uncategorized".into(),
        default_inflow_account: "income:uncategorized".into(),
        categorization_rules: Vec::new(),
        row_filter: None,
        amount_sign: AmountSign::AsProvided,
        amount_rounding: finance::import::AmountRounding::Reject,
        date_formats: vec![CsvDateFormat::IsoYearMonthDay],
        row_policy: CsvRowPolicy::Strict,
    };
    let mut bank = parse_csv(
        b"Date;Amount;Description;Reference\n2026-08-02;-12.34;Synthetic transfer;bank\n",
        &mapping,
    )
    .unwrap()
    .remove(0);
    bank.proposed_account = "liabilities:card:review".into();
    let mut card = bank.clone();
    card.id = "card-candidate".into();
    card.fingerprint = "card-fingerprint".into();
    card.booked_at = "2026-08-01".into();
    card.amount_cents = 1234;
    card.source_account = "liabilities:card:review".into();
    card.proposed_account = "assets:bank:checking".into();
    let (store, _schema) = store("transfer_pair");
    store
        .stage_candidates(&[bank.clone(), card.clone()], "2026-08-09")
        .unwrap();

    assert!(store
        .review_transfer_pair(
            &bank.id,
            &card.id,
            &bank.proposed_account,
            "2026-08-09"
        )
        .unwrap());
    assert_eq!(
        store.candidate(&bank.id).unwrap().unwrap().state,
        CandidateState::Confirmed
    );
    assert_eq!(
        store.candidate(&card.id).unwrap().unwrap().state,
        CandidateState::Duplicate
    );
}

#[test]
fn reviewed_holdings_replace_atomically_and_preserve_an_empty_review() {
    let (store, _schema) = store("holdings");
    assert_eq!(store.holding_projection().unwrap(), None);
    let snapshot = ReviewedHoldingsSnapshot {
        schema_version: 2,
        snapshot_id: "synthetic-snapshot".into(),
        reviewed_at: "2026-08-09".into(),
        coverage: HoldingsCoverage::Partial,
        holdings: vec![Holding {
            instrument: "ACME".into(),
            quantity: Quantity {
                mantissa: 1250,
                scale: 3,
            },
            latest_unit_price: Some(Quantity {
                mantissa: 101234,
                scale: 4,
            }),
            currency: "EUR".into(),
        }],
        sources: vec![ReviewedHoldingsSource {
            source_key: "synthetic-broker".into(),
            snapshot_id: "synthetic-source-snapshot".into(),
            reviewed_at: "2026-08-09".into(),
            coverage: HoldingsCoverage::Partial,
        }],
    };
    store.replace_holding_projection(&snapshot).unwrap();
    assert_eq!(store.holding_projection().unwrap(), Some(snapshot));

    let empty = ReviewedHoldingsSnapshot {
        schema_version: 2,
        snapshot_id: "synthetic-empty".into(),
        reviewed_at: "2026-08-10".into(),
        coverage: HoldingsCoverage::Complete,
        holdings: vec![],
        sources: vec![ReviewedHoldingsSource {
            source_key: "synthetic-broker".into(),
            snapshot_id: "synthetic-empty-source".into(),
            reviewed_at: "2026-08-10".into(),
            coverage: HoldingsCoverage::Complete,
        }],
    };
    store.replace_holding_projection(&empty).unwrap();
    assert_eq!(store.holding_projection().unwrap(), Some(empty));
    store.clear_holding_projection().unwrap();
    assert_eq!(store.holding_projection().unwrap(), None);
}
