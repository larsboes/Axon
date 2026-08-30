//! HTTP-Oberflaeche der Capability.
//!
//! Die Rechnung passiert hier, die Anzeige nicht: jeder Endpunkt liefert entweder JSON oder
//! fertiges SVG/HTML, das aus dem Modell erzeugt wurde. Ein Frontend, das eigene Masse haelt
//! oder eigene Regeln auslegt, waere die Drift, die dieses Programm verhindern soll.
//!
//! Gebunden wird ueber `axon_server::serve_local` — Loopback, wie es die Bind-Policy von
//! `axon doctor` fuer jede Capability in beiden Roots verlangt.

use crate::clearance::check_layout;
use crate::model::Model;
use crate::plan;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::routing::put;
use axum::{Json, Router};
use std::sync::Arc;

struct AppState {
    flat: String,
}

/// Das Modell wird pro Anfrage frisch gelesen, nicht beim Start zwischengespeichert.
///
/// Absicht: die TOML-Dateien sind von Hand editierbar und werden von Hand editiert. Ein
/// Prozess, der beim Start eine Kopie zieht, zeigt nach der ersten Korrektur einen Plan, der
/// den Zahlen widerspricht, aus denen er zu stammen behauptet. Das Lesen kostet unter einer
/// Millisekunde; ein veralteter Plan kostet eine Fehlentscheidung.
fn load(state: &AppState) -> Result<Model, (StatusCode, String)> {
    Model::load(&state.flat).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn health() -> &'static str {
    "ok"
}

/// Was diese Capability beantwortet, als Daten neben `/health` — dieselbe Zusage wie in jeder
/// anderen Capability mit HTTP-Oberflaeche. Ein veralteter Katalog waere schlimmer als keiner,
/// weil er geglaubt wird; `route_manifest::undeclared_routes` liest deshalb unten den Router
/// aus diesem Quelltext und faellt um, wenn eine Route hier fehlt.
const ROUTES: &[route_manifest::Route] = &[
    r(
        "GET",
        "/",
        "Alle Layouts als Seite, bei jeder Anfrage aus dem Modell erzeugt.",
    ),
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "Dieser Katalog."),
    r(
        "GET",
        "/api/model",
        "Der gemessene Raum: Polygon, Waende, Oeffnungen, und was daran geschaetzt ist.",
    ),
    r(
        "GET",
        "/api/layouts",
        "Jedes Layout mit Verdikt, Verstosszahlen und Korridorbreiten.",
    ),
    r(
        "GET",
        "/api/layouts/:name",
        "Ein Layout: die volle Pruefung und der fertige Plan als SVG.",
    ),
    r(
        "GET",
        "/api/flats",
        "Welche Wohnungen es gibt und gegen welche dieser Prozess rechnet.",
    ),
    r(
        "GET",
        "/api/inventory",
        "Jedes Stueck und jeder Bedarf, mit Zustand (owned/wanted/gone).",
    ),
    r(
        "GET",
        "/api/wishlist",
        "Was noch fehlt, was es kostet, und wie viele Monatssalden das sind.",
    ),
    r(
        "GET",
        "/api/placements/:flat",
        "Wo die Stuecke in dieser Wohnung tatsaechlich stehen.",
    ),
    r(
        "PUT",
        "/api/items/:id",
        "Ein Stueck aendern. Nimmt die Item-Form, die /api/inventory liefert.",
    ),
    r(
        "PUT",
        "/api/placements/:flat/:item",
        "Ein Stueck in dieser Wohnung platzieren. Body: {x, y, rot}.",
    ),
];

/// Kurzform, damit die Tabelle oben wie eine Tabelle liest.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<serde_json::Value> {
    Json(route_manifest::manifest("interior", ROUTES))
}

async fn api_model(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    Ok(Json(serde_json::json!({
        "flat": m.room.flat,
        "area_m2": m.room.area_m2(),
        "hoehe": m.room.hauptraum.hoehe,
        "polygon": m.room.hauptraum.polygon,
        "bad": m.room.bad,
        "terrasse": m.room.terrasse,
        "waende": m.room.waende,
        "oeffnungen": m.room.oeffnungen,
        "fix_moebel": m.room.fix_moebel,
        "todo": m.room.todo.offen,
        "katalog_groesse": m.catalogue.len(),
        "ungemessen": m.uncertainties().into_iter()
            .map(|(id, label, f)| serde_json::json!({ "id": id, "label": label, "felder": f }))
            .collect::<Vec<_>>(),
    })))
}

async fn api_layouts(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let names = m
        .layout_names()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut out = Vec::new();
    for n in names {
        let Ok(l) = m.load_layout(&n) else { continue };
        let Ok(r) = check_layout(&m, &l) else {
            continue;
        };
        out.push(serde_json::json!({
            "id": n, "name": l.name, "pass": r.pass,
            "hard": r.hard.len(), "soft": r.soft.len(),
            "corridors": r.metrics.corridors,
            "occupied_m2": r.metrics.occupied_area_m2,
        }));
    }
    Ok(Json(out))
}

async fn api_layout(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let l = m
        .load_layout(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let r = check_layout(&m, &l).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let svg = plan::svg(&m, &l).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

fn store() -> Result<crate::store::Store, (StatusCode, String)> {
    crate::store::Store::open(&axon_config::database_path())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn boom<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Welche Wohnungen unter `flats/` liegen, und welche dieser Prozess bedient.
///
/// Die uebrigen Endpunkte antworten fuer GENAU EINE Wohnung — die aus `AXON_INTERIOR_FLAT`
/// oder die einzige vorhandene. Das ist heute richtig und wird es nicht bleiben: PRD B28 will
/// zwei Raeume nebeneinander, und dann traegt jeder Pfad die Wohnung. Bis dahin sagt dieser
/// Endpunkt wenigstens, dass es eine Auswahl gibt, statt sie zu verschweigen.
async fn api_flats(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let alle =
        crate::model::flats().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "flats": alle, "aktiv": s.flat })))
}

async fn api_inventory() -> Result<impl IntoResponse, (StatusCode, String)> {
    let rows = store()?.catalogue().map_err(boom)?;
    let out: Vec<_> = rows
        .into_values()
        .map(
            |(item, state)| serde_json::json!({ "item": item, "state": state.map(|s| s.as_str()) }),
        )
        .collect();
    Ok(Json(out))
}

/// Der offene Bedarf, und was er in Monatssalden kostet — die Naht zwischen `interior` und
/// `finance` (PRD B29). Beide Zahlen kommen aus derselben Datei und keine ueber HTTP.
///
/// Zwei Summen statt einer, weil die Daten zwei Arten von Preis kennen: ein Produkt hat einen,
/// ein Slot hat eine Schaetzspanne. Sie in eine Zahl zu falten hiesse, eine Spanne als Preis
/// auszugeben, und das ist die Praezision, die sie nicht hat.
async fn api_wishlist() -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store()?;
    let rows = st.catalogue().map_err(boom)?;
    let offen: Vec<_> = rows
        .values()
        .filter(|(_, s)| *s == Some(crate::store::State::Wanted))
        .map(|(i, _)| i)
        .collect();

    let untere: i64 = offen
        .iter()
        .map(|i| i.preis_cent.or(i.kosten_min_cent).unwrap_or(0))
        .sum();
    let obere: i64 = offen
        .iter()
        .map(|i| i.preis_cent.or(i.kosten_max_cent).unwrap_or(0))
        .sum();
    let ohne_preis = offen
        .iter()
        .filter(|i| i.preis_cent.is_none() && i.kosten_min_cent.is_none())
        .count();

    let conn = st.borrow_connection().map_err(boom)?;
    let saldo = crate::budget::monatssaldo(&conn).map_err(boom)?;
    let monate = saldo
        .as_ref()
        .and_then(|s| crate::budget::monate_bis_bezahlt(untere, s));

    Ok(Json(serde_json::json!({
        "items": offen,
        "summe_untere_kante_cent": untere,
        "summe_obere_kante_cent": obere,
        // Ein Posten ohne Preis zaehlt mit 0 in die Summe. Die Summe waere sonst still zu
        // klein, und diese Zahl ist die Warnung davor.
        "posten_ohne_preis": ohne_preis,
        "monatssaldo": saldo,
        "monate_bis_bezahlt": monate,
    })))
}

async fn api_placements(
    Path(flat): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(store()?.placements(&flat).map_err(boom)?))
}

async fn api_put_item(
    Path(id): Path<String>,
    Json(mut item): Json<crate::store::Item>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Der Pfad gewinnt gegen den Rumpf. Ein Formular, das eine andere Id schickt als die URL,
    // wuerde sonst still eine zweite Zeile anlegen statt die gemeinte zu aendern.
    item.id = id;
    store()?.upsert_item(&item).map_err(boom)?;
    Ok(Json(serde_json::json!({ "id": item.id, "ok": true })))
}

#[derive(serde::Deserialize)]
struct PlacementBody {
    x: i32,
    y: i32,
    #[serde(default)]
    rot: i32,
}

async fn api_put_placement(
    Path((flat, item)): Path<(String, String)>,
    Json(body): Json<PlacementBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    store()?
        .place(&crate::store::Placement {
            item_id: item.clone(),
            flat: flat.clone(),
            x: body.x,
            y: body.y,
            rot: body.rot,
        })
        .map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "flat": flat, "item": item, "ok": true }),
    ))
}

async fn index(State(s): State<Arc<AppState>>) -> Result<Html<String>, (StatusCode, String)> {
    let m = load(&s)?;
    let names = m
        .layout_names()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let layouts: Vec<_> = names.iter().filter_map(|n| m.load_layout(n).ok()).collect();
    plan::page(&m, &layouts)
        .map(Html)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn serve(flat: &str, port: u16) {
    let state = Arc::new(AppState {
        flat: flat.to_string(),
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/model", get(api_model))
        .route("/api/layouts", get(api_layouts))
        .route("/api/layouts/:name", get(api_layout))
        .route("/api/flats", get(api_flats))
        .route("/api/inventory", get(api_inventory))
        .route("/api/wishlist", get(api_wishlist))
        .route("/api/placements/:flat", get(api_placements))
        .route("/api/items/:id", put(api_put_item))
        .route("/api/placements/:flat/:item", put(api_put_placement))
        .with_state(state);
    axon_server::serve_local("interior", port, app).await;
}

#[cfg(test)]
mod route_manifest_tests {
    /// Ein Katalog, der luegt, wird geglaubt. Das hier liest den Router aus seinem eigenen
    /// Quelltext, also faellt ein `.route()` ohne Zusammenfassung hier um, statt eine
    /// Oberflaeche auszuliefern, die sich selbst falsch beschreibt.
    #[test]
    fn der_katalog_nennt_jede_ausgelieferte_route() {
        let fehlend = route_manifest::undeclared_routes(include_str!("api.rs"), super::ROUTES);
        assert!(
            fehlend.is_empty(),
            "ausgeliefert, aber nicht beschrieben: {fehlend:?}"
        );
    }
}
