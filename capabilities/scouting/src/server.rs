use std::path::PathBuf;

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use scouting::adapters::cfp_conferences::CfpConferencesAdapter;
use scouting::adapters::euro_hackathons::EuroHackathonsAdapter;
use scouting::adapters::luma::LumaAdapter;
use scouting::adapters::meetup::MeetupAdapter;
use scouting::config::Config;
use scouting::event_route::{classify_opportunity, classify_ranked, EventRoute};
use scouting::pipeline::run;
use scouting::score::{load_opp_embeddings, load_telos_profiles};
use scouting::source::{SearchQuery, SourceAdapter};
use scouting::sources::create_adapter;
use scouting::store::Store;

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r("GET", "/discover", "Ranked opportunities for the Discover view."),
    r("GET", "/sources", "Declared opportunity sources and their state."),
    r("GET", "/opportunities", "Stored opportunities. Optional include_dismissed."),
    r("POST", "/opportunities/:id/status", "Set an opportunity's status (saved, dismissed)."),
];

/// Shorthand so the table above reads as a table.
const fn r(method: &'static str, path: &'static str, summary: &'static str) -> route_manifest::Route {
    route_manifest::Route { method, path, summary }
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("scouting", ROUTES))
}

#[derive(Debug, Deserialize)]
struct DiscoverParams {
    adapter: Option<String>,
    location: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
    opp_embeddings: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ScoredResult {
    id: String,
    rank: usize,
    score: f64,
    source: String,
    title: String,
    date: Option<String>,
    location: Option<String>,
    city: Option<String>,
    matched_focus: Option<String>,
    rationale: String,
    url: String,
    opportunity_type: String,
    status: String,
    vault_link: Option<String>,
    event_route: Option<EventRoute>,
}

#[derive(Debug, serde::Serialize)]
struct ClassifiedOpportunity {
    #[serde(flatten)]
    opportunity: scouting::store::RankedRow,
    event_route: Option<EventRoute>,
}

#[derive(Debug, serde::Serialize)]
struct DiscoverResponse {
    adapter: String,
    opportunity_type: String,
    total_scored: usize,
    new_count: usize,
    vault_links: usize,
    store_total: i64,
    results: Vec<ScoredResult>,
}

async fn discover_handler(Query(params): Query<DiscoverParams>) -> Json<Value> {
    let adapter_name = params
        .adapter
        .clone()
        .unwrap_or_else(|| "euro_hackathons".into());
    let location = params.location.clone();
    let query_text = params.query.clone().unwrap_or_default();
    let limit = params.limit.unwrap_or(20);
    let opp_emb_path = params.opp_embeddings.clone();

    let result = tokio::task::spawn_blocking(move || -> Option<DiscoverResponse> {
        // Config::load() and the adapter/store construction below all do
        // sync file/DB I/O (matches capabilities/transit's server.rs — same
        // spawn_blocking discipline, see its handle_health comment for why
        // this matters under #[tokio::main]).
        let cfg = Config::load();

        // A configured source id is authoritative. If its adapter cannot be
        // constructed, fail this scan; silently running a different built-in
        // source would attach the wrong provenance to the user's action.
        let adapter: Box<dyn SourceAdapter> = if let Some(source) = cfg
            .sources
            .iter()
            .find(|source| source.id == adapter_name && source.enabled)
        {
            create_adapter(source).ok()?
        } else {
            match adapter_name.as_str() {
                "luma" => Box::new(LumaAdapter::new()),
                "meetup" => Box::new(MeetupAdapter::new()),
                "cfp" | "cfp_conferences" => Box::new(CfpConferencesAdapter::new()),
                "euro_hackathons" => {
                    let cache_dir = PathBuf::from("infra/data/scouting-cache/euro_hackathons");
                    if cache_dir.exists() {
                        Box::new(EuroHackathonsAdapter::with_cache(cache_dir))
                    } else {
                        Box::new(EuroHackathonsAdapter::new())
                    }
                }
                _ => return None,
            }
        };

        let query = SearchQuery {
            query: query_text,
            location,
            limit,
            ..Default::default()
        };

        let telos = load_telos_profiles(&cfg.interest_profile_dir.to_string_lossy(), &cfg.sources);
        let events_dir = cfg.events_dir.as_deref();

        let opp_embeddings = opp_emb_path
            .or_else(|| {
                cfg.opp_embeddings_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .map(|p| load_opp_embeddings(&p));

        let mut store = Store::open(&cfg.database_url).ok();

        let report = run(
            &*adapter,
            &query,
            &telos,
            opp_embeddings.as_ref(),
            store.as_mut(),
            events_dir,
        )
        .ok()?;
        let opportunity_type = adapter.opportunity_type().as_str().to_string();

        // The pipeline owns scoring while the store owns human decisions and
        // Obsidian matches. Re-read those small persisted fields so a discovery
        // response is immediately actionable instead of returning a transient,
        // second shape of the same opportunity.
        let persisted = store
            .as_ref()
            .and_then(|st| st.list_top(report.store_total.max(1) as usize, true).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect::<std::collections::HashMap<_, _>>();

        let results: Vec<ScoredResult> = report
            .scored
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let persisted_row = persisted.get(&s.opportunity.id);
                let event_route = classify_opportunity(&s.opportunity, cfg.geo.as_ref());
                ScoredResult {
                    id: s.opportunity.id,
                    rank: i + 1,
                    score: s.score,
                    source: s.opportunity.source,
                    title: s.opportunity.title,
                    date: s.opportunity.starts_at,
                    location: s.opportunity.location,
                    city: s.opportunity.city,
                    matched_focus: s.matched_focus,
                    rationale: s.rationale,
                    url: s.opportunity.url,
                    opportunity_type: s.opportunity.opportunity_type.as_str().to_string(),
                    status: persisted_row
                        .map(|row| row.status.clone())
                        .unwrap_or_else(|| "new".to_string()),
                    vault_link: persisted_row.and_then(|row| row.vault_link.clone()),
                    event_route,
                }
            })
            .collect();

        Some(DiscoverResponse {
            adapter: adapter_name,
            opportunity_type,
            total_scored: results.len(),
            new_count: report.new_count,
            vault_links: report.vault_links,
            store_total: report.store_total,
            results,
        })
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(response) => Json(json!(response)),
        None => Json(json!({"error": "pipeline execution failed"})),
    }
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "scouting",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Returns the list of configured sources (from scouting.json's `sources[]`).
/// Useful for dashboards and automation to discover what's available.
async fn sources_handler() -> Json<Value> {
    let cfg = Config::load();
    let mut sources: Vec<Value> = cfg
        .sources
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "adapter": s.adapter,
                "enabled": s.enabled,
                "configured": true,
                "root_path": s.root_path.as_ref().map(|p| p.to_string_lossy()),
                "url": s.url,
                "events_glob": s.events_glob,
                "opportunities_glob": s.opportunities_glob,
                "opportunity_type": s.opportunity_type.as_str(),
                "profiles_glob": s.profiles_glob,
                // Null when the profile resolves under root_path, which is the
                // compatible form and most entries.
                "profile_root": s.profile_root.as_ref().map(|p| p.to_string_lossy()),
                "doc_path": s.doc_path.as_ref().map(|p| p.to_string_lossy()),
            })
        })
        .collect();
    // The EuroHackathons adapter is the one built-in network source with a
    // live-verified contract. Keep experimental adapters out of the dashboard
    // source picker until they have the same evidence.
    if !sources
        .iter()
        .any(|source| source["id"] == "euro_hackathons")
    {
        sources.push(json!({
            "id": "euro_hackathons",
            "adapter": "euro_hackathons",
            "enabled": true,
            "configured": false,
            "root_path": null,
            "url": "https://eurohackathons.com",
            "events_glob": null,
            "opportunities_glob": null,
            "opportunity_type": "hackathon",
            "profiles_glob": null,
            "profile_root": null,
            "doc_path": null,
        }));
    }
    Json(json!({
        "sources": sources,
        "count": sources.len()
    }))
}

#[derive(Debug, Deserialize)]
struct OpportunityParams {
    limit: Option<usize>,
    include_dismissed: Option<bool>,
}

async fn opportunities_handler(
    Query(params): Query<OpportunityParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let include_dismissed = params.include_dismissed.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(internal_error)?;
        let store_total = store.count().map_err(internal_error)?;
        let opportunities = store
            .list_top(limit, include_dismissed)
            .map_err(internal_error)?
            .into_iter()
            .map(|opportunity| {
                let event_route = classify_ranked(&opportunity, cfg.geo.as_ref());
                ClassifiedOpportunity {
                    opportunity,
                    event_route,
                }
            })
            .collect::<Vec<_>>();
        Ok(Json(json!({
            "count": opportunities.len(),
            "store_total": store_total,
            "opportunities": opportunities,
        })))
    })
    .await
    .map_err(internal_error)?
}

#[derive(Debug, Deserialize)]
struct StatusBody {
    status: String,
}

async fn set_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_url).map_err(internal_error)?;
        match store.set_status(&id, &body.status) {
            Ok(true) => Ok(Json(json!({ "id": id, "status": body.status }))),
            Ok(false) => Err((
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "opportunity not found" })),
            )),
            Err(error) => Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error.to_string() })),
            )),
        }
    })
    .await
    .map_err(internal_error)?
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
}

// No /pulse or transit-fare proxy route here — same reasoning as
// transit-server (dashboard/README.md). The
// existing transit_fare adapter already pulls transit data in-process via
// the transit crate (see Cargo.toml), so scouting has no HTTP-proxy need
// toward transit in the first place.

#[path = "../../../libs/axon-server/src/lib.rs"]
#[allow(dead_code)]
mod axon_server;

#[tokio::main]
async fn main() {
    // Config::load() at this point is plain file/env parsing, no DB
    // connection — safe to call directly in the async context (unlike
    // discover_handler's Store::open, which stays inside spawn_blocking).
    let cfg = Config::load();

    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health_handler))
        .route("/discover", get(discover_handler))
        .route("/sources", get(sources_handler))
        .route("/opportunities", get(opportunities_handler))
        .route("/opportunities/:id/status", post(set_status_handler))
        // CorsLayer stays here rather than in axon_server: it is a per-capability
        // security decision and belongs where it can be seen. It is defensible only
        // because serve_local binds loopback — the browser calling this is on the
        // same machine, through the dashboard's proxy.
        .layer(CorsLayer::permissive());

    // Was 0.0.0.0, which put an unauthenticated POST /opportunities/:id/status on
    // the LAN. Nothing documented that bind as a decision; it was the last of the
    // three divergences libs/axon-server exists to end.
    axon_server::serve_local("scout-server", cfg.port, app).await;
}

// The self-describing surface, on the same include terms as the other libs.
#[path = "../../../libs/route-manifest/src/lib.rs"]
#[allow(dead_code)]
mod route_manifest;

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing =
            super::route_manifest::undeclared_routes(include_str!("server.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
