# Golden-Vektoren für den TV-Planer-Port

Erzeugt aus dem Original-JS-Planer des Web-Themes (`src/js/tv/schedule/*.js`, dazu `src/js/features/tvdata/config.js`),
damit `crates/lfx-tv-schedule` bitgleich gegen das Original geprüft werden kann (Gate G5, Teil 1).
Alle Daten sind synthetisch (erfundene Titel, erfundene Genres und Ids); es gibt keine echten Titel, Cover oder Server-Daten.

Quelle der Vektoren: Web-Repo, Commit siehe `crates/lfx-tv-schedule/tests/golden/meta.json` (`webCommit`).
Stand dieser Ausgabe: `acbdefe09f574e48a285b0435ad7a3185674761c`.

## Neu erzeugen

```sh
# Wurzel des Web-Repos; Standard: drei Ordner über tools/golden (also das Elternrepo dieses Ordners)
export LENZFLIX_WEB_DIR=/pfad/zum/web-repo        # optional
node tools/golden/gen.mjs                         # schreibt nach crates/lfx-tv-schedule/tests/golden/
node tools/golden/gen.mjs --out <ordner>          # alternativer Ausgabeordner (auch GOLDEN_OUT)
node tools/golden/selftest.mjs                    # Determinismus, Zeitzonen-Unabhängigkeit, Gleichheit mit dem eingecheckten Stand
```

Node 20 oder neuer (getestet mit 24). Keine Abhängigkeiten, keine Netzwerkzugriffe. Der Generator setzt `TZ=UTC`;
`selftest.mjs` prüft zusätzlich Läufe unter anderen Zeitzonen (die Vektoren dürfen nicht von der Lokalzeit abhängen).
Der Web-Commit steht in `meta.json`; zeigt `webSchedulerDirty` auf `true`, wurde mit uncommitteten Änderungen erzeugt.
Ändert sich der Planer im Web-Repo, die Vektoren neu erzeugen, den neuen Commit hier eintragen und den Rust-Port nachziehen.

## Dateien (`crates/lfx-tv-schedule/tests/golden/`)

| Datei | Inhalt |
|---|---|
| `meta.json` | Web-Commit, Node/V8-Version, Fenster (`w0`..`w3`), Dateiliste |
| `primitives.json` | `fnv1a`, `fmix32`, `bitrev32`, `mulberry32`, `keyHash`, `cmpStr`, Zeitfunktionen (`offsetMs`, `wallToUtc`, `dayBounds`, `broadcastDayOf`, `formatClock`, `formatDay`, `weekOf`, `weekStart`, `admitDay`, `dayOfDate`, `parseDate`, `ticksToMs`), `ratingAge`, `maxAge`, `normGenre`, `lowerMedian`, `slotOf`, `ageAllowed`, `packSubset`, `closedownDay`, `parseChannelConfigFmt` (tvdata/config.js) |
| `primitives-pool.json` | `streamOfAge`, `bucketCount`, `CAP_MS`, `makeEntries`, `bucket`, `rotation`, `rotationWithBorrow` |
| `builder.json` | `createDayBuilder`-Szenarien (Operationen mit Ergebnissen, `finish()`) und `fillSegment`-Szenarien |
| `catalog-<name>.raw.json` | Rohkatalog (`movies`, `series`, `episodes`, `boxsets`, `loadedAt`, Feldreihenfolge wie `KEEP` in `data.js`) und `boxsetContents` (Mock für `getBoxSetItems`) |
| `catalog-<name>.norm.json` | `normalizeCatalog`-Ergebnis (`movies`, `series`, `itemsOrder` = Einfügereihenfolge der `items`-Map mit Art, `boxsets`, `genres`, `genreLabel`-Proben, `genresApi`) |
| `plans-<name>-<variante>-<fenster>[-pN].json` | `channels(D)` und `planDay(no, D)` für alle 18 Sender je Tag im Fenster; `prepareCalls` = Reihenfolge der `getBoxSetItems`-Aufrufe in `prepare(days)` |
| `now-<name>.json` | `nowOn`/`upcoming` an Stichzeitpunkten, Zeitumstellungs-Stichpunkte (`dst`), Fehlerfälle von `planDay` (`errors`), bei `thin` lange Vorschau |

Kataloge: `thin` (dünn, wenige Genres → Sender ohne Genre, Testbild-Tage), `normal` (400 Filme, 40 Serien), `large` (2400 Filme,
120 Serien, ~4000 Folgen), `edge` (Randfälle: Unicode-Genres ß/İ/Σ/NFD, Zahl-Ids, Id-Kollision Film/Folge, doppelte Ids mit
Gleichstand, Folgen ohne Laufzeit, kaputte Datumswerte, Serie ohne planbare Folgen).
Varianten: `default`, `cfgstr` (Senderbelegung als String), `cfgobj` (als Objekt, nur `normal`), `noprepare` (kein `prepare`-Aufruf, nur `normal`).
Fenster: `w0` = 14 Sendetage ab 2026-11-01, `w1` = Herbst-Zeitumstellung 2026 (25-h-Tag), `w2` = Frühjahr 2027 (23-h-Tag), `w3` = Herbst 2027.

Ablauf je Plan-Datei: frischer `createScheduler(normalizeCatalog(raw), { channelConfig, getBoxSetItems })`, dann (außer `noprepare`)
`await prepare(days)` mit allen Tagen des Fensters, danach `channels(D)` und `planDay(no, D)`. `getBoxSetItems(id)` liefert
`boxsetContents[id]` (Liste), `{"$":"throw"}` heißt: wirft einen Fehler, `null` wird als `null` zurückgegeben.
`nowOn`/`upcoming` laufen auf dem Scheduler des Fensters `w0` (Variante `default`), `dst`/`errors` auf frischen Schedulern ohne `prepare`.
Der Generator prüft selbst, dass Normalisierung und Pläne unabhängig von Eingabe- und Aufrufreihenfolge sind.

## JSON-Konventionen

Zahlen sind exakt (`JSON.stringify`), Reihenfolgen sind exakt die des JS-Laufs. Nicht in JSON darstellbare Werte stehen als Marker-Objekt `{"$": …}`:
`{"$":"undefined"}`, `{"$":"NaN"}`, `{"$":"Infinity"}`, `{"$":"-Infinity"}`, `{"$":"-0"}`, `{"$":"u16","u":[Code-Units…]}` (Zeichenketten mit einzelnen Surrogaten).
Eigenschaften mit dem Wert `undefined` fehlen im Objekt (wie bei `JSON.stringify`). Beispiel: `premiere: Infinity` bei Folgen ohne Premierendatum.
`parseDateLegacy` in `primitives.json` dokumentiert V8-Sonderverhalten (Legacy-Parser, teils lokalzeitabhängig) und ist NICHT bindend für den Port.
