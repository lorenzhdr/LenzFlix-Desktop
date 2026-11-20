# Paritätsliste Web (LenzFlix) → App (LENZFLIX Desktop)

Pflegedatei. Jede Zeile ist eine Funktion, die die App erreichen muss, bevor es ein Release (1.0.0) gibt. Status wird nur hier geändert (Hotspot-Datei, nur der Orchestrator pflegt sie).

- Pfade der Web-Quelle sind relativ zu `src/js/` des Web-Repos (CSS: `src/css/`, Doku: `docs/`). Die Vorlage wird nur gelesen, nie verändert.
- Designs: `flix` (LENZFLIX), `plus` (LENZ+), `prime` (LENZPRIME), `retro` (LENZ//84), `store` (VIDEOTHEK LENZ), `alle` = flix/plus/prime/retro.
- Prüfebenen: **Unit** (reine Logik, Golden-Vektoren, Offline-Render), **Mock** (Crate-Integrationstest gegen `lfx-mock-jellyfin`, kein Netz), **E2E** (echte App mit Fenster und Dev-Automation gegen den Mock, `--ignored`, ein Thread; Spalte nennt die Testdatei), **Snapshot** (Slint-Software-Renderer mit synthetischen Fixtures, bzw. UI-Schnappschuss-Screenshots, vom Worker angesehen), **Live** (echter Jellyfin-Server, nur vom Orchestrator/Nutzer; bisher gegen keinen echten Server gelaufen), **unprüfbar** (hier nicht prüfbar, ehrlich markiert).
- Status (Stand 20.11.2026, nach Integration W1–W4, W5 läuft): `umgesetzt (Mock/E2E)`, `umgesetzt (nur Unit)`, `umgesetzt (Snapshot)`, `teilweise (fehlt: …)`, `offen`, `unprüfbar (Grund)`, `in Arbeit (W5)` (Bereiche Downloads/Offline, Fernsteuerung/SyncPlay, Statistik, Tray, Feinschliff laufen parallel; Zeilen tragen dort zusätzlich den Stand vor W5). `umgesetzt` heißt nie „geprüft gegen den echten Server“: keine Zeile ist per Live bestätigt.
- Bewusst nicht übernommen: Browser-Eigenheiten (Proxy-Klicks, DOM-Karte, MUI-Selektoren, jsDelivr-Nachladen, `ApiClient`/`localStorage`-Zugangsdaten, Custom-CSS-Einbindung). Die App ruft die Jellyfin-REST-API direkt.
- Nicht prüfbar hier (gilt für alle Bereiche): macOS-Laufzeit (nur CI-Build, kein Mac), Transkodierung (Testkonto ohne), Live-TV ohne Tuner, HDR-Darstellung ohne Nutzer-Freigabe am Dev-Rechner.
- Nur Deutsch (UI). Keine Übernahme von Web-DisplayPreferences-Schreibzugriffen: die App schreibt nie in die Web-Einstellungen (eigener Speicher).

## 1. Kern, Design-System, Navigation

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Design-Tokens (Farben, Abstände, Radien, Schatten) | `css/tokens.css` | alle | `lfx-ui` (Theme-Global) | Snapshot (22 Referenzen, Windows ARM64) | umgesetzt (Snapshot) | Per `xtask` aus der Vorlage extrahiert, nur `--lfx-*`-Tokens. Windows x64 nicht geprüft; macOS-CI prüft nur „nicht leer“. |
| Schriften (Inter, Source Sans 3, Figtree, VT323, Space Mono) | `assets/fonts`, Lizenztexte | alle | `lfx-ui` (eingebettet) | Snapshot | umgesetzt (Snapshot) | Lizenztexte mitliefern; Retro-Schriften nur retro. Inter 400 wirkt im Software-Rendering leicht; Gewichte nicht gegen den Browser verglichen. |
| Design-Wechsel flix/plus/prime/retro zur Laufzeit | `core/theme.js`, `core/themecss.js` | alle | `lfx-ui` + `lfx-shell` | E2E (`w3_picker`) | umgesetzt (Mock/E2E) | Eigene Ablage in App-Einstellungen, nie Web-DisplayPreferences |
| Design-Picker mit Vorschau | `core/themepicker.js`, `core/popover.js` | alle | `lfx-ui` | E2E (`w3_picker`) | umgesetzt (Mock/E2E) | Online-Pille (`core/online.js`) nur mit konfiguriertem Zusatzdienst. Online-Pille bewusst nicht umgesetzt; Vorschau aus Rechtecken statt Web-CSS; Tagesmischung per `dayseed`. |
| Slot für VIDEOTHEK LENZ (store) | `features/store/*` | store | `lfx-3d` (Slot) | E2E (`w3_picker`) | umgesetzt (Mock/E2E) | Nur Slot vorbereitet, kein 3D, kein VR, kein Mehrspieler. Abweichung: kein Crate `lfx-3d`, der ausgegraute Platz „bald“ lebt im Picker (`lfx-ui`). |
| Router, Seitenzustände, Zurück | `core/router.js`, `shared/routes.js` | alle | `lfx-nav` | Mock/E2E | umgesetzt (Mock/E2E) | Routen-Enum ersetzt Hash-Router; zweiphasig (erst Unmount, dann Mount). Abweichung: `Route`/`Router` liegen in `lfx-app` (`route.rs`), `lfx-nav` enthält nur Fokus/Eingabe. |
| Streaming-Kopfleiste (Logo, Bibliotheken, Suche, Profil) | `features/nav.js`, `css/js-ui/header.css` | flix | `lfx-shell` | Mock/E2E, Snapshot | teilweise (fehlt: Kopfleiste auf der TV-Seite; auf der Detailseite nur per Maus, nicht per Tastatur) | Eigene Titelleiste, Name „LENZFLIX“. Titelleiste bleibt getrennt von der Kopfleiste; keine Glocke (Web hat keine). |
| Handy-/Schmal-Navigation | `features/mobilenav.js` | flix | `lfx-shell` | – | offen | Schmales Fenster statt Handy; `isNarrow`-Schwelle prüfen. Nicht umgesetzt; Bibliotheksseiten unter 900 px unaufgeräumt (Werkzeuge können überlaufen). |
| Kopfleiste, Tabbar, Profilmenü | `features/plus/header*.js`, `features/prime/header*.js`, `features/retro/header*.js`, `*/tabbar.js`, `shared/profile.js` | plus, prime, retro | `lfx-shell` | Mock/E2E (`w3_plus`, `prime_e2e`, `e2e_retro`) | teilweise (fehlt: prime Reihenfolge Filme vor Serien und Bibliotheken-Flyout; plus Tab-Reihenfolge des Profils; prime-Glas ohne Unschärfe) | Abmelden/Benutzerwechsel nativ (Abschnitt 8) |
| Tab-Titel, Favicon, theme-color | `core/doctitle.js`, `core/favicon.js`, `core/themecolor.js`, `shared/pagetitle.js` | alle | `lfx-shell` (Fenstertitel, Icon) | Unit/Snapshot | teilweise (Titelleiste mit Titel vorhanden; Fenstertitel „Seite · Design“ und App-Icon je Design nicht belegt) | Fenstertitel „Seite · Design“; Tab-Icon je Design als App-Icon-Variante offen. ICO/ICNS aus `xtask` (ein Icon). |
| Datensparmodus mit Teilschaltern und Bildgrößen | `core/datasaver.js`, `core/imgscale.js`, `shared/dsswitch.js`, `features/dsbtn.js` | alle | `lfx-core` + `lfx-ui` | Unit, E2E (`home_e2e`) | teilweise (fehlt: Anbindung in Bibliotheks-/Detailseiten und `tvwarm`; Bildgrößen dort fest 2.0/1.0) | Teile trailer/hero/images/prefetch/lazy/tvwarm; VIDEOTHEK ausgenommen. Startseite: Panel, 6 Teile, Speicher je Benutzer, Wirkung auf hero/trailer/images/prefetch/lazy. |
| Geheimmenü (5 Klicks auf Logo in 3 s) | `features/secretmenu.js`, `secretmenu/panel.js` | flix | `lfx-shell` | E2E (`home_e2e`) | umgesetzt (Mock/E2E) | Bewusst nicht in der README. Abweichung: Abschnitt „TV (Test)“ nicht übernommen, „Cache leeren“ nur Hinweis-Toast. |
| Ambient-Farbe aus dem Cover | `core/ambient.js`, `core/color.js` | flix | `lfx-ui` | Unit (JS-Vektoren), Screenshot | umgesetzt (nur Unit) | Farbermittlung aus dem Bild nativ. Farbmathe gegen Vektoren aus `color.js`; Darstellung (600 ms Überblendung) nur per Screenshot gesehen. |
| Ladebild beim Start der Wiedergabe | `core/startcover.js` | alle | `lfx-player` | E2E (`e2e_player`) | umgesetzt (Mock/E2E) | Entfällt als DOM-Beobachter, wird Player-Zustand. Startbild (Backdrop, Titel, Spinner) bei verzögertem Server. |
| Schwarzer Player-Hintergrund | `features/playerbg.js` | alle | `lfx-player` | E2E | umgesetzt (Mock/E2E) | Selbstverständlich im nativen Player |
| Toast, Sheet, Kontextmenü | `core/toast.js`, `core/sheet.js`, `features/contextmenu.js`, `core/itemmenu.js` | alle | `lfx-ui` | E2E (`e2e_detail`) | umgesetzt (Mock/E2E) | Kein Proxy-Klick; eigene Menüs mit Rechteprüfung. Abweichung: Langdruck (Touch) fehlt, kein „Teilen“. |
| Tastatur-/Fokussteuerung | Tests `modal-focus-keyboard`, `nav-taborder` | alle | `lfx-nav` | Unit (37 Tests `lfx-nav`), E2E | teilweise (fehlt: D-Pad als Achse, rechter Stick; Gamepad-Hardware ungeprüft; Medien-Leiste auf Fremdseiten nur per Maus) | Gemeinsames Fokussystem für Maus, Tastatur, Gamepad (Couch, Abschnitt 17). Fremdseiten-Fokus liegt je Seite in eigenen Graphen. |

## 2. Startseite (LENZFLIX)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Startseiten-Hülle, Aufbau und Cache | `features/home.js`, `core/homecache.js` | flix | `lfx-store` + `lfx-ui` | Unit (TtlCache-Regeln), Mock/E2E | teilweise (fehlt: Zeilen-Cache/TTL und SQLite-Seitencache in der App belegt) | Eigener Cache (SQLite) statt Seiten-Cache. Zeilen, Pools, Cache-Regeln als Logik in `lfx-core` (Golden gegen Web-JS). |
| Billboard mit Logo, Trailer, Ambient | `features/billboard.js`, `billboard/format.js`, `shared/trailer.js` | flix | `lfx-ui` + `lfx-player` | E2E (`home_e2e`) | umgesetzt (Mock/E2E) | Trailer per mpv; Datensparteil `trailer`/`hero` beachten. Abweichungen: Beschreibung wie Web aus; Bildwechsel ohne Überblendung; Trailer nur Windows (macOS kein Video). |
| Zeilen: Weiterschauen, Nächste Folge, Zuletzt, Genres, Ähnliche | `features/rows.js`, `rows/sources.js`, `rows/prefetch.js`, `rows/card.js` | flix | `lfx-store` + `lfx-ui` | Mock/E2E | teilweise (fehlt: Neue-Folgen-Info `newSeriesIds`; Karten zeigen nur Titeltext, keine Logos) | Quellen `/UserItems/Resume`, `/Shows/NextUp`, `/Items/Latest`, `/Genres`, `/Items/{id}/Similar`. Genre-Zeilen im Mock leer (je Genre 2–3 Titel, Schwelle 8). |
| Charts-Zeile mit Rückfall | `core/charts.js`, Test `rows-charts-fallback` | flix | `lfx-store` | Mock/E2E | umgesetzt (Mock/E2E) | Nur mit konfiguriertem Zusatzdienst, sonst Zeile aus. Mock bildet `/cw/charts` nach; ohne Charts-Dienst Ersatzzeile „Top 10: Neu auf …“. |
| Karten (Poster/Landschaft, Fortschritt, Badges) | `features/rows/card.js`, `shared/newcontent.js`, `shared/trend.js` | alle | `lfx-ui` | Snapshot, E2E | umgesetzt (Mock/E2E) | „Neu“- und Trend-Badges. Top-10-Ziffer liegt über der linken Kartenkante (Raster-Pitch gleich für alle Zeilen). |
| Hovercard mit Kurzinfo und Aktionen | `features/hovercard.js`, `hovercard/meta.js` | flix | `lfx-ui` | E2E (`e2e_detail`) | umgesetzt (Mock/E2E) | Nur mit Zeiger; Aktionen Play, Liste, Gesehen, Bewertung. Maus-Ereignisse nur über Automation/Slint-Rückrufe, echte Maus ungeprüft; Hovercard in plus aus; Kassetten-Vorschau retro fehlt (Zeile Abschnitt 3). |
| Detail-Modal (Staffeln, Folgen, Ähnliche) | `features/modal.js`, `modal/data.js`, `modal/render.js` | flix | `lfx-ui` | E2E (`e2e_detail`) | umgesetzt (Mock/E2E) | Teilt Daten mit Detailseite (Abschnitt 6). Abweichung: Ähnliche/Personen als Kacheln statt Textlinks. |
| „Neu und beliebt“ (`lfx=new`) | `features/newpopular.js` | flix | `lfx-ui` | E2E (`w2_library`) | teilweise (fehlt: Charts-Quelle für „Am häufigsten geschaut“, nur Rückfallpfad „Neueste …“; Eintragsknöpfe nur Play/Info) | Sortierung `DateCreated`, clientseitig filtern (Server ignoriert `MinDateCreated`). Kalendertag mit fester MEZ-Stunde. |
| Serien-Seite und Filme-Seite (Genre, Raster, Sortierung) | `features/library.js`, `features/library/*` | flix | `lfx-ui` | E2E (`w2_library`) | umgesetzt (Mock/E2E) | Pool-Schlüssel `lib:<kind>:<genre|all>:`, Billboard und Pool nie gleicher Cache-Schlüssel. Erweiterung: Sortierung „Bewertung“; Seitengröße 40, Endlos-Scroll per `GridPager`. |
| Meine Liste (Typ, Sortierung) | `features/mylist.js` | flix, retro | `lfx-ui` | E2E (`w2_library`) | umgesetzt (Mock/E2E) | `/Items?Filters=IsFavorite`; „Z–A“ als Idee. Sortierung A–Z/Neu/Jahr wie Web; „Z–A“ weiter nur Idee. |
| Gesehen/Liste/Bewertung UI-unabhängig | `shared/titleactions.js` | alle | `lfx-api` + `lfx-core` | E2E (`e2e_detail`) | teilweise (fehlt: Abarbeitung der Outbox-Einträge favorite/played/rating nach Wiederverbindung) | `POST/DELETE` Favorit, Played, Rating. Aktionen mit Server-Bestätigung, Rückgängig-Toast; bei Netzfehler Outbox-Eintrag. Der Abgleich in `lfx-offline` kennt nur die Art `offline.userdata`. |
| Download-Aktion mit Rechteprüfung | `core/download.js` | alle | `lfx-offline` | Mock | in Arbeit (W5) | Nutzt Offline-Modus; Token nie in der URL loggen. Stand vor W5: Rechteprüfung + Anzeige in Detail/Menü, Aktion nur Hinweis „folgt“; Manager in `lfx-offline` (Mock/Unit). |

## 3. LENZ+, LENZPRIME, LENZ//84

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Startseite Hero, Zeilen, Marken-Kacheln | `features/plus/home*.js`, `prime/home*.js`, `retro/home*.js`, `*/hero.js`, `*/rows.js`, `shared/collections.js`, `shared/hub.js` | plus, prime, retro | `lfx-ui` (Designs) | Mock/E2E (`w3_*`, `prime_e2e`, `e2e_retro`) | teilweise (fehlt: plus Karussell mit Seitenvorschau und Top-10-Rang; prime Poster-/„Mehr“-Zeilen; retro typabhängige Pfadzeile) | BoxSet-Kacheln aus `/Items`. Retro-Hero mit Terminal-Fenster, Kassetten-Karten; Trailer: plus und retro keiner, prime im Hero. |
| Titel-Vollseite (`lfx=title`) | `features/plus/title.js`, `prime/title.js`, `retro/title.js` | plus, prime, retro | `lfx-ui` | Mock/E2E (`w3_plus`, `prime_e2e`) | teilweise (fehlt: plus/prime-Tabs „Vorschläge/Details“; retro nur Tokens ohne eigenen Aufbau) | Gemeinsame Daten mit nativer Detailseite |
| Browse und Hub | `features/plus/browse.js`, `retro/browse.js`, `shared/hub.js` | plus, retro | `lfx-ui` | Mock/E2E | teilweise (fehlt: eigene Browse-/Hub-Seiten je Bibliothek; die Serien/Filme-Seite dient als Hub) | Sentinel-Nachladen (Test `retro-browse-sentinel`). Nachladen per Endlos-Scroll statt Sentinel. |
| Native Detailseite gestylt | `*/nativedetail.js` | plus, prime, retro | `lfx-ui` | Snapshot | teilweise (fehlt: prime-/retro-eigener Aufbau, dort nur Token-Anpassung; plus eigene Titelseite vorhanden) | Entfällt als eigenes Feature; Detailseite (Abschnitt 6) wird je Design gestylt |
| Vorschau/Kassette beim Überfahren | `features/prime/hover*.js`, `retro/hover.js` | prime, retro | `lfx-ui` | Mock/E2E (`prime_e2e`) | teilweise (fehlt: retro Kassetten-Vorschau, dort nur LENZFLIX-Variante mit Retro-Farben) | Nur mit Zeiger. prime: Vorschau-Karte mit Gate-Logik (Unit) umgesetzt. |
| Design-Suche | `features/plus/search.js`, `prime/search.js`, `retro/search.js`, `shared/search.js` | plus, prime, retro | `lfx-ui` | Mock/E2E | teilweise (fehlt: eigene plus-/retro-Suchoberfläche, dort gemeinsame Suche mit Tokens; prime Suchfeld als Chip) | Fließt in die native Suche (Abschnitt 7) |
| Retro-Effekte: CRT, Boot, Glitch, Terminal, Cheat-Code, Code-Regen | `features/retro/fx/*` | retro | `lfx-ui` (fx-Modul) | E2E (`e2e_retro`), Snapshot; Wirkung unprüfbar | teilweise (fehlt: Heading-Glitch RGB-Versatz, Text-Glow, Subpixel-Raster; Sepia/Vignette/Overdrive nur als Näherung ohne Shader) | Aus im Player und im Admin; Wirkung am Bildschirm vom Nutzer beurteilen. Boot, Terminal, Cheat-Code, Code-Regen, Glitch-Zeitplan umgesetzt; aus im Player/TV/Admin; Wirkung am Bildschirm vom Nutzer beurteilen. |
| Retro-Sound (synthetisiert) | `features/retro/fx/sound.js` | retro | `lfx-platform` (Audio) | Unit (Offline-Render); Hörprobe unprüfbar | umgesetzt (nur Unit) | Standard aus; Klangqualität nicht automatisch prüfbar. Klang ungehört (nur Offline-Render); PolyBLEP statt WebAudio, nicht bitgleich; Standard aus. Hörprobe durch den Nutzer nötig. |
| Datensparteile je Design | `tests/*-datasaver*.test.js` | alle | `lfx-core` | Unit | teilweise (Kernlogik `lfx-core` Unit; Anbindung je Design nicht belegt) | Reagiert auf Gesamt- und Teilschalter |

## 4. TV-Modus (IPTV-Seite, Plan bitgleich zum Web)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Planer: Hash, Zeit, Katalog, Pool, Füller, Serien, Premieren, Themen-Tage | `tv/schedule/*` (`hash.js`, `time.js`, `catalog.js`, `pool.js`, `fill.js`, `series.js`, `premiere.js`, `specials.js`, `themeday.js`, `rating.js`, `plan.js`) | alle | `lfx-tv-schedule` | Unit (Golden, 18 Tests) | umgesetzt (nur Unit) | Deterministisch; Golden-Vektoren aus dem Web (Tests `tv-schedule-*`, `tv-special-*`, `tv-themeday-*`); UTF-16-Einheiten, `Math.imul`, IndexMap, stabile Sortierung, `dedupe` per JSON-Reihenfolge. Bitgleich: 37 063 Vergleiche, 153 523 Planeinträge; Mutationstest belegt. Vektoren (26 MB) werden in CI erzeugt (`web-commit.txt`), nicht eingecheckt. |
| Zeit nur über MEZ/MESZ-Arithmetik | `tv/schedule/time.js` | alle | `lfx-tv-schedule` | Unit (Golden) | umgesetzt (nur Unit) | Kein `Intl`, keine Lokalzeit, kein Zufall außer `hash.js` |
| Katalog laden, Cache, Senderbelegung | `features/tvdata/{data,deps,config}.js` | alle | `lfx-tv` + `lfx-store` | Mock/E2E (`e2e_tvui`) | umgesetzt (Mock/E2E) | Admin-Senderbelegung Plätze 5–16 (`tvChannels`); IndexedDB-Name `lfx-live` entfällt in der App. Abweichungen: sequentielles Laden, Cache nur im Speicher ohne TTL, Cache-Schlüssel mit Sendetag in der Planzone. `LocalTrailerCount` ist im DTO ergänzt (`lfx-api` dto.rs), der lfx-tv-Adapter nutzt es noch nicht (keine lokalen Trailer als Füller). |
| Kanalliste, EPG-Raster, Info-Karte, Ziffernwahl | `tv/tvmode/{channel-list,epg-grid,info-card,info,digits,keys}.js` | alle | `lfx-tv` + `lfx-ui` | Snapshot/E2E (`e2e_tvui`), Unit | teilweise (fehlt: Vorschaubilder in Senderliste/Info-Karte, dort farbige Logo-Kacheln) | Retro-TV-Look nur per Styling. Abweichung: Tabs „Jetzt im TV“/„Programm“ statt Raster unter der Bühne. |
| Controller, Steuerung, Track-Menü, Geschwindigkeit | `tv/tvmode/{controller,controls,track-menu}.js` | alle | `lfx-tv` | Unit (178 Tests `lfx-tv`), E2E | teilweise (fehlt: Ton-/Untertitelwahl und Track-Menü in der TV-Oberfläche, Trailer-Füller) | Tests `tvmode-controller-*` als Vorlage |
| Eigener Player ohne Fortschrittsmeldung | `tv/player/{index,stream,tracks,watch,warm,neighbors}.js` | alle | `lfx-player` (TV-Profil) | Mock/E2E (`e2e_tvui`) | umgesetzt (Mock/E2E) | Keine Spuren in „Weiterschauen“; Fallen: kein `StartTimeTicks` an die HLS-URL, `ActiveEncodings` löschen, `ApiKey`, `SubtitleStreamIndex`/`MediaSourceId`-Regeln aus `docs/tvmode.md`. Mock sieht kein Playing/Progress/Stopped. Echter Server nicht geprüft. |
| HLS-Remux, HEVC nur wenn unterstützt | `tv/player/stream.js` | alle | `lfx-player` | Unit, Mock | teilweise (fehlt: Anbindung der Rückfallkette/Stream-Einordnung aus `lfx-tv` an `lfx-player`; dort nur Logik) | Transkodierung selbst unprüfbar (Testkonto ohne). mpv spielt fast alles direkt; Transkodierung unprüfbar (Testkonto ohne). |
| Vorwärmen der Nachbarsender, Zap-Statistik | `tv/player/warm.js`, `tv/ui/zapstats.js` | alle | `lfx-tv` | Unit | teilweise (Logik in `lfx-tv` Unit; nicht angebunden: kein Vorwärmen, keine Zap-Anzeige) | Datensparteil `tvwarm` |
| Bild-in-Bild, Hintergrundwiedergabe | `tv/tvmode/pip.js` | alle | `lfx-platform` + `lfx-player` | – | offen | Fensterverhalten je Plattform; macOS unprüfbar. Bewusste Abweichung: kein Bild-in-Bild am Desktop, Verlassen der Seite stoppt sofort. |
| Logos, Vollbild, Overlay-Helfer | `tv/ui/{logos,fullscreen,overlay-util}.js` | alle | `lfx-ui` | Snapshot, E2E | umgesetzt (Mock/E2E) | Eigene Marken-Logos nur aus dem Repo. Abweichung: Vollbild = Vollfläche im Fenster, kein exklusives Fenster-Vollbild. |
| TV-Symbol/Einstieg in der Kopfleiste | `features/tvbtn.js`, `features/tvmode/*` | alle | `lfx-shell` | E2E | umgesetzt (Mock/E2E) | Entsperrung des Players entfällt (kein Browser-Gestenzwang) |

## 5. Wiedergabe-Kern und Player-OSD

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| mpv-Einbindung (dynamisches Laden, LGPL), Wiedergabe-Zustand | (Web: jellyfin-web) | alle | `lfx-mpv-sys` + `lfx-player` | Unit (headless libmpv), E2E | umgesetzt (Mock/E2E) | Nur `libloading`, DLL nie im Repo; `xtask fetch-mpv`. Nur Windows lokal; x64 und macOS ungeprüft. Hülle ohne Node-API (`command_node`/`get_property_node`). |
| Direktwiedergabe, Rückfallkette, Fehlercodes | `core/playback.js`, Tests `playback-*` | alle | `lfx-player` | Mock/E2E (`e2e_player`) | umgesetzt (Mock/E2E) | Kein Proxy-Klick; `PlaybackInfo` und Profil selbst bauen. Live (echter Server) offen; HLS-Rückfall und Fehlercodes P/M/V nur gegen Mock. |
| Fortschrittsmeldung `/Sessions/Playing*` | (Web: jellyfin-web) | alle | `lfx-api` + `lfx-player` | Mock/E2E | umgesetzt (Mock/E2E) | Start, Fortschritt, Stopp; nur nicht im TV-Modus |
| OSD: Fortschrittsleiste, Lautstärke, Play/Pause, Vor/Zurück | (Web: jellyfin-web), Bildmitte `features/osdcenter.js` | alle | `lfx-ui` (Player-OSD) | Mock/E2E (`e2e_player`), Screenshot | umgesetzt (Mock/E2E) | Bildmitte ⟲10 / Play / ⟳30 als nativer Overlay. Lautstärkeregler rot statt weiß; Mausrad-Lautstärke fehlt. |
| Tonspur- und Untertitelwahl | (Web: jellyfin-web) | alle | `lfx-player` + `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Standard je Benutzer-Einstellungen; Server-Fallen: Brennen vermeiden |
| Untertitel-Rendering (SRT, ASS) | (Web: jellyfin-web) | alle | `lfx-player` (mpv) | Unit (SRT), E2E | umgesetzt (Mock/E2E) | mpv rendert; externe Spuren per Stream-URL. Nur Größe und Hintergrund geprüft; ASS und echte Server-Spuren ungeprüft. |
| Kapitel mit Marken und Namen | (Web: jellyfin-web) | alle | `lfx-player` + `lfx-ui` | Mock/E2E | teilweise (fehlt: Kapitelbilder; Marken und Namen vorhanden) | Kapitelbilder per `/Items/{id}/Images/Chapter` |
| Trickplay-Vorschau beim Scrubben | (Web: jellyfin-web) | alle | `lfx-player` + `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Kacheln aus `/Videos/{id}/Trickplay/...` |
| Intro/Abspann überspringen (Media Segments) | (Web: jellyfin-web) | alle | `lfx-player` | Mock/E2E | umgesetzt (Mock/E2E) | Segmente per `/MediaSegments/{id}`; Datenlage am Server abhängig. Datenlage am echten Server abhängig. |
| Nächste Folge („Up Next“), Autoplay | (Web: jellyfin-web) | alle | `lfx-player` | Mock/E2E | umgesetzt (Mock/E2E) | Nächste Folge per `/Shows/NextUp`. Fehlend: Vorschaubild der nächsten Folge. |
| Wiedergabegeschwindigkeit, Seitenverhältnis, Audio-Verzögerung | (Web: jellyfin-web) | alle | `lfx-player` | Unit, E2E | teilweise (fehlt: Seitenverhältnis, Audio-Verzögerung; Tempo mit 7 Stufen vorhanden) | Unabhängig vom Server |
| Qualität/Bitrate wählen (Transkodierung) | (Web: jellyfin-web) | alle | `lfx-player` | Mock/E2E; Server-Umwandlung unprüfbar | unprüfbar (Testkonto ohne Transkodierung; Menü und `max_bitrate` nur gegen Mock) | Testkonto ohne Transkodierung |
| HDR-Wiedergabe (HDR10, HLG und weitere HDR-Formate) (Gate G3) | – | alle | `lfx-player` + `lfx-mpv-sys` | Unit, Messung ohne Anzeige | unprüfbar (HDR-Sichttest ausstehend: Windows-HDR am Dev-Rechner aus, Nutzer-Freigabe nötig) | Nur mit Nutzer-Freigabe am HDR-fähigen Gerät; Modelle A/B/C laut Plan. Umgesetzt und nur ohne HDR gemessen: Formatplan (`play_hdr`, SDR/PQ/scRGB), Shell-Pfad, `SetHDRMetaData`. Offen: Weißpegel scRGB, Bildeindruck; `gamma` meldet bt.1886 bei PQ, Erkennung über sig-peak/colormatrix absichern; für scRGB `hdr-compute-peak=no`. |
| Hardware-Dekodierung (d3d11va), Bildausgabe in Slint | – | alle | `lfx-player` + `lfx-mpv-sys` | Messung, E2E (Windows ARM64) | umgesetzt (Mock/E2E) | Windows ARM64/x64 live; macOS unprüfbar. d3d11va, 4K60 mit 59,5–60 fps (Testclips); Resize-Salven 47 fps. Echter Server und x64 ungeprüft; macOS unprüfbar. |
| Bild-in-Bild, Vollbild | (Web: jellyfin-web) | alle | `lfx-platform` | E2E | teilweise (fehlt: Bild-in-Bild; macOS-Vollbild nur Hinweis-Toast; Fenstergrößenwechsel im Vollbild nicht beobachtet) | Fensterverwaltung je Plattform |

## 6. Detailseite

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Film-Detailseite (Backdrop, Logo, Infos, Besetzung, Ähnliche) | `css/modern/detail.css`, `docs/dom-map.md` (Detail) | alle | `lfx-ui` (Detail) | Mock/E2E (`e2e_detail`) | umgesetzt (Mock/E2E) | Eigene Seite statt jellyfin-web |
| Serie: Staffeln, Folgen, Gesehen-Markierung | `features/modal/data.js` | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | `/Shows/{id}/Seasons|Episodes` |
| Play/Fortsetzen/Trailer/Zufall/Favorit/Gesehen/Download/Mehr | `shared/titleactions.js`, `core/itemmenu.js` | alle | `lfx-ui` | Mock/E2E | teilweise (fehlt: Download-Aktion (W5), nur Hinweis; Trailer-/Zufall-Knopf nicht belegt) | Kein Proxy-Klick |
| Ton/Untertitel vor dem Start wählen | `docs/dom-map.md` (`.trackSelections`) | alle | `lfx-ui` + `lfx-player` | Mock/E2E | umgesetzt (Mock/E2E) | Versionen/MediaSources auswählen. Tonspur 2 in der Startmeldung belegt; Auswahl per Weiterschalten statt Dropdown. |
| Sammlungen (BoxSet), Playlists | `shared/collections.js` | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Details nur aus der API ableitbar (Web nicht abgedeckt). Wiedergabelisten zusätzlich in den Medienseiten (Musik). |
| Personenseite, Studio, Genre-Ziele | – | alle | `lfx-ui` | Mock/E2E | teilweise (Filmografie vorhanden; fehlt: Personenfotos, Studio-/Genre-Ziele nicht belegt) | Web delegiert an jellyfin-web |

## 7. Suche

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Suche mit Sofortergebnissen, Gruppen nach Typ | `shared/search.js`, `css/modern/search.css`, Tests `search`, `shared-search-order` | alle | `lfx-ui` | Mock/E2E (`w2_library`) | umgesetzt (Mock/E2E) | `/Items?searchTerm`, `/Persons`, `/Genres`; Reihenfolge laut Tests. Abweichung wie Web: `/Items` und `/Persons` statt `/Search/Hints`; kein Hinweis-Dropdown. |
| Verlauf/Vorschläge | – | alle | `lfx-store` | E2E | umgesetzt (Mock/E2E) | Lokal, nie an den Server. Schlüssel `lib.recent_searches` lokal. |
| Tastaturkürzel (`/`) | – | alle | `lfx-nav` | – | offen | Neu in der App. Nicht belegt. |

## 8. Anmeldung, Server, Benutzer

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Serverwahl und manuelle Adresse | `docs/dom-map.md` (Login) | alle | `lfx-session` + `lfx-ui` | Mock/E2E (`e2e`, `account_e2e`) | umgesetzt (Mock/E2E) | Eingegebene URL im Log schwärzen |
| LAN-Suche (UDP-Broadcast, Jellyfin-Erkennung) | – | alle | `lfx-session` + `lfx-platform` | Mock (Code), Live unprüfbar | teilweise (Code und Knopf vorhanden; nie gegen echtes Netz oder Broadcast-Mock geprüft) | Port 7359; Netzabhängig |
| Anmeldung mit Name/Passwort, „Angemeldet bleiben“ | `css/modern/login.css` | alle | `lfx-session` | Mock/E2E (`e2e`) | umgesetzt (Mock/E2E) | Token im OS-Schlüsselbund, nie in Dateien. Token nur im Speicher bei `--home`; echter Schlüsselbund nur mit `--keyring`. |
| Quick Connect | – | alle | `lfx-session` | Mock/E2E | umgesetzt (Mock/E2E) | `/QuickConnect/*` |
| „Wer schaut?“ (mehrere Benutzer je Server) | – | alle | `lfx-session` + `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Öffentliche Benutzerliste, Kennwortfrage nach Server-Einstellung |
| Mehrere Server, Umschalten | – | alle | `lfx-session` | Mock/E2E (`account_e2e`) | umgesetzt (Mock/E2E) | Je Server eigener Schlüsselbund-Eintrag |
| Passwort vergessen, Abmelden | `docs/dom-map.md` | alle | `lfx-session` | Mock/E2E | teilweise (Abmelden vorhanden; Passwort vergessen nicht belegt) | Abmelden löscht Token, Server-Eintrag bleibt |
| Token-/URL-Schwärzung in Logs | – | alle | `lfx-core` | Unit (5 Tests Schwärzung) | umgesetzt (nur Unit) | Pflicht, mit Test. Dazu Debug-Ausgaben ohne Token/URL in `lfx-api`, `lfx-session`. |
| Schlüsselbund (Windows Credential Manager, macOS Keychain) | – | alle | `lfx-platform` | Unit (echter Test ignoriert, lokal ok); macOS unprüfbar | umgesetzt (nur Unit) | macOS unprüfbar. Windows Credential Manager lokal gelaufen; macOS-Keychain nur CI-Build. |

## 9. Bibliotheken (Raster)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Bibliotheksübersicht und -raster (Filme, Serien, Sammlungen, Heimvideos, Musikvideos) | `css/modern/library.css`, `shared/libraries.js` | alle | `lfx-ui` | Mock/E2E | teilweise (fehlt: Einstieg in Sammlungen und Medienseiten aus der Kopfleiste; derzeit Suche-Chips) | Eigene Seiten statt `#/list`, `#/mixed`. Sammlungen/Boxsets/Playlists als Raster mit Alphabet-Leiste. |
| Filter (Genre, Jahr, Gesehen, Favorit, Bewertung u. a.) | – | alle | `lfx-ui` + `lfx-store` | Mock/E2E | teilweise (fehlt: Filter Jahr, Gesehen, Favorit, Bewertung; nur Genre und Typ) | Über `/Items`-Parameter und `/Filters` |
| Sortierung | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Sortierfelder wie jellyfin-web |
| Alphabet-Leiste (`NameStartsWith`) | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Nur in der Sammlungsansicht. |
| Seitenweise Nachladen mit virtuellem Raster | – | alle | `lfx-ui` | Mock/E2E (`w2_library`) | umgesetzt (Mock/E2E) | Slint-Listen mit Virtualisierung; Leistung Live prüfen. Leistung am echten Server und Mausrad/Touch-Scroll nicht gemessen. |
| Bildcache und Skalierung | `core/imgscale.js` | alle | `lfx-core` | Unit (`lfx-images`, 36 Tests) | umgesetzt (nur Unit) | Plattenkontingent konfigurierbar. Ort: `lfx-images` (Speicher-/Plattencache, Blurhash) plus `lfx-core` (Skalen). Plattenkontingent nicht in den Einstellungen konfigurierbar. |

## 10. Benutzer-Einstellungen (nativ)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Einstellungsmenü (Anzeige, Startseite, Wiedergabe, Untertitel, Steuerung, Quick Connect, Profil) | `shared/pagetitle.js` (nur Titel) | alle | `lfx-ui` (Einstellungen) | Mock/E2E (`account_e2e`) | umgesetzt (Mock/E2E) | Web: nur jellyfin-web |
| Wiedergabe-Defaults (Sprache, Audio, Qualität, Autoplay) | – | alle | `lfx-store` + `lfx-player` | Mock/E2E, Unit | teilweise (fehlt: Verdrahtung der lokalen Schlüssel `playback.max_bitrate`, `ctl.*` im Player; Serverwerte voll) | Lokal; Serverseitig lesen, nie in die Web-DisplayPreferences schreiben. Serverseitig GET→ändern→POST ohne Verlust unbekannter Felder. |
| Untertitel-Defaults und Aussehen | – | alle | `lfx-store` + `lfx-player` | Unit, Mock/E2E | teilweise (fehlt: Verbindung der Einstellungsseite `sub.*` mit dem Player-Stil `player.sub_style`, zwei Schlüsselsätze) | Größe, Farbe, Hintergrund |
| Startseiten-Abschnitte anpassen | – | alle | `lfx-store` | – | offen | Eigene Konfiguration der App. Reihenfolge der Startseiten-Zeilen nur als Hinweis „im Browser“. |
| Profil (Name, Passwort, Bild) | – | alle | `lfx-ui` + `lfx-api` | Mock/E2E (`account_e2e`) | umgesetzt (Mock/E2E) | Abweichung: Avatar per Pfadfeld statt Datei-Dialog; Upload gegen echten Server ungeprüft. |
| App-Einstellungen (Design, Datensparen, Autostart, Tray, Update) | – | alle | `lfx-core` | Unit, E2E | teilweise (Design, Datensparmodus, Couch umgesetzt; Autostart, Tray, Update-Verdrahtung in Arbeit (W5)) | Eigenes Konfig-Format |

## 11. Musik

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Musikbibliothek: Künstler, Alben, Titel, Genres | – (Web nur Link, jellyfin-web) | alle | `lfx-ui` + `lfx-store` | Mock/E2E (`w4_media`) | teilweise (fehlt: eigene Genre-Seiten, ein Genre startet nur einen Mix) | Web nicht abgedeckt. Alben, Künstler, Titel, Wiedergabelisten umgesetzt. |
| Audio-Player (Mini-Player, Vollbild-Player) | – | alle | `lfx-player` + `lfx-ui` | Mock/E2E (`w4_media`) | umgesetzt (Mock/E2E) | Nutzt mpv ohne Video. Mini-Leiste auf Fremdseiten nur per Maus; SMTC-Anzeige nicht angesehen. |
| Warteschlange, Zufall, Wiederholen | – | alle | `lfx-player` | Unit (11), Mock/E2E | umgesetzt (Mock/E2E) |  |
| Playlists (anlegen, bearbeiten, abspielen) | – | alle | `lfx-ui` + `lfx-api` | Mock/E2E | umgesetzt (Mock/E2E) | `/Playlists` |
| Texte (Lyrics) | – | alle | `lfx-ui` | – | offen | Nur wenn der Server Lyrics liefert |
| Gapless/Crossfade | – | alle | `lfx-player` | – | offen | Hörprobe vom Nutzer. Hörprobe vom Nutzer nötig. |

## 12. Bücher und Fotos

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Bücher: Liste, Detail | – | alle | `lfx-ui` | Mock/E2E (`w4_media`) | umgesetzt (Mock/E2E) | Web nicht abgedeckt |
| Bücher lesen (EPUB/PDF/CBZ) | – | alle | `lfx-ui` | Mock/E2E | teilweise (fehlt: eingebauter Leser EPUB/PDF/CBZ, Hörbuch-Fortschritt; Lesen = Download und Systemprogramm) | Umfang beim Start klären (Lesen vs. Öffnen extern). Bewusste Abweichung: kein eigener Renderer (große Abhängigkeiten). |
| Fotos: Alben, Raster, Vollbild, Diashow | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Web nicht abgedeckt. Fotos der Bibliothek `homevideos`; Videos darin werden nicht gelistet. |

## 13. Jellyfin-Live-TV

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Programmführer, Kanäle | `features/nav.js` (nur Link) | alle | `lfx-ui` + `lfx-api` | Mock/E2E (`w4_media`); Tuner unprüfbar | umgesetzt (Mock/E2E) | Server ohne Tuner; nur Mock. Nur Mock, kein Tuner. |
| Aufnahmen, Timer, Serienaufnahmen | – | alle | `lfx-ui` | Mock/E2E; Tuner unprüfbar | teilweise (fehlt: Serienaufnahmen/Serien-Timer) | Nur Mock |
| Live-Wiedergabe (HLS-Livestream) | – | alle | `lfx-player` | Mock/E2E; Tuner unprüfbar | teilweise (fehlt: `close_live_stream` beim Verlassen) | Nicht mit dem LENZFLIX-TV-Modus verwechseln (Abschnitt 4). Echter Server (OpenToken-Weg) nur über PlaybackInfo-Weg im Mock. |

## 14. Admin (häufige Aufgaben nativ, Rest im Browser)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Übersicht (Server, Version, Aktivität) | – (Theme aus auf Dashboard) | alle | `lfx-ui` (Admin) | Mock/E2E (`account_e2e`) | umgesetzt (Mock/E2E) | Nur für Administratoren sichtbar; Retro-Effekte aus. Nur für Administratoren; Retro-Effekte aus. |
| Laufende Sitzungen, Fernsteuern, Beenden | – | alle | `lfx-ui` + `lfx-api` | Mock/E2E | umgesetzt (Mock/E2E) | `/Sessions` |
| Benutzer anlegen/bearbeiten/Rechte | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Eigenes Konto geschützt. |
| Bibliotheken scannen, Metadaten aktualisieren | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | `/Library/Refresh` |
| Geplante Aufgaben (anzeigen, starten) | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | `/ScheduledTasks` |
| Logs ansehen | – | alle | `lfx-ui` | Mock/E2E | umgesetzt (Mock/E2E) | Tokens/URLs in angezeigten Logs schwärzen. Schwärzung in angezeigten Logs nicht belegt. |
| Alles andere (Plugins, Netzwerk, Transkodierung, …) | – | alle | `lfx-shell` | Mock/E2E | umgesetzt (Mock/E2E) | Knopf „Im Browser öffnen“ (`/web/#/dashboard`). Browser-Start in Tests nur als Adresse vermerkt. |

## 15. Offline und Downloads

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Titel/Staffel laden mit Fortschritt, Pause, Abbruch | `core/download.js` (nur Link) | alle | `lfx-offline` | Mock | in Arbeit (W5) | Download-Manager neu; `ApiKey` nie loggen. Stand vor W5: Manager in `lfx-offline` (Pause, Fortsetzen per Range, SHA-256, Transcode-Variante) Mock/Unit-getestet; UI und Verdrahtung W5. |
| Offline-Bibliothek und Offline-Wiedergabe | – | alle | `lfx-offline` + `lfx-player` | Mock | in Arbeit (W5) | Metadaten, Bilder, Untertitel mitspeichern. Stand vor W5: `LocalPlayback::to_play_request()` gebaut; mpv-Lauf mit lokaler Datei fehlt. |
| Fortschritt offline sammeln, später synchronisieren | – | alle | `lfx-offline` + `lfx-api` | Mock | in Arbeit (W5) | Konfliktregel: neuester Stand gewinnt. Stand vor W5: `OutboxSyncer` (neuester Stand gewinnt) Mock/Unit-getestet; automatischer Start bei Wiederverbindung fehlt. |
| Speicherplatz-Verwaltung, Löschen | – | alle | `lfx-offline` | Unit | in Arbeit (W5) | Kontingent in den Einstellungen. Stand vor W5: Belegungsanzeige und Löschen vorhanden, Kontingent-UI fehlt. |

## 16. Betriebssystem-Integration

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Medientasten, SMTC (Windows), Now Playing (macOS) | – | alle | `lfx-platform` | E2E (Windows); macOS unprüfbar | teilweise (fehlt: macOS Now Playing, dort Stub; SMTC-Anzeige nicht angesehen) | Titel, Cover, Fortschritt. Wachhalten per `SetThreadExecutionState`; Windows-Verbindung im E2E belegt. |
| Tray-Symbol (Windows/macOS Menüleiste) | – | alle | `lfx-platform` | – | in Arbeit (W5) | Minimieren in den Tray, Schnellaktionen |
| Auto-Update (Minisign-Signatur) | – | alle | `lfx-platform` + `xtask` | Unit (26 Tests `lfx-update`) | teilweise (Crate mit Signatur-/Download-/Installlogik Unit; fehlt: echter Minisign-Schlüssel (Platzhalter), Anbindung in der App, Live-Abruf; W5 polish) | Unsigniert (macOS ad-hoc); Signaturprüfung als Unit-Test. Schema `latest.json` mit Assets je Ziel; Installer-Start nur Logik getestet. |
| Installer NSIS (Windows), DMG (macOS) | – | alle | `xtask` + CI | Staging getestet; Installer unprüfbar | unprüfbar (makensis lokal nicht installiert, `installer.nsi` nie kompiliert; DMG und `release.yml` nie gelaufen) | Windows-Installer live, DMG nur CI. `xtask package` (Staging, ICO/ICNS) getestet. Erster Tag-Lauf in CI ist der echte Test. |
| Autostart, Einzelinstanz, Dateizuordnung | – | alle | `lfx-platform` | – | in Arbeit (W5) |  |
| Eigene Titelleiste, Fensterzustand | – | alle | `lfx-shell` | Unit (Hit-Test), Live-Selbsttest | teilweise (fehlt: interaktive Prüfung von Ziehen, Rand-Resize, Snap-Layouts-Flyout; macOS-Ampel unprüfbar) | Name „LENZFLIX“; macOS-Ampel unprüfbar. WM_NCHITTEST-Antworten per `SendMessage` belegt. Befund: Fenster ohne `--maximized` startet auf dem Dev-Rechner teils außerhalb des Bildschirms. |
| Über-Dialog mit AboutSlint | – | alle | `lfx-ui` | Screenshot | umgesetzt (Mock/E2E) | Pflicht laut Slint-Lizenz |
| HiDPI/90 Hz-Darstellung | – | alle | `lfx-shell` | Bench (Shell-Beispiel), Live | teilweise (Bench der Shell: 89–90 fps, p95 11,6–12,5 ms bei Ziel ≤ 11,7 ms; Messung der vollen App steht aus) | Dev-Rechner 2944×1840, DPR 2. Gerät: Adreno X1-85, 2944×1840, DPR 2. Software-Rückfall verfehlt das Ziel (80 fps). |

## 17. Fernsteuerung, SyncPlay, Couch-Modus

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Fernsteuerung (App als Ziel anderer Clients) | – | alle | `lfx-session` + `lfx-player` | Mock | in Arbeit (W5) | WebSocket `/socket`, Allgemeine Befehle. Stand vor W5: `lfx-remote` (WebSocket, Capabilities, KeepAlive, Backoff) Mock/Unit-getestet; App-Anbindung W5. |
| Andere Geräte steuern (Cast-Menü) | `docs/dom-map.md` (AppBar-Knopf) | alle | `lfx-ui` + `lfx-api` | Mock | in Arbeit (W5) |  |
| SyncPlay (Gruppe erstellen, beitreten, synchron) | `docs/dom-map.md` (Menü `#app-sync-play-menu`) | alle | `lfx-session` + `lfx-player` | Mock | in Arbeit (W5) | Nichts mit dem VIDEOTHEK-Mitschauen zu tun. Stand vor W5: SyncPlay-Kern (Drift/Zeitplan) Unit, API Mock. |
| Couch-Modus: Gamepad-Navigation, große Ziele | `css/legacy-tv/tv.css` (Look) | alle | `lfx-nav` + `lfx-ui` | Unit, E2E (simuliertes Gamepad) | teilweise (Couch = 125 % Zoom + dicker Fokusring + LENZFLIX-Look, nicht das volle TV-Layout; Gamepad-Hardware und Fernbedienungs-Quelle ungeprüft) | Alle Designs steuerbar, Look immer LENZFLIX-TV; Controller-Hardware nur Live durch Nutzer. Automatik: Gamepad an, Mausbewegung zurück. |
| Bildschirmtastatur im Couch-Modus | – | alle | `lfx-ui` | Mock/E2E | teilweise (Tastatur per Knopf in der Suche; fehlt: Auto-Öffnen per Gamepad) |  |

## 18. Statistik und Charts

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Charts und Trend-Badges | `core/charts.js`, `shared/trend.js`, `docs/charts-api.md` | alle | `lfx-store` + `lfx-ui` | Mock/E2E | teilweise (Charts-Zeile und Trend-Badges umgesetzt; „Neu und beliebt“ ohne Charts; Rest in Arbeit (W5)) | Zusatzdienst per Adresse in den Einstellungen; ohne Dienst aus |
| Statistik-Seite nativ (Verlauf, Heatmap, Listen) | `core/stats.js`, `features/statshint.js`, `docs/stats-api.md`, Tests `stats-page*` | alle | `lfx-ui` + `lfx-api` | Mock | in Arbeit (W5) | Nativ statt externe Seite; Heatmap-Rendering selbst. Profilmenü zeigt bisher nur einen Platzhalter-Toast. |
| Statistik-Hinweis | `features/statshint.js` | flix | `lfx-ui` | Snapshot | in Arbeit (W5) |  |

## 19. VIDEOTHEK LENZ (store)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Slot im Design-Picker (nicht anwählbar, Hinweis) | `features/store/*` | store | `lfx-3d` | E2E (`w3_picker`) | umgesetzt (Mock/E2E) | Slot vorbereitet; kein 3D, kein VR, kein Mehrspieler, kein Wetter |

## 20. Qualität und Betrieb

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Mock-Jellyfin-Server mit synthetischen Fixtures | – | – | `lfx-mock-jellyfin` | Mock | umgesetzt (Mock/E2E) | Erfundene Titel, generierte Platzhalter-Cover. Erweiterungen je Welle opt-in; PNG-Cover gültig (24×36). |
| Golden-Vektoren aus dem Web-Planer | `tests/tv-schedule-*.test.js` | – | `lfx-tv-schedule` | Unit | umgesetzt (nur Unit) | Erzeugt mit dem Web-Code (`tools/golden/gen.mjs`). Web-Planer-Vektoren (26 MB) werden in CI erzeugt; `lfx-core`-Golden `web.json` (116 KiB) und Ambient-Vektoren eingecheckt. |
| CI (Windows ARM64/x64, macOS arm64) | – | – | `xtask` + Workflows | – | unprüfbar (`ci.yml`/`release.yml` nie auf GitHub gelaufen; macOS nur dort baubar) | macOS nur Bauen/Testen |
| Fehlerprotokoll mit Schwärzung | – | – | `lfx-app` (`logging.rs`) | Unit | umgesetzt (nur Unit) | Logdatei `<data>/logs/lenzflix.log`, jede Zeile durch `scrub` (URLs, Token, ApiKey, Passwort, DeviceId, Bearer). |

## Bewusste Abweichungen vom Web (Stand 20.11.2026)

- Kein Crate `lfx-3d`: der VIDEOTHEK-Platz ist nur eine ausgegraute Karte „bald“ im Picker.
- `Route`/`Router` in `lfx-app`, nicht in `lfx-nav`; Detailseite, Modal, Hovercard, Kontextmenü, Suche u. a. sind eigene native Seiten statt jellyfin-web.
- Couch-Modus = 125 % Zoom + dicker Fokusring + LENZFLIX-Look, nicht das volle Web-TV-Layout.
- TV-Modus: Tabs statt Raster unter der Bühne, kein Bild-in-Bild (Verlassen stoppt), Vollbild nur als Vollfläche im Fenster, Katalog-Cache nur im Speicher, sequentielles Laden.
- Retro-Effekte ohne Shader (Näherungen); retro Terminal-Taste `^` statt Backquote.
- Bücher werden heruntergeladen und extern geöffnet (kein eingebauter Leser); Avatar-Upload per Pfadfeld.
- Suche nutzt `/Items` und `/Persons` wie das Web, nicht `/Search/Hints`.
- Desktop-Erweiterungen ohne Web-Vorbild: Sortierung „Bewertung“, `dedupe_across_rows` (nicht angewendet), DPR-Faktor der Bildgrößen, manueller Remote-Trailer.
- Online-Pille des Web-Pickers entfällt; Zugangsdaten im OS-Schlüsselbund statt `localStorage`.

## Nicht prüfbar / offene Prüfungen (Stand 20.11.2026)

- HDR (G3): Sichttest nur mit Nutzer bei eingeschaltetem Windows-HDR; bis dahin bleibt die Zeile „unprüfbar“. Weißpegel scRGB, Bildeindruck und PQ-Erkennung offen.
- Kein Lauf gegen einen echten Jellyfin-Server (Anmeldung durch den Nutzer in der Dev-App offen).
- macOS: nur Kompilierung/CI; Laufzeit, Keychain, Now Playing, DMG unprüfbar.
- Windows x64 nur gebaut, nicht gelaufen; Snapshots nur auf ARM64 erzeugt.
- Installer: `makensis` nicht installiert, `installer.nsi`, `release.yml`, `ci.yml` noch nie ausgeführt.
- Gamepad-Hardware, echte Mausbedienung (nur injiziert), Snap-Layouts-Flyout, Klangqualität (Retro), SMTC-Anzeige: nicht angesehen/gehört.
- Gefundene Fallen: Fenster ohne `--maximized` startet teils außerhalb des Bildschirms; E2E `e2e_player` unter Last flaky (`awake`-Zeitpunkt); Desktop-Screenshots werden bei parallelen Fenstern verdorben (`LFX_SHOT_UI_ONLY=1`).
