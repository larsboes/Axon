//! Das Repository darf keine Wohnung kennen.
//!
//! Q59 (PRD §8.3) hat diese Capability in das oeffentliche Axon-Repository verschoben. Damit
//! das sicher ist, muss der generische Teil generisch SEIN und nicht nur so heissen. Dieser
//! Test ist die Bedingung dafuer.
//!
//! Er hat beim ersten Lauf am 2026-08-30 drei echte Lecks gefunden — `search.rs` hielt
//! [0, 420, 0, 590] als Standard-Suchband, also die Masse dieser Wohnung, `plan.rs` trug ihren
//! Namen im Seitentitel, und `main.rs` nannte ihn in einer Konstante und einem Doc-Kommentar.
//!
//! Er prueft seit dem Umzug `tests/` MIT, und das war kein Zierrat: die aufgezeichnete
//! Vorlage der TypeScript-Fassung lag als `tests/ts-baseline.json` genau ein Verzeichnis neben
//! dem Code, den dieser Test las, und enthielt Korridorbreiten und Moebelmasse der echten
//! Wohnung. Ein Containment-Test, der nur `src/` liest, haette sie mit veroeffentlicht. Sie
//! liegt jetzt im Overlay neben der Wohnung, die sie beschreibt.

use std::fs;
use std::path::{Path, PathBuf};

/// Masse dieser Wohnung, die in diesem Repository nichts zu suchen haben. Bewusst als Literale
/// gefuehrt: die Liste IST die Behauptung, und sie muss mitwachsen, wenn das Modell Zahlen
/// dazugewinnt.
const VERBOTENE_MASSE: &[&str] = &["420", "460", "590", "263", "196", "329"];
const VERBOTENE_NAMEN: &[&str] = &["Beuel", "beuel-sued", "Petersbergweg"];

/// Nur Text. Ein Verzeichnis, in dem eine Wohnung stehen koennte, wird durchsucht, nicht
/// ausgenommen — `tests/fixtures/` ist erfundene Geometrie und muss das beweisen koennen.
const ENDUNGEN: &[&str] = &["rs", "toml", "json", "md"];

/// `unter` ist ein Unterverzeichnis oder "." fuer das Wurzelverzeichnis der Capability.
/// Das Wurzelverzeichnis gehoert dazu: die README nannte bis zum Umzug das Verzeichnis der
/// echten Wohnung beim Namen, und ein Test, der nur Quelltext liest, haette das durchgelassen.
fn dateien(unter: &str) -> Vec<(String, String)> {
    let wurzel = Path::new(env!("CARGO_MANIFEST_DIR")).join(unter);
    let mut offen = vec![wurzel.clone()];
    let mut out = Vec::new();
    while let Some(dir) = offen.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let p: PathBuf = e.path();
            if p.is_dir() {
                // Bauausgabe und die beiden Unterbaeume, die eigene Laeufe haben.
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !matches!(name, "target" | "src" | "tests") {
                    offen.push(p);
                }
                continue;
            }
            let passt = p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| ENDUNGEN.contains(&x));
            if !passt {
                continue;
            }
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let name = p
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            out.push((name, text));
        }
    }
    out.sort();
    out
}

/// In Rust duerfen Kommentare die Zahlen nennen — dort erklaeren sie, warum etwas so ist, und
/// dieser Test selbst ist das Beispiel. In Daten gibt es diese Trennung nicht: eine TOML- oder
/// JSON-Datei, die eine Wandlaenge nennt, nennt sie, ob mit `#` davor oder ohne.
fn pruefbar(name: &str, src: &str) -> String {
    if !name.ends_with(".rs") {
        return src.to_string();
    }
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Als eigenstaendige Zahl, nicht als Teilfolge: 460 in 1460 ist keine Wandlaenge.
fn nennt_zahl(zeile: &str, mass: &str) -> bool {
    zeile.match_indices(mass).any(|(i, _)| {
        let davor = i == 0 || !zeile.as_bytes()[i - 1].is_ascii_digit();
        let j = i + mass.len();
        let danach = j >= zeile.len() || !zeile.as_bytes()[j].is_ascii_digit();
        davor && danach
    })
}

#[test]
fn keine_wohnungsdaten_im_repository() {
    let mut funde = Vec::new();
    let alle = dateien("src")
        .into_iter()
        .chain(dateien("tests"))
        .chain(dateien("."));
    for (name, src) in alle {
        // Eine Datei, deren Aufgabe das Erkennen von Markern ist, muss sie enthalten. Genau
        // diese Ausnahme, mit genau dieser Begruendung, fuehrt `tools/check-publication-
        // hygiene.sh` fuer sich selbst und seinen Test — hier ist es dieselbe Datei und
        // dieselbe einzige. Jede andere hat keinen Grund, eine dieser Zahlen zu tragen.
        if name.ends_with("containment.rs") {
            continue;
        }
        // Der Name der Wohnung ist auch in einem Kommentar ein Leck, sobald das Repo
        // oeffentlich ist — und dieses ist es.
        for verboten in VERBOTENE_NAMEN {
            if src.contains(verboten) {
                funde.push(format!("{name}: nennt \"{verboten}\""));
            }
        }
        let text = pruefbar(&name, &src);
        for (n, zeile) in text.lines().enumerate() {
            for mass in VERBOTENE_MASSE {
                if nennt_zahl(zeile, mass) {
                    funde.push(format!(
                        "{name}:{}: Literal {mass} — {}",
                        n + 1,
                        zeile.trim()
                    ));
                }
            }
        }
    }
    assert!(
        funde.is_empty(),
        "Wohnungsdaten im oeffentlichen Repository. Sie gehoeren in das Overlay, unter \
         data/interior/, nicht hierher:\n  {}",
        funde.join("\n  ")
    );
}

/// Der Test muss die Dateien auch wirklich sehen. Ein Verzeichniswanderer, der nichts findet,
/// besteht jede Behauptung.
#[test]
fn der_test_liest_alle_drei_baeume() {
    let src = dateien("src");
    let tests = dateien("tests");
    let wurzel = dateien(".");
    assert!(src.len() >= 8, "src/ hat {} Dateien", src.len());
    assert!(
        tests.iter().any(|(n, _)| n.contains("fixtures")),
        "die Musterwohnung wird nicht mitgelesen: {:?}",
        tests.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        wurzel.iter().any(|(n, _)| n == "README.md"),
        "die README wird nicht mitgelesen: {:?}",
        wurzel.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
}

/// Der Medienpfad kommt vom Client, also darf er nicht aus `media/` herausfuehren.
///
/// Geprueft wird die Aufloesung selbst und nicht der HTTP-Weg: `canonicalize` beidseitig, dann
/// `starts_with`. Eine Pruefung auf die Zeichenfolge `..` waere die naheliegende Fassung und die
/// falsche — sie versagt still, sobald ein Symlink aus dem Verzeichnis zeigt, und genau das
/// prueft der letzte Fall hier.
#[test]
fn ein_medienpfad_fuehrt_nicht_aus_dem_verzeichnis_heraus() {
    let tmp = std::env::temp_dir().join(format!("interior-media-{}", std::process::id()));
    let media = tmp.join("media");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(media.join("produkt")).expect("Testbaum");
    std::fs::write(media.join("produkt/bild.png"), b"x").expect("Bild");
    std::fs::write(tmp.join("geheim.txt"), b"x").expect("Datei daneben");

    // Dieselbe Aufloesung, die `api_media` fuehrt.
    let erlaubt = |pfad: &str| -> bool {
        let wurzel = match media.canonicalize() {
            Ok(w) => w,
            Err(_) => return false,
        };
        match wurzel.join(pfad).canonicalize() {
            Ok(ziel) => ziel.starts_with(&wurzel) && ziel.is_file(),
            Err(_) => false,
        }
    };

    assert!(
        erlaubt("produkt/bild.png"),
        "das Bild selbst wird geliefert"
    );
    assert!(!erlaubt("../geheim.txt"), "eine Ebene hoeher: nein");
    assert!(
        !erlaubt("produkt/../../geheim.txt"),
        "ueber einen Umweg: nein"
    );
    assert!(!erlaubt("produkt"), "ein Verzeichnis ist keine Datei");

    // Der Fall, den eine `..`-Pruefung durchliesse: ein Symlink ohne einen einzigen Punkt.
    #[cfg(unix)]
    {
        let link = media.join("raus.txt");
        std::os::unix::fs::symlink(tmp.join("geheim.txt"), &link).expect("Symlink");
        assert!(
            !erlaubt("raus.txt"),
            "ein Symlink aus dem Verzeichnis heraus wird nicht geliefert"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
