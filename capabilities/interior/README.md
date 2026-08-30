# interior — Raumplanung

Misst Layouts gegen die Räumungsregeln einer Wohnung und sucht Positionen, die sie erfüllen.

```
interior model                     gemessener Raum, und was daran noch geraten ist
interior layouts                   Layouts auf der Platte
interior check <layout> [--json]   gegen rules.toml prüfen (Exit 1 bei hartem Verstoß)
interior search <layout> --move a,b [--step 20] [--limit 6] [--band id=x0,x1,y0,y1] [--out name]
interior plan [layout...] --out f  Pläne als HTML mit Verdikt
interior inventory                 was da ist, was fehlt, und was das Fehlende kostet
interior import                    inventory/*.toml in die Tabellen (wiederholbar)
interior serve                     HTTP-API, Port aus service.toml
```

Die Oberfläche ist `/interior` in der Axon-Shell (`dashboard/src/routes/interior/`). Sie
**rendert und rechnet nicht**: jedes Verdikt, jede Korridorbreite und der Plan als SVG kommen
fertig aus dieser Capability. Eine zweite Auslegung einer Räumungsregel im Frontend wäre genau
die Drift, gegen die das hier existiert.

## Diese Capability kennt keine Wohnung

Jede Zahl kommt zur Laufzeit aus dem privaten Overlay, aufgelöst über
`axon_config::overlay_data_dir("interior")`. Ohne `AXON_PERSONAL_ROOT` bricht sie ab statt zu
raten: eine Planung gegen erfundene Maße wäre schlimmer als gar keine.

Welche Wohnung gemeint ist, entscheidet `AXON_INTERIOR_FLAT` oder `--flat`; liegt genau eine
unter `flats/`, ist es die. Liegen mehrere und ist keine gewählt, ist das ein **Fehler und keine
Vorauswahl** — ein stiller Standard ist der Weg, auf dem ein Plan der falschen Wohnung als der
richtige durchgeht.

`tests/containment.rs` ist die Bedingung dafür, dass das hier stehen darf: es liest `src/`,
`tests/` und die Dateien im Wurzelverzeichnis und fällt um, sobald eine Wandlänge oder ein
Wohnungsname darin auftaucht. Beim ersten Lauf fand es drei Lecks (PRD §8.3, B24).

## Wo die Zahlen herkommen

Zwei Orte, und die Grenze ist **hat die Zahl eine Begründung, die mitwandern muss** (PRD Q60).

**Zeilen** — das Inventar, in der geteilten SQLite-Datei unter `interior_`:

| | |
|---|---|
| `interior_item` | jedes Stück und jeder Bedarf: Maße, Preis in Cent, Herkunft, was es an Platz verlangt |
| `interior_item_state` | `owned` / `wanted` / `gone` **über die Zeit**. Ein Wunsch, der gekauft wird, ist zwei Zeilen und nicht eine überschriebene |
| `interior_placement` | wo ein Stück in einer Wohnung wirklich steht — im Unterschied zu einem Layout, das ein Vorschlag ist |

**Dateien** — die Geometrie, unter `<overlay>/data/interior/flats/<id>/`:

| | |
|---|---|
| `room.toml` | Polygon, Wände, Öffnungen, feste Einbauten, `[[routen]]` |
| `rules.toml` | Laufwege, Abstände, Lichtkorridor, die Regeltexte |
| `layouts/*.toml` | die Varianten, über die argumentiert wird |

`room.toml` trägt datierte Korrekturkommentare, und drei davon sind das Protokoll eines Fehlers,
der einen falschen Plan erzeugt hat. Eine Tabelle hat dafür keine Spalte, und ein Maß ohne seine
Korrekturgeschichte ist, wie derselbe Fehler zweimal gemacht wird.

`inventory/*.toml` im Overlay ist seit B25 die **Migrationsquelle** und keine zweite Wahrheit:
`interior import` liest sie in die Tabellen, wiederholbar und ohne dabei Zustandsgeschichte zu
erfinden. Nichts hier korrigiert Maße — die sind mit einem Bandmaß auf Papier entstanden.

## Die Musterwohnung

`tests/fixtures/overlay/` ist ein vollständiges `data/interior/` mit **ausgedachter** Geometrie.
Sie ist der Grund, warum diese Capability öffentlich stehen und trotzdem geprüft sein kann:
`tests/engine.rs` läuft überall gegen sie, `tests/live_parity.rs` prüft gegen die echte Wohnung
und überspringt sich selbst ohne Overlay.

Ihre `rules.toml` führt bewusst andere Schwellen als jede reale — `bett_zugang_zweite_seite`
steht dort auf 40. Wer diese Zahl im Code sucht, findet sie nicht: jede Schwelle wird über ihren
Namen aus der Datei gelesen, damit eine Regel, die einen Wert erfindet statt ihn nachzuschlagen,
beim Lesen auffällt.

Es sind **zwei** Wohnungen, und die zweite existiert wegen einer einzigen Eigenschaft:
`muster-nordkueche` stellt die Küchenzeile an die Nordwand, wird also von Süden angelaufen. Bis
2026-08-30 rechnete der Prüfer diese Zone immer nach Norden — richtig für genau eine Wohnung,
falsch für diese, und nichts hätte es gemeldet. Nebenbei sind zwei Wohnungen der Fall, in dem
`default_flat()` sich weigern muss zu raten.

```bash
AXON_PERSONAL_ROOT=capabilities/interior/tests/fixtures/overlay \
AXON_INTERIOR_FLAT=muster target/debug/interior check a-frei
```

## Was die Maschine tut

Zwei Primitive tragen die Arbeit: eine **exakte** euklidische Distanztransformation, die jedem
freien Punkt seinen Abstand zum nächsten belegten gibt, und eine **Maximum-Bottleneck-Suche**,
die die engste Stelle des breitesten Weges liefert. Ein Korridor der Breite W hat auf seiner
Mittellinie die Freiheit W/2 — die verdoppelte Engstelle ist deshalb die Breite, die ein Mensch
tatsächlich bekommt. Genähert statt exakt verschiebt Korridorbreiten um mehrere Zentimeter, und
genau daran entscheidet sich hier Bestehen oder Verstoß.

Weich warnt, hart blockiert. Es gibt absichtlich keinen Weg, ein Bestehen zu melden, solange ein
harter Verstoß offen ist.

## Herkunft

Bis 2026-08-30 TypeScript im privaten Overlay — der einzige Ausreißer gegenüber core Axon, das
durchgehend ein Rust-Workspace ist. Die Portierung ist gegen ein aufgezeichnetes Protokoll der
alten Engine abgesichert: zehn Layouts, jedes Verdikt, jede Korridorbreite, jeder
Katalogeintrag. Das Protokoll beschreibt **eine** Wohnung und liegt deshalb im Overlay neben
ihr, nicht hier.

Zwei Dinge wurden dabei gemessen statt vermutet:

- `widestPath` suchte das Maximum bei jeder Entnahme mit einem linearen Durchlauf über alle
  ~10.000 Rasterzellen — O(N²) je Route. Der Kommentar daneben behauptete, das schlage einen
  Heap „in both clarity and speed". Mit Heap: **67,7 ms → 2,7 ms**. In Rust: **0,18 ms**.
- Die Suche prüft seitdem **erschöpfend statt bis zum ersten Treffer**: 3,5 Mio. Kandidaten in
  rund 100 s, parallel über rayon. Vorher lieferte sie die erste zulässige Lösung, jetzt die
  beste von allen.

Nach core Axon verschoben am 2026-08-30 durch PRD Q59. Der Grund für die Overlay-Lage vom
2026-08-20 — *„die Seite, die sie ausliefert, bettet Fotos einer Wohnung ein"* — ist an dem Tag
gestorben, an dem die Seite eine HTTP-API wurde, die Medien erst auf Anfrage ausliefert.

## Was die Suche nicht kann

`wandkontakt_cm` bevorzugt Möbel mit einer Wand im Rücken, weil der Räumungsprüfer nur Abstände
kennt und einen Esstisch mitten im Raum für genauso richtig hält wie einen an der Wand. Das Maß
ist grob und **bestraft einen Raumtrenner dafür, dass er seine Aufgabe erfüllt** — ein Regal
quer im Raum bekommt 0 cm. Die Rangfolge ist eine Hilfe, kein Urteil.

## Was die Wohnung erklären muss

Zwei Dinge standen bis 2026-08-30 als Literale im Prüfer und beschrieben genau eine Wohnung
(PRD B26a). Sie stehen jetzt in `room.toml`:

```toml
[[fix_moebel]]
id = "kuechenzeile"
# seite ist Geometrie, abstand ist der NAME einer Schwelle in rules.toml — nicht die Zahl.
anlaufzone = { seite = "nord", abstand = "vor_kuechenzeile" }

[[routen]]
von  = "eingangstuer"
nach = "kuechenzeile"
```

Wegpunkte sind Öffnungen außer Fenstern und feste Möbel mit `anlaufzone`. Ein Name, den es nicht
gibt, ist ein **Fehler** und keine Route, die still ausfällt — die alte Fassung übersprang sie,
und der Bericht sah danach aus wie einer über eine Wohnung mit weniger Wegen. Keine `[[routen]]`
heißt: kein Weg wird gemessen. Das ist eine Aussage, keine Voreinstellung.

Was noch nicht deklariert ist: die Regel-**Kennungen**. `R1`, `R2`, `R3`, `R4`, `R7` stehen als
Literale im Code und ihre Texte in `rules.toml`. Das ist eine geteilte Namenskonvention über alle
Wohnungen und nicht wohnungsspezifisch — aber es ist auch keine, die eine Wohnung ändern könnte.

## Was ein Stück von sich aus verlangt

Bis 2026-08-31 riet die Prüfung aus dem Namen: `bett*` war ein Bett, `schrank*` ein Schrank, und
mit der Vermutung kam jede Schwelle mit. Das ist einmal teuer danebengegangen — `^couch` fing
`couchtisch`, also wurde ein Couchtisch gegen die Regeln eines Sofas geprüft, gefunden erst, als
ein echter Esstisch dazukam.

Ein Stück kann jetzt selbst sagen, was es braucht (PRD Q61 / B26):

```toml
opens        = "sued"                     # welche Seite Türen/Schubladen braucht
open_clear   = 65                         # cm davor, am Stück gemessen, nicht geerbt
wall_ok      = false                      # diese Seite darf NICHT an eine Wand
expands      = { dir = "sued", to = 165 } # Gesamttiefe ausgeklappt, nicht der Zuwachs
access_sides = 1                          # ein Bett für eine Person braucht eine Seite
access_clear = 60
```

`opens` und `expands.dir` gelten in der Ausrichtung des Stücks und drehen mit `rot` mit. Damit
sieht die Prüfung etwas, das die Namensfassung nicht sehen konnte: ein Schrank mit den Türen zur
Wand ist kein gedrehter Schrank, sondern ein unbenutzbarer.

**Wer nichts erklärt, wird weiter am Namen gemessen.** Das ist Absicht und kein halber Umbau:
42 Zeilen an einem Tag umzustellen wäre ein Stichtag, an dem sich Verdikte ändern, ohne dass
jemand die Zahlen dahinter geprüft hat. Die Musterwohnung erklärt vier Stücke und zwei bewusst
nicht, damit beide Wege geprüft bleiben.

## Was der offene Bedarf kostet

`GET /api/wishlist` legt zwei Zahlen nebeneinander, die aus **derselben Datei** kommen und
keine über HTTP: die Summe der `wanted`-Zeilen, und den Median des Monatssaldos aus
`finance_transaction_projection` über die letzten abgeschlossenen Monate (PRD B29).

Kein Budget — niemand hat eine Obergrenze gesetzt, und `budget.rs` erfindet keine. Der laufende
Kalendermonat fällt heraus, weil ein halber Monat mit ganzen Fixkosten keine Messung ist. Ist
der Median nicht positiv, gibt es **keine** Monatszahl: aus einem Saldo, aus dem nichts übrig
bleibt, lässt sich nichts ansparen, und „−3,2 Monate" wäre eine Rechnung, die so tut, als
beantworte sie etwas.
