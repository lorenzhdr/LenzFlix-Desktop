# Paritätsliste Web (LenzFlix) → App (LENZFLIX Desktop)

Pflegedatei. Jede Zeile ist eine Funktion, die die App erreichen muss, bevor es ein Release (1.0.0) gibt. Status wird nur hier geändert (Hotspot-Datei, nur der Orchestrator pflegt sie).

- Pfade der Web-Quelle sind relativ zu `src/js/` des Web-Repos (CSS: `src/css/`, Doku: `docs/`). Die Vorlage wird nur gelesen, nie verändert.
- Designs: `flix` (LENZFLIX), `plus` (LENZ+), `prime` (LENZPRIME), `retro` (LENZ//84), `store` (VIDEOTHEK LENZ), `alle` = flix/plus/prime/retro.
- Prüfebenen: **Unit** (reine Logik, Golden-Vektoren), **Mock** (gegen `lfx-mock-jellyfin`, kein Netz), **Snapshot** (UI-Snapshot mit synthetischen Fixtures), **Live** (echter Jellyfin-Server, nur vom Orchestrator/Nutzer), **unprüfbar** (hier nicht prüfbar, ehrlich markiert).
- Status: `offen` überall (Stand Phase 0, 20.11.2026). Spätere Werte: `in Arbeit`, `fertig`, `geprüft`.
- Bewusst nicht übernommen: Browser-Eigenheiten (Proxy-Klicks, DOM-Karte, MUI-Selektoren, jsDelivr-Nachladen, `ApiClient`/`localStorage`-Zugangsdaten, Custom-CSS-Einbindung). Die App ruft die Jellyfin-REST-API direkt.
- Nicht prüfbar hier (gilt für alle Bereiche): macOS-Laufzeit (nur CI-Build, kein Mac), Transkodierung (Testkonto ohne), Live-TV ohne Tuner, HDR-Darstellung ohne Nutzer-Freigabe am Dev-Rechner.
- Nur Deutsch (UI). Keine Übernahme von Web-DisplayPreferences-Schreibzugriffen: die App schreibt nie in die Web-Einstellungen (eigener Speicher).

## 1. Kern, Design-System, Navigation

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Design-Tokens (Farben, Abstände, Radien, Schatten) | `css/tokens.css` | alle | `lfx-ui` (Theme-Global) | Snapshot | offen | Per `xtask` aus der Vorlage extrahiert, nur `--lfx-*`-Tokens |
| Schriften (Inter, Source Sans 3, Figtree, VT323, Space Mono) | `assets/fonts`, Lizenztexte | alle | `lfx-ui` (eingebettet) | Snapshot | offen | Lizenztexte mitliefern; Retro-Schriften nur retro |
| Design-Wechsel flix/plus/prime/retro zur Laufzeit | `core/theme.js`, `core/themecss.js` | alle | `lfx-ui` + `lfx-shell` | Snapshot | offen | Eigene Ablage in App-Einstellungen, nie Web-DisplayPreferences |
| Design-Picker mit Vorschau | `core/themepicker.js`, `core/popover.js` | alle | `lfx-ui` | Snapshot | offen | Online-Pille (`core/online.js`) nur mit konfiguriertem Zusatzdienst |
| Slot für VIDEOTHEK LENZ (store) | `features/store/*` | store | `lfx-3d` (Slot) | unprüfbar | offen | Nur Slot vorbereitet, kein 3D, kein VR, kein Mehrspieler |
| Router, Seitenzustände, Zurück | `core/router.js`, `shared/routes.js` | alle | `lfx-nav` | Unit | offen | Routen-Enum ersetzt Hash-Router; zweiphasig (erst Unmount, dann Mount) |
| Streaming-Kopfleiste (Logo, Bibliotheken, Suche, Profil) | `features/nav.js`, `css/js-ui/header.css` | flix | `lfx-shell` | Snapshot | offen | Eigene Titelleiste, Name „LENZFLIX“ |
| Handy-/Schmal-Navigation | `features/mobilenav.js` | flix | `lfx-shell` | Snapshot | offen | Schmales Fenster statt Handy; `isNarrow`-Schwelle prüfen |
| Kopfleiste, Tabbar, Profilmenü | `features/plus/header*.js`, `features/prime/header*.js`, `features/retro/header*.js`, `*/tabbar.js`, `shared/profile.js` | plus, prime, retro | `lfx-shell` | Snapshot | offen | Abmelden/Benutzerwechsel nativ (Abschnitt 8) |
| Tab-Titel, Favicon, theme-color | `core/doctitle.js`, `core/favicon.js`, `core/themecolor.js`, `shared/pagetitle.js` | alle | `lfx-shell` (Fenstertitel, Icon) | Unit | offen | Fenstertitel „Seite · Design“; Tab-Icon je Design als App-Icon-Variante offen |
| Datensparmodus mit Teilschaltern und Bildgrößen | `core/datasaver.js`, `core/imgscale.js`, `shared/dsswitch.js`, `features/dsbtn.js` | alle | `lfx-core` + `lfx-ui` | Unit | offen | Teile trailer/hero/images/prefetch/lazy/tvwarm; VIDEOTHEK ausgenommen |
| Geheimmenü (5 Klicks auf Logo in 3 s) | `features/secretmenu.js`, `secretmenu/panel.js` | flix | `lfx-shell` | Snapshot | offen | Bewusst nicht in der README |
| Ambient-Farbe aus dem Cover | `core/ambient.js`, `core/color.js` | flix | `lfx-ui` | Unit | offen | Farbermittlung aus dem Bild nativ |
| Ladebild beim Start der Wiedergabe | `core/startcover.js` | alle | `lfx-player` | Snapshot | offen | Entfällt als DOM-Beobachter, wird Player-Zustand |
| Schwarzer Player-Hintergrund | `features/playerbg.js` | alle | `lfx-player` | Snapshot | offen | Selbstverständlich im nativen Player |
| Toast, Sheet, Kontextmenü | `core/toast.js`, `core/sheet.js`, `features/contextmenu.js`, `core/itemmenu.js` | alle | `lfx-ui` | Snapshot | offen | Kein Proxy-Klick; eigene Menüs mit Rechteprüfung |
| Tastatur-/Fokussteuerung | Tests `modal-focus-keyboard`, `nav-taborder` | alle | `lfx-nav` | Unit | offen | Gemeinsames Fokussystem für Maus, Tastatur, Gamepad (Couch, Abschnitt 17) |

## 2. Startseite (LENZFLIX)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Startseiten-Hülle, Aufbau und Cache | `features/home.js`, `core/homecache.js` | flix | `lfx-store` + `lfx-ui` | Mock | offen | Eigener Cache (SQLite) statt Seiten-Cache |
| Billboard mit Logo, Trailer, Ambient | `features/billboard.js`, `billboard/format.js`, `shared/trailer.js` | flix | `lfx-ui` + `lfx-player` | Mock/Snapshot | offen | Trailer per mpv; Datensparteil `trailer`/`hero` beachten |
| Zeilen: Weiterschauen, Nächste Folge, Zuletzt, Genres, Ähnliche | `features/rows.js`, `rows/sources.js`, `rows/prefetch.js`, `rows/card.js` | flix | `lfx-store` + `lfx-ui` | Mock | offen | Quellen `/UserItems/Resume`, `/Shows/NextUp`, `/Items/Latest`, `/Genres`, `/Items/{id}/Similar` |
| Charts-Zeile mit Rückfall | `core/charts.js`, Test `rows-charts-fallback` | flix | `lfx-store` | Mock | offen | Nur mit konfiguriertem Zusatzdienst, sonst Zeile aus |
| Karten (Poster/Landschaft, Fortschritt, Badges) | `features/rows/card.js`, `shared/newcontent.js`, `shared/trend.js` | alle | `lfx-ui` | Snapshot | offen | „Neu“- und Trend-Badges |
| Hovercard mit Kurzinfo und Aktionen | `features/hovercard.js`, `hovercard/meta.js` | flix | `lfx-ui` | Snapshot | offen | Nur mit Zeiger; Aktionen Play, Liste, Gesehen, Bewertung |
| Detail-Modal (Staffeln, Folgen, Ähnliche) | `features/modal.js`, `modal/data.js`, `modal/render.js` | flix | `lfx-ui` | Mock/Snapshot | offen | Teilt Daten mit Detailseite (Abschnitt 6) |
| „Neu und beliebt“ (`lfx=new`) | `features/newpopular.js` | flix | `lfx-ui` | Mock | offen | Sortierung `DateCreated`, clientseitig filtern (Server ignoriert `MinDateCreated`) |
| Serien-Seite und Filme-Seite (Genre, Raster, Sortierung) | `features/library.js`, `features/library/*` | flix | `lfx-ui` | Mock/Snapshot | offen | Pool-Schlüssel `lib:<kind>:<genre|all>:`, Billboard und Pool nie gleicher Cache-Schlüssel |
| Meine Liste (Typ, Sortierung) | `features/mylist.js` | flix, retro | `lfx-ui` | Mock | offen | `/Items?Filters=IsFavorite`; „Z–A“ als Idee |
| Gesehen/Liste/Bewertung UI-unabhängig | `shared/titleactions.js` | alle | `lfx-api` + `lfx-core` | Mock | offen | `POST/DELETE` Favorit, Played, Rating |
| Download-Aktion mit Rechteprüfung | `core/download.js` | alle | `lfx-offline` | Mock | offen | Nutzt Offline-Modus; Token nie in der URL loggen |

## 3. LENZ+, LENZPRIME, LENZ//84

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Startseite Hero, Zeilen, Marken-Kacheln | `features/plus/home*.js`, `prime/home*.js`, `retro/home*.js`, `*/hero.js`, `*/rows.js`, `shared/collections.js`, `shared/hub.js` | plus, prime, retro | `lfx-ui` (Designs) | Mock/Snapshot | offen | BoxSet-Kacheln aus `/Items` |
| Titel-Vollseite (`lfx=title`) | `features/plus/title.js`, `prime/title.js`, `retro/title.js` | plus, prime, retro | `lfx-ui` | Mock/Snapshot | offen | Gemeinsame Daten mit nativer Detailseite |
| Browse und Hub | `features/plus/browse.js`, `retro/browse.js`, `shared/hub.js` | plus, retro | `lfx-ui` | Mock/Snapshot | offen | Sentinel-Nachladen (Test `retro-browse-sentinel`) |
| Native Detailseite gestylt | `*/nativedetail.js` | plus, prime, retro | `lfx-ui` | Snapshot | offen | Entfällt als eigenes Feature; Detailseite (Abschnitt 6) wird je Design gestylt |
| Vorschau/Kassette beim Überfahren | `features/prime/hover*.js`, `retro/hover.js` | prime, retro | `lfx-ui` | Snapshot | offen | Nur mit Zeiger |
| Design-Suche | `features/plus/search.js`, `prime/search.js`, `retro/search.js`, `shared/search.js` | plus, prime, retro | `lfx-ui` | Mock | offen | Fließt in die native Suche (Abschnitt 7) |
| Retro-Effekte: CRT, Boot, Glitch, Terminal, Cheat-Code, Code-Regen | `features/retro/fx/*` | retro | `lfx-ui` (fx-Modul) | Snapshot/unprüfbar | offen | Aus im Player und im Admin; Wirkung am Bildschirm vom Nutzer beurteilen |
| Retro-Sound (synthetisiert) | `features/retro/fx/sound.js` | retro | `lfx-platform` (Audio) | unprüfbar | offen | Standard aus; Klangqualität nicht automatisch prüfbar |
| Datensparteile je Design | `tests/*-datasaver*.test.js` | alle | `lfx-core` | Unit | offen | Reagiert auf Gesamt- und Teilschalter |

## 4. TV-Modus (IPTV-Seite, Plan bitgleich zum Web)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Planer: Hash, Zeit, Katalog, Pool, Füller, Serien, Premieren, Themen-Tage | `tv/schedule/*` (`hash.js`, `time.js`, `catalog.js`, `pool.js`, `fill.js`, `series.js`, `premiere.js`, `specials.js`, `themeday.js`, `rating.js`, `plan.js`) | alle | `lfx-tv-schedule` | Unit | offen | Deterministisch; Golden-Vektoren aus dem Web (Tests `tv-schedule-*`, `tv-special-*`, `tv-themeday-*`); UTF-16-Einheiten, `Math.imul`, IndexMap, stabile Sortierung, `dedupe` per JSON-Reihenfolge |
| Zeit nur über MEZ/MESZ-Arithmetik | `tv/schedule/time.js` | alle | `lfx-tv-schedule` | Unit | offen | Kein `Intl`, keine Lokalzeit, kein Zufall außer `hash.js` |
| Katalog laden, Cache, Senderbelegung | `features/tvdata/{data,deps,config}.js` | alle | `lfx-tv` + `lfx-store` | Mock | offen | Admin-Senderbelegung Plätze 5–16 (`tvChannels`); IndexedDB-Name `lfx-live` entfällt in der App |
| Kanalliste, EPG-Raster, Info-Karte, Ziffernwahl | `tv/tvmode/{channel-list,epg-grid,info-card,info,digits,keys}.js` | alle | `lfx-tv` + `lfx-ui` | Snapshot/Unit | offen | Retro-TV-Look nur per Styling |
| Controller, Steuerung, Track-Menü, Geschwindigkeit | `tv/tvmode/{controller,controls,track-menu}.js` | alle | `lfx-tv` | Unit/Mock | offen | Tests `tvmode-controller-*` als Vorlage |
| Eigener Player ohne Fortschrittsmeldung | `tv/player/{index,stream,tracks,watch,warm,neighbors}.js` | alle | `lfx-player` (TV-Profil) | Mock/Live | offen | Keine Spuren in „Weiterschauen“; Fallen: kein `StartTimeTicks` an die HLS-URL, `ActiveEncodings` löschen, `ApiKey`, `SubtitleStreamIndex`/`MediaSourceId`-Regeln aus `docs/tvmode.md` |
| HLS-Remux, HEVC nur wenn unterstützt | `tv/player/stream.js` | alle | `lfx-player` | Mock | offen | Transkodierung selbst unprüfbar (Testkonto ohne) |
| Vorwärmen der Nachbarsender, Zap-Statistik | `tv/player/warm.js`, `tv/ui/zapstats.js` | alle | `lfx-tv` | Unit | offen | Datensparteil `tvwarm` |
| Bild-in-Bild, Hintergrundwiedergabe | `tv/tvmode/pip.js` | alle | `lfx-platform` + `lfx-player` | unprüfbar | offen | Fensterverhalten je Plattform; macOS unprüfbar |
| Logos, Vollbild, Overlay-Helfer | `tv/ui/{logos,fullscreen,overlay-util}.js` | alle | `lfx-ui` | Snapshot | offen | Eigene Marken-Logos nur aus dem Repo |
| TV-Symbol/Einstieg in der Kopfleiste | `features/tvbtn.js`, `features/tvmode/*` | alle | `lfx-shell` | Snapshot | offen | Entsperrung des Players entfällt (kein Browser-Gestenzwang) |

## 5. Wiedergabe-Kern und Player-OSD

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| mpv-Einbindung (dynamisches Laden, LGPL), Wiedergabe-Zustand | (Web: jellyfin-web) | alle | `lfx-mpv-sys` + `lfx-player` | Unit/Live | offen | Nur `libloading`, DLL nie im Repo; `xtask fetch-mpv` |
| Direktwiedergabe, Rückfallkette, Fehlercodes | `core/playback.js`, Tests `playback-*` | alle | `lfx-player` | Mock/Live | offen | Kein Proxy-Klick; `PlaybackInfo` und Profil selbst bauen |
| Fortschrittsmeldung `/Sessions/Playing*` | (Web: jellyfin-web) | alle | `lfx-api` + `lfx-player` | Mock | offen | Start, Fortschritt, Stopp; nur nicht im TV-Modus |
| OSD: Fortschrittsleiste, Lautstärke, Play/Pause, Vor/Zurück | (Web: jellyfin-web), Bildmitte `features/osdcenter.js` | alle | `lfx-ui` (Player-OSD) | Snapshot | offen | Bildmitte ⟲10 / Play / ⟳30 als nativer Overlay |
| Tonspur- und Untertitelwahl | (Web: jellyfin-web) | alle | `lfx-player` + `lfx-ui` | Mock | offen | Standard je Benutzer-Einstellungen; Server-Fallen: Brennen vermeiden |
| Untertitel-Rendering (SRT, ASS) | (Web: jellyfin-web) | alle | `lfx-player` (mpv) | Live | offen | mpv rendert; externe Spuren per Stream-URL |
| Kapitel mit Marken und Namen | (Web: jellyfin-web) | alle | `lfx-player` + `lfx-ui` | Mock | offen | Kapitelbilder per `/Items/{id}/Images/Chapter` |
| Trickplay-Vorschau beim Scrubben | (Web: jellyfin-web) | alle | `lfx-player` + `lfx-ui` | Mock | offen | Kacheln aus `/Videos/{id}/Trickplay/...` |
| Intro/Abspann überspringen (Media Segments) | (Web: jellyfin-web) | alle | `lfx-player` | Mock | offen | Segmente per `/MediaSegments/{id}`; Datenlage am Server abhängig |
| Nächste Folge („Up Next“), Autoplay | (Web: jellyfin-web) | alle | `lfx-player` | Mock | offen | Nächste Folge per `/Shows/NextUp` |
| Wiedergabegeschwindigkeit, Seitenverhältnis, Audio-Verzögerung | (Web: jellyfin-web) | alle | `lfx-player` | Unit | offen | Unabhängig vom Server |
| Qualität/Bitrate wählen (Transkodierung) | (Web: jellyfin-web) | alle | `lfx-player` | unprüfbar | offen | Testkonto ohne Transkodierung |
| HDR-Wiedergabe (HDR10, HLG und weitere HDR-Formate) (Gate G3) | – | alle | `lfx-player` + `lfx-mpv-sys` | unprüfbar | offen | Nur mit Nutzer-Freigabe am HDR-fähigen Gerät; Modelle A/B/C laut Plan |
| Hardware-Dekodierung (d3d11va), Bildausgabe in Slint | – | alle | `lfx-player` + `lfx-mpv-sys` | Live | offen | Windows ARM64/x64 live; macOS unprüfbar |
| Bild-in-Bild, Vollbild | (Web: jellyfin-web) | alle | `lfx-platform` | unprüfbar | offen | Fensterverwaltung je Plattform |

## 6. Detailseite

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Film-Detailseite (Backdrop, Logo, Infos, Besetzung, Ähnliche) | `css/modern/detail.css`, `docs/dom-map.md` (Detail) | alle | `lfx-ui` (Detail) | Mock/Snapshot | offen | Eigene Seite statt jellyfin-web |
| Serie: Staffeln, Folgen, Gesehen-Markierung | `features/modal/data.js` | alle | `lfx-ui` | Mock | offen | `/Shows/{id}/Seasons|Episodes` |
| Play/Fortsetzen/Trailer/Zufall/Favorit/Gesehen/Download/Mehr | `shared/titleactions.js`, `core/itemmenu.js` | alle | `lfx-ui` | Mock | offen | Kein Proxy-Klick |
| Ton/Untertitel vor dem Start wählen | `docs/dom-map.md` (`.trackSelections`) | alle | `lfx-ui` + `lfx-player` | Mock | offen | Versionen/MediaSources auswählen |
| Sammlungen (BoxSet), Playlists | `shared/collections.js` | alle | `lfx-ui` | Mock | offen | Details nur aus der API ableitbar (Web nicht abgedeckt) |
| Personenseite, Studio, Genre-Ziele | – | alle | `lfx-ui` | Mock | offen | Web delegiert an jellyfin-web |

## 7. Suche

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Suche mit Sofortergebnissen, Gruppen nach Typ | `shared/search.js`, `css/modern/search.css`, Tests `search`, `shared-search-order` | alle | `lfx-ui` | Mock | offen | `/Items?searchTerm`, `/Persons`, `/Genres`; Reihenfolge laut Tests |
| Verlauf/Vorschläge | – | alle | `lfx-store` | Unit | offen | Lokal, nie an den Server |
| Tastaturkürzel (`/`) | – | alle | `lfx-nav` | Unit | offen | Neu in der App |

## 8. Anmeldung, Server, Benutzer

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Serverwahl und manuelle Adresse | `docs/dom-map.md` (Login) | alle | `lfx-session` + `lfx-ui` | Mock | offen | Eingegebene URL im Log schwärzen |
| LAN-Suche (UDP-Broadcast, Jellyfin-Erkennung) | – | alle | `lfx-session` + `lfx-platform` | Mock/Live | offen | Port 7359; Netzabhängig |
| Anmeldung mit Name/Passwort, „Angemeldet bleiben“ | `css/modern/login.css` | alle | `lfx-session` | Mock | offen | Token im OS-Schlüsselbund, nie in Dateien |
| Quick Connect | – | alle | `lfx-session` | Mock | offen | `/QuickConnect/*` |
| „Wer schaut?“ (mehrere Benutzer je Server) | – | alle | `lfx-session` + `lfx-ui` | Mock/Snapshot | offen | Öffentliche Benutzerliste, Kennwortfrage nach Server-Einstellung |
| Mehrere Server, Umschalten | – | alle | `lfx-session` | Mock | offen | Je Server eigener Schlüsselbund-Eintrag |
| Passwort vergessen, Abmelden | `docs/dom-map.md` | alle | `lfx-session` | Mock | offen | Abmelden löscht Token, Server-Eintrag bleibt |
| Token-/URL-Schwärzung in Logs | – | alle | `lfx-core` | Unit | offen | Pflicht, mit Test |
| Schlüsselbund (Windows Credential Manager, macOS Keychain) | – | alle | `lfx-platform` | Live/unprüfbar | offen | macOS unprüfbar |

## 9. Bibliotheken (Raster)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Bibliotheksübersicht und -raster (Filme, Serien, Sammlungen, Heimvideos, Musikvideos) | `css/modern/library.css`, `shared/libraries.js` | alle | `lfx-ui` | Mock/Snapshot | offen | Eigene Seiten statt `#/list`, `#/mixed` |
| Filter (Genre, Jahr, Gesehen, Favorit, Bewertung u. a.) | – | alle | `lfx-ui` + `lfx-store` | Mock | offen | Über `/Items`-Parameter und `/Filters` |
| Sortierung | – | alle | `lfx-ui` | Mock | offen | Sortierfelder wie jellyfin-web |
| Alphabet-Leiste (`NameStartsWith`) | – | alle | `lfx-ui` | Mock | offen | |
| Seitenweise Nachladen mit virtuellem Raster | – | alle | `lfx-ui` | Unit/Snapshot | offen | Slint-Listen mit Virtualisierung; Leistung Live prüfen |
| Bildcache und Skalierung | `core/imgscale.js` | alle | `lfx-core` | Unit | offen | Plattenkontingent konfigurierbar |

## 10. Benutzer-Einstellungen (nativ)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Einstellungsmenü (Anzeige, Startseite, Wiedergabe, Untertitel, Steuerung, Quick Connect, Profil) | `shared/pagetitle.js` (nur Titel) | alle | `lfx-ui` (Einstellungen) | Snapshot | offen | Web: nur jellyfin-web |
| Wiedergabe-Defaults (Sprache, Audio, Qualität, Autoplay) | – | alle | `lfx-store` + `lfx-player` | Unit | offen | Lokal; Serverseitig lesen, nie in die Web-DisplayPreferences schreiben |
| Untertitel-Defaults und Aussehen | – | alle | `lfx-store` + `lfx-player` | Unit | offen | Größe, Farbe, Hintergrund |
| Startseiten-Abschnitte anpassen | – | alle | `lfx-store` | Unit | offen | Eigene Konfiguration der App |
| Profil (Name, Passwort, Bild) | – | alle | `lfx-ui` + `lfx-api` | Mock | offen | |
| App-Einstellungen (Design, Datensparen, Autostart, Tray, Update) | – | alle | `lfx-core` | Unit | offen | Eigenes Konfig-Format |

## 11. Musik

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Musikbibliothek: Künstler, Alben, Titel, Genres | – (Web nur Link, jellyfin-web) | alle | `lfx-ui` + `lfx-store` | Mock | offen | Web nicht abgedeckt |
| Audio-Player (Mini-Player, Vollbild-Player) | – | alle | `lfx-player` + `lfx-ui` | Mock/Live | offen | Nutzt mpv ohne Video |
| Warteschlange, Zufall, Wiederholen | – | alle | `lfx-player` | Unit | offen | |
| Playlists (anlegen, bearbeiten, abspielen) | – | alle | `lfx-ui` + `lfx-api` | Mock | offen | `/Playlists` |
| Texte (Lyrics) | – | alle | `lfx-ui` | Mock | offen | Nur wenn der Server Lyrics liefert |
| Gapless/Crossfade | – | alle | `lfx-player` | Live | offen | Hörprobe vom Nutzer |

## 12. Bücher und Fotos

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Bücher: Liste, Detail | – | alle | `lfx-ui` | Mock | offen | Web nicht abgedeckt |
| Bücher lesen (EPUB/PDF/CBZ) | – | alle | `lfx-ui` | Unit/Snapshot | offen | Umfang beim Start klären (Lesen vs. Öffnen extern) |
| Fotos: Alben, Raster, Vollbild, Diashow | – | alle | `lfx-ui` | Mock/Snapshot | offen | Web nicht abgedeckt |

## 13. Jellyfin-Live-TV

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Programmführer, Kanäle | `features/nav.js` (nur Link) | alle | `lfx-ui` + `lfx-api` | unprüfbar | offen | Server ohne Tuner; nur Mock |
| Aufnahmen, Timer, Serienaufnahmen | – | alle | `lfx-ui` | unprüfbar | offen | Nur Mock |
| Live-Wiedergabe (HLS-Livestream) | – | alle | `lfx-player` | unprüfbar | offen | Nicht mit dem LENZFLIX-TV-Modus verwechseln (Abschnitt 4) |

## 14. Admin (häufige Aufgaben nativ, Rest im Browser)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Übersicht (Server, Version, Aktivität) | – (Theme aus auf Dashboard) | alle | `lfx-ui` (Admin) | Mock | offen | Nur für Administratoren sichtbar; Retro-Effekte aus |
| Laufende Sitzungen, Fernsteuern, Beenden | – | alle | `lfx-ui` + `lfx-api` | Mock | offen | `/Sessions` |
| Benutzer anlegen/bearbeiten/Rechte | – | alle | `lfx-ui` | Mock | offen | |
| Bibliotheken scannen, Metadaten aktualisieren | – | alle | `lfx-ui` | Mock | offen | `/Library/Refresh` |
| Geplante Aufgaben (anzeigen, starten) | – | alle | `lfx-ui` | Mock | offen | `/ScheduledTasks` |
| Logs ansehen | – | alle | `lfx-ui` | Mock | offen | Tokens/URLs in angezeigten Logs schwärzen |
| Alles andere (Plugins, Netzwerk, Transkodierung, …) | – | alle | `lfx-shell` | unprüfbar | offen | Knopf „Im Browser öffnen“ (`/web/#/dashboard`) |

## 15. Offline und Downloads

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Titel/Staffel laden mit Fortschritt, Pause, Abbruch | `core/download.js` (nur Link) | alle | `lfx-offline` | Mock | offen | Download-Manager neu; `ApiKey` nie loggen |
| Offline-Bibliothek und Offline-Wiedergabe | – | alle | `lfx-offline` + `lfx-player` | Mock/Live | offen | Metadaten, Bilder, Untertitel mitspeichern |
| Fortschritt offline sammeln, später synchronisieren | – | alle | `lfx-offline` + `lfx-api` | Mock | offen | Konfliktregel: neuester Stand gewinnt |
| Speicherplatz-Verwaltung, Löschen | – | alle | `lfx-offline` | Unit | offen | Kontingent in den Einstellungen |

## 16. Betriebssystem-Integration

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Medientasten, SMTC (Windows), Now Playing (macOS) | – | alle | `lfx-platform` | Live (Windows) / unprüfbar (macOS) | offen | Titel, Cover, Fortschritt |
| Tray-Symbol (Windows/macOS Menüleiste) | – | alle | `lfx-platform` | Live (Windows) / unprüfbar (macOS) | offen | Minimieren in den Tray, Schnellaktionen |
| Auto-Update (Minisign-Signatur) | – | alle | `lfx-platform` + `xtask` | Unit/Mock | offen | Unsigniert (macOS ad-hoc); Signaturprüfung als Unit-Test |
| Installer NSIS (Windows), DMG (macOS) | – | alle | `xtask` + CI | unprüfbar (macOS) | offen | Windows-Installer live, DMG nur CI |
| Autostart, Einzelinstanz, Dateizuordnung | – | alle | `lfx-platform` | Live | offen | |
| Eigene Titelleiste, Fensterzustand | – | alle | `lfx-shell` | Live | offen | Name „LENZFLIX“; macOS-Ampel unprüfbar |
| Über-Dialog mit AboutSlint | – | alle | `lfx-ui` | Snapshot | offen | Pflicht laut Slint-Lizenz |
| HiDPI/90 Hz-Darstellung | – | alle | `lfx-shell` | Live | offen | Dev-Rechner 2944×1840, DPR 2 |

## 17. Fernsteuerung, SyncPlay, Couch-Modus

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Fernsteuerung (App als Ziel anderer Clients) | – | alle | `lfx-session` + `lfx-player` | Mock | offen | WebSocket `/socket`, Allgemeine Befehle |
| Andere Geräte steuern (Cast-Menü) | `docs/dom-map.md` (AppBar-Knopf) | alle | `lfx-ui` + `lfx-api` | Mock | offen | |
| SyncPlay (Gruppe erstellen, beitreten, synchron) | `docs/dom-map.md` (Menü `#app-sync-play-menu`) | alle | `lfx-session` + `lfx-player` | Mock | offen | Nichts mit dem VIDEOTHEK-Mitschauen zu tun |
| Couch-Modus: Gamepad-Navigation, große Ziele | `css/legacy-tv/tv.css` (Look) | alle | `lfx-nav` + `lfx-ui` | Unit/Snapshot | offen | Alle Designs steuerbar, Look immer LENZFLIX-TV; Controller-Hardware nur Live durch Nutzer |
| Bildschirmtastatur im Couch-Modus | – | alle | `lfx-ui` | Snapshot | offen | |

## 18. Statistik und Charts

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Charts und Trend-Badges | `core/charts.js`, `shared/trend.js`, `docs/charts-api.md` | alle | `lfx-store` + `lfx-ui` | Mock | offen | Zusatzdienst per Adresse in den Einstellungen; ohne Dienst aus |
| Statistik-Seite nativ (Verlauf, Heatmap, Listen) | `core/stats.js`, `features/statshint.js`, `docs/stats-api.md`, Tests `stats-page*` | alle | `lfx-ui` + `lfx-api` | Mock/Snapshot | offen | Nativ statt externe Seite; Heatmap-Rendering selbst |
| Statistik-Hinweis | `features/statshint.js` | flix | `lfx-ui` | Snapshot | offen | |

## 19. VIDEOTHEK LENZ (store)

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Slot im Design-Picker (nicht anwählbar, Hinweis) | `features/store/*` | store | `lfx-3d` | Snapshot | offen | Slot vorbereitet; kein 3D, kein VR, kein Mehrspieler, kein Wetter |

## 20. Qualität und Betrieb

| Funktion | Web-Quelle | Design(s) | App-Ort | Prüfebene | Status | Bemerkung |
|---|---|---|---|---|---|---|
| Mock-Jellyfin-Server mit synthetischen Fixtures | – | – | `lfx-mock-jellyfin` | Mock | offen | Erfundene Titel, generierte Platzhalter-Cover |
| Golden-Vektoren aus dem Web-Planer | `tests/tv-schedule-*.test.js` | – | `lfx-tv-schedule` | Unit | offen | Erzeugt mit dem Web-Code, im Repo als JSON |
| CI (Windows ARM64/x64, macOS arm64) | – | – | `xtask` + Workflows | unprüfbar (macOS) | offen | macOS nur Bauen/Testen |
| Fehlerprotokoll mit Schwärzung | – | – | `lfx-core` | Unit | offen | |
