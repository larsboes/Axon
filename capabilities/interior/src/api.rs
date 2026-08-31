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
use serde::Serialize;
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
        "GET",
        "/api/media/*pfad",
        "Ein Bild aus dem Medienverzeichnis des Overlays. Nur von dort, und nur auf Anfrage.",
    ),
    r(
        "GET",
        "/api/layouts/:name/allowed",
        "Erlaubte Positionen eines Stuecks als Lauflaengen. Harte Kanten fuers Ziehen. ?ref=&rot=",
    ),
    r(
        "POST",
        "/api/layouts/:name/preview",
        "Verdikt und Plan zu einer Aufstellung, ohne sie zu schreiben. Fuers Drehen noetig.",
    ),
    r(
        "POST",
        "/api/layouts",
        "Ein neues Layout anlegen. Ueberschreibt nie ein bestehendes.",
    ),
    r(
        "PUT",
        "/api/layouts/:name",
        "Die Positionen eines Layouts ersetzen und sofort neu pruefen. Der Kopf der Datei bleibt.",
    ),
    r(
        "PUT",
        "/api/placements",
        "Wo die Stuecke wirklich stehen, fuer die aktive Wohnung. Body: {items}.",
    ),
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
        "/api/layouts/:name/toleranz",
        "Bis zu welchem Messfehler das Verdikt haelt, und woran es dann kippt.",
    ),
    r(
        "GET",
        "/api/layouts/:name/sonne",
        "Wann im Jahr welches Stueck in direkter Sonne steht. Braucht [lage] in room.toml.",
    ),
    r(
        "GET",
        "/api/layouts/:name/einbringung",
        "Kommt jedes Stueck durch die Tuer und bis an seinen Platz.",
    ),
    r(
        "GET",
        "/api/passt",
        "Passt ein gedachtes Stueck durch die Tuer? ?b=&t=&zerlegbar= — die Frage vor dem Kauf.",
    ),
    r(
        "GET",
        "/api/deklaration",
        "Wer wird noch am Namen gemessen, was waere die Zeile, und was aendert sie.",
    ),
    r(
        "GET",
        "/api/kaufen",
        "Welcher Bedarf zuerst, kumuliert, und wann er aus dem Monatssaldo erreicht ist.",
    ),
    r(
        "POST",
        "/api/search",
        "Eine Suche anstossen. Antwortet 202 mit einer Auftragsnummer, nicht mit dem Ergebnis.",
    ),
    r(
        "POST",
        "/api/compose",
        "Eine ganze Wohnung stellen lassen. Ebenfalls ein Auftrag: die Strahlsuche rechnet Minuten.",
    ),
    r(
        "GET",
        "/api/auftraege/:id",
        "Was aus einem Auftrag geworden ist: laeuft, fertig mit Ergebnis, oder gescheitert.",
    ),
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

#[derive(serde::Deserialize)]
struct LayoutBody {
    items: Vec<crate::model::PlacedItem>,
}

/// Die Positionen eines bestehenden Layouts ersetzen und sofort neu pruefen.
///
/// Antwortet mit dem Verdikt und dem fertigen Plan, damit die Oberflaeche nach einem Zug nichts
/// selbst zu rechnen hat. Der Kopf der Datei bleibt stehen — `layout_io` erhaelt ihn, und wo er
/// nicht genuegt, bricht es ab statt zu kuerzen.
async fn api_put_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    crate::layout_io::update(&model, &id, &body.items)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let l = model
        .load_layout(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let r = check_layout(&model, &l).map_err(boom)?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Ein Bild aus `<overlay>/data/interior/media/`, auf Anfrage und nur von dort.
///
/// **Der Pfad kommt vom Client, also wird er aufgeloest und geprueft, nicht zusammengesetzt.**
/// `canonicalize` beidseitig und dann ein `starts_with`: ein `..`, ein absoluter Pfad oder ein
/// Symlink, der aus dem Verzeichnis zeigt, faellt damit auf, statt eine Datei auszuliefern, die
/// niemand gemeint hat. Eine Pruefung auf die Zeichenfolge `..` allein waere die Fassung, die
/// bei einem Symlink still versagt.
///
/// `service.toml` nennt diese Trennung als den Grund, aus dem die Capability oeffentlich stehen
/// darf: das Bundle enthaelt kein Foto, die Dateien liegen im Overlay, und geliefert wird erst
/// auf Anfrage.
async fn api_media(Path(pfad): Path<String>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let wurzel = crate::model::data_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("media");
    let wurzel = wurzel
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "kein media-Verzeichnis".to_string()))?;
    let ziel = wurzel
        .join(&pfad)
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, format!("kein Medium `{pfad}`")))?;
    if !ziel.starts_with(&wurzel) || !ziel.is_file() {
        return Err((
            StatusCode::FORBIDDEN,
            format!("`{pfad}` liegt nicht unter media/"),
        ));
    }
    let typ = match ziel
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        // Kein Standardtyp: was hier unbekannt ist, wird nicht geraten und nicht geliefert.
        _ => return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "kein Bildformat".into())),
    };
    let bytes = std::fs::read(&ziel).map_err(boom)?;
    Ok(([(axum::http::header::CONTENT_TYPE, typ)], bytes))
}

/// Wo die linke obere Ecke eines Stuecks liegen darf — harte Kanten fuers Ziehen.
///
/// Die Oberflaeche fragt einmal beim Aufnehmen und rastet danach auf die Liste ein. Sie
/// bekommt Lauflaengen und keine Geometrie: der Hauptraum ist ein Sechseck, und auf ein
/// umschliessendes Rechteck zu klemmen wuerde ein Moebel in der Kerbe abstellen, in der das
/// Bad liegt.
async fn api_allowed(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::extract::Query(q): axum::extract::Query<AllowedQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let base = model
        .load_layout(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let a = crate::search::allowed_positions(&model, &base, &q.reference, q.rot)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(a))
}

#[derive(serde::Deserialize)]
struct AllowedQuery {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    rot: i32,
}

/// Wie ein Layout AUSSAEHE und ausfiele, ohne es zu schreiben.
///
/// Verschieben kann die Oberflaeche selbst zeichnen: eine Verschiebung ist eine Translation und
/// aendert an der Grundflaeche nichts. **Drehen kann sie nicht** — bei 90 Grad tauschen Breite
/// und Tiefe, und `opens` und `expands_dir` drehen mit. Das im Browser nachzubauen waere eine
/// zweite Fassung von `footprint` und `Seite::gedreht`, also genau die Doppelung, gegen die
/// diese Capability existiert. Sie fragt stattdessen hier nach.
async fn api_preview_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let vorlage = model
        .load_layout(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let l = crate::model::Layout {
        name: vorlage.name,
        items: body.items,
        id: id.clone(),
    };
    let r = check_layout(&model, &l).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Ein neues Layout anlegen. Ueberschreibt nie ein bestehendes.
async fn api_post_layout(
    State(s): State<Arc<AppState>>,
    Json(body): Json<NewLayout>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let layout = crate::model::Layout {
        name: body.name.clone(),
        items: body.items,
        id: body.id.clone(),
    };
    crate::layout_io::create(&model, &body.id, &layout, &body.notiz)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    let l = model
        .load_layout(&body.id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let r = check_layout(&model, &l).map_err(boom)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": body.id, "check": r })),
    ))
}

#[derive(serde::Deserialize)]
struct NewLayout {
    id: String,
    name: String,
    items: Vec<crate::model::PlacedItem>,
    #[serde(default = "standard_notiz")]
    notiz: String,
}

fn standard_notiz() -> String {
    "In der Oberflaeche gestellt.".to_string()
}

/// Wo die Stuecke in dieser Wohnung WIRKLICH stehen — nicht, was vorgeschlagen ist.
///
/// Der Unterschied ist der Grund, aus dem `interior_placement` eine eigene Tabelle ist: ein
/// Layout ist ein Vorschlag und eine Datei, eine Platzierung ist der Zustand und eine Zeile.
/// Die Tabelle stand seit B25 leer, weil nichts sie geschrieben hat.
async fn api_put_placements(
    State(s): State<Arc<AppState>>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store()?;
    for it in &body.items {
        st.place(&crate::store::Placement {
            item_id: it.reference.clone(),
            flat: s.flat.clone(),
            x: it.x,
            y: it.y,
            rot: it.rot,
        })
        .map_err(boom)?;
    }
    Ok(Json(
        serde_json::json!({ "flat": s.flat, "gesetzt": body.items.len() }),
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
        .route("/api/layouts", get(api_layouts).post(api_post_layout))
        .route("/api/layouts/:name", get(api_layout).put(api_put_layout))
        .route("/api/layouts/:name/preview", post(api_preview_layout))
        .route("/api/layouts/:name/allowed", get(api_allowed))
        .route("/api/placements", put(api_put_placements))
        .route("/api/flats", get(api_flats))
        .route("/api/inventory", get(api_inventory))
        .route("/api/media/*pfad", get(api_media))
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
        .route("/api/layouts/:name/toleranz", get(api_toleranz))
        .route("/api/layouts/:name/sonne", get(api_sonne))
        .route("/api/layouts/:name/einbringung", get(api_einbringung))
        .route("/api/passt", get(api_passt))
        .route("/api/deklaration", get(api_deklaration))
        .route("/api/kaufen", get(api_kaufen))
        .route("/api/search", post(api_search))
        .route("/api/compose", post(api_compose))
        .route("/api/auftraege/:id", get(api_auftrag))
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
    use super::{merge_patch, Auftraege, Auftragsstand, AUFTRAEGE_MAX};
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

    /// Zehn Auftraege anlegen und die jungen davon fertig melden.
    fn volle_karte(laufen: u64) -> Auftraege {
        let mut a = Auftraege::default();
        for _ in 0..AUFTRAEGE_MAX {
            a.anlegen().expect("unter der Grenze");
        }
        for id in laufen..AUFTRAEGE_MAX as u64 {
            a.stand.get_mut(&id).expect("gerade angelegt").1 = Auftragsstand::Fertig {
                ergebnis: json!(null),
            };
        }
        a
    }

    /// Verdraengt wird der aelteste FERTIGE, nicht der aelteste.
    ///
    /// Bis 2026-08-31 warf `anlegen` blind den ersten Schluessel weg. Traf es einen, der noch
    /// rechnete, schrieb sein Hintergrundfaden das Ergebnis in eine Nummer, die es nicht mehr
    /// gab — und der Abholer bekam 404 auf eine Suche, die Minuten gelaufen war.
    #[test]
    fn ein_laufender_auftrag_wird_nicht_verdraengt() {
        let mut a = volle_karte(1);
        let neu = a.anlegen().expect("neun fertige machen Platz");
        assert!(a.stand.contains_key(&0), "der laufende steht noch da");
        assert!(
            !a.stand.contains_key(&1),
            "der aelteste fertige ist gewichen"
        );
        assert!(a.stand.contains_key(&neu));
        assert_eq!(a.stand.len(), AUFTRAEGE_MAX);
    }

    /// Rechnen alle zehn, ist die Absage die einzige ehrliche Antwort.
    #[test]
    fn eine_volle_karte_aus_laufenden_lehnt_ab() {
        let mut a = volle_karte(AUFTRAEGE_MAX as u64);
        let grund = a.anlegen().expect_err("es gibt nichts zu verdraengen");
        assert!(grund.contains("rechnen bereits"), "{grund}");
        assert_eq!(a.stand.len(), AUFTRAEGE_MAX, "und keiner ist verschwunden");
        assert!((0..AUFTRAEGE_MAX as u64).all(|id| a.stand.contains_key(&id)));
    }
}

// ---------------------------------------------------------------- lange Rechnungen

/// Was aus einer Suche geworden ist.
///
/// **Warum das ueberhaupt eine eigene Form braucht.** `search` prueft die Kandidaten
/// erschoepfend — 3,5 Millionen in rund hundert Sekunden (PRD §13.1). Eine HTTP-Anfrage, die
/// hundert Sekunden offen steht, ist keine Anfrage mehr, sondern eine Wette auf jeden Proxy
/// und jedes Zeitlimit dazwischen. Bis 2026-08-31 gab es die Suche deshalb nur auf der
/// Kommandozeile: die teuerste Rechnung dieser Capability war von der Oberflaeche aus, die sie
/// braucht, nicht erreichbar.
///
/// Der Auftrag ist die Antwort darauf, und er ist absichtlich das kleinste, was funktioniert:
/// eine Nummer, ein Zustand, ein Ergebnis. Keine Warteschlange, keine Wiederaufnahme, keine
/// Tabelle. Er lebt im Prozess und stirbt mit ihm — was richtig ist, weil sein Ergebnis eine
/// Liste von Vorschlaegen ist und keine Tatsache ueber die Wohnung. Wer den Vorschlag behalten
/// will, schreibt ihn als Layout.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "zustand", rename_all = "snake_case")]
pub enum Auftragsstand {
    Laeuft { seit_ms: u128 },
    Fertig { ergebnis: serde_json::Value },
    Gescheitert { grund: String },
}

#[derive(Default)]
struct Auftraege {
    naechste: u64,
    stand: std::collections::BTreeMap<u64, (std::time::Instant, Auftragsstand)>,
}

/// Wie viele fertige Auftraege aufgehoben werden.
///
/// Ohne Grenze waechst die Karte mit jeder Suche, und ein Prozess, der wochenlang laeuft,
/// haelt jedes Ergebnis fest, das je jemand angesehen hat. Zehn, weil die Oberflaeche das
/// letzte abholt und die davor nur noch Verlauf sind.
const AUFTRAEGE_MAX: usize = 10;

impl Auftraege {
    /// Platz schaffen und die naechste Nummer ziehen — oder ablehnen.
    ///
    /// Verdraengt wird nur, was schon fertig ist. Ein laufender Auftrag hat seinen Schreiber
    /// noch im Hintergrund: verschwindet der Eintrag, schreibt der in nichts, und der Abholer
    /// bekommt 404 auf eine Rechnung, die minutenlang lief. Sind alle zehn belegt, ist die
    /// ehrliche Antwort eine Absage und keine stillschweigend verlorene Suche.
    fn anlegen(&mut self) -> Result<u64, String> {
        while self.stand.len() >= AUFTRAEGE_MAX {
            // Die Schluessel steigen, also ist der erste verdraengbare auch der aelteste.
            let Some(alt) = self
                .stand
                .iter()
                .find(|(_, (_, s))| !matches!(s, Auftragsstand::Laeuft { .. }))
                .map(|(id, _)| *id)
            else {
                return Err(format!(
                    "{AUFTRAEGE_MAX} Auftraege rechnen bereits — warte, bis einer fertig ist"
                ));
            };
            self.stand.remove(&alt);
        }
        let id = self.naechste;
        self.naechste += 1;
        self.stand.insert(
            id,
            (
                std::time::Instant::now(),
                Auftragsstand::Laeuft { seit_ms: 0 },
            ),
        );
        Ok(id)
    }
}

fn auftraege() -> &'static std::sync::Mutex<Auftraege> {
    static A: std::sync::OnceLock<std::sync::Mutex<Auftraege>> = std::sync::OnceLock::new();
    A.get_or_init(|| std::sync::Mutex::new(Auftraege::default()))
}

/// Eine Rechnung im Hintergrund starten und sofort ihre Nummer zurueckgeben.
///
/// `spawn_blocking`, weil `search` und `compose` rayon benutzen und minutenlang rechnen: auf
/// einem async-Thread wuerde das jede andere Anfrage dieses Prozesses anhalten. Dieselbe
/// Begruendung, aus der `punctuality` und `finance` ihre schweren Wege dorthin legen (PRD B20).
fn im_hintergrund<F>(f: F) -> Result<u64, (StatusCode, String)>
where
    F: FnOnce() -> Result<serde_json::Value, String> + Send + 'static,
{
    let id = auftraege()
        .lock()
        .expect("Auftragskarte")
        .anlegen()
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    tokio::task::spawn_blocking(move || {
        let ergebnis = f();
        let mut a = auftraege().lock().expect("Auftragskarte");
        if let Some((_, stand)) = a.stand.get_mut(&id) {
            *stand = match ergebnis {
                Ok(v) => Auftragsstand::Fertig { ergebnis: v },
                Err(e) => Auftragsstand::Gescheitert { grund: e },
            };
        }
    });
    Ok(id)
}

async fn api_auftrag(Path(id): Path<u64>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let a = auftraege().lock().map_err(boom)?;
    let (start, stand) = a
        .stand
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("kein Auftrag {id}")))?;
    // Die Laufzeit wird beim Lesen gerechnet und nicht fortgeschrieben: ein Auftrag, der sie
    // selbst zaehlen muesste, braeuchte einen zweiten Faden fuer nichts.
    let stand = match stand {
        Auftragsstand::Laeuft { .. } => Auftragsstand::Laeuft {
            seit_ms: start.elapsed().as_millis(),
        },
        fertig => fertig.clone(),
    };
    Ok(Json(serde_json::json!({ "id": id, "stand": stand })))
}

#[derive(serde::Deserialize)]
struct SucheAnfrage {
    layout: String,
    #[serde(default)]
    move_refs: Vec<String>,
    #[serde(default = "raster_standard")]
    step: i32,
    #[serde(default = "suche_grenze")]
    limit: usize,
}

fn raster_standard() -> i32 {
    20
}
/// Dieselbe Zahl wie `--limit` auf der Kommandozeile (`main.rs`, `cmd_search`). Ohne sie stand
/// hier `usize::default()`, und die Null heisst in `search::search` *unbegrenzt*: dieselbe
/// Anfrage lieferte auf der Oberflaeche Tausende Treffer und im Terminal sechs.
fn suche_grenze() -> usize {
    6
}

/// Eine Suche anstossen. Antwortet mit der Auftragsnummer, nicht mit dem Ergebnis.
async fn api_search(
    State(s): State<Arc<AppState>>,
    Json(anfrage): Json<SucheAnfrage>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if anfrage.move_refs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "`move_refs` ist leer — ohne bewegliche Moebel gibt es nichts zu suchen".into(),
        ));
    }
    let flat = s.flat.clone();
    let id = im_hintergrund(move || {
        let model = Model::load(&flat).map_err(|e| e.to_string())?;
        let base = model
            .load_layout(&anfrage.layout)
            .map_err(|e| e.to_string())?;
        let spec = crate::search::Spec {
            move_refs: anfrage.move_refs,
            step: anfrage.step,
            bands: Default::default(),
            limit: anfrage.limit,
        };
        let rep = crate::search::search(&model, &base, &spec).map_err(|e| e.to_string())?;
        serde_json::to_value(rep).map_err(|e| e.to_string())
    })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "auftrag": id })),
    ))
}

#[derive(serde::Deserialize)]
struct ComposeAnfrage {
    refs: Vec<String>,
    #[serde(default = "compose_raster")]
    step: i32,
    #[serde(default = "compose_strahl")]
    beam: usize,
    #[serde(default = "compose_grenze")]
    limit: usize,
    #[serde(default = "compose_drehungen")]
    rotations: Vec<i32>,
}

fn compose_raster() -> i32 {
    25
}
fn compose_strahl() -> usize {
    60
}
/// Dieselbe Zahl wie `--limit` auf der Kommandozeile (`main.rs`, `cmd_compose`).
fn compose_grenze() -> usize {
    5
}
fn compose_drehungen() -> Vec<i32> {
    vec![0, 90]
}

/// Eine ganze Wohnung stellen lassen. Ebenfalls ein Auftrag: die Strahlsuche rechnet Minuten.
async fn api_compose(
    State(s): State<Arc<AppState>>,
    Json(anfrage): Json<ComposeAnfrage>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if anfrage.refs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "`refs` ist leer — ohne Stuecke gibt es nichts zu stellen".into(),
        ));
    }
    let flat = s.flat.clone();
    let id = im_hintergrund(move || {
        let model = Model::load(&flat).map_err(|e| e.to_string())?;
        let spec = crate::search::ComposeSpec {
            refs: anfrage.refs,
            step: anfrage.step,
            beam: anfrage.beam,
            rotations: anfrage.rotations,
            limit: anfrage.limit,
        };
        let out = crate::search::compose(&model, &spec).map_err(|e| e.to_string())?;
        serde_json::to_value(out).map_err(|e| e.to_string())
    })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "auftrag": id })),
    ))
}

// ---------------------------------------------------------------- die neuen Auskuenfte

async fn api_toleranz(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(nicht_gefunden)?;
    Ok(Json(crate::toleranz::robustheit(&model, &l).map_err(boom)?))
}

async fn api_sonne(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(nicht_gefunden)?;
    Ok(Json(crate::sonne::bericht(&model, &l).map_err(boom)?))
}

async fn api_einbringung(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(nicht_gefunden)?;
    let mut out = Vec::new();
    for it in &l.items {
        out.push(crate::einbringung::einbringung(&model, &l, &it.reference).map_err(boom)?);
    }
    Ok(Json(out))
}

#[derive(serde::Deserialize)]
struct StueckMasse {
    b: i32,
    t: i32,
    #[serde(default)]
    zerlegbar: bool,
}

/// Passt ein gedachtes Stueck durch die Tuer? Die Frage VOR dem Kauf.
async fn api_passt(
    State(s): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<StueckMasse>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    Ok(Json(crate::einbringung::durch_die_tuer(
        &model,
        q.b,
        q.t,
        q.zerlegbar,
    )))
}

async fn api_deklaration(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    Ok(Json(crate::deklaration::uebersicht(&model).map_err(boom)?))
}

async fn api_kaufen(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let st = store()?;
    let conn = st.borrow_connection().map_err(boom)?;
    let saldo = crate::budget::monatssaldo(&conn).map_err(boom)?;
    Ok(Json(
        crate::budget::kaufreihenfolge(&model, saldo).map_err(boom)?,
    ))
}

fn nicht_gefunden(e: crate::model::ModelError) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, e.to_string())
}
