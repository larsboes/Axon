//! The places binary: the HTTP server by default (`service.toml` runs it bare),
//! one-shot backfills as subcommands (README "Backfills"). Backfills are CLI
//! verbs rather than routes because each runs once, prints counts and exits —
//! a route would be a standing surface for a non-standing job.

mod server;

fn usage() -> ! {
    eprintln!(
        "usage: places-server [server | backfill <amex|cities|stations|takeout|travelers|vault>]"
    );
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("server") => {
            tokio::runtime::Runtime::new()
                .expect("tokio runtime could not start")
                .block_on(server::serve());
        }
        Some("backfill") => {
            let config = places::config::Config::load();
            let store = match places::store::PlacesStore::open(&config.database_path) {
                Ok(store) => store,
                Err(error) => {
                    eprintln!("places backfill: cannot open store: {error}");
                    std::process::exit(1);
                }
            };
            let today = places::today();
            let result = match args.get(1).map(String::as_str) {
                Some("amex") => places::backfill::amex(&store, &today),
                Some("cities") => places::backfill::cities(&store, &today),
                Some("stations") => places::backfill::stations(&store, &today),
                Some("takeout") => places::backfill::takeout(&store, &today),
                Some("travelers") => places::backfill::travelers(&store, &today),
                Some("vault") => places::backfill::vault(&store, &today),
                _ => usage(),
            };
            if let Err(error) = result {
                eprintln!("places backfill failed: {error}");
                std::process::exit(1);
            }
        }
        Some(_) => usage(),
    }
}
