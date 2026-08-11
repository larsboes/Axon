//! Trips' first binary other than the server.
//!
//! One command, deliberately: turning a sentence into a plan draft nobody has
//! submitted. It posts to the local model rung directly rather than depending on
//! `libs/inference`, because this is one request to a loopback URL and a role
//! lookup would be more machinery than the call it wraps.
//!
//! No HTTP route for this, on purpose. The question it answers is "does a small
//! local model turn a travel sentence into a valid form", and until that has
//! started a real trip more than twice it does not need a surface.

use std::io::Read;

const USAGE: &str = "\
Usage:
  trips draft-intent \"somewhere warm in October, under 300 euro, by train\"
  trips draft-intent -            read the sentence from stdin

Prints a CreatePlan-shaped draft plus what it could not resolve. Persists
nothing and resolves no station: every destination comes back as a place slug
with null coordinates, exactly as typed text does.

Environment:
  AXON_INTENT_URL     chat-completions endpoint (default http://127.0.0.1:8091/v1/chat/completions)
  AXON_INTENT_MODEL   model name (default apple-on-device)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("draft-intent"), Some(text)) => {
            let sentence = if text == "-" {
                read_stdin()
            } else {
                text.to_string()
            };
            if let Err(error) = draft(&sentence) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn read_stdin() -> String {
    let mut buffer = String::new();
    let _ = std::io::stdin().read_to_string(&mut buffer);
    buffer
}

fn draft(sentence: &str) -> Result<(), String> {
    let sentence = sentence.trim();
    if sentence.is_empty() {
        return Err("give me a sentence to draft from".into());
    }
    let url = std::env::var("AXON_INTENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8091/v1/chat/completions".into());
    let model = std::env::var("AXON_INTENT_MODEL").unwrap_or_else(|_| "apple-on-device".into());

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("client build: {e}"))?;
    let response = client
        .post(&url)
        .json(&trips::intent::request_body(&model, sentence))
        .send()
        .map_err(|e| {
            format!(
                "could not reach the local model at {url} ({e}). Start it with \
                 `tools/service-runner.sh start foundation-models`, or point \
                 AXON_INTENT_URL somewhere else."
            )
        })?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    let body: serde_json::Value = response.json().map_err(|e| format!("unreadable reply: {e}"))?;
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("the reply carried no message content")?;

    let drafted = trips::intent::draft_from_model_json(sentence, content)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&drafted).map_err(|e| e.to_string())?
    );
    Ok(())
}
