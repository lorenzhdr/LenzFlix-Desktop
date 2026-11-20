//! W2-detail: Detail-Modal, Detailseite (Route `Detail{id}`), Kontextmenü und Aktionen (Favorit, Gesehen, Bewertung).
//!
//! Modal und Seite teilen sich den Inhalt ([`DetailCtl`] und `ui/pages/detail-view.slint`): Daten laden
//! (Titel, Staffeln, Folgen, Ähnliche bzw. Sammlung/Filmografie), in Slint-Strukturen abbilden, Fokuszonen
//! (`detail_logic::detail_zones`) und Aktionen. Das Modal legt seine Zonen mit Fokusfalle (`push_trap`) auf den
//! Graphen der darunterliegenden Seite, die Seite hat einen eigenen Graphen.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use lfx_api::{ApiError, BaseItem, Client, ItemKind, ItemsQuery, SortBy, UserItemData};
use lfx_core::billboard::{format_rating, format_runtime, hero_images};
use lfx_core::images::{CardKind, card_images, display_title};
use lfx_images::Priority;
use lfx_nav::{FocusGraph, NavAction, NavOutcome, Target};
use lfx_store::NewOutboxEntry;
use lfx_ui::{
    DetailData, DetailState as Ui, EpisodeData, MainWindow, MenuEntry, PersonChip, SimilarCard,
    ToastKind,
};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::app::{App, post};
use crate::detail_logic::{
    self as logic, DetailCounts, MenuAction, MenuItem, SEC_ACT, SEC_EPS, SEC_LINK, SEC_PEOPLE,
    SEC_SEASON, SEC_SIM, SEC_TRACKS, SECTION_COUNT,
};
use crate::hovercard::HoverCtl;
use crate::route::Route;

const ITEM_FIELDS: [&str; 17] = [
    "Overview",
    "Genres",
    "People",
    "Studios",
    "Taglines",
    "OfficialRating",
    "ProductionYear",
    "RunTimeTicks",
    "ChildCount",
    "RecursiveItemCount",
    "RemoteTrailers",
    "MediaSources",
    "MediaStreams",
    "Width",
    "ImageTags",
    "BackdropImageTags",
    "UserData",
];
const LIST_FIELDS: [&str; 6] = [
    "Overview",
    "UserData",
    "ImageTags",
    "RunTimeTicks",
    "ProductionYear",
    "OfficialRating",
];
/// Lebensdauer geladener Titel im Zwischenspeicher (Web: `ITEM_TTL`).
pub const ITEM_TTL: Duration = Duration::from_secs(5 * 60);

/// Wo der Detailinhalt gerade gezeigt wird.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    #[default]
    None,
    Modal,
    Page,
}

/// Vorauswahl für den nächsten Start (Ton, Untertitel, Version, „von Anfang an“).
#[derive(Clone, Debug, Default)]
pub struct PlayChoice {
    pub from_start: bool,
    pub source: Option<String>,
    pub audio: Option<i32>,
    pub sub: Option<i32>,
}

/// Geöffnetes Kontextmenü.
pub struct MenuCtl {
    pub item: BaseItem,
    pub entries: Vec<MenuItem>,
    pub focus: usize,
}

/// Zustand von Detail, Menü und Hovercard (hängt als `State::detail` an der App).
#[derive(Default)]
pub struct DetailCtl {
    pub surface: Surface,
    pub generation: u64,
    pub item: Option<BaseItem>,
    pub seasons: Vec<BaseItem>,
    pub season_idx: usize,
    pub episodes: Vec<BaseItem>,
    pub ep_cache: HashMap<String, Vec<BaseItem>>,
    pub selected_ep: Option<String>,
    pub similar: Vec<BaseItem>,
    pub similar_title: String,
    pub next_up: Option<BaseItem>,
    pub can_download: Option<bool>,
    pub src_idx: usize,
    pub audio: Option<i32>,
    pub sub: Option<i32>,
    pub backdrop: Option<Image>,
    pub logo: Option<Image>,
    pub act_kinds: Vec<i32>,
    pub track_kinds: Vec<i32>,
    pub season_kinds: Vec<i32>,
    pub ep_model: Option<Rc<VecModel<EpisodeData>>>,
    pub sim_model: Option<Rc<VecModel<SimilarCard>>>,
    pub menu: Option<MenuCtl>,
    pub choice: Option<PlayChoice>,
    pub cache: HashMap<String, (Instant, BaseItem)>,
    pub season_list_open: bool,
    pub hover: HoverCtl,
}

/// Ergebnis des Ladens (nur `Send`-Typen).
struct Loaded {
    item: BaseItem,
    seasons: Vec<BaseItem>,
    season_idx: usize,
    episodes: Vec<BaseItem>,
    selected_ep: Option<String>,
    similar: Vec<BaseItem>,
    similar_title: &'static str,
    next_up: Option<BaseItem>,
    can_download: bool,
}

fn err_text(e: &ApiError) -> String {
    match e {
        ApiError::NotFound => "Der Titel wurde auf dem Server nicht gefunden.".into(),
        ApiError::Network(_) | ApiError::Timeout => {
            "Keine Verbindung zum Server. Bitte später erneut versuchen.".into()
        }
        _ => "Der Titel konnte nicht geladen werden.".into(),
    }
}

async fn fetch_detail(client: &Client, id: &str, need_policy: bool) -> Result<Loaded, String> {
    let mut item = client
        .item(id, &ITEM_FIELDS)
        .await
        .map_err(|e| err_text(&e))?;
    let mut preferred: Option<String> = None;
    let mut selected: Option<String> = None;
    // Folgen und Staffeln öffnen die Serie (wie das Modal der Web-Vorlage).
    if matches!(item.kind, ItemKind::Episode | ItemKind::Season)
        && let Some(sid) = item.series_id.clone()
    {
        preferred = if item.kind == ItemKind::Season {
            Some(item.id.clone())
        } else {
            item.season_id.clone()
        };
        if item.kind == ItemKind::Episode {
            selected = Some(item.id.clone());
        }
        item = client
            .item(&sid, &ITEM_FIELDS)
            .await
            .map_err(|e| err_text(&e))?;
    }
    let mut seasons = Vec::new();
    let mut season_idx = 0;
    let mut episodes = Vec::new();
    let similar;
    let mut similar_title = "Ähnliche Titel";
    let mut next_up = None;
    match item.kind {
        ItemKind::Series => {
            seasons = client
                .seasons(&item.id, &["Overview", "UserData", "ImageTags"])
                .await
                .map(|q| q.items)
                .unwrap_or_default();
            season_idx = logic::initial_season(&seasons, preferred.as_deref()).unwrap_or(0);
            if let Some(s) = seasons.get(season_idx) {
                episodes = client
                    .episodes(&item.id, Some(&s.id), &LIST_FIELDS)
                    .await
                    .map(|q| q.items)
                    .unwrap_or_default();
            } else {
                episodes = client
                    .episodes(&item.id, None, &LIST_FIELDS)
                    .await
                    .map(|q| q.items)
                    .unwrap_or_default();
            }
            next_up = client
                .next_up(Some(&item.id), 1, &ITEM_FIELDS)
                .await
                .ok()
                .and_then(|q| q.items.into_iter().next());
            similar = client
                .similar(&item.id, 12, &LIST_FIELDS)
                .await
                .map(|q| q.items)
                .unwrap_or_default();
        }
        ItemKind::BoxSet => {
            similar_title = "Titel in dieser Sammlung";
            similar = client
                .items(
                    &ItemsQuery::new()
                        .parent_id(&item.id)
                        .sort_by(&[SortBy::PremiereDate, SortBy::SortName])
                        .fields(&LIST_FIELDS),
                )
                .await
                .map(|q| q.items)
                .unwrap_or_default();
        }
        ItemKind::Person => {
            similar_title = "Filme und Serien";
            similar = client
                .items(
                    &ItemsQuery::new()
                        .person_ids(&[&item.id])
                        .recursive(true)
                        .include_types(&[ItemKind::Movie, ItemKind::Series])
                        .sort_by(&[SortBy::ProductionYear, SortBy::SortName])
                        .fields(&LIST_FIELDS)
                        .limit(60),
                )
                .await
                .map(|q| q.items)
                .unwrap_or_default();
        }
        _ => {
            similar = client
                .similar(&item.id, 12, &LIST_FIELDS)
                .await
                .map(|q| q.items)
                .unwrap_or_default();
        }
    }
    let can_download = if need_policy {
        client
            .me()
            .await
            .ok()
            .and_then(|u| u.policy)
            .is_some_and(|p| p.enable_content_downloading)
    } else {
        false
    };
    Ok(Loaded {
        item,
        seasons,
        season_idx,
        episodes,
        selected_ep: selected,
        similar,
        similar_title,
        next_up,
        can_download,
    })
}

fn ss(s: impl Into<String>) -> SharedString {
    SharedString::from(s.into())
}

/// Anzeigedaten des Hauptelements.
fn build_data(ctl: &DetailCtl, can_download: bool) -> DetailData {
    let Some(item) = &ctl.item else {
        return DetailData {
            has_item: false,
            ..DetailData::default()
        };
    };
    let ud = item.user_data.clone().unwrap_or_default();
    let target = play_target(ctl);
    let tud = target
        .as_ref()
        .and_then(|t| t.user_data.clone())
        .unwrap_or_default();
    let resume = tud.playback_position_ticks > 0 && !tud.played;
    let series = item.kind == ItemKind::Series;
    let play_label = if resume {
        "Fortsetzen"
    } else if series && ud.played {
        "Erneut ansehen"
    } else {
        "Abspielen"
    };
    let playable = logic::is_playable(item);
    DetailData {
        id: ss(item.id.clone()),
        title: ss(display_title(item)),
        subtitle: ss(person_subtitle(item)),
        backdrop: ctl.backdrop.clone().unwrap_or_default(),
        has_backdrop: ctl.backdrop.is_some(),
        logo: ctl.logo.clone().unwrap_or_default(),
        has_logo: ctl.logo.is_some(),
        year: ss(item
            .production_year
            .map(|y| y.to_string())
            .unwrap_or_default()),
        length: ss(logic::duration_text(item)),
        quality: ss(logic::quality_label(item, 3000)),
        rating: ss(format_rating(item.official_rating.as_deref())),
        rating_text: ss(logic::rating_text(item.official_rating.as_deref())),
        tagline: ss(item.taglines.first().cloned().unwrap_or_default()),
        overview: ss(item.overview.clone().unwrap_or_default()),
        cast_line: ss(logic::people_of(item, "Actor")
            .iter()
            .take(3)
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
            .join(", ")),
        genres_line: ss(item.genres.join(", ")),
        studios_line: ss(item
            .studios
            .iter()
            .map(|s| s.name.clone())
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(", ")),
        play_label: ss(play_label),
        can_play: playable && target.is_some(),
        can_played: logic::can_mark_played(item),
        can_like: playable || item.kind == ItemKind::BoxSet,
        can_download: can_download && matches!(item.kind, ItemKind::Movie | ItemKind::Video),
        fav: ud.is_favorite,
        like: ud.likes == Some(true),
        played: ud.played,
        progress: (logic::progress_percent(target.as_ref().unwrap_or(item)) / 100.0) as f32,
        has_item: true,
    }
}

fn person_subtitle(item: &BaseItem) -> String {
    if item.kind == ItemKind::Person {
        "Person".into()
    } else {
        String::new()
    }
}

/// Was „Abspielen“ startet: der Titel selbst, bei Serien die nächste Folge (sonst die erste der Staffel).
fn play_target(ctl: &DetailCtl) -> Option<BaseItem> {
    let item = ctl.item.as_ref()?;
    match item.kind {
        ItemKind::Series => ctl
            .next_up
            .clone()
            .or_else(|| ctl.episodes.first().cloned()),
        ItemKind::Season => ctl.episodes.first().cloned(),
        _ if logic::is_playable(item) => Some(item.clone()),
        _ => None,
    }
}

fn episode_data(ep: &BaseItem, thumb: Option<Image>) -> EpisodeData {
    let ud = ep.user_data.clone().unwrap_or_default();
    EpisodeData {
        number: ss(ep.index_number.map(|n| n.to_string()).unwrap_or_default()),
        title: ss(ep.name.clone()),
        runtime: ss(format_runtime(ep.run_time_ticks)),
        overview: ss(ep.overview.clone().unwrap_or_default()),
        has_thumb: thumb.is_some(),
        thumb: thumb.unwrap_or_default(),
        progress: if ud.played {
            0.0
        } else {
            (logic::progress_percent(ep) / 100.0) as f32
        },
        played: ud.played,
    }
}

fn similar_data(it: &BaseItem, image: Option<Image>) -> SimilarCard {
    SimilarCard {
        title: ss(it.name.clone()),
        year: ss(it
            .production_year
            .map(|y| y.to_string())
            .unwrap_or_default()),
        rating: ss(format_rating(it.official_rating.as_deref())),
        runtime: ss(logic::duration_text(it)),
        overview: ss(it.overview.clone().unwrap_or_default()),
        has_image: image.is_some(),
        image: image.unwrap_or_default(),
        fav: it.user_data.as_ref().is_some_and(|u| u.is_favorite),
    }
}

/// Personen für den Info-Block: Regie, Besetzung (bis 8), Drehbuch.
fn people_chips(item: &BaseItem) -> Vec<(PersonChip, String)> {
    let mut out = Vec::new();
    let mut add = |label: &str, kind: &str, max: usize| {
        for p in logic::people_of(item, kind).into_iter().take(max) {
            out.push((
                PersonChip {
                    name: ss(p.name.clone()),
                    role: ss(p.role.clone().unwrap_or_default()),
                    label: ss(label),
                    initial: ss(p.name.chars().next().map(String::from).unwrap_or_default()),
                },
                p.id.clone(),
            ));
        }
    };
    add("Regie", "Director", 2);
    add("Besetzung", "Actor", 8);
    add("Drehbuch", "Writer", 2);
    out
}

impl App {
    /// Kurzer Zugriff auf das Global (Slint-Globals borgen vom Fenster).
    pub fn with_ui<R>(&self, f: impl FnOnce(&MainWindow, &Ui<'_>) -> R) -> R {
        let win = self.win();
        let ui = win.global::<Ui>();
        f(&win, &ui)
    }

    // ---------------------------------------------------------------- Öffnen und Schließen

    /// Karte angeklickt (Startseite, Bibliotheksseiten): öffnet das Detail-Modal.
    pub fn on_card_activate(self: &Rc<Self>, id: &str) {
        let seed = {
            let st = self.st.borrow();
            st.home
                .rows
                .iter()
                .flat_map(|r| r.items.iter())
                .chain(st.home.pool.iter())
                .find(|i| i.id == id)
                .cloned()
        };
        self.open_modal(id, seed);
    }

    /// Öffnet das Modal; `seed` zeigt schon vor der Antwort Titel und Bild.
    pub fn open_modal(self: &Rc<Self>, id: &str, seed: Option<BaseItem>) {
        self.close_hovercard_now();
        self.close_menu();
        if self.st.borrow().detail.surface == Surface::Modal {
            // Wechsel des Titels im offenen Modal (Ähnliche Titel)
            self.st.borrow_mut().detail.surface = Surface::None;
        }
        self.st.borrow_mut().detail.surface = Surface::Modal;
        self.start_detail_load(id, seed);
        self.with_ui(|_, ui| ui.set_modal_open(true));
        self.install_detail_graph();
        self.show_detail_focus();
    }

    /// Schließt das Modal und stellt den Fokus der Seite darunter wieder her.
    pub fn close_modal(self: &Rc<Self>) {
        if self.st.borrow().detail.surface != Surface::Modal {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            st.detail.surface = Surface::None;
            st.detail.generation += 1;
            if st.graph.trap_active() {
                st.graph.pop_trap();
            }
            for s in 0..SECTION_COUNT {
                st.graph.remove_zone(logic::detail_zone(s));
            }
        }
        self.close_menu();
        self.with_ui(|_, ui| {
            ui.set_modal_open(false);
            ui.set_focus_sec(-1);
            ui.set_season_list_open(false);
        });
        self.show_focus();
    }

    /// Detailseite betreten (Route `Detail{id}`).
    pub fn enter_detail_page(self: &Rc<Self>, id: &str) {
        self.close_hovercard_now();
        self.close_menu();
        {
            let mut st = self.st.borrow_mut();
            st.detail.surface = Surface::Page;
            st.graph = FocusGraph::new();
        }
        self.with_ui(|_, ui| ui.set_modal_open(false));
        self.start_detail_load(id, None);
        self.install_detail_graph();
        self.show_detail_focus();
    }

    /// Beim Verlassen jeder Seite: Detailseite aufräumen.
    pub fn detail_leave_route(self: &Rc<Self>) {
        let surface = self.st.borrow().detail.surface;
        self.close_hovercard_now();
        self.close_menu();
        if surface == Surface::Page {
            let mut st = self.st.borrow_mut();
            st.detail.surface = Surface::None;
            st.detail.generation += 1;
        }
        if self.st.borrow().detail.surface == Surface::Modal {
            self.close_modal();
        }
    }

    /// Öffnet die Detailseite eines Titels (schließt vorher das Modal).
    pub fn open_detail_page(self: &Rc<Self>, id: &str) {
        self.close_modal();
        self.go(Route::Detail { id: id.to_owned() });
    }

    // ---------------------------------------------------------------- Laden

    fn start_detail_load(self: &Rc<Self>, id: &str, seed: Option<BaseItem>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        if self.st.borrow().images.is_none() {
            self.ensure_images(&client);
        }
        let epoch = {
            let mut st = self.st.borrow_mut();
            let d = &mut st.detail;
            d.generation += 1;
            d.item = seed;
            d.seasons.clear();
            d.season_idx = 0;
            d.episodes.clear();
            d.ep_cache.clear();
            d.selected_ep = None;
            d.similar.clear();
            d.next_up = None;
            d.src_idx = 0;
            d.audio = None;
            d.sub = None;
            d.backdrop = None;
            d.logo = None;
            d.season_list_open = false;
            d.generation
        };
        let need_policy = self.st.borrow().detail.can_download.is_none();
        self.refresh_detail_ui();
        self.with_ui(|_, ui| {
            ui.set_loading(true);
            ui.set_error("".into());
        });
        let id = id.to_owned();
        self.rt.spawn(async move {
            let res = fetch_detail(&client, &id, need_policy).await;
            post(move |app| app.detail_loaded(epoch, res));
        });
    }

    fn detail_loaded(self: &Rc<Self>, epoch: u64, res: Result<Loaded, String>) {
        if self.st.borrow().detail.generation != epoch {
            return;
        }
        match res {
            Err(text) => {
                tracing::warn!("Detail: {text}");
                self.with_ui(|_, ui| {
                    ui.set_loading(false);
                    ui.set_error(ss(text));
                });
            }
            Ok(l) => {
                {
                    let mut st = self.st.borrow_mut();
                    let d = &mut st.detail;
                    if d.can_download.is_none() {
                        d.can_download = Some(l.can_download);
                    }
                    d.item = Some(l.item);
                    d.seasons = l.seasons;
                    d.season_idx = l.season_idx;
                    if let Some(s) = d.seasons.get(l.season_idx) {
                        d.ep_cache.insert(s.id.clone(), l.episodes.clone());
                    }
                    d.episodes = l.episodes;
                    d.selected_ep = l.selected_ep;
                    d.similar = l.similar;
                    d.similar_title = l.similar_title.to_owned();
                    d.next_up = l.next_up;
                    d.src_idx = 0;
                }
                self.with_ui(|_, ui| ui.set_loading(false));
                self.refresh_detail_ui();
                self.load_detail_images();
                self.install_detail_graph();
                // Eine gewählte Folge bekommt den Fokus (Web: `selectedEpisodeId`).
                let sel = {
                    let st = self.st.borrow();
                    st.detail
                        .selected_ep
                        .as_ref()
                        .and_then(|id| st.detail.episodes.iter().position(|e| &e.id == id))
                };
                if let Some(i) = sel {
                    let _ = self.st.borrow_mut().graph.set_focus(Target {
                        zone: logic::detail_zone(SEC_EPS),
                        index: i,
                    });
                } else {
                    self.focus_detail_start();
                }
                self.show_detail_focus();
            }
        }
    }

    fn load_detail_images(self: &Rc<Self>) {
        let (item, eps, sim, epoch) = {
            let st = self.st.borrow();
            let d = &st.detail;
            (
                d.item.clone(),
                d.episodes.clone(),
                d.similar.clone(),
                d.generation,
            )
        };
        if let Some(item) = item {
            let imgs = hero_images(&item, 2.0, 1.0);
            let backdrop = imgs
                .backdrop
                .or_else(|| card_images(&item, CardKind::Landscape, false, 2.0, 1.0).main);
            if let Some(r) = backdrop {
                self.fetch_image(&r, Priority::Visible, move |app, img| {
                    if app.st.borrow().detail.generation != epoch {
                        return;
                    }
                    app.st.borrow_mut().detail.backdrop = Some(img);
                    app.refresh_detail_data();
                });
            }
            if let Some(r) = imgs.logo {
                self.fetch_image(&r, Priority::Visible, move |app, img| {
                    if app.st.borrow().detail.generation != epoch {
                        return;
                    }
                    app.st.borrow_mut().detail.logo = Some(img);
                    app.refresh_detail_data();
                });
            }
        }
        for (i, ep) in eps.iter().enumerate() {
            if let Some(r) = card_images(ep, CardKind::Landscape, false, 1.0, 1.0).main {
                self.fetch_image(&r, Priority::Visible, move |app, img| {
                    if app.st.borrow().detail.generation != epoch {
                        return;
                    }
                    let model = app.st.borrow().detail.ep_model.clone();
                    if let Some(m) = model
                        && let Some(mut row) = m.row_data(i)
                    {
                        row.thumb = img;
                        row.has_thumb = true;
                        m.set_row_data(i, row);
                    }
                });
            }
        }
        for (i, it) in sim.iter().enumerate() {
            if let Some(r) = card_images(it, CardKind::Landscape, false, 1.0, 1.0).main {
                self.fetch_image(&r, Priority::Visible, move |app, img| {
                    if app.st.borrow().detail.generation != epoch {
                        return;
                    }
                    let model = app.st.borrow().detail.sim_model.clone();
                    if let Some(m) = model
                        && let Some(mut row) = m.row_data(i)
                    {
                        row.image = img;
                        row.has_image = true;
                        m.set_row_data(i, row);
                    }
                });
            }
        }
    }

    // ---------------------------------------------------------------- Anzeige

    /// Schreibt alle Detaildaten (neu) in die Oberfläche.
    pub fn refresh_detail_ui(self: &Rc<Self>) {
        let (
            data,
            eps,
            seasons,
            season_idx,
            sim,
            sim_title,
            chips,
            info_title,
            info_rating,
            texts,
            played_season,
        ) = {
            let st = self.st.borrow();
            let d = &st.detail;
            let can_dl = d.can_download.unwrap_or(false);
            let data = build_data(d, can_dl);
            let eps: Vec<EpisodeData> = d.episodes.iter().map(|e| episode_data(e, None)).collect();
            let seasons: Vec<SharedString> = d.seasons.iter().map(|s| ss(s.name.clone())).collect();
            let sim: Vec<SimilarCard> = d.similar.iter().map(|s| similar_data(s, None)).collect();
            let chips: Vec<PersonChip> = d
                .item
                .as_ref()
                .map(|i| people_chips(i).into_iter().map(|(c, _)| c).collect())
                .unwrap_or_default();
            let info_title = d
                .item
                .as_ref()
                .map(|i| ss(i.name.clone()))
                .unwrap_or_default();
            let info_rating = d
                .item
                .as_ref()
                .map(|i| {
                    let r = format_rating(i.official_rating.as_deref());
                    let t = logic::rating_text(i.official_rating.as_deref());
                    if r.is_empty() {
                        String::new()
                    } else {
                        format!("{r} · {t}")
                    }
                })
                .unwrap_or_default();
            let texts = track_texts(d);
            let played_season = !d.episodes.is_empty()
                && d.episodes
                    .iter()
                    .all(|e| e.user_data.as_ref().is_some_and(|u| u.played));
            (
                data,
                eps,
                seasons,
                d.season_idx as i32,
                sim,
                ss(d.similar_title.clone()),
                chips,
                info_title,
                ss(info_rating),
                texts,
                played_season,
            )
        };
        // Die Modelle bleiben erhalten, damit Bilder nachgeladen werden können; Bilder, die schon da waren, gehen
        // bei einem Neuaufbau verloren und kommen über `load_detail_images` zurück.
        let ep_model = Rc::new(VecModel::from(eps));
        let sim_model = Rc::new(VecModel::from(sim));
        {
            let mut st = self.st.borrow_mut();
            st.detail.ep_model = Some(ep_model.clone());
            st.detail.sim_model = Some(sim_model.clone());
        }
        self.with_ui(|_, ui| {
            ui.set_data(data);
            ui.set_episodes(ModelRc::from(ep_model));
            ui.set_seasons(ModelRc::new(VecModel::from(seasons)));
            ui.set_season_index(season_idx);
            ui.set_season_played(played_season);
            ui.set_similar(ModelRc::from(sim_model));
            ui.set_similar_title(sim_title);
            ui.set_people(ModelRc::new(VecModel::from(chips)));
            ui.set_info_title(info_title);
            ui.set_info_rating(info_rating);
            ui.set_version_text(ss(texts.0));
            ui.set_audio_text(ss(texts.1));
            ui.set_sub_text(ss(texts.2));
        });
    }

    /// Nur die Kopfdaten (nach Bild- oder Statuswechsel), ohne die Listen neu aufzubauen.
    pub fn refresh_detail_data(self: &Rc<Self>) {
        let data = {
            let st = self.st.borrow();
            build_data(&st.detail, st.detail.can_download.unwrap_or(false))
        };
        self.with_ui(|_, ui| ui.set_data(data));
    }

    /// Aktualisiert Gesehen-/Favoriten-Zustände aller sichtbaren Teile nach einer Änderung.
    fn refresh_after_mutation(self: &Rc<Self>) {
        // Modelle zeilenweise anpassen, damit geladene Bilder bleiben
        let (eps, sim, texts) = {
            let st = self.st.borrow();
            (
                st.detail.episodes.clone(),
                st.detail.similar.clone(),
                track_texts(&st.detail),
            )
        };
        let (ep_model, sim_model) = {
            let st = self.st.borrow();
            (st.detail.ep_model.clone(), st.detail.sim_model.clone())
        };
        if let Some(m) = ep_model {
            for (i, e) in eps.iter().enumerate() {
                if let Some(mut row) = m.row_data(i) {
                    let ud = e.user_data.clone().unwrap_or_default();
                    row.played = ud.played;
                    row.progress = if ud.played {
                        0.0
                    } else {
                        (logic::progress_percent(e) / 100.0) as f32
                    };
                    m.set_row_data(i, row);
                }
            }
        }
        if let Some(m) = sim_model {
            for (i, it) in sim.iter().enumerate() {
                if let Some(mut row) = m.row_data(i) {
                    row.fav = it.user_data.as_ref().is_some_and(|u| u.is_favorite);
                    m.set_row_data(i, row);
                }
            }
        }
        let played_season = !eps.is_empty()
            && eps
                .iter()
                .all(|e| e.user_data.as_ref().is_some_and(|u| u.played));
        self.with_ui(|_, ui| {
            ui.set_season_played(played_season);
            ui.set_version_text(ss(texts.0));
            ui.set_audio_text(ss(texts.1));
            ui.set_sub_text(ss(texts.2));
        });
        self.refresh_detail_data();
        self.refresh_hovercard_flags();
    }

    // ---------------------------------------------------------------- Fokus

    fn detail_counts(&self) -> DetailCounts {
        let mut st = self.st.borrow_mut();
        let modal = st.detail.surface == Surface::Modal;
        let can_dl = st.detail.can_download.unwrap_or(false);
        let d = &mut st.detail;
        let data = build_data(d, can_dl);
        let mut act = Vec::new();
        if data.can_play {
            act.push(0);
        }
        if d.item.is_some() {
            act.push(1);
            if data.can_like {
                act.push(2);
            }
            if data.can_played {
                act.push(3);
            }
            if data.can_download {
                act.push(4);
            }
            act.push(5);
        }
        d.act_kinds = act;
        let (v, a, s) = track_texts(d);
        let mut tracks = Vec::new();
        if !v.is_empty() {
            tracks.push(0);
        }
        if !a.is_empty() {
            tracks.push(1);
        }
        if !s.is_empty() {
            tracks.push(2);
        }
        d.track_kinds = tracks;
        let mut season = Vec::new();
        if !d.episodes.is_empty() {
            season.push(0);
            if d.seasons.len() > 1 {
                season.push(1);
            }
        }
        d.season_kinds = season;
        let people = d.item.as_ref().map_or(0, |i| people_chips(i).len());
        DetailCounts {
            actions: d.act_kinds.len(),
            tracks: d.track_kinds.len(),
            season: d.season_kinds.len(),
            episodes: d.episodes.len(),
            similar: d.similar.len(),
            people,
            link: usize::from(modal),
            sim_cols: 3,
            people_cols: 4,
        }
    }

    /// Spaltenzahl der Raster (wie `sim-cols`/`ppl-cols` in Slint: schmal = 2 bzw. 2).
    fn detail_cols(&self) -> (usize, usize) {
        let win = self.win();
        let size = win.window().size();
        let scale = win.window().scale_factor().max(0.1);
        let w = size.width as f32 / scale;
        let view_w = if self.st.borrow().detail.surface == Surface::Modal {
            (w * 0.94).min(850.0)
        } else {
            w
        };
        if view_w > 700.0 { (3, 4) } else { (2, 2) }
    }

    /// Registriert die Detail-Zonen im Graphen (Modal mit Fokusfalle).
    pub fn install_detail_graph(self: &Rc<Self>) {
        let mut counts = self.detail_counts();
        let (sc, pc) = self.detail_cols();
        counts.sim_cols = sc;
        counts.people_cols = pc;
        let zones = logic::detail_zones(&counts);
        let modal = self.st.borrow().detail.surface == Surface::Modal;
        let mut st = self.st.borrow_mut();
        let old = st.graph.focused();
        for z in zones {
            if st.graph.zone(z.id).is_some() {
                st.graph.update_zone(z);
            } else {
                st.graph.register_zone(z);
            }
        }
        if modal && !st.graph.trap_active() {
            let ids: Vec<_> = (0..SECTION_COUNT).map(logic::detail_zone).collect();
            st.graph.push_trap(&ids);
        }
        // Fokus nur behalten, wenn er noch gültig ist
        if let Some(t) = old
            && logic::zone_section(t.zone).is_some()
        {
            let _ = st.graph.set_focus(t);
        }
        if st
            .graph
            .focused()
            .is_none_or(|t| logic::zone_section(t.zone).is_none())
        {
            drop(st);
            self.focus_detail_start();
        }
    }

    fn focus_detail_start(self: &Rc<Self>) {
        let mut st = self.st.borrow_mut();
        for s in [SEC_ACT, SEC_TRACKS, SEC_SEASON, SEC_EPS] {
            if st.graph.focus_zone(logic::detail_zone(s)) {
                return;
            }
        }
    }

    /// Ob Detail-Modal oder -Seite die Fokusanzeige besitzt.
    pub fn detail_owns_focus(self: &Rc<Self>) -> bool {
        let surface = self.st.borrow().detail.surface;
        match surface {
            Surface::None => false,
            Surface::Modal => {
                if !self.st.borrow().graph.trap_active() {
                    // Der Graph darunter wurde neu gebaut (Laden, Größenänderung): Zonen und Falle erneuern.
                    self.install_detail_graph();
                }
                true
            }
            Surface::Page => matches!(self.st.borrow().router.current(), Route::Detail { .. }),
        }
    }

    /// Fokus des Graphen → `focus-sec`/`focus-idx` in Slint.
    pub fn show_detail_focus(self: &Rc<Self>) {
        let (sec, idx) = {
            let st = self.st.borrow();
            let Some(t) = st.graph.focused() else {
                drop(st);
                self.with_ui(|_, ui| ui.set_focus_sec(-1));
                return;
            };
            let Some(sec) = logic::zone_section(t.zone) else {
                drop(st);
                self.with_ui(|_, ui| ui.set_focus_sec(-1));
                return;
            };
            let d = &st.detail;
            let idx = match sec {
                SEC_ACT => d.act_kinds.get(t.index).copied().unwrap_or(0),
                SEC_TRACKS => d.track_kinds.get(t.index).copied().unwrap_or(0),
                SEC_SEASON => d.season_kinds.get(t.index).copied().unwrap_or(0),
                _ => t.index as i32,
            };
            (sec as i32, idx)
        };
        self.with_ui(|_, ui| {
            ui.set_focus_sec(sec);
            ui.set_focus_idx(idx);
        });
    }

    // ---------------------------------------------------------------- Eingabe

    /// Tastatur/Gamepad vor der Seitenlogik: Menü, Modal, Detailseite. `true` = verarbeitet.
    pub fn detail_handle_action(self: &Rc<Self>, action: NavAction) -> bool {
        // Kontextmenü hat Vorrang
        if self.st.borrow().detail.menu.is_some() {
            self.menu_action(action);
            return true;
        }
        let surface = self.st.borrow().detail.surface;
        let on_home = matches!(self.st.borrow().router.current(), Route::Home);
        if surface == Surface::None {
            // Info-Taste öffnet das Modal statt der Hovercard; Menü-Taste das Kontextmenü.
            self.close_hovercard_now();
            if on_home {
                match action {
                    NavAction::Info => {
                        if let Some(item) = self.focused_home_item() {
                            self.open_modal(&item.id.clone(), Some(item));
                            return true;
                        }
                    }
                    NavAction::Menu => {
                        if let Some(item) = self.focused_home_item() {
                            let (w, h) = self.window_logical();
                            self.open_menu_for(item, w * 0.3, h * 0.5, true);
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            return false;
        }
        if surface == Surface::Page
            && !matches!(self.st.borrow().router.current(), Route::Detail { .. })
        {
            return false;
        }
        match action {
            NavAction::Back => {
                if self.st.borrow().detail.season_list_open {
                    self.set_season_list(false);
                } else if surface == Surface::Modal {
                    self.close_modal();
                } else {
                    return false;
                }
                true
            }
            NavAction::Home | NavAction::Search => {
                if surface == Surface::Modal {
                    self.close_modal();
                }
                false
            }
            NavAction::Menu | NavAction::Info => {
                if let Some(item) = self.focused_detail_item() {
                    let (w, h) = self.window_logical();
                    self.open_menu_for(item, w * 0.5, h * 0.4, true);
                }
                true
            }
            NavAction::Play | NavAction::PlayPause => {
                self.detail_play();
                true
            }
            _ => {
                let outcome = self.st.borrow_mut().graph.apply(action);
                match outcome {
                    NavOutcome::Moved { .. } => self.show_detail_focus(),
                    NavOutcome::Activated(t) => self.detail_activate(t),
                    _ => {}
                }
                true
            }
        }
    }

    pub fn window_logical(&self) -> (f32, f32) {
        let win = self.win();
        let size = win.window().size();
        let s = win.window().scale_factor().max(0.1);
        (size.width as f32 / s, size.height as f32 / s)
    }

    /// Element unter dem Fokus (Menü-/Info-Taste): Folge in der Folgenliste, Karte bei Ähnlichen, sonst Hauptelement.
    fn focused_detail_item(&self) -> Option<BaseItem> {
        let st = self.st.borrow();
        let t = st.graph.focused()?;
        let d = &st.detail;
        match logic::zone_section(t.zone)? {
            SEC_EPS => d.episodes.get(t.index).cloned(),
            SEC_SIM => d.similar.get(t.index).cloned(),
            _ => d.item.clone(),
        }
    }

    /// Fokussierte Karte der Startseite (oder der Titel im Billboard).
    fn focused_home_item(&self) -> Option<BaseItem> {
        if let Some(i) = self.focused_item() {
            return Some(i);
        }
        let st = self.st.borrow();
        let t = st.graph.focused()?;
        if t.zone == crate::focus::Z_HERO {
            return st.home.pool.get(st.home.hero_idx).cloned();
        }
        None
    }

    fn detail_activate(self: &Rc<Self>, t: Target) {
        let Some(sec) = logic::zone_section(t.zone) else {
            return;
        };
        let (kind, ep, sim, person, id) = {
            let st = self.st.borrow();
            let d = &st.detail;
            (
                match sec {
                    SEC_ACT => d.act_kinds.get(t.index).copied(),
                    SEC_TRACKS => d.track_kinds.get(t.index).copied(),
                    SEC_SEASON => d.season_kinds.get(t.index).copied(),
                    _ => None,
                },
                d.episodes.get(t.index).cloned(),
                d.similar.get(t.index).cloned(),
                d.item
                    .as_ref()
                    .and_then(|i| people_chips(i).into_iter().nth(t.index).map(|(_, id)| id)),
                d.item.as_ref().map(|i| i.id.clone()),
            )
        };
        match sec {
            SEC_ACT => match kind {
                Some(0) => self.detail_play(),
                Some(1) => self.detail_toggle_fav(),
                Some(2) => self.detail_toggle_like(),
                Some(3) => self.detail_toggle_played(),
                Some(4) => self.download_placeholder(),
                Some(5) => {
                    let item = self.st.borrow().detail.item.clone();
                    if let Some(item) = item {
                        let (w, h) = self.window_logical();
                        self.open_menu_for(item, w * 0.5, h * 0.4, true);
                    }
                }
                _ => {}
            },
            SEC_TRACKS => {
                if let Some(k) = kind {
                    self.cycle_track(k);
                }
            }
            SEC_SEASON => match kind {
                Some(0) => self.toggle_season_played(),
                Some(1) => {
                    let (n, cur) = {
                        let st = self.st.borrow();
                        (st.detail.seasons.len(), st.detail.season_idx)
                    };
                    if n > 0 {
                        self.pick_season((cur + 1) % n);
                    }
                }
                _ => {}
            },
            SEC_EPS => {
                if let Some(ep) = ep {
                    self.play_with_choice(&ep, PlayChoice::default());
                }
            }
            SEC_SIM => {
                if let Some(it) = sim {
                    self.detail_open_related(&it);
                }
            }
            SEC_PEOPLE => {
                if let Some(pid) = person {
                    self.open_detail_page(&pid);
                }
            }
            SEC_LINK => {
                if let Some(id) = id {
                    self.open_detail_page(&id);
                }
            }
            _ => {}
        }
    }

    /// Klick auf eine ähnliche Karte: im Modal wechselt der Inhalt, auf der Seite öffnet sich die andere Seite.
    pub fn detail_open_related(self: &Rc<Self>, it: &BaseItem) {
        let surface = self.st.borrow().detail.surface;
        if surface == Surface::Modal {
            self.open_modal(&it.id.clone(), Some(it.clone()));
        } else {
            self.open_detail_page(&it.id.clone());
        }
    }

    // ---------------------------------------------------------------- Aktionen im Detail

    pub fn detail_play(self: &Rc<Self>) {
        let (target, choice) = {
            let st = self.st.borrow();
            let d = &st.detail;
            let src = d.item.as_ref().and_then(|i| {
                let srcs = sources_of(d, i);
                (srcs.len() > 1)
                    .then(|| srcs.get(d.src_idx).map(|s| s.id.clone()))
                    .flatten()
            });
            (
                play_target(d),
                PlayChoice {
                    from_start: false,
                    source: src,
                    audio: d.audio,
                    sub: d.sub,
                },
            )
        };
        if let Some(t) = target {
            self.play_with_choice(&t, choice);
        }
    }

    /// Startet einen Titel mit Vorauswahl; schließt vorher das Modal.
    pub fn play_with_choice(self: &Rc<Self>, item: &BaseItem, choice: PlayChoice) {
        self.close_modal();
        self.close_hovercard_now();
        self.st.borrow_mut().detail.choice = Some(choice);
        self.play_item(item);
    }

    /// Nimmt die Vorauswahl für den Start (einmalig); `play.rs` ruft das beim Vorbereiten.
    pub fn take_play_choice(&self) -> Option<PlayChoice> {
        self.st.borrow_mut().detail.choice.take()
    }

    /// Serie abspielen, ohne dass die Folge schon bekannt ist (Hovercard, Menü): nächste Folge oder erste Folge.
    pub fn play_series_id(self: &Rc<Self>, series_id: &str) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        let id = series_id.to_owned();
        self.rt.spawn(async move {
            let next = client
                .next_up(Some(&id), 1, &LIST_FIELDS)
                .await
                .ok()
                .and_then(|q| q.items.into_iter().next());
            let ep = match next {
                Some(e) => Some(e),
                None => client
                    .episodes(&id, None, &LIST_FIELDS)
                    .await
                    .ok()
                    .and_then(|q| q.items.into_iter().next()),
            };
            post(move |app| match ep {
                Some(e) => app.play_with_choice(&e, PlayChoice::default()),
                None => app.toast("Keine Folge zum Abspielen gefunden.", ToastKind::Error),
            });
        });
    }

    /// Spielt ein beliebiges Element (Film, Folge oder Serie).
    pub fn play_any(self: &Rc<Self>, item: &BaseItem, from_start: bool) {
        match item.kind {
            ItemKind::Series | ItemKind::Season => {
                self.play_series_id(if item.kind == ItemKind::Season {
                    item.series_id.as_deref().unwrap_or(&item.id)
                } else {
                    &item.id
                })
            }
            _ => self.play_with_choice(
                item,
                PlayChoice {
                    from_start,
                    ..PlayChoice::default()
                },
            ),
        }
    }

    fn download_placeholder(self: &Rc<Self>) {
        // Der Download-Manager kommt mit W5; bis dahin nur die Rechteprüfung und diese Rückmeldung.
        self.toast(
            "Herunterladen folgt in einer späteren Version.",
            ToastKind::Info,
        );
    }

    pub fn detail_toggle_fav(self: &Rc<Self>) {
        let id = self.st.borrow().detail.item.as_ref().map(|i| i.id.clone());
        if let Some(id) = id {
            self.toggle_fav(&id);
        }
    }

    pub fn detail_toggle_like(self: &Rc<Self>) {
        let id = self.st.borrow().detail.item.as_ref().map(|i| i.id.clone());
        if let Some(id) = id {
            self.toggle_like(&id);
        }
    }

    pub fn detail_toggle_played(self: &Rc<Self>) {
        let id = self.st.borrow().detail.item.as_ref().map(|i| i.id.clone());
        if let Some(id) = id {
            self.toggle_played(&id);
        }
    }

    fn toggle_season_played(self: &Rc<Self>) {
        let (sid, on) = {
            let st = self.st.borrow();
            let d = &st.detail;
            let Some(s) = d.seasons.get(d.season_idx) else {
                return;
            };
            let all = !d.episodes.is_empty()
                && d.episodes
                    .iter()
                    .all(|e| e.user_data.as_ref().is_some_and(|u| u.played));
            (s.id.clone(), !all)
        };
        self.set_played(&sid, on);
    }

    pub fn pick_season(self: &Rc<Self>, i: usize) {
        let (sid, series, cached, epoch) = {
            let mut st = self.st.borrow_mut();
            let d = &mut st.detail;
            let Some(s) = d.seasons.get(i).cloned() else {
                return;
            };
            d.season_list_open = false;
            d.season_idx = i;
            d.generation += 0;
            let cached = d.ep_cache.get(&s.id).cloned();
            (
                s.id,
                d.item.as_ref().map(|it| it.id.clone()).unwrap_or_default(),
                cached,
                d.generation,
            )
        };
        self.with_ui(|_, ui| ui.set_season_list_open(false));
        if let Some(eps) = cached {
            self.set_episodes(eps);
            return;
        }
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let eps = client
                .episodes(&series, Some(&sid), &LIST_FIELDS)
                .await
                .map(|q| q.items);
            post(move |app| {
                if app.st.borrow().detail.generation != epoch {
                    return;
                }
                match eps {
                    Ok(e) => {
                        app.st
                            .borrow_mut()
                            .detail
                            .ep_cache
                            .insert(sid.clone(), e.clone());
                        app.set_episodes(e);
                    }
                    Err(_) => app.toast("Folgen konnten nicht geladen werden.", ToastKind::Error),
                }
            });
        });
    }

    fn set_episodes(self: &Rc<Self>, eps: Vec<BaseItem>) {
        {
            let mut st = self.st.borrow_mut();
            st.detail.episodes = eps;
            st.detail.selected_ep = None;
        }
        self.refresh_detail_ui();
        self.load_detail_images();
        self.install_detail_graph();
        let _ = self.st.borrow_mut().graph.set_focus(Target {
            zone: logic::detail_zone(SEC_SEASON),
            index: 0,
        });
        self.show_detail_focus();
    }

    fn set_season_list(self: &Rc<Self>, open: bool) {
        self.st.borrow_mut().detail.season_list_open = open;
        self.with_ui(|_, ui| ui.set_season_list_open(open));
    }

    /// Ton, Untertitel oder Version der Vorauswahl weiterschalten (0 Version, 1 Ton, 2 Untertitel).
    pub fn cycle_track(self: &Rc<Self>, kind: i32) {
        {
            let mut st = self.st.borrow_mut();
            let d = &mut st.detail;
            let Some(item) = d.item.clone() else {
                return;
            };
            let srcs = sources_of(d, &item);
            if srcs.is_empty() {
                return;
            }
            match kind {
                0 => {
                    d.src_idx = (d.src_idx + 1) % srcs.len();
                    d.audio = None;
                    d.sub = None;
                }
                1 => {
                    let src = &srcs[d.src_idx.min(srcs.len() - 1)];
                    let opts: Vec<i32> = logic::streams_of(src, "Audio")
                        .iter()
                        .map(|s| s.index)
                        .collect();
                    if !opts.is_empty() {
                        let cur = d
                            .audio
                            .or(src.default_audio_stream_index)
                            .unwrap_or(opts[0]);
                        d.audio = Some(logic::cycle_value(&opts, cur));
                    }
                }
                _ => {
                    let src = &srcs[d.src_idx.min(srcs.len() - 1)];
                    let mut opts = vec![-1];
                    opts.extend(logic::streams_of(src, "Subtitle").iter().map(|s| s.index));
                    let cur = d.sub.or(src.default_subtitle_stream_index).unwrap_or(-1);
                    d.sub = Some(logic::cycle_value(&opts, cur));
                }
            }
        }
        let texts = track_texts(&self.st.borrow().detail);
        self.with_ui(|_, ui| {
            ui.set_version_text(ss(texts.0));
            ui.set_audio_text(ss(texts.1));
            ui.set_sub_text(ss(texts.2));
        });
    }

    // ---------------------------------------------------------------- Kontextmenü

    pub fn open_menu_for(self: &Rc<Self>, item: BaseItem, x: f32, y: f32, keyboard: bool) {
        self.close_hovercard_now();
        let can_dl = self.st.borrow().detail.can_download.unwrap_or(false);
        let entries = logic::menu_for(&item, can_dl);
        let ui_entries: Vec<MenuEntry> = entries
            .iter()
            .map(|e| MenuEntry {
                label: ss(e.label.clone()),
                enabled: e.enabled,
                checked: e.checked,
            })
            .collect();
        self.st.borrow_mut().detail.menu = Some(MenuCtl {
            item,
            entries,
            focus: 0,
        });
        self.with_ui(|_, ui| {
            ui.set_menu_entries(ModelRc::new(VecModel::from(ui_entries)));
            ui.set_menu_x(x);
            ui.set_menu_y(y);
            ui.set_menu_focus(if keyboard { 0 } else { -1 });
            ui.set_menu_open(true);
        });
        // Fenster fragt den Download-Recht beim ersten Menü nach
        if self.st.borrow().detail.can_download.is_none() {
            self.fetch_download_right();
        }
    }

    fn fetch_download_right(self: &Rc<Self>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let ok = client
                .me()
                .await
                .ok()
                .and_then(|u| u.policy)
                .is_some_and(|p| p.enable_content_downloading);
            post(move |app| app.st.borrow_mut().detail.can_download = Some(ok));
        });
    }

    pub fn close_menu(self: &Rc<Self>) {
        let had = self.st.borrow_mut().detail.menu.take().is_some();
        if had {
            self.with_ui(|_, ui| ui.set_menu_open(false));
        }
    }

    fn menu_action(self: &Rc<Self>, action: NavAction) {
        let (len, focus) = {
            let st = self.st.borrow();
            let Some(m) = &st.detail.menu else { return };
            (m.entries.len(), m.focus)
        };
        match action {
            NavAction::Down | NavAction::Up => {
                let step: isize = if action == NavAction::Down { 1 } else { -1 };
                let mut i = focus as isize;
                for _ in 0..len {
                    i = (i + step).rem_euclid(len as isize);
                    let enabled = self
                        .st
                        .borrow()
                        .detail
                        .menu
                        .as_ref()
                        .is_some_and(|m| m.entries[i as usize].enabled);
                    if enabled {
                        break;
                    }
                }
                if let Some(m) = self.st.borrow_mut().detail.menu.as_mut() {
                    m.focus = i as usize;
                }
                self.with_ui(|_, ui| ui.set_menu_focus(i as i32));
            }
            NavAction::Activate => self.menu_pick(focus),
            NavAction::Back | NavAction::Menu | NavAction::Info => self.close_menu(),
            _ => {}
        }
    }

    /// Eintrag des Menüs ausführen.
    pub fn menu_pick(self: &Rc<Self>, i: usize) {
        let picked = {
            let st = self.st.borrow();
            let Some(m) = &st.detail.menu else { return };
            m.entries
                .get(i)
                .filter(|e| e.enabled)
                .map(|e| (e.action, m.item.clone()))
        };
        let Some((action, item)) = picked else { return };
        self.close_menu();
        match action {
            MenuAction::Play => self.play_any(&item, false),
            MenuAction::FromStart => self.play_any(&item, true),
            MenuAction::ToggleFav => self.toggle_fav(&item.id),
            MenuAction::TogglePlayed => self.toggle_played(&item.id),
            MenuAction::Download => self.download_placeholder(),
            MenuAction::Details => self.open_detail_page(&item.id),
            MenuAction::GoSeries => {
                if let Some(s) = item.series_id.clone() {
                    self.open_detail_page(&s);
                }
            }
            MenuAction::GoSeason => {
                if let Some(s) = item.season_id.clone() {
                    self.open_detail_page(&s);
                }
            }
        }
    }

    // ---------------------------------------------------------------- Benutzerdaten (Favorit, Gesehen, Bewertung)

    /// Benutzerdaten eines Elements aus allen bekannten Orten (Startseite, Detail, Hovercard, Menü).
    pub fn user_data_of(&self, id: &str) -> UserItemData {
        let st = self.st.borrow();
        let mut found = None;
        for_each_item(&st, |it| {
            if found.is_none() && it.id == id {
                found = Some(it.user_data.clone().unwrap_or_default());
            }
        });
        found.unwrap_or_default()
    }

    /// Ändert die Benutzerdaten eines Elements überall, wo es angezeigt wird.
    pub fn mutate_user_data(self: &Rc<Self>, id: &str, f: &dyn Fn(&mut UserItemData)) {
        {
            let mut st = self.st.borrow_mut();
            for_each_item_mut(&mut st, |it| {
                if it.id == id {
                    f(it.user_data.get_or_insert_with(UserItemData::default));
                }
            });
        }
        self.refresh_after_mutation();
    }

    pub fn toggle_fav(self: &Rc<Self>, id: &str) {
        let new = !self.user_data_of(id).is_favorite;
        self.mutate_user_data(id, &|u| u.is_favorite = new);
        self.send_user_action(id, UserAction::Favorite(new));
    }

    pub fn toggle_like(self: &Rc<Self>, id: &str) {
        let cur = self.user_data_of(id).likes;
        let new = if cur == Some(true) { None } else { Some(true) };
        self.mutate_user_data(id, &|u| u.likes = new);
        self.send_user_action(id, UserAction::Like(new));
    }

    pub fn toggle_played(self: &Rc<Self>, id: &str) {
        let new = !self.user_data_of(id).played;
        self.set_played(id, new);
    }

    pub fn set_played(self: &Rc<Self>, id: &str, played: bool) {
        // Serien und Staffeln: die geladenen Folgen mitziehen
        let (is_series, is_season) = {
            let st = self.st.borrow();
            let d = &st.detail;
            (
                d.item
                    .as_ref()
                    .is_some_and(|i| i.id == id && i.kind == ItemKind::Series),
                d.seasons.iter().any(|s| s.id == id),
            )
        };
        let ids: Vec<String> = if is_series || is_season {
            self.st
                .borrow()
                .detail
                .episodes
                .iter()
                .map(|e| e.id.clone())
                .collect()
        } else {
            Vec::new()
        };
        let apply = move |u: &mut UserItemData| {
            u.played = played;
            u.playback_position_ticks = 0;
            if played {
                u.play_count = u.play_count.max(1);
            }
        };
        self.mutate_user_data(id, &apply);
        for e in &ids {
            self.mutate_user_data(e, &apply);
        }
        self.send_user_action(id, UserAction::Played(played));
    }

    fn send_user_action(self: &Rc<Self>, id: &str, action: UserAction) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        let (server_id, user_id) = (active.server_id.clone(), active.user_id.clone());
        let id = id.to_owned();
        self.rt.spawn(async move {
            let res = match action {
                UserAction::Favorite(v) => client.set_favorite(&id, v).await,
                UserAction::Played(v) => client.set_played(&id, v).await,
                UserAction::Like(v) => client.set_rating(&id, v).await,
            };
            post(move |app| match res {
                Ok(srv) => app.mutate_user_data(&id, &move |u| match action {
                    UserAction::Favorite(_) => u.is_favorite = srv.is_favorite,
                    UserAction::Played(_) => {
                        u.played = srv.played;
                        u.play_count = srv.play_count;
                    }
                    UserAction::Like(_) => u.likes = srv.likes,
                }),
                Err(ApiError::Network(_) | ApiError::Timeout) => {
                    // Offline: Meldung vormerken, die Anzeige bleibt wie gewählt.
                    let (kind, payload) = action.outbox();
                    let mut payload = payload;
                    payload["id"] = serde_json::Value::String(id.clone());
                    let r = app.store.outbox_push(&NewOutboxEntry {
                        server_id,
                        user_id,
                        kind: kind.to_owned(),
                        payload,
                    });
                    if let Err(err) = r {
                        tracing::warn!("Outbox nicht beschreibbar: {err}");
                    }
                    app.toast(
                        "Keine Verbindung: Die Änderung wird später übertragen.",
                        ToastKind::Info,
                    );
                }
                Err(e) => {
                    tracing::warn!("Aktion fehlgeschlagen: {e}");
                    app.mutate_user_data(&id, &move |u| match action {
                        UserAction::Favorite(v) => u.is_favorite = !v,
                        UserAction::Played(v) => u.played = !v,
                        UserAction::Like(v) => {
                            u.likes = if v.is_some() { None } else { Some(true) }
                        }
                    });
                    app.toast("Konnte nicht gespeichert werden.", ToastKind::Error);
                }
            });
        });
    }
}

/// Eine Änderung der Benutzerdaten, die zum Server geht.
#[derive(Clone, Copy, Debug)]
pub enum UserAction {
    Favorite(bool),
    Played(bool),
    Like(Option<bool>),
}

impl UserAction {
    /// Art und Inhalt des Outbox-Eintrags (ohne Id).
    pub fn outbox(self) -> (&'static str, serde_json::Value) {
        match self {
            Self::Favorite(v) => ("favorite", serde_json::json!({ "value": v })),
            Self::Played(v) => ("played", serde_json::json!({ "value": v })),
            Self::Like(v) => ("rating", serde_json::json!({ "likes": v })),
        }
    }
}

fn for_each_item(st: &crate::app::State, mut f: impl FnMut(&BaseItem)) {
    for r in &st.home.rows {
        r.items.iter().for_each(&mut f);
    }
    st.home.pool.iter().for_each(&mut f);
    let d = &st.detail;
    d.item.iter().for_each(&mut f);
    d.seasons.iter().for_each(&mut f);
    d.episodes.iter().for_each(&mut f);
    d.similar.iter().for_each(&mut f);
    d.next_up.iter().for_each(&mut f);
    d.hover.item.iter().for_each(&mut f);
    d.menu.iter().for_each(|m| f(&m.item));
}

fn for_each_item_mut(st: &mut crate::app::State, mut f: impl FnMut(&mut BaseItem)) {
    for r in &mut st.home.rows {
        r.items.iter_mut().for_each(&mut f);
    }
    st.home.pool.iter_mut().for_each(&mut f);
    let d = &mut st.detail;
    d.item.iter_mut().for_each(&mut f);
    d.seasons.iter_mut().for_each(&mut f);
    d.episodes.iter_mut().for_each(&mut f);
    d.similar.iter_mut().for_each(&mut f);
    d.next_up.iter_mut().for_each(&mut f);
    d.hover.item.iter_mut().for_each(&mut f);
    d.menu.iter_mut().for_each(|m| f(&mut m.item));
    for (_, it) in d.cache.values_mut() {
        f(it);
    }
    for eps in d.ep_cache.values_mut() {
        eps.iter_mut().for_each(&mut f);
    }
}

/// Medienquellen des Spurwahl-Ziels: bei Serien die der nächsten Folge, sonst die des Titels.
fn sources_of(d: &DetailCtl, item: &BaseItem) -> Vec<lfx_api::MediaSource> {
    let src = match item.kind {
        ItemKind::Series | ItemKind::Season => d.next_up.as_ref(),
        _ => Some(item),
    };
    src.map(|i| i.media_sources.clone()).unwrap_or_default()
}

/// Anzeigetexte der Spurwahl: (Version, Ton, Untertitel); leer = ausgeblendet.
fn track_texts(d: &DetailCtl) -> (String, String, String) {
    let Some(item) = &d.item else {
        return Default::default();
    };
    if !logic::is_playable(item) || item.kind == ItemKind::Season {
        return Default::default();
    }
    let srcs = sources_of(d, item);
    let Some(src) = srcs.get(d.src_idx.min(srcs.len().saturating_sub(1))) else {
        return Default::default();
    };
    let version = if srcs.len() > 1 {
        src.name
            .clone()
            .unwrap_or_else(|| format!("Version {}", d.src_idx + 1))
    } else {
        String::new()
    };
    let audio_streams = logic::streams_of(src, "Audio");
    let audio = if audio_streams.len() > 1 {
        let cur = d
            .audio
            .or(src.default_audio_stream_index)
            .unwrap_or(audio_streams[0].index);
        audio_streams.iter().find(|s| s.index == cur).map_or_else(
            || logic::stream_label(audio_streams[0]),
            |s| logic::stream_label(s),
        )
    } else {
        String::new()
    };
    let subs = logic::streams_of(src, "Subtitle");
    let sub = if subs.is_empty() {
        String::new()
    } else {
        let cur = d.sub.or(src.default_subtitle_stream_index).unwrap_or(-1);
        subs.iter()
            .find(|s| s.index == cur)
            .map_or_else(|| "Aus".to_owned(), |s| logic::stream_label(s))
    };
    (version, audio, sub)
}

/// Verbindet die Rückrufe des Globals `DetailState` mit der App.
pub fn wire(app: &Rc<App>, ui: &MainWindow) {
    let g = ui.global::<Ui>();
    macro_rules! on {
        ($setter:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
            let a = app.clone();
            g.$setter(move |$($arg),*| {
                let $app = &a;
                $body
            });
        }};
    }
    on!(on_play, |a| a.detail_play());
    on!(on_toggle_fav, |a| a.detail_toggle_fav());
    on!(on_toggle_like, |a| a.detail_toggle_like());
    on!(on_toggle_played, |a| a.detail_toggle_played());
    on!(on_download, |a| a.download_placeholder());
    on!(on_open_menu, |a| {
        let item = a.st.borrow().detail.item.clone();
        if let Some(item) = item {
            let (w, h) = a.window_logical();
            a.open_menu_for(item, w * 0.5, h * 0.4, false);
        }
    });
    on!(on_cycle_track, |a, k| a.cycle_track(k));
    on!(on_toggle_season_played, |a| a.toggle_season_played());
    on!(on_toggle_season_list, |a| {
        let open = !a.st.borrow().detail.season_list_open;
        a.set_season_list(open);
    });
    on!(on_pick_season, |a, i| a.pick_season(i.max(0) as usize));
    on!(on_episode_clicked, |a, i| {
        let ep =
            a.st.borrow()
                .detail
                .episodes
                .get(i.max(0) as usize)
                .cloned();
        if let Some(ep) = ep {
            a.play_with_choice(&ep, PlayChoice::default());
        }
    });
    on!(on_episode_played, |a, i| {
        let id =
            a.st.borrow()
                .detail
                .episodes
                .get(i.max(0) as usize)
                .map(|e| e.id.clone());
        if let Some(id) = id {
            a.toggle_played(&id);
        }
    });
    on!(on_episode_menu, |a, i| {
        let ep =
            a.st.borrow()
                .detail
                .episodes
                .get(i.max(0) as usize)
                .cloned();
        if let Some(ep) = ep {
            let (w, h) = a.window_logical();
            a.open_menu_for(ep, w * 0.5, h * 0.4, false);
        }
    });
    on!(on_similar_clicked, |a, i| {
        let it = a.st.borrow().detail.similar.get(i.max(0) as usize).cloned();
        if let Some(it) = it {
            a.detail_open_related(&it);
        }
    });
    on!(on_similar_fav, |a, i| {
        let id =
            a.st.borrow()
                .detail
                .similar
                .get(i.max(0) as usize)
                .map(|e| e.id.clone());
        if let Some(id) = id {
            a.toggle_fav(&id);
        }
    });
    on!(on_person_clicked, |a, i| {
        let pid = a.st.borrow().detail.item.as_ref().and_then(|it| {
            people_chips(it)
                .into_iter()
                .nth(i.max(0) as usize)
                .map(|(_, id)| id)
        });
        if let Some(pid) = pid {
            a.open_detail_page(&pid);
        }
    });
    on!(on_open_page, |a| {
        let id = a.st.borrow().detail.item.as_ref().map(|i| i.id.clone());
        if let Some(id) = id {
            a.open_detail_page(&id);
        }
    });
    on!(on_close, |a| a.close_modal());
    on!(on_back, |a| a.apply_action(NavAction::Back));
    on!(on_menu_picked, |a, i| a.menu_pick(i.max(0) as usize));
    on!(on_menu_close, |a| a.close_menu());
    crate::hovercard::wire(app, &g);
}
