//! Zeichnet ein Layout als SVG.
//!
//! Nichts hier kennt ein Mass. Jede Zahl auf dem Blatt kommt zur Laufzeit aus dem Modell, und
//! die Zeichnung entsteht aus denselben Koordinaten, gegen die der Pruefer urteilt — Bild und
//! Verdikt koennen deshalb nicht auseinanderlaufen. Genau diese Drift ist der Grund, warum es
//! keine handgezeichnete Planseite gibt.
//!
//! Selbsttragend: keine externe Schrift, kein Skript, kein Bildhost. Das SVG funktioniert aus
//! einer Datei, im Browser und in einer Vorschau ohne Netz.

use crate::clearance::{check_layout, kind_of, Kind};
use crate::model::{footprint, Layout, Model, ModelError, Rect};

const PAD: i32 = 40;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn short(kind: Kind, label: &str) -> String {
    match kind {
        Kind::Bed => "Bett".into(),
        Kind::Desk => "Schreibtisch".into(),
        Kind::Wardrobe => "Schrank".into(),
        Kind::Couch => "Couch".into(),
        Kind::Table => "Tisch".into(),
        Kind::CoffeeTable => "Couchtisch".into(),
        Kind::Shelf => "Regal".into(),
        Kind::Other => label
            .split([',', '('])
            .next()
            .unwrap_or(label)
            .trim()
            .to_string(),
    }
}

pub fn svg(model: &Model, layout: &Layout) -> Result<String, ModelError> {
    let room = &model.room;
    let poly = &room.hauptraum.polygon;
    let mut xs: Vec<i32> = poly.iter().map(|p| p[0]).collect();
    let mut ys: Vec<i32> = poly.iter().map(|p| p[1]).collect();
    if let Some(b) = &room.bad {
        xs.extend(b.x);
        ys.extend(b.y);
    }
    if let Some(t) = &room.terrasse {
        xs.extend(t.x);
        ys.extend(t.y);
    }
    let (min_x, max_x) = (
        xs.iter().min().unwrap() - PAD,
        xs.iter().max().unwrap() + PAD,
    );
    let (min_y, max_y) = (
        ys.iter().min().unwrap() - PAD,
        ys.iter().max().unwrap() + PAD,
    );

    let mut p = String::new();
    // Breite UND Hoehe setzen, im Verhaeltnis der viewBox. Nur `width` zu setzen laesst
    // Vorschau-Renderer (qlmanage, Quick Look) auf ein Quadrat zurueckfallen und schneidet
    // den Plan ab — die viewBox allein regelt das nicht.
    let (vw, vh) = (max_x - min_x, max_y - min_y);
    let out_w = 1100;
    let out_h = (out_w as f64 * vh as f64 / vw as f64).round() as i32;
    p.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{min_x} {min_y} {vw} {vh}" width="{out_w}" height="{out_h}">"##));
    p.push_str(&format!(
        r##"<rect x="{min_x}" y="{min_y}" width="{}" height="{}" fill="#FBFAF8"/>"##,
        max_x - min_x,
        max_y - min_y
    ));

    // Terrasse zuerst, damit die Aussenwand darueber liegt. Gestrichelt und mit "ca.", weil sie
    // geschaetzt ist — eine Aussenflaeche, die aussieht wie eine gemessene, ist eine Falle.
    if let Some(t) = &room.terrasse {
        let (w, h) = (t.x[1] - t.x[0], t.y[1] - t.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="none" stroke="#9A968E" stroke-width="3" stroke-dasharray="18 12"/>"##,
            t.x[0], t.y[0]));
        let (cx, cy) = (t.x[0] + w / 2, t.y[0] + h / 2);
        p.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" transform="rotate(-90 {cx} {cy})" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">TERRASSE{}</text>"##,
            if t.geschaetzt { " (ca.)" } else { "" }));
    }

    let pts: Vec<String> = poly.iter().map(|q| format!("{},{}", q[0], q[1])).collect();
    p.push_str(&format!(
        r##"<polygon points="{}" fill="#F4F1EB" stroke="#16181A" stroke-width="5" stroke-linejoin="miter"/>"##,
        pts.join(" ")));

    if let Some(b) = &room.bad {
        let (w, h) = (b.x[1] - b.x[0], b.y[1] - b.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="#E2DDD2" stroke="#9A968E" stroke-width="3"/>"##,
            b.x[0], b.y[0]));
        p.push_str(&format!(
            r##"<text x="{}" y="{}" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">BAD</text>"##,
            b.x[0] + w / 2, b.y[0] + h / 2));
    }

    for f in &room.fix_moebel {
        let (w, h) = (f.x[1] - f.x[0], f.y[1] - f.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="none" stroke="#6E655A" stroke-width="3" stroke-dasharray="12 8"/>"##,
            f.x[0], f.y[0]));
        p.push_str(&format!(
            r##"<text x="{}" y="{}" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">KUECHE</text>"##,
            f.x[0] + w / 2, f.y[0] + h / 2 + 40));
    }

    // Oeffnungen: Glastuer voll, Fenster gestrichelt, Tuer grau — drei Signaturen, weil sie
    // drei verschiedene Dinge sind. Die TypeScript-Vorlage zeichnete Fenster wie Glastueren.
    for o in &room.oeffnungen {
        let Some((a, b)) = room.opening_span(o) else {
            continue;
        };
        let (stroke, dash) = match o.typ.as_deref() {
            Some("tuer") => ("#5B5F63", r##" stroke-dasharray="16 10""##),
            Some("fenster") => ("#B0764A", r##" stroke-dasharray="4 6""##),
            _ => ("#B0764A", ""),
        };
        p.push_str(&format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="9"{dash}/>"##,
            a[0], a[1], b[0], b[1]
        ));
    }

    for it in &layout.items {
        let (w, d, _) = footprint(it, &model.catalogue)?;
        let r = Rect {
            x: it.x,
            y: it.y,
            w,
            d,
        };
        let label = short(
            kind_of(it),
            model
                .catalogue
                .get(&it.reference)
                .map_or(&it.reference, |c| &c.label),
        );
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{d}" fill="#DAD2C4" stroke="#6E655A" stroke-width="3"/>"##,
            r.x, r.y));
        let (cx, cy) = (r.x + w / 2, r.y + d / 2);
        // Quer beschriften, wenn der Text sonst ueber die Kante laeuft.
        let turn = label.chars().count() as i32 * 11 > w && d > w;
        let rot = if turn {
            format!(r##" transform="rotate(-90 {cx} {cy})""##)
        } else {
            String::new()
        };
        p.push_str(&format!(
            r##"<text x="{cx}" y="{cy}"{rot} font-family="Helvetica,Arial" font-size="19" fill="#16181A" text-anchor="middle" dominant-baseline="central">{}</text>"##,
            esc(&label)));
    }

    p.push_str("</svg>");
    Ok(p)
}

/// Plan plus Verdikt in einer Datei — damit ein Bild nie ohne sein Urteil weitergereicht wird.
pub fn page(model: &Model, layouts: &[Layout]) -> Result<String, ModelError> {
    // Der Titel kommt aus dem Modell. Hier stand der Name dieser einen Wohnung im Quelltext.
    let mut h = String::from(
        r##"<!doctype html><meta charset="utf-8"><title>Wohnung — Plaene</title>
<style>
:root{color-scheme:light dark}
body{margin:0;background:#FBFAF8;color:#16181A;font:16px/1.6 ui-sans-serif,-apple-system,Helvetica,Arial;padding:40px}
@media(prefers-color-scheme:dark){body{background:#131416;color:#E9E7E3}}
h1{font-size:1.8rem;margin:0 0 4px}
h2{font-size:1.15rem;margin:0}
.wrap{max-width:1180px;margin:0 auto}
.card{margin:38px 0;padding:24px;border:1px solid #C9C6C0;border-radius:14px;background:#fff}
@media(prefers-color-scheme:dark){.card{background:#1A1C1F;border-color:#3C4045}}
svg{width:100%;height:auto;display:block;margin:14px 0}
.pass{color:#2E7D32;font-weight:600}.fail{color:#B4402F;font-weight:600}
.warn{color:#9A6A00}
ul{margin:8px 0;padding-left:20px}
.meta{font:13px ui-monospace,Menlo,monospace;color:#5B5F63}
</style><div class="wrap"><h1>{TITEL} — Pläne</h1>
<p class="meta">Aus dem gemessenen Modell erzeugt. Jede Zahl stammt aus <code>room.toml</code>,
<code>rules.toml</code> und dem Inventar; nichts auf dieser Seite ist von Hand gesetzt.</p>"##)
        .replace("{TITEL}", &esc(&model.room.flat.name));

    for l in layouts {
        let r = check_layout(model, l)?;
        h.push_str(&format!(
            r##"<div class="card"><h2>{}</h2><p class="{}">{}</p>"##,
            esc(&l.name),
            if r.pass { "pass" } else { "fail" },
            if r.pass {
                "bestanden — keine harte Regel verletzt"
            } else {
                "durchgefallen"
            }
        ));
        h.push_str(&svg(model, l)?);
        h.push_str(&format!(
            r##"<p class="meta">belegt {:.2} m² · frei {:.2} m² · {}</p>"##,
            r.metrics.occupied_area_m2,
            r.metrics.free_area_m2,
            r.metrics
                .corridors
                .iter()
                .map(|c| format!(
                    "{} → {}: {}",
                    c.from,
                    c.to,
                    c.width_cm.map_or("kein Weg".into(), |w| format!("{w} cm"))
                ))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
        if !r.hard.is_empty() {
            h.push_str("<ul>");
            for v in &r.hard {
                h.push_str(&format!(
                    r##"<li class="fail">{}: {}</li>"##,
                    v.rule,
                    esc(&v.message)
                ));
            }
            h.push_str("</ul>");
        }
        if !r.soft.is_empty() {
            h.push_str("<ul>");
            for v in &r.soft {
                h.push_str(&format!(
                    r##"<li class="warn">{}: {}</li>"##,
                    v.rule,
                    esc(&v.message)
                ));
            }
            h.push_str("</ul>");
        }
        if !r.uncertainties.is_empty() {
            h.push_str(&format!(
                r##"<p class="meta">gemessen? nein: {}</p>"##,
                r.uncertainties
                    .iter()
                    .map(|u| format!("{} ({})", esc(&u.label), u.fields.join(", ")))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        h.push_str("</div>");
    }
    h.push_str("</div>");
    Ok(h)
}
