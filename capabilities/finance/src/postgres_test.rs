use finance::accounting::{Amount, JournalTransaction, Posting};
use finance::analytics::{project, TransactionRow};
use finance::import::{parse_csv, CandidateState, CsvMapping};
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
        reference_column: Some("Reference".into()),
        currency_column: None,
        default_currency: "EUR".into(),
        source_account: "assets:bank:checking".into(),
    };
    let candidates = parse_csv(
        b"Date;Amount;Description;Reference\n2026-08-01;-10.00;Synthetic market;one\n",
        &mapping,
    )
    .unwrap();
    let (store, _schema) = store("candidates");
    assert_eq!(store.stage_candidates(&candidates, "2026-08-08").unwrap(), (1, 0));
    assert_eq!(store.stage_candidates(&candidates, "2026-08-08").unwrap(), (0, 1));
    assert_eq!(store.list_candidates().unwrap()[0].state, CandidateState::Pending);
    store
        .review_candidate(
            &candidates[0].id,
            CandidateState::Rejected,
            "expenses:food",
            "2026-08-08",
        )
        .unwrap();
    assert_eq!(store.list_candidates().unwrap()[0].state, CandidateState::Rejected);
}
