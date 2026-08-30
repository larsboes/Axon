use std::path::Path;

use comms::redaction_eval::evaluate_file;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "eval/redaction-corpus.json".to_string());
    let report = match evaluate_file(Path::new(&path)) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("redaction evaluation error: {error}");
            std::process::exit(2);
        }
    };

    println!("type\tlang\tcaught/total\trecall");
    for ((entity_type, language), (caught, total)) in &report.by_type_language {
        println!(
            "{entity_type}\t{language}\t{caught}/{total}\t{:.1}%",
            *caught as f64 * 100.0 / *total as f64
        );
    }
    if !report.leaks.is_empty() {
        println!("\nleaked (present in the document that would be sent):");
        for leak in &report.leaks {
            println!(
                "  {} [{}] {}: {}",
                leak.fixture, leak.language, leak.entity_type, leak.value
            );
        }
    }
    println!(
        "\n{} fixture(s), {} marker(s) written, recall {}/{} = {:.1}%",
        report.fixtures,
        report.markers_written,
        report.caught(),
        report.total(),
        report.recall_percent()
    );
    match report.minimum_recall_percent {
        Some(minimum) => println!(
            "gate: recall >= {minimum:.1}% — {}",
            if report.passed() { "PASS" } else { "FAIL" }
        ),
        None => println!("gate: none declared — this run measures, it does not judge"),
    }

    if !report.passed() {
        std::process::exit(1);
    }
}
