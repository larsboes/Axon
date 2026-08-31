//! interior — Raumplanung.
//!
//! Welche Wohnung gemeint ist, entscheidet `AXON_INTERIOR_FLAT` oder `--flat`; liegt genau
//! eine unter `data/interior/flats/`, ist es die. Kein Name steht hier im Quelltext — er stand
//! es bis 2026-08-30, und `tests/containment.rs` sorgt dafuer, dass er nicht zurueckkommt.
//!
//! Liest das gemessene Modell aus dem privaten Overlay, prueft Layouts gegen die
//! Raeumungsregeln und sucht Positionen, die sie erfuellen. `check` beendet sich mit einem
//! Fehlercode, wenn eine harte Regel verletzt ist, und taugt damit als Gate.
//!
//! `search --out` schreibt den besten Treffer als echtes Layout auf die Platte. Das ist kein
//! Komfort: bis zum 2026-08-30 lebten Suchergebnisse ausschliesslich in einer Terminalausgabe
//! und waren mit der Sitzung weg.

use interior::clearance::{check_layout, CheckResult};
use interior::model::{default_flat, Layout, Model};
use interior::plan;
use interior::search::{search, Spec};
use std::collections::BTreeMap;

fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[0m")
}
fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
}
fn red(s: &str) -> String {
    format!("\x1b[31m{s}\x1b[0m")
}
fn green(s: &str) -> String {
    format!("\x1b[32m{s}\x1b[0m")
}
fn yellow(s: &str) -> String {
    format!("\x1b[33m{s}\x1b[0m")
}

fn usage() -> ! {
    eprintln!(
        r#"
{t} — Raumplanung

  {model}                        gemessener Raum, und was daran noch geraten ist
  {layouts}                      Layouts auf der Platte
  {check} <layout> [--json]      gegen rules.toml pruefen (Exit 1 bei hartem Verstoss)
  {search} <layout> --move a,b   Positionen rastern, bis nichts mehr verletzt ist
                                 [--step 20] [--limit 6] [--band id=x0,x1,y0,y1]
                                 [--out <name>]  besten Treffer als Layout schreiben
  {plan} [layout...] [--out f]   Plaene als HTML mit Verdikt; ohne Layouts: alle
  {inventory}                    was da ist und was fehlt, mit Zustand und Preis
  {import}                       inventory/*.toml in die Tabellen (wiederholbar)
  {serve}                        HTTP-API fuer die Oberflaeche
"#,
        t = bold("interior"),
        model = bold("model"),
        layouts = bold("layouts"),
        plan = bold("plan"),
        inventory = bold("inventory"),
        import = bold("import"),
        check = bold("check"),
        search = bold("search"),
        serve = bold("serve"),
    );
    std::process::exit(1)
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| argv.get(i + 1).cloned())
}

fn report(r: &CheckResult) {
    let status = if r.pass {
        green("BESTANDEN")
    } else {
        red("DURCHGEFALLEN")
    };
    println!("\n{}  {}\n", bold(&r.layout), status);
    if !r.hard.is_empty() {
        println!(
            "{}",
            red(&bold(&format!("  {} harte Verstoesse", r.hard.len())))
        );
        for v in &r.hard {
            println!("    {} {}  {}", red("x"), bold(&v.rule), v.message);
        }
        println!();
    }
    if !r.soft.is_empty() {
        println!(
            "{}",
            yellow(&bold(&format!("  {} weiche Warnungen", r.soft.len())))
        );
        for v in &r.soft {
            println!("    {} {}  {}", yellow("!"), bold(&v.rule), v.message);
        }
        println!();
    }
    if r.hard.is_empty() && r.soft.is_empty() {
        println!("{}\n", green("  keine Verstoesse"));
    }
    let m = &r.metrics;
    println!("{}", bold("  Flaeche"));
    println!(
        "    Raum {:.2} m²   belegt {:.2} m²   frei {:.2} m²",
        m.room_area_m2, m.occupied_area_m2, m.free_area_m2
    );
    for c in &m.corridors {
        match c.width_cm {
            Some(w) => println!("    {} → {}: {} cm", c.from, c.to, w),
            None => println!("    {} → {}: {}", c.from, c.to, red("kein Weg")),
        }
    }
    if !r.uncertainties.is_empty() {
        println!("\n{}", yellow(&bold("  gemessen? nein.")));
        for u in &r.uncertainties {
            println!("    · {} — {}", u.label, u.fields.join(", "));
        }
        println!("{}", dim("    Jede Zahl oben erbt diese Schaetzungen."));
    }
    println!();
}

fn write_layout(model: &Model, name: &str, layout: &Layout, note: &str) -> std::io::Result<()> {
    let mut s = String::new();
    s.push_str(&format!("# {note}\n#\n# Von `interior search` erzeugt, nicht von Hand gesetzt. Jede Position hat die volle\n# Raeumungspruefung bestanden; die Begruendung steht im Suchbericht daneben.\n\n"));
    s.push_str(&format!("name = \"{}\"\n\n", layout.name));
    for it in &layout.items {
        s.push_str("[[item]]\n");
        s.push_str(&format!("ref = \"{}\"\n", it.reference));
        s.push_str(&format!("x = {}\ny = {}\nrot = {}\n", it.x, it.y, it.rot));
        if let Some(sz) = it.size {
            s.push_str(&format!("size = [{}, {}]\n", sz[0], sz[1]));
        }
        s.push('\n');
    }
    std::fs::write(model.layouts_dir().join(format!("{name}.toml")), s)
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().map(String::as_str) else {
        usage()
    };

    if cmd == "serve" {
        interior_serve().await;
        return;
    }

    // Beide brauchen keine Wohnung: das Inventar ueberlebt jede.
    if cmd == "import" || cmd == "inventory" {
        let code = match cmd {
            "import" => inventory_import(&argv),
            _ => inventory_show(),
        };
        std::process::exit(code);
    }

    let flat = match flag(&argv, "flat").map(Ok).unwrap_or_else(default_flat) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            std::process::exit(2);
        }
    };
    let model = match Model::load(&flat) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}", red(&format!("Modell laedt nicht: {e}")));
            std::process::exit(2);
        }
    };

    match cmd {
        "model" => {
            println!("\n{}\n", bold(&model.room.flat.name));
            println!(
                "  planbar    {:.2} m²  {}",
                model.room.area_m2(),
                dim("(aus dem Polygon gerechnet)")
            );
            println!(
                "  Hoehe      {}",
                if model.room.hauptraum.hoehe == 0 {
                    red("nicht gemessen")
                } else {
                    model.room.hauptraum.hoehe.to_string()
                }
            );
            println!(
                "  Waende     {}",
                model
                    .room
                    .waende
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "  Oeffnungen {}",
                model
                    .room
                    .oeffnungen
                    .iter()
                    .map(|o| o.id.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("  Katalog    {} Eintraege", model.catalogue.len());
            if !model.room.todo.offen.is_empty() {
                println!("\n{}", yellow(&bold("  noch zu messen")));
                for t in &model.room.todo.offen {
                    println!("    · {t}");
                }
            }
            let unc = model.uncertainties();
            if !unc.is_empty() {
                println!(
                    "\n{}",
                    yellow(&bold(&format!("  geschaetzte Masse ({})", unc.len())))
                );
                for (_, label, fields) in unc {
                    println!("    · {label} — {}", fields.join(", "));
                }
            }
            println!();
        }
        "layouts" => {
            for n in model.layout_names().unwrap_or_default() {
                println!("  {n}");
            }
        }
        "check" => {
            let Some(name) = argv.get(1) else { usage() };
            let layout = match model.load_layout(name) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let r = match check_layout(&model, &layout) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            if argv.contains(&"--json".to_string()) {
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            } else {
                report(&r);
            }
            std::process::exit(if r.pass { 0 } else { 1 });
        }
        "search" => {
            let Some(name) = argv.get(1) else { usage() };
            let move_refs: Vec<String> = flag(&argv, "move")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if move_refs.is_empty() {
                eprintln!(
                    "  --move <id>[,<id>] fehlt — ohne bewegliche Moebel gibt es nichts zu suchen"
                );
                std::process::exit(1);
            }
            let mut bands: BTreeMap<String, [i32; 4]> = BTreeMap::new();
            for (i, a) in argv.iter().enumerate() {
                if a != "--band" {
                    continue;
                }
                let Some(spec) = argv.get(i + 1) else {
                    continue;
                };
                let Some((id, nums)) = spec.split_once('=') else {
                    continue;
                };
                let v: Vec<i32> = nums
                    .split(',')
                    .filter_map(|n| n.trim().parse().ok())
                    .collect();
                if v.len() == 4 {
                    bands.insert(id.into(), [v[0], v[1], v[2], v[3]]);
                }
            }
            let spec = Spec {
                move_refs,
                step: flag(&argv, "step")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20),
                bands,
                limit: flag(&argv, "limit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(6),
            };
            let base = match model.load_layout(name) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let rep = match search(&model, &base, &spec) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            println!(
                "\n{}  {}\n",
                bold(&format!("Suche in \"{}\"", rep.base)),
                dim(&format!(
                    "beweglich: {} · Raster {} cm",
                    rep.moved.join(", "),
                    rep.step
                ))
            );
            if rep.hits.is_empty() {
                println!("  {}", red("keine verstossfreie Position gefunden"));
                println!(
                    "{}",
                    dim(&format!(
                        "  Das ist ein Ergebnis, kein Fehler: bei Raster {} cm existiert keine.",
                        rep.step
                    ))
                );
                println!(
                    "{}",
                    dim("  Feiner suchen (--step 10) oder ein Moebel mehr freigeben (--move).")
                );
            } else {
                for h in &rep.hits {
                    let tag = if h.soft == 0 {
                        green("0 Warnungen")
                    } else {
                        yellow(&format!("{} Warnungen", h.soft))
                    };
                    println!(
                        "  {}  {}",
                        tag,
                        dim(&format!(
                            "Wandkontakt {} cm · engste Route {} cm",
                            h.wandkontakt_cm, h.bottleneck_cm
                        ))
                    );
                    println!(
                        "    {}",
                        h.places
                            .iter()
                            .map(|(k, p)| format!("{k} {},{}", p[0], p[1]))
                            .collect::<Vec<_>>()
                            .join("  ")
                    );
                }
                if let Some(out) = flag(&argv, "out") {
                    let best = &rep.hits[0];
                    let mut items = base.items.clone();
                    for it in items.iter_mut() {
                        if let Some(p) = best.places.get(&it.reference) {
                            it.x = p[0];
                            it.y = p[1];
                        }
                    }
                    let l = Layout {
                        name: out.clone(),
                        items,
                        id: out.clone(),
                    };
                    let note = format!("Bester Treffer der Suche in \"{}\": {} Warnungen, {} cm Wandkontakt, engste Route {} cm.", rep.base, best.soft, best.wandkontakt_cm, best.bottleneck_cm);
                    match write_layout(&model, &out, &l, &note) {
                        Ok(()) => println!(
                            "\n  {} {}",
                            green("geschrieben:"),
                            model.layouts_dir().join(format!("{out}.toml")).display()
                        ),
                        Err(e) => eprintln!("\n  {} {e}", red("Schreiben fehlgeschlagen:")),
                    }
                }
            }
            println!(
                "{}",
                dim(&format!(
                    "\n  {} Kandidaten nach Vorfilter, {} voll geprueft, {} Treffer in {:.1} s\n",
                    rep.candidates_after_filter,
                    rep.fully_checked,
                    rep.hits.len(),
                    rep.elapsed_ms as f64 / 1000.0
                ))
            );
        }
        "plan" => {
            let names: Vec<String> = argv[1..]
                .iter()
                .take_while(|a| !a.starts_with("--"))
                .cloned()
                .collect();
            let names = if names.is_empty() {
                model.layout_names().unwrap_or_default()
            } else {
                names
            };
            let mut layouts = Vec::new();
            for n in &names {
                match model.load_layout(n) {
                    Ok(l) => layouts.push(l),
                    Err(e) => {
                        eprintln!("{}", red(&e.to_string()));
                        std::process::exit(2);
                    }
                }
            }
            let html = match plan::page(&model, &layouts) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let out = flag(&argv, "out").unwrap_or_else(|| "plaene.html".into());
            match std::fs::write(&out, &html) {
                Ok(()) => println!(
                    "  {} {out}  ({} Plaene, {:.0} KB)",
                    green("geschrieben:"),
                    layouts.len(),
                    html.len() as f64 / 1024.0
                ),
                Err(e) => {
                    eprintln!("{}", red(&format!("Schreiben fehlgeschlagen: {e}")));
                    std::process::exit(2);
                }
            }
        }
        _ => usage(),
    }
}

async fn interior_serve() {
    // Der Port kommt aus service.toml und wird als AXON_PORT gesetzt; 8092 ist der Wert, unter
    // dem die Vorgaengerfassung erreichbar war, damit vorhandene Lesezeichen weiter stimmen.
    let port: u16 = std::env::var("AXON_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8092);
    let flat = match default_flat() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    interior::api::serve(&flat, port).await;
}

/// `inventory/*.toml` in die Tabellen. Der Bericht sagt, was passiert ist — ein Import, der
/// "ok" meldet und nichts geschrieben hat, ist die stille Variante des Fehlers, gegen den er
/// existiert.
///
/// **Seit PRD Q64 (2026-08-31) verweigert er sich, sobald Zeilen da sind.** Bis dahin war er
/// wiederholbar, und das war richtig, solange die Dateien die einzige Quelle waren. Sobald die
/// Oberflaeche schreibt, ist Wiederholbarkeit das Gegenteil: derselbe Befehl, der gestern nichts
/// kaputt machte, setzt heute jede Eingabe auf den Stand der Dateien zurueck — still, mit
/// Exit-Code 0 und einer Erfolgsmeldung. `--force` macht daraus eine Entscheidung.
fn inventory_import(argv: &[String]) -> i32 {
    let dir = match interior::model::data_dir() {
        Ok(d) => d.join("inventory"),
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    let store = match interior::store::Store::open(&axon_config::database_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&format!("Datenbank nicht erreichbar: {e}")));
            return 2;
        }
    };
    let force = argv.iter().any(|a| a == "--force");
    match store.item_count() {
        Ok(n) if n > 0 && !force => {
            eprintln!(
                "\n  {}\n\n  Die Tabellen sind seit PRD Q64 die Wahrheit, nicht `inventory/*.toml`.\n  \
                 Ein Import wuerde jede Aenderung aus der Oberflaeche auf den Stand der Dateien\n  \
                 zuruecksetzen.\n\n  {}\n",
                red(&format!("{n} Eintraege stehen schon in der Datenbank.")),
                dim("Wenn genau das gemeint ist: interior import --force")
            );
            return 2;
        }
        Err(e) => {
            eprintln!("{}", red(&format!("Datenbank nicht lesbar: {e}")));
            return 2;
        }
        _ => {}
    }
    if force {
        eprintln!(
            "  {}",
            dim("--force: die Dateien ueberschreiben die Tabellen")
        );
    }
    match interior::import::inventory(&store, &dir) {
        Ok(b) => {
            println!(
                "\n  {} {} Stuecke, {} Bedarfe, {} Zustandswechsel\n  {}\n",
                green("importiert:"),
                b.pieces,
                b.slots,
                b.zustandswechsel,
                dim(&format!("aus {}", dir.display()))
            );
            0
        }
        Err(e) => {
            eprintln!("{}", red(&format!("Import fehlgeschlagen: {e}")));
            2
        }
    }
}

/// Was da ist, was fehlt, und was das Fehlende kostet.
fn inventory_show() -> i32 {
    use interior::store::State;
    let store = match interior::store::Store::open(&axon_config::database_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&format!("Datenbank nicht erreichbar: {e}")));
            return 2;
        }
    };
    let rows = match store.catalogue() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    if rows.is_empty() {
        println!(
            "\n  {}\n",
            yellow("leer — `interior import` fuellt die Tabellen")
        );
        return 0;
    }
    let mut summe: i64 = 0;
    for state in [State::Owned, State::Wanted, State::Gone] {
        let gruppe: Vec<_> = rows
            .values()
            .filter(|(_, s)| *s == Some(state))
            .map(|(i, _)| i)
            .collect();
        if gruppe.is_empty() {
            continue;
        }
        println!(
            "\n{}",
            bold(&format!("  {} ({})", state.as_str(), gruppe.len()))
        );
        for i in gruppe {
            let masse = match (i.b, i.t) {
                (Some(b), Some(t)) => format!("{b}×{t}"),
                _ => "—".into(),
            };
            if state == State::Wanted {
                summe += i.preis_cent.or(i.kosten_min_cent).unwrap_or(0);
            }
            let geld = match (i.preis_cent, i.kosten_min_cent, i.kosten_max_cent) {
                (Some(p), _, _) => format!("{:.2} €", p as f64 / 100.0),
                (None, Some(lo), Some(hi)) => {
                    format!("{:.0}–{:.0} €", lo as f64 / 100.0, hi as f64 / 100.0)
                }
                _ => String::new(),
            };
            let flag = if i.is_uncertain() {
                yellow("~")
            } else {
                " ".into()
            };
            println!("    {flag} {:<34} {:>9}  {}", i.id, masse, dim(&geld));
        }
    }
    println!(
        "\n  {} {:.2} €\n  {}\n",
        bold("offener Bedarf:"),
        summe as f64 / 100.0,
        dim("untere Kante: Produktpreis, sonst das Minimum der Schaetzung. `~` = Masse geraten.")
    );
    0
}
