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
use axum::routing::{get, post, put};
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
    r(
        "POST",
        "/api/items",
        "Einen Eintrag anlegen. Zustand ist Pflicht, sonst taucht er in keiner Liste auf.",
    ),
    r(
        "PATCH",
        "/api/items/:id",
        "Genannte Felder aendern, ungenannte stehen lassen. Ausdrueckliches null loescht eines.",
    ),
    r(
        "GET",
        "/api/items/:id/state",
        "Die Zustandsgeschichte eines Eintrags, aelteste zuerst.",
    ),
    r(
        "POST",
        "/api/items/:id/state",
        "Einen Zustandswechsel anhaengen: owned, wanted oder gone. Haengt an, setzt nicht.",
    ),
    r(
        "POST",
        "/api/items/:id/impact",
        "Was ein Feld mit den Verdikten machen wuerde, ohne es zu schreiben.",
    ),
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

/// Einen Rumpf ueber einen bestehenden Eintrag legen.
///
/// Herausgezogen, weil `PATCH` und die Vorschau dieselbe Regel brauchen und zwei Fassungen
/// davon genau die Drift waeren, gegen die diese Capability existiert.
fn merge_patch(
    alt: &crate::store::Item,
    patch: serde_json::Value,
) -> Result<crate::store::Item, (StatusCode, String)> {
    let serde_json::Value::Object(patch) = patch else {
        return Err((StatusCode::BAD_REQUEST, "Rumpf ist kein Objekt".into()));
    };
    let mut merged = match serde_json::to_value(alt).map_err(|e| boom(e.to_string()))? {
        serde_json::Value::Object(m) => m,
        _ => unreachable!("Item serialisiert als Objekt"),
    };
    for (k, v) in patch {
        if k == "id" {
            continue; // Der Pfad gewinnt, wie bei PUT.
        }
        if !merged.contains_key(&k) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("`{k}` ist kein Feld eines Eintrags"),
            ));
        }
        merged.insert(k, v);
    }
    serde_json::from_value(serde_json::Value::Object(merged)).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Rumpf passt nicht auf einen Eintrag: {e}"),
        )
    })
}

/// Genannte Felder aendern, ungenannte in Ruhe lassen.
///
/// `PUT` daneben ersetzt den ganzen Eintrag und ist damit fuer ein Formular die falsche Form:
/// `Item` fuehrt 40 Felder, eine Maske zeigt sechs, und was sie nicht schickt, waere weg. Das
/// ist derselbe stille Verlust, den `deny_unknown_fields` beim Import verhindert — nur in die
/// andere Richtung.
///
/// Zusammengefuehrt wird auf JSON-Ebene und nicht ueber eine zweite Struktur mit lauter
/// `Option<Option<_>>`: die Zeile ist die Wahrheit, also wird sie gelesen, mit dem Rumpf
/// ueberschrieben und zurueckgeschrieben. Ein ausdrueckliches `null` loescht ein Feld, ein
/// fehlender Schluessel laesst es stehen — der Unterschied, den ein Formular braucht.
async fn api_patch_item(
    Path(id): Path<String>,
    Json(patch): Json<serde_json::Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store()?;
    let (item, _) = st
        .item(&id)
        .map_err(boom)?
        .ok_or((StatusCode::NOT_FOUND, format!("kein Eintrag `{id}`")))?;

    let mut item = merge_patch(&item, patch)?;
    item.id = id;
    st.upsert_item(&item).map_err(boom)?;
    Ok(Json(serde_json::json!({ "id": item.id, "ok": true })))
}

#[derive(serde::Deserialize)]
struct NewItem {
    #[serde(flatten)]
    item: crate::store::Item,
    /// Pflicht, und deshalb kein `Option`: ein Eintrag ohne Zustand taucht in keiner Liste auf,
    /// weil jede Abfrage auf den letzten Zustand joint. Ihn beim Anlegen zu vergessen hiesse,
    /// eine Zeile zu schreiben, die niemand je sieht.
    state: crate::store::State,
    #[serde(default)]
    note: Option<String>,
}

async fn api_post_item(
    Json(body): Json<NewItem>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store()?;
    if body.item.id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "`id` fehlt".into()));
    }
    if st.item(&body.item.id).map_err(boom)?.is_some() {
        return Err((
            StatusCode::CONFLICT,
            format!("`{}` gibt es schon — PATCH aendert ihn", body.item.id),
        ));
    }
    st.upsert_item(&body.item).map_err(boom)?;
    st.record_state(
        &body.item.id,
        body.state,
        body.note.as_deref().or(Some("in der Oberflaeche angelegt")),
    )
    .map_err(boom)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": body.item.id, "state": body.state.as_str(), "ok": true })),
    ))
}

#[derive(serde::Deserialize)]
struct StateBody {
    state: crate::store::State,
    #[serde(default)]
    note: Option<String>,
}

/// Einen Zustandswechsel anhaengen.
///
/// Anhaengen und nicht setzen: ein Wunsch, der gekauft wird, ist eine Zeile mehr und kein
/// ueberschriebenes Feld (PRD B25). Genau diese Spanne verbindet die Wunschliste spaeter mit
/// `finance`, und ein `UPDATE` haette sie gekostet.
///
/// `changed: false` heisst, der Zustand galt schon — kein Fehler, aber auch keine erfundene
/// zweite Zeile.
async fn api_post_state(
    Path(id): Path<String>,
    Json(body): Json<StateBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store()?;
    if st.item(&id).map_err(boom)?.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("kein Eintrag `{id}`")));
    }
    let changed = st
        .record_state(&id, body.state, body.note.as_deref())
        .map_err(boom)?;
    Ok(Json(serde_json::json!({
        "id": id, "state": body.state.as_str(), "changed": changed
    })))
}

async fn api_state_history(
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let rows = store()?.state_history(&id).map_err(boom)?;
    let out: Vec<_> = rows
        .into_iter()
        .map(|(s, since, note)| serde_json::json!({ "state": s.as_str(), "since": since, "note": note }))
        .collect();
    Ok(Json(out))
}

/// Was eine Aenderung mit den Verdikten machen WUERDE, ohne sie zu schreiben.
///
/// Der Grund, aus dem das ein Endpunkt ist und kein Kommentar in einer Anleitung: die
/// Raeumungsfelder aus PRD Q61 sind genau die, deren Wirkung man nicht sieht, bevor man sie
/// setzt. `opens` am Kleiderschrank kostet je nach Richtung 2 oder 4 Layouts, und das stand
/// nirgends — es musste am 2026-08-31 von Hand ausgerechnet werden, einmal je Richtung. Wer ein
/// Feld in der Oberflaeche fuellt, soll dieselbe Rechnung sehen, bevor er speichert.
///
/// Schreibt nichts. Der Katalog wird im Speicher gepatcht und jedes Layout neu gerechnet.
async fn api_item_impact(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(patch): Json<serde_json::Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let vorher = verdikte(&model)?;

    let alt = model
        .catalogue
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("kein Eintrag `{id}`")))?;
    let neu = merge_patch(alt, patch)?;

    let mut model = load(&s)?;
    model.catalogue.insert(id.clone(), neu);
    let nachher = verdikte(&model)?;

    let mut geaendert = Vec::new();
    for (name, (pass, hard, soft)) in &vorher {
        let Some((p2, h2, s2)) = nachher.get(name) else {
            continue;
        };
        if pass != p2 || hard != h2 || soft != s2 {
            geaendert.push(serde_json::json!({
                "layout": name,
                "vorher": { "pass": pass, "hard": hard, "soft": soft },
                "nachher": { "pass": p2, "hard": h2, "soft": s2 },
            }));
        }
    }
    Ok(Json(serde_json::json!({
        "item": id,
        "layouts": vorher.len(),
        "bestanden_vorher": vorher.values().filter(|(p, _, _)| *p).count(),
        "bestanden_nachher": nachher.values().filter(|(p, _, _)| *p).count(),
        "geaendert": geaendert,
    })))
}

type Verdikt = std::collections::BTreeMap<String, (bool, Vec<String>, Vec<String>)>;

fn verdikte(model: &Model) -> Result<Verdikt, (StatusCode, String)> {
    let mut out = Verdikt::new();
    for name in model.layout_names().map_err(|e| boom(e.to_string()))? {
        let Ok(l) = model.load_layout(&name) else {
            continue;
        };
        let Ok(r) = check_layout(model, &l) else {
            continue;
        };
        let mut hard: Vec<String> = r.hard.iter().map(|v| v.rule.clone()).collect();
        let mut soft: Vec<String> = r.soft.iter().map(|v| v.rule.clone()).collect();
        hard.sort();
        soft.sort();
        out.insert(name, (r.pass, hard, soft));
    }
    Ok(out)
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
        .route("/api/items", post(api_post_item))
        .route("/api/items/:id", put(api_put_item).patch(api_patch_item))
        .route(
            "/api/items/:id/state",
            get(api_state_history).post(api_post_state),
        )
        .route("/api/items/:id/impact", post(api_item_impact))
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

#[cfg(test)]
mod tests {
    use super::merge_patch;
    use crate::store::Item;
    use serde_json::json;

    fn schrank() -> Item {
        Item {
            id: "schrank".into(),
            label: "Ein Schrank".into(),
            b: Some(100),
            t: Some(60),
            h: Some(200),
            open_clear: Some(65),
            unsicher: vec!["h".into()],
            ..Default::default()
        }
    }

    /// Der eigentliche Punkt von PATCH: ein Formular zeigt sechs Felder, der Eintrag hat 40,
    /// und die ungenannten 34 muessen den Vorgang ueberleben. `PUT` konnte das nie.
    #[test]
    fn ein_genanntes_feld_aendert_sich_und_die_uebrigen_bleiben() {
        let neu = merge_patch(&schrank(), json!({ "b": 120 })).expect("Patch passt");
        assert_eq!(neu.b, Some(120));
        assert_eq!(neu.t, Some(60), "ungenannt, also unveraendert");
        assert_eq!(neu.h, Some(200));
        assert_eq!(neu.label, "Ein Schrank");
        assert_eq!(neu.open_clear, Some(65));
        assert_eq!(
            neu.unsicher,
            vec!["h".to_string()],
            "Listen ueberleben auch"
        );
    }

    /// Ein ausdrueckliches `null` loescht. Das ist der Unterschied zu "nicht geschickt", und
    /// ohne ihn koennte die Oberflaeche ein Feld setzen, aber nie zuruecknehmen.
    #[test]
    fn ein_ausdrueckliches_null_loescht_ein_feld() {
        let neu = merge_patch(&schrank(), json!({ "open_clear": null })).expect("Patch passt");
        assert_eq!(neu.open_clear, None);
        assert_eq!(neu.b, Some(100), "der Rest bleibt");
    }

    /// Ein Tippfehler im Feldnamen ist ein Fehler und keine stille Nulloperation.
    ///
    /// Dieselbe Haltung wie `deny_unknown_fields` beim Import, aus demselben Anlass: dort hat
    /// serde neun Felder jahrelang stumm verworfen (PRD B25). Ein PATCH, der `tiefe` statt `t`
    /// schickt und `ok` zurueckgibt, ist genau dieser Fehler mit umgekehrtem Vorzeichen.
    #[test]
    fn ein_unbekanntes_feld_ist_ein_fehler() {
        let e = merge_patch(&schrank(), json!({ "tiefe": 42 })).expect_err("`tiefe` gibt es nicht");
        assert!(e.1.contains("tiefe"), "die Meldung nennt das Feld: {}", e.1);
    }

    /// Die Id aus dem Rumpf wird ignoriert; der Pfad gewinnt.
    ///
    /// Sonst legt ein Formular, das eine fremde Id mitschickt, still eine zweite Zeile an,
    /// statt die gemeinte zu aendern.
    #[test]
    fn der_rumpf_kann_die_id_nicht_umschreiben() {
        let neu = merge_patch(&schrank(), json!({ "id": "etwas_anderes", "b": 110 }))
            .expect("Patch passt");
        assert_eq!(neu.id, "schrank");
        assert_eq!(neu.b, Some(110));
    }

    #[test]
    fn ein_rumpf_der_kein_objekt_ist_wird_abgelehnt() {
        assert!(merge_patch(&schrank(), json!([1, 2, 3])).is_err());
    }
}
