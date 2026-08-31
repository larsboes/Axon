//! Wo die Sonne im Laufe des Jahres auf den Boden faellt.
//!
//! R5 fragt bisher, ob der Schreibtisch mit dem Gesicht oder dem Ruecken zur Verglasung
//! steht — eine Aussage ueber die Achse, und keine ueber Licht. Sie kommt ohne Ort, ohne
//! Datum und ohne Uhrzeit aus, und deshalb kann sie nicht beantworten, was jemand am
//! Schreibtisch wirklich wissen will: *faellt mir im Maerz um neun die Sonne auf den
//! Bildschirm?*
//!
//! Diese Datei rechnet das. Zwei Schritte, beide gewoehnliche Geometrie:
//!
//! 1. **Wo steht die Sonne** — Azimut und Hoehe aus Datum, Uhrzeit und Ort, nach dem Verfahren
//!    der NOAA. Reine Trigonometrie, keine Tabelle, keine Naeherung ueber Mittelwerte.
//! 2. **Wohin faellt ihr Licht** — die Verglasung ist eine senkrechte Flaeche zwischen zwei
//!    Hoehen; das Licht durch sie zeichnet auf dem Boden ein Parallelogramm, dessen Versatz
//!    `hoehe / tan(sonnenhoehe)` betraegt. Steht ein Moebel darin, liegt es in der Sonne.
//!
//! ## Was die Wohnung dafuer sagen muss, und was passiert, wenn sie schweigt
//!
//! ```toml
//! [lage]
//! breite = 50.7            # Grad Nord
//! laenge = 7.1             # Grad Ost
//! utc_offset_h = 2         # die Zeitzone MIT Sommerzeit, wenn die Uhrzeiten Sommerzeit sind
//! nordrichtung_grad = 0    # welche Kompassrichtung im Plan nach oben zeigt
//!
//! [[oeffnungen]]
//! id = "terrassentuer"
//! glas_von_cm = 0          # Unterkante ueber dem Boden
//! glas_bis_cm = 210        # Oberkante
//! ```
//!
//! Fehlt `[lage]`, wird **nicht gerechnet**. Ein erfundener Standort ergaebe einen Schattenwurf
//! auf den Zentimeter genau, der auf nichts beruht — das waere schlimmer als keine Antwort.
//! Dasselbe gilt fuer eine Verglasung ohne Hoehen: die Ausdehnung des Lichtflecks haengt
//! linear an ihnen. Beides meldet der Bericht als Luecke, wie es `CheckResult::nicht_geprueft`
//! seit PRD B31 fuer jede nicht messbare Regel tut.
//!
//! ## Wo dieses Modell zu grob ist, und in welche Richtung
//!
//! Der Lichtfleck ist der **ungehinderte** Wurf: was zwischen Fenster und Boden steht, wirft
//! keinen Schatten. Damit meldet die Rechnung eher zu viel Sonne als zu wenig — die
//! vorsichtige Richtung fuer eine Blendungsfrage. Nachbarhaeuser, Baeume und Balkonplatten
//! kennt sie nicht; ein Nordbalkon ueber der Terrassentuer wuerde jede Zahl hier senken.

use crate::model::{Model, ModelError, Opening, Pt, Rect};
use serde::Serialize;

/// Wo die Sonne steht.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Sonnenstand {
    /// Grad ueber dem Horizont. Negativ heisst: unter dem Horizont.
    pub hoehe_grad: f64,
    /// Kompassgrad, von Nord im Uhrzeigersinn.
    pub azimut_grad: f64,
}

/// Sonnenstand nach dem Verfahren der NOAA (General Solar Position Calculations).
///
/// `tag_im_jahr` und `stunde_utc` statt eines Datumstyps: diese Capability haengt an keiner
/// Kalenderbibliothek, und die Frage hier ist eine astronomische, keine kalendarische.
/// `jahr` geht nur ueber die Julianische Tageszahl ein.
pub fn sonnenstand(
    jahr: i32,
    tag_im_jahr: u32,
    stunde_utc: f64,
    breite: f64,
    laenge: f64,
) -> Sonnenstand {
    // Julianische Tageszahl fuer den Jahresanfang, plus der Tag. Genauigkeit im Bereich von
    // Sekunden reicht: gefragt ist, ob die Sonne auf einen Schreibtisch faellt.
    let a = ((14 - 1) / 12) as f64;
    let y = jahr as f64 + 4800.0 - a;
    let m = 1.0 + 12.0 * a - 3.0;
    let jd_jahresanfang = 1.0 + (153.0 * m + 2.0).div_euclid(5.0) + 365.0 * y + (y / 4.0).floor()
        - (y / 100.0).floor()
        + (y / 400.0).floor()
        - 32045.0;
    let jd = jd_jahresanfang + (tag_im_jahr as f64 - 1.0) + (stunde_utc - 12.0) / 24.0;

    let t = (jd - 2451545.0) / 36525.0;
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m_anom = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let m_rad = m_anom.to_radians();
    let c = (1.914602 - t * (0.004817 + 0.000014 * t)) * m_rad.sin()
        + (0.019993 - 0.000101 * t) * (2.0 * m_rad).sin()
        + 0.000289 * (3.0 * m_rad).sin();
    let wahre_laenge = l0 + c;
    let omega = 125.04 - 1934.136 * t;
    let lambda = wahre_laenge - 0.00569 - 0.00478 * omega.to_radians().sin();
    let eps0 = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let eps = eps0 + 0.00256 * omega.to_radians().cos();

    let deklination = (eps.to_radians().sin() * lambda.to_radians().sin())
        .asin()
        .to_degrees();

    // Zeitgleichung: der Unterschied zwischen wahrer und mittlerer Sonnenzeit, in Minuten.
    let var_y = (eps.to_radians() / 2.0).tan().powi(2);
    let zeitgleichung = 4.0
        * (var_y * (2.0 * l0.to_radians()).sin() - 2.0 * e * m_rad.sin()
            + 4.0 * e * var_y * m_rad.sin() * (2.0 * l0.to_radians()).cos()
            - 0.5 * var_y * var_y * (4.0 * l0.to_radians()).sin()
            - 1.25 * e * e * (2.0 * m_rad).sin())
        .to_degrees();

    let wahre_sonnenzeit = (stunde_utc * 60.0 + zeitgleichung + 4.0 * laenge).rem_euclid(1440.0);
    let stundenwinkel = if wahre_sonnenzeit / 4.0 < 0.0 {
        wahre_sonnenzeit / 4.0 + 180.0
    } else {
        wahre_sonnenzeit / 4.0 - 180.0
    };

    let (b, d, h) = (
        breite.to_radians(),
        deklination.to_radians(),
        stundenwinkel.to_radians(),
    );
    let zenit = (b.sin() * d.sin() + b.cos() * d.cos() * h.cos())
        .clamp(-1.0, 1.0)
        .acos();
    let hoehe = 90.0 - zenit.to_degrees();

    // Azimut aus dem Zenitwinkel. Der Nenner wird null, wenn die Sonne im Zenit steht; dann
    // ist der Azimut nicht definiert und jede Antwort gleich richtig.
    let nenner = b.cos() * zenit.sin();
    let azimut = if nenner.abs() < 1e-9 {
        180.0
    } else {
        let cos_az = ((b.sin() * zenit.cos()) - d.sin()) / nenner;
        let az = cos_az.clamp(-1.0, 1.0).acos().to_degrees();
        if stundenwinkel > 0.0 {
            (az + 180.0).rem_euclid(360.0)
        } else {
            (540.0 - az).rem_euclid(360.0)
        }
    };

    Sonnenstand {
        hoehe_grad: hoehe,
        azimut_grad: azimut,
    }
}

/// Der Lichtfleck einer Verglasung auf dem Boden: ein Parallelogramm in Raumkoordinaten.
#[derive(Debug, Clone, Serialize)]
pub struct Lichtfleck {
    pub oeffnung: String,
    /// Vier Ecken, gegen den Uhrzeigersinn oder mit — fuer den Ueberschneidungstest egal.
    pub ecken: [[f64; 2]; 4],
}

/// Die Richtung, in die das Licht laeuft, in Plankoordinaten.
///
/// `azimut` ist ein Kompasswinkel; der Plan haengt ueber `nordrichtung_grad` daran. Das Licht
/// laeuft von der Sonne WEG, also in die Gegenrichtung ihres Azimuts.
fn lichtrichtung(azimut_grad: f64, nordrichtung_grad: f64) -> [f64; 2] {
    let b = (azimut_grad + 180.0 - nordrichtung_grad).to_radians();
    // Plan: +x nach Osten, +y nach Sueden. Norden ist also -y.
    [b.sin(), -b.cos()]
}

/// Wohin das Licht durch diese Oeffnung faellt — oder `None`, wenn es das nicht tut.
///
/// Drei Gruende fuer `None`, und keiner davon ist ein Fehler: die Sonne steht unter dem
/// Horizont, sie steht hinter der Wand, oder die Oeffnung sagt ihre Glashoehen nicht.
pub fn lichtfleck(
    model: &Model,
    o: &Opening,
    stand: Sonnenstand,
    nordrichtung_grad: f64,
) -> Option<Lichtfleck> {
    let (von_cm, bis_cm) = (o.glas_von_cm?, o.glas_bis_cm?);
    if stand.hoehe_grad <= 0.5 {
        // Unter einem halben Grad ist der Versatz groesser als jede Wohnung, und die Rechnung
        // liefe gegen unendlich statt gegen eine Aussage.
        return None;
    }
    let seg = crate::clearance::opening_segment(&model.room, o)?;
    let d = lichtrichtung(stand.azimut_grad, nordrichtung_grad);
    // Scheint sie ueberhaupt auf diese Fassade? Das Licht muss nach innen laufen.
    if d[0] * seg.normal[0] + d[1] * seg.normal[1] <= 0.0 {
        return None;
    }
    let tan_h = stand.hoehe_grad.to_radians().tan();
    let versatz = |z: f64| z / tan_h;
    let (nah, fern) = (versatz(bis_cm as f64), versatz(von_cm as f64));
    // Die OBERkante wirft am weitesten? Nein — sie steht am hoechsten, also faellt ihr Strahl
    // am weitesten in den Raum. Die Unterkante zeichnet den nahen Rand.
    let punkt = |p: Pt, w: f64| [p[0] as f64 + d[0] * w, p[1] as f64 + d[1] * w];
    Some(Lichtfleck {
        oeffnung: o.id.clone(),
        ecken: [
            punkt(seg.a, fern),
            punkt(seg.b, fern),
            punkt(seg.b, nah),
            punkt(seg.a, nah),
        ],
    })
}

/// Ueberschneiden sich ein Rechteck und ein Parallelogramm?
///
/// Trennachsensatz ueber die Kantennormalen beider Formen. Beide sind konvex, also ist eine
/// trennende Achse gleichbedeutend mit „beruehren sich nicht" — kein Naeherungsverfahren,
/// sondern ein Beweis in beide Richtungen.
pub fn beruehren(r: &Rect, l: &Lichtfleck) -> bool {
    let rechteck = [
        [r.x as f64, r.y as f64],
        [r.right() as f64, r.y as f64],
        [r.right() as f64, r.bottom() as f64],
        [r.x as f64, r.bottom() as f64],
    ];
    let achsen = |p: &[[f64; 2]]| -> Vec<[f64; 2]> {
        (0..p.len())
            .map(|i| {
                let q = p[(i + 1) % p.len()];
                [-(q[1] - p[i][1]), q[0] - p[i][0]]
            })
            .collect()
    };
    for achse in achsen(&rechteck).into_iter().chain(achsen(&l.ecken)) {
        let laenge = (achse[0] * achse[0] + achse[1] * achse[1]).sqrt();
        if laenge < 1e-9 {
            continue;
        }
        let proj = |p: &[[f64; 2]]| {
            p.iter().fold((f64::MAX, f64::MIN), |(lo, hi), q| {
                let v = (q[0] * achse[0] + q[1] * achse[1]) / laenge;
                (lo.min(v), hi.max(v))
            })
        };
        let (a0, a1) = proj(&rechteck);
        let (b0, b1) = proj(&l.ecken);
        if a1 < b0 || b1 < a0 {
            return false;
        }
    }
    true
}

/// Ein geprueftes Datum mit seiner Uhrzeit.
#[derive(Debug, Clone, Serialize)]
pub struct Sonnenstunde {
    pub tag: &'static str,
    pub stunde_lokal: u32,
    pub hoehe_grad: f64,
    pub azimut_grad: f64,
    /// Welche Stuecke in diesem Moment im direkten Licht liegen.
    pub getroffen: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sonnenbericht {
    pub layout: String,
    pub stunden: Vec<Sonnenstunde>,
    /// Je Stueck: in wie vielen der geprueften Stunden es im direkten Licht liegt.
    pub treffer_je_stueck: std::collections::BTreeMap<String, usize>,
    /// Verglasungen, deren Hoehen fehlen — sie werfen hier kein Licht und muessten es.
    pub ohne_glashoehen: Vec<String>,
}

/// Die vier Tage, an denen sich das Jahr entscheidet.
///
/// Wintersonnenwende und Sommersonnenwende sind die Extreme, die Tagundnachtgleichen der
/// Mittelwert. Wer diese vier kennt, kennt die Spanne — jeder weitere Tag liegt dazwischen.
/// Die Tagesnummern sind fuer ein Gemeinjahr und koennen im Schaltjahr um einen Tag danebe
/// liegen; auf den Sonnenstand wirkt sich das um Bruchteile eines Grades aus.
pub const TAGE: &[(&str, u32)] = &[
    ("21. Maerz", 80),
    ("21. Juni", 172),
    ("23. September", 266),
    ("21. Dezember", 355),
];

/// Von wann bis wann geprueft wird, in lokaler Uhrzeit.
pub const STUNDEN: std::ops::RangeInclusive<u32> = 8..=18;

/// Das laufende Kalenderjahr, aus der Uhr statt aus dem Quelltext.
///
/// Bis 2026-08-31 stand hier die Zahl 2026. Ein Bericht, der jedes Jahr denselben Sonnenstand
/// rechnet, laeuft mit dem Kalender auseinander — und zwar lautlos, weil seine Zahlen weiter
/// auf die Kommastelle genau aussehen.
///
/// Einmal ermittelt und behalten: `stunden_in_der_sonne` haengt in der Suchschleife ueber
/// Millionen Aufstellungen, und ein Systemaufruf je Stunde je Kandidat waere dort teuer. Ein
/// Prozess, der ueber den Jahreswechsel laeuft, rechnet also mit dem Jahr seines Starts weiter
/// — der Unterschied liegt bei Bruchteilen eines Grades, derselbe Bereich, in dem `TAGE` das
/// Schaltjahr ignoriert.
fn jahr_jetzt() -> i32 {
    static JAHR: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *JAHR.get_or_init(|| {
        let sekunden = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        jahr_von_tagen(sekunden.div_euclid(86_400))
    })
}

/// Tage seit 1970-01-01 zum Kalenderjahr, nach Howard Hinnants `civil_from_days`.
///
/// Von Hand gerechnet und nicht aus einer Kalenderbibliothek: fuer eine einzige Jahreszahl
/// waere das eine Abhaengigkeit zu viel, und `capabilities/transit/src/main.rs` traegt
/// dieselbe Rechnung aus demselben Grund.
fn jahr_von_tagen(tage: i64) -> i32 {
    let z = tage + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    // Hinnants Jahr beginnt im Maerz, damit der Schalttag ans Ende faellt. Die beiden Monate
    // davor gehoeren deshalb schon ins naechste Kalenderjahr.
    let maerzjahr = yoe + era * 400;
    (maerzjahr + i64::from((5 * doy + 2) / 153 >= 10)) as i32
}

/// In wie vielen der geprueften Stunden liegt dieses Rechteck im direkten Licht?
///
/// Die schmale Fassung fuer `clearance.rs`: eine Regel braucht eine Zahl und keinen Bericht,
/// und sie wird in einer Schleife aufgerufen, die Millionen Aufstellungen prueft.
pub fn stunden_in_der_sonne(model: &Model, r: &Rect) -> Result<usize, ModelError> {
    let Some(lage) = model.room.lage else {
        return Err(ModelError::Missing(
            "room.toml fuehrt kein [lage] — ohne Ort gibt es keinen Sonnenstand".into(),
        ));
    };
    let mut n = 0;
    for (_, tag) in TAGE {
        for h in STUNDEN {
            let stand = sonnenstand(
                jahr_jetzt(),
                *tag,
                h as f64 - lage.utc_offset_h,
                lage.breite,
                lage.laenge,
            );
            let getroffen = model
                .room
                .oeffnungen
                .iter()
                .filter(|o| {
                    o.typ
                        .as_deref()
                        .is_some_and(|t| t.starts_with("glastuer") || t.starts_with("fenster"))
                })
                .filter_map(|o| lichtfleck(model, o, stand, lage.nordrichtung_grad))
                .any(|f| beruehren(r, &f));
            if getroffen {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Wie viele Stunden dieser Bericht ueberhaupt prueft. Der Nenner zu jeder Trefferzahl.
pub fn gepruefte_stunden() -> usize {
    TAGE.len() * STUNDEN.count()
}

/// Wann im Jahr welches Moebel in der Sonne steht.
pub fn bericht(model: &Model, layout: &crate::model::Layout) -> Result<Sonnenbericht, ModelError> {
    let lage = model.room.lage.ok_or_else(|| {
        ModelError::Missing(
            "room.toml fuehrt kein [lage] — ohne Breite, Laenge und Nordrichtung ist ein \
             Sonnenstand eine erfundene Zahl"
                .into(),
        )
    })?;

    let verglasungen: Vec<&Opening> = model
        .room
        .oeffnungen
        .iter()
        .filter(|o| {
            o.typ
                .as_deref()
                .is_some_and(|t| t.starts_with("glastuer") || t.starts_with("fenster"))
        })
        .collect();
    let ohne_glashoehen: Vec<String> = verglasungen
        .iter()
        .filter(|o| o.glas_von_cm.is_none() || o.glas_bis_cm.is_none())
        .map(|o| o.id.clone())
        .collect();

    let mut stunden = Vec::new();
    let mut treffer_je_stueck: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for (name, tag) in TAGE {
        for h in STUNDEN {
            let utc = h as f64 - lage.utc_offset_h;
            let stand = sonnenstand(jahr_jetzt(), *tag, utc, lage.breite, lage.laenge);
            let flecken: Vec<Lichtfleck> = verglasungen
                .iter()
                .filter_map(|o| lichtfleck(model, o, stand, lage.nordrichtung_grad))
                .collect();
            let mut getroffen = Vec::new();
            for it in &layout.items {
                let (w, d, _) = crate::model::footprint(it, &model.catalogue)?;
                let r = Rect {
                    x: it.x,
                    y: it.y,
                    w,
                    d,
                };
                if flecken.iter().any(|f| beruehren(&r, f)) {
                    getroffen.push(it.reference.clone());
                    *treffer_je_stueck.entry(it.reference.clone()).or_insert(0) += 1;
                }
            }
            stunden.push(Sonnenstunde {
                tag: name,
                stunde_lokal: h,
                hoehe_grad: stand.hoehe_grad,
                azimut_grad: stand.azimut_grad,
                getroffen,
            });
        }
    }

    Ok(Sonnenbericht {
        layout: layout.name.clone(),
        stunden,
        treffer_je_stueck,
        ohne_glashoehen,
    })
}

#[cfg(test)]
mod jahr_tests {
    use super::{jahr_jetzt, jahr_von_tagen};

    /// Feste Tageszahlen statt der Uhr: ein Test, der `jahr_jetzt()` gegen ein Literal haelt,
    /// waere genau der Fehler, den diese Aenderung behebt.
    #[test]
    fn die_jahresgrenze_liegt_am_richtigen_tag() {
        assert_eq!(jahr_von_tagen(0), 1970, "1970-01-01 ist Tag null");
        // 56 Jahre a 365 Tage plus 14 Schalttage (2000 zaehlt mit, es ist durch 400 teilbar).
        assert_eq!(jahr_von_tagen(20_454), 2026, "2026-01-01");
        assert_eq!(jahr_von_tagen(20_453), 2025, "der Tag davor");
        assert_eq!(jahr_von_tagen(-1), 1969, "und rueckwaerts kippt es auch");
    }

    /// Die Uhr liefert ueberhaupt ein Jahr aus diesem Jahrhundert.
    #[test]
    fn die_uhr_liefert_ein_plausibles_jahr() {
        let j = jahr_jetzt();
        assert!((2020..2100).contains(&j), "{j}");
    }
}
