# ADR 0003: TV-Planer als bitgleicher Rust-Port

- Status: angenommen
- Datum: 2026-11-20
- Betroffene Crates: `lfx-tv-schedule`

## Kontext
Der TV-Modus des Web-Themes erzeugt den Sendeplan deterministisch; die Desktop-App muss auf allen Clients denselben Plan liefern (Gate G5). Der JS-Planer hat Fallen: UTF-16-Code-Units (fnv1a, cmpStr), `Math.imul`, `>>>`, `Math.round`, `Date.parse` wie V8, Einfügereihenfolge von Map/Set, stabile Sortierung, Gleichstand der Deduplizierung per `JSON.stringify` in der Reihenfolge der Felder.

## Entscheidung
`lfx-tv-schedule` portiert den Planer Funktion für Funktion mit `#![forbid(unsafe_code)]`, ohne Uhr, IO und Zufall. Abgesichert wird er durch Golden-Vektoren, die aus dem Web-Code erzeugt werden (Node-Skript `tools/golden/gen.mjs`); der Web-Stand ist per Commit in `tools/golden/web-commit.txt` festgelegt. Die Vektor-JSON (ca. 26 MB) ist gitignoriert und wird in der CI erzeugt.

## Alternativen
- Neuimplementierung nach Spezifikation statt Portierung: Abweichungen im Plan wären nicht auszuschließen.
- Eingecheckte Golden-Dateien: zu groß, Drift gegenüber dem Web-Code schwer erkennbar.

## Folgen
- Ergebnis: Alle 51 Vektor-Dateien bitgleich; `LFX_GOLDEN_REQUIRED=1 cargo test -p lfx-tv-schedule` = 18 Tests grün, keine Ausnahmen. 37.063 Vergleiche auf oberster Ebene, 153.523 verglichene Planeinträge. Mutationstest (Horizont 4 h -> 5 h) lässt 8 von 18 Tests fehlschlagen. Gate G5 erfüllt.
- CI: je Matrix-Job wird der Web-Commit ausgecheckt, `gen.mjs` erzeugt die Vektoren (setzt `TZ=UTC` selbst), danach läuft `cargo test` mit `LFX_GOLDEN_REQUIRED=1`. Ohne Vektor-Ordner überspringt der Test mit Meldung, mit `REQUIRED=1` schlägt er fehl. Der CI-Schritt selbst wurde noch nicht ausgeführt.
- `prepare` ist IO-frei als Zustandsmaschine: `next_boxset_request(days)` und `provide_boxset(id, ids)` in derselben Abfolge wie im JS; dazu ein Komfort-`prepare(days, loader)` mit synchronem Loader. Die asynchrone Hülle der App ruft die Zustandsmaschine selbst auf.
- Technische Entscheidungen: `serde_json` mit `float_roundtrip` (sonst 1 ULP Abweichung bei mulberry32); `JsVal`-Modell für `Number`/`String`/`JSON.stringify`; Planer- und tvdata-Parser der Senderbelegung getrennt wie im Original; Zeiten als i64 ms.
- Bewusste Abweichungen ohne Ergebnisfolge: `shorts` pro Tag eager statt lazy; `provide_boxset` leert den Memo-Cache (im JS veraltet ein vorher berechneter Plan für Sender 18).
- Grenzen: nur ISO-Formen von `Date.parse` nachgebaut; nicht von Vektoren abgedeckte Randfälle (extreme Zahlenformate, `addedAt` mit Nachkommaanteil) sind dokumentiert, nicht getestet.
- Risiko: Der Web-Commit muss öffentlich verfügbar sein; bei Änderungen am Web-Planer ist `web-commit.txt` bewusst nachzuziehen und die Parität erneut zu prüfen.
