//! Suche: Suchfeld mit Live-Ergebnissen (Verzögerung 250 ms), Ergebniszeilen je Art, zuletzt gesucht, Genre- und
//! Bibliotheks-Chips sowie eine einfache Bildschirmtastatur für den Couch-Betrieb.
//!
//! Abweichung vom Auftrag: die Treffer kommen wie im Web aus `/Items` (Filme, Serien, Folgen, Sammlungen, Musik) und
//! `/Persons`, nicht aus `/Search/Hints`, weil die Karten volle Elemente (Bild, Jahr) brauchen.

use std::rc::Rc;
use std::time::Duration;

use lfx_api::{BaseItem, Client, ItemKind, ItemsQuery};
use lfx_core::images::{
    CardKind, ImageRef, Slot, landscape_for, own_image, pick_poster, pixel_width,
};
use lfx_core::rows::logic::base_query;
use lfx_images::Priority;
use lfx_nav::{FocusGraph, InputContext, Rect, Target, Zone, ZoneId};
use lfx_ui::{CardAspect, CardData, LibChip, LibraryState, RowData};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::app::{App, post};
use crate::library::{LibGeo, LibRow, Mode, Z_CHIP0, Z_FIELD, Z_KBD, Z_KBD2, Z_ROW0, card_data};
use crate::route::{LibraryView, Route};

const MIN_LEN: usize = 2;
const RECENT_KEY: &str = "lib.recent_searches";
const RECENT_MAX: usize = 8;
const DEBOUNCE_MS: u64 = 250;

/// Tasten der Bildschirmtastatur (0..40 Zeichen, 40 Leerzeichen, 41 Löschen).
pub fn keyboard_keys() -> Vec<String> {
    let mut v: Vec<String> = ('A'..='Z').map(|c| c.to_string()).collect();
    v.extend(('0'..='9').map(|c| c.to_string()));
    v.extend(["Ä", "Ö", "Ü", "ß"].iter().map(|s| (*s).to_owned()));
    v.push("Leerzeichen".to_owned());
    v.push("Löschen".to_owned());
    v
}

/// Was ein Chip der Startansicht tut.
#[derive(Clone, Debug)]
pub enum ChipAct {
    Term(String),
    Go(Route),
}

#[derive(Default)]
pub struct SearchState {
    pub recent: Vec<String>,
    pub term: String,
    /// Zählt Suchläufe; veraltete Antworten werden verworfen.
    pub gen_id: u64,
    pub chips: Vec<(i32, String, ChipAct)>,
    pub kbd_open: bool,
    pub timer: slint::Timer,
    pub sections: Vec<String>,
}

struct SearchData {
    movies: Vec<BaseItem>,
    series: Vec<BaseItem>,
    episodes: Vec<BaseItem>,
    people: Vec<BaseItem>,
    boxsets: Vec<BaseItem>,
    music: Vec<BaseItem>,
}

async fn fetch_search(client: &Client, term: &str) -> SearchData {
    let q = |kinds: &[ItemKind], limit: u32| {
        base_query()
            .search_term(term)
            .include_types(kinds)
            .recursive(true)
            .limit(limit)
    };
    let get = |query: ItemsQuery| {
        let c = client.clone();
        async move { c.items(&query).await.map(|r| r.items).unwrap_or_default() }
    };
    let people_query = ItemsQuery::new()
        .search_term(term)
        .limit(8)
        .fields(&["PrimaryImageAspectRatio"])
        .image_type_limit(1);
    let c2 = client.clone();
    let (movies, series, episodes, boxsets, music, people) = tokio::join!(
        get(q(&[ItemKind::Movie], 40)),
        get(q(&[ItemKind::Series], 40)),
        get(q(&[ItemKind::Episode], 12)),
        get(q(&[ItemKind::BoxSet], 12)),
        get(q(
            &[ItemKind::MusicAlbum, ItemKind::MusicArtist, ItemKind::Audio],
            12
        )),
        async move {
            c2.persons(&people_query)
                .await
                .map(|r| r.items)
                .unwrap_or_default()
        },
    );
    SearchData {
        movies,
        series,
        episodes,
        people,
        boxsets,
        music,
    }
}

fn card_ref(item: &BaseItem, wide: bool) -> Option<ImageRef> {
    if wide {
        let (pick, _) = landscape_for(item)?;
        return Some(ImageRef {
            pick,
            max_width: pixel_width(Slot::CardLandscape, false, 2.0, 1.0),
            quality: Slot::CardLandscape.quality(),
        });
    }
    let pick = pick_poster(item).or_else(|| own_image(item, lfx_api::ImageType::Primary))?;
    Some(ImageRef {
        pick,
        max_width: pixel_width(Slot::CardPoster, false, 2.0, 1.0),
        quality: Slot::CardPoster.quality(),
    })
}

impl App {
    fn with_search<R>(&self, f: impl FnOnce(&mut SearchState) -> R) -> R {
        f(&mut self.st.borrow_mut().lib.search)
    }

    // ------------------------------------------------------------ Einstieg

    /// Baut die Suchseite auf. Gibt den Eingabekontext zurück (Textfeld hat zuerst den Fokus).
    pub fn search_enter(self: &Rc<Self>, epoch: u64) -> InputContext {
        let recent = self.search_load_recent();
        self.with_search(|s| {
            s.recent = recent;
            s.term.clear();
            s.gen_id += 1;
            s.chips.clear();
            s.sections.clear();
            s.kbd_open = false;
        });
        self.with_lib(|g| {
            g.set_query("".into());
            g.set_loading(false);
            g.set_kbd_keys(ModelRc::new(VecModel::from(
                keyboard_keys()
                    .iter()
                    .map(|k| SharedString::from(k.as_str()))
                    .collect::<Vec<_>>(),
            )));
        });
        self.search_rebuild_chips();
        self.search_load_chips(epoch);
        self.st.borrow_mut().graph = FocusGraph::new();
        self.lib_focus_field_later();
        InputContext::TextInput
    }

    fn search_load_recent(&self) -> Vec<String> {
        let Some(a) = self.session.active() else {
            return Vec::new();
        };
        self.store
            .get_setting(&a.server_id, &a.user_id, RECENT_KEY)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
            .unwrap_or_default()
    }

    fn search_remember(self: &Rc<Self>, term: &str) {
        let term = term.trim().to_owned();
        if term.chars().count() < MIN_LEN {
            return;
        }
        let list = self.with_search(|s| {
            s.recent.retain(|x| !x.eq_ignore_ascii_case(&term));
            s.recent.insert(0, term);
            s.recent.truncate(RECENT_MAX);
            s.recent.clone()
        });
        self.save_setting(RECENT_KEY, serde_json::json!(list));
    }

    /// Genres und Bibliotheken für die Startansicht laden.
    fn search_load_chips(self: &Rc<Self>, epoch: u64) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let (genres, views) = tokio::join!(
                client.genres(None, &[ItemKind::Movie, ItemKind::Series]),
                client.views()
            );
            let genres: Vec<String> = genres
                .map(|g| g.items.into_iter().map(|i| i.name).collect())
                .unwrap_or_default();
            let views: Vec<BaseItem> = views.map(|v| v.items).unwrap_or_default();
            post(move |app| app.search_chips_loaded(epoch, genres, views));
        });
    }

    fn search_chips_loaded(self: &Rc<Self>, epoch: u64, genres: Vec<String>, views: Vec<BaseItem>) {
        if self.st.borrow().lib.epoch != epoch {
            return;
        }
        let mut chips: Vec<(i32, String, ChipAct)> = Vec::new();
        for g in genres.into_iter().take(8) {
            chips.push((1, g.clone(), ChipAct::Term(g)));
        }
        for v in views {
            let route = match v.collection_type.as_deref() {
                Some("movies") => Route::Library {
                    kind: lfx_core::library::LibraryKind::Movies,
                    genre: None,
                    view: LibraryView::Rows,
                    sort: lfx_core::library::LibSort::Az,
                },
                Some("tvshows") => Route::Library {
                    kind: lfx_core::library::LibraryKind::Series,
                    genre: None,
                    view: LibraryView::Rows,
                    sort: lfx_core::library::LibSort::Az,
                },
                _ => Route::Collection {
                    id: v.id.clone(),
                    name: v.name.clone(),
                },
            };
            chips.push((2, v.name, ChipAct::Go(route)));
        }
        let recent: Vec<(i32, String, ChipAct)> = self.with_search(|s| {
            s.recent
                .iter()
                .map(|r| (0, r.clone(), ChipAct::Term(r.clone())))
                .collect()
        });
        let all: Vec<_> = recent.into_iter().chain(chips).collect();
        self.with_search(|s| s.chips = all);
        self.search_rebuild_chips();
        self.lib_rebuild_graph();
    }

    fn search_rebuild_chips(self: &Rc<Self>) {
        let chips = self.with_search(|s| s.chips.clone());
        let mut model: Vec<LibChip> = Vec::new();
        let mut pos = [0i32; 3];
        let recent: Vec<String> = self.with_search(|s| s.recent.clone());
        // Zuletzt gesucht kommt immer aus dem aktuellen Stand, nicht aus dem Ladezustand.
        let list: Vec<(i32, String)> = recent
            .iter()
            .map(|r| (0, r.clone()))
            .chain(
                chips
                    .iter()
                    .filter(|c| c.0 != 0)
                    .map(|c| (c.0, c.1.clone())),
            )
            .collect();
        for (g, label) in list {
            let p = pos[g as usize];
            pos[g as usize] += 1;
            model.push(LibChip {
                label: label.as_str().into(),
                group: g,
                pos: p,
            });
        }
        let mask = model.iter().fold(0, |m, c| m | (1 << c.group));
        self.with_lib(|l| {
            l.set_chips(ModelRc::new(VecModel::from(model)));
            l.set_chips_mask(mask);
        });
    }

    // ------------------------------------------------------------ Eingabe

    /// Setzt den Suchtext (Automation, Tastatur, Chips) und startet die Suche mit Verzögerung.
    pub fn search_set_text(self: &Rc<Self>, text: &str) {
        self.with_lib(|g| g.set_query(text.into()));
        self.search_query_changed();
    }

    fn search_query_changed(self: &Rc<Self>) {
        let term = self.with_lib(|g| g.get_query().to_string());
        let weak = Rc::downgrade(self);
        self.st.borrow().lib.search.timer.stop();
        if term.trim().chars().count() < MIN_LEN {
            self.search_clear_results();
            return;
        }
        self.st.borrow().lib.search.timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(DEBOUNCE_MS),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.search_run();
                }
            },
        );
    }

    fn search_clear_results(self: &Rc<Self>) {
        self.with_search(|s| {
            s.gen_id += 1;
            s.term.clear();
            s.sections.clear();
        });
        {
            let mut st = self.st.borrow_mut();
            st.lib.rows.clear();
        }
        self.with_lib(|g| {
            g.set_rows(ModelRc::new(VecModel::<RowData>::default()));
            g.set_loading(false);
            g.set_empty_title("".into());
        });
        self.lib_rebuild_graph();
    }

    fn search_run(self: &Rc<Self>) {
        let term = self
            .with_lib(|g| g.get_query().to_string())
            .trim()
            .to_owned();
        if term.chars().count() < MIN_LEN {
            return;
        }
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        let gen_id = self.with_search(|s| {
            s.gen_id += 1;
            s.term = term.clone();
            s.gen_id
        });
        self.with_lib(|g| g.set_loading(true));
        self.rt.spawn(async move {
            let data = fetch_search(&client, &term).await;
            post(move |app| app.search_loaded(gen_id, data));
        });
    }

    fn search_loaded(self: &Rc<Self>, gen_id: u64, data: SearchData) {
        if self.with_search(|s| s.gen_id) != gen_id {
            return;
        }
        self.with_lib(|g| g.set_loading(false));
        let now = App::now_ms();
        // (Titel, Elemente, breit?)
        let defs: Vec<(&str, Vec<BaseItem>, bool)> = vec![
            ("Filme", data.movies, false),
            ("Serien", data.series, false),
            ("Folgen", data.episodes, true),
            ("Personen", data.people, false),
            ("Sammlungen", data.boxsets, false),
            ("Musik", data.music, false),
        ];
        let mut rows: Vec<(LibRow, bool)> = Vec::new();
        for (title, items, wide) in defs {
            if items.is_empty() {
                continue;
            }
            let cards: Vec<CardData> = items
                .iter()
                .map(|it| card_data(it, CardKind::Landscape, None, None, now))
                .collect();
            rows.push((
                LibRow {
                    id: format!("search:{title}"),
                    title: title.to_owned(),
                    kind: CardKind::Landscape,
                    aspect: if wide {
                        CardAspect::Wide
                    } else {
                        CardAspect::Poster
                    },
                    items,
                    model: Rc::new(VecModel::from(cards)),
                },
                wide,
            ));
        }
        let row_data: Vec<RowData> = rows
            .iter()
            .map(|(r, _)| RowData {
                id: r.id.as_str().into(),
                title: r.title.as_str().into(),
                aspect: r.aspect,
                items: ModelRc::from(r.model.clone()),
            })
            .collect();
        let empty = rows.is_empty();
        let term = self.with_search(|s| s.term.clone());
        self.with_lib(|g| {
            g.set_rows(ModelRc::new(VecModel::from(row_data)));
            g.set_empty_title(if empty {
                format!("Keine Treffer für „{term}“").as_str().into()
            } else {
                "".into()
            });
        });
        self.with_search(|s| s.sections = rows.iter().map(|(r, _)| r.title.clone()).collect());
        // Bilder (Poster bzw. Querformat je Zeile)
        let epoch = self.st.borrow().lib.epoch;
        let mut jobs: Vec<(usize, usize, ImageRef)> = Vec::new();
        for (r, (row, wide)) in rows.iter().enumerate() {
            for (i, it) in row.items.iter().enumerate() {
                if let Some(rf) = card_ref(it, *wide) {
                    jobs.push((r, i, rf));
                }
            }
        }
        self.st.borrow_mut().lib.rows = rows.into_iter().map(|(r, _)| r).collect();
        for (r, i, rf) in jobs {
            self.lib_fetch_image(&rf, Priority::Visible, move |app, img| {
                if app.st.borrow().lib.epoch != epoch || app.with_search(|s| s.gen_id) != gen_id {
                    return;
                }
                let model = app.st.borrow().lib.rows.get(r).map(|x| x.model.clone());
                if let Some(m) = model
                    && let Some(mut c) = m.row_data(i)
                {
                    c.image = img;
                    c.has_image = true;
                    m.set_row_data(i, c);
                }
            });
        }
        self.lib_rebuild_graph();
    }

    // ------------------------------------------------------------ Fokus

    /// Fokuszonen der Suchseite (die Navigationszone liegt schon in `g`).
    pub fn search_graph(&self, mut g: FocusGraph, geo: &LibGeo) -> FocusGraph {
        let st = self.st.borrow();
        let s = &st.lib.search;
        let fw = (geo.width - 2.0 * geo.gutter - 150.0).min(600.0);
        g.register_zone(
            Zone::horizontal(
                Z_FIELD,
                Rect::new(geo.gutter, geo.nav_h + 16.0, fw + 142.0, 44.0),
                2,
            )
            .with_items(vec![
                Rect::new(geo.gutter, geo.nav_h + 16.0, fw, 44.0),
                Rect::new(
                    geo.gutter + fw + 12.0,
                    geo.nav_h + 12.0,
                    130.0,
                    Self::btn_h(),
                ),
            ])
            .remembering(true),
        );
        let mut y_res = geo.nav_h + 76.0;
        if s.kbd_open {
            let kw = (geo.width - 2.0 * geo.gutter).min(760.0) / 10.0;
            let items: Vec<Rect> = (0..40)
                .map(|i| {
                    Rect::new(
                        geo.gutter + (i % 10) as f32 * kw,
                        y_res + (i / 10) as f32 * 44.0,
                        kw - 6.0,
                        38.0,
                    )
                })
                .collect();
            g.register_zone(
                Zone::grid(
                    Z_KBD,
                    Rect::new(geo.gutter, y_res, kw * 10.0, 176.0),
                    40,
                    10,
                )
                .with_items(items)
                .remembering(true),
            );
            let sp = vec![
                Rect::new(geo.gutter, y_res + 176.0, kw * 5.0 - 6.0, 38.0),
                Rect::new(geo.gutter + kw * 5.0, y_res + 176.0, kw * 5.0 - 6.0, 38.0),
            ];
            g.register_zone(
                Zone::horizontal(
                    Z_KBD2,
                    Rect::new(geo.gutter, y_res + 176.0, kw * 10.0, 38.0),
                    2,
                )
                .with_items(sp),
            );
            y_res += 236.0;
        }
        let has_query = !self.lib_query_text().is_empty();
        if has_query {
            for (r, row) in st.lib.rows.iter().enumerate() {
                let y = y_res + r as f32 * geo.row_h + 36.0;
                let items: Vec<Rect> = (0..row.items.len())
                    .map(|i| {
                        Rect::new(
                            geo.gutter + i as f32 * (geo.card_w + geo.gap),
                            y,
                            geo.card_w,
                            geo.card_h,
                        )
                    })
                    .collect();
                g.register_zone(
                    Zone::horizontal(
                        ZoneId(Z_ROW0 + r as u32),
                        Rect::new(
                            geo.gutter,
                            y,
                            (items.len() as f32 * (geo.card_w + geo.gap)).max(1.0),
                            geo.card_h,
                        ),
                        items.len(),
                    )
                    .with_items(items)
                    .remembering(true),
                );
            }
        } else {
            // Chips je Gruppe (Zuletzt gesucht, Genres, Bibliotheken)
            let mut counts = [0usize; 3];
            let recent_n = s.recent.len();
            counts[0] = recent_n;
            for c in s.chips.iter().filter(|c| c.0 != 0) {
                counts[c.0 as usize] += 1;
            }
            let labels: [Vec<String>; 3] = [
                s.recent.clone(),
                s.chips
                    .iter()
                    .filter(|c| c.0 == 1)
                    .map(|c| c.1.clone())
                    .collect(),
                s.chips
                    .iter()
                    .filter(|c| c.0 == 2)
                    .map(|c| c.1.clone())
                    .collect(),
            ];
            let mut y = y_res + 8.0;
            for (gi, list) in labels.iter().enumerate() {
                if list.is_empty() {
                    continue;
                }
                y += 32.0;
                let mut x = geo.gutter;
                let items: Vec<Rect> = list
                    .iter()
                    .map(|l| {
                        let w = l.chars().count() as f32 * 8.0 + 32.0;
                        let r = Rect::new(x, y, w, 34.0);
                        x += w + 10.0;
                        r
                    })
                    .collect();
                g.register_zone(
                    Zone::horizontal(
                        ZoneId(Z_CHIP0 + gi as u32),
                        Rect::new(geo.gutter, y, (x - geo.gutter).max(1.0), 34.0),
                        items.len(),
                    )
                    .with_items(items)
                    .remembering(true),
                );
                y += 52.0;
            }
        }
        g
    }

    fn btn_h() -> f32 {
        52.0
    }

    fn lib_query_text(&self) -> String {
        self.with_lib(|g| g.get_query().to_string())
    }

    /// Fokus auf die Ergebnisse bzw. Chips setzen. `true`, wenn es etwas gab.
    fn search_focus_results(self: &Rc<Self>) -> bool {
        let has_query = !self.lib_query_text().is_empty();
        let order: Vec<ZoneId> = if has_query {
            vec![ZoneId(Z_ROW0)]
        } else {
            (0..3).map(|g| ZoneId(Z_CHIP0 + g)).collect()
        };
        let moved = {
            let mut st = self.st.borrow_mut();
            order.into_iter().any(|z| st.graph.focus_zone(z))
        };
        if moved {
            self.st.borrow_mut().ctx = InputContext::Browse;
            self.win().invoke_focus_root();
            self.lib_show_focus();
        }
        moved
    }

    /// Zurück im Suchfeld: erst zu den Ergebnissen, dann erst weg von der Seite.
    pub fn search_back(self: &Rc<Self>) -> bool {
        let in_field = self
            .st
            .borrow()
            .graph
            .focused()
            .is_some_and(|t| t.zone == Z_FIELD && t.index == 0);
        if in_field {
            return self.search_focus_results();
        }
        false
    }

    // ------------------------------------------------------------ Aktivierung

    pub fn search_activate(self: &Rc<Self>, t: Target) {
        if t.zone == Z_FIELD {
            if t.index == 1 {
                self.search_toggle_kbd();
            } else {
                // Enter im Feld: sofort suchen und zu den Ergebnissen
                let term = self.lib_query_text();
                self.st.borrow().lib.search.timer.stop();
                if term.trim().chars().count() >= MIN_LEN {
                    self.search_run();
                }
                self.search_focus_results();
            }
        } else if t.zone == Z_KBD || t.zone == Z_KBD2 {
            let idx = if t.zone == Z_KBD2 {
                40 + t.index
            } else {
                t.index
            };
            self.search_key(idx);
        } else if t.zone.0 >= Z_CHIP0 {
            self.search_chip((t.zone.0 - Z_CHIP0) as i32, t.index);
        } else if let Some(r) = t.zone.0.checked_sub(Z_ROW0).map(|r| r as usize) {
            let item = self
                .st
                .borrow()
                .lib
                .rows
                .get(r)
                .and_then(|row| row.items.get(t.index).cloned());
            if let Some(item) = item {
                let term = self.with_search(|s| s.term.clone());
                self.search_remember(&term);
                self.lib_open_detail(&item);
            }
        }
    }

    fn search_toggle_kbd(self: &Rc<Self>) {
        let open = self.with_search(|s| {
            s.kbd_open = !s.kbd_open;
            s.kbd_open
        });
        self.with_lib(|g| g.set_kbd_open(open));
        self.lib_rebuild_graph();
    }

    fn search_key(self: &Rc<Self>, idx: usize) {
        let keys = keyboard_keys();
        let mut text = self.lib_query_text();
        match idx {
            40 => text.push(' '),
            41 => {
                text.pop();
            }
            i => {
                if let Some(k) = keys.get(i) {
                    text.push_str(&k.to_lowercase());
                }
            }
        }
        self.search_set_text(&text);
    }

    fn search_chip(self: &Rc<Self>, group: i32, pos: usize) {
        let act = if group == 0 {
            self.with_search(|s| s.recent.get(pos).cloned().map(ChipAct::Term))
        } else {
            self.with_search(|s| {
                s.chips
                    .iter()
                    .filter(|c| c.0 == group)
                    .nth(pos)
                    .map(|c| c.2.clone())
            })
        };
        match act {
            Some(ChipAct::Term(t)) => {
                self.search_set_text(&t);
                // sofort suchen
                self.st.borrow().lib.search.timer.stop();
                self.search_run();
            }
            Some(ChipAct::Go(r)) => self.go(r),
            None => {}
        }
    }

    /// Ereignisse der Suchoberfläche.
    pub fn search_event(self: &Rc<Self>, kind: &str, a: usize, b: usize) {
        match kind {
            "query" => self.search_query_changed(),
            "field" => self.search_activate(Target {
                zone: Z_FIELD,
                index: 0,
            }),
            "kbd-toggle" => self.search_toggle_kbd(),
            "key" => self.search_key(a),
            "chip" => self.search_chip(a as i32, b),
            _ => {}
        }
    }

    pub fn search_state_json(&self) -> serde_json::Value {
        let st = self.st.borrow();
        let s = &st.lib.search;
        let ui = self.win();
        let g = ui.global::<LibraryState>();
        serde_json::json!({
            "term": s.term,
            "query": g.get_query().to_string(),
            "sections": s.sections,
            "section_counts": st.lib.rows.iter().map(|r| r.items.len()).collect::<Vec<_>>(),
            "recent": s.recent,
            "chips": s.chips.iter().map(|c| c.1.clone()).collect::<Vec<_>>(),
            "kbd_open": s.kbd_open,
            "active": st.lib.mode == Mode::Search,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_has_40_chars_and_two_specials() {
        let k = keyboard_keys();
        assert_eq!(k.len(), 42);
        assert_eq!(k[40], "Leerzeichen");
        assert_eq!(k[41], "Löschen");
        assert_eq!(k[0], "A");
        assert_eq!(k[26], "0");
    }
}
