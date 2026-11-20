//! W4-media: Medienseiten (Musik, Bücher, Fotos, Live-TV) – Zustand, Laden, Anzeige, Fokus und Ereignisse.
//!
//! Die Seite `Route::Media` gehört dieser Datei samt `media_audio.rs` (Audio-Player), `media_extra.rs`
//! (Foto-Betrachter, Programmführer, Bücher) und `media_auto.rs` (Automation). Die reine Logik steht in
//! `media_model.rs`. Fokus: eigene Zonen (siehe `ui/pages/media.slint`), Rust zählt Zone und Index.

use std::rc::Rc;

use lfx_api::{BaseItem, ImageType, ItemKind, ItemsQuery, SortBy, SortOrder, TimerInfo};
use lfx_core::images::{ImagePick, ImageRef, own_image};
use lfx_images::Priority;
use lfx_nav::NavAction;
use lfx_ui::{MediaCard, MediaTrack, MediaUi, Page, ToastKind};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::app::{App, post};
use crate::media_audio::AudioCtl;
use crate::media_extra::{GuideData, Viewer};
use crate::media_model::{LIVE_TABS, MUSIC_TABS, MediaKind, MediaRoute};
use crate::route::Route;

// Fokuszonen (wie in media.slint)
pub const Z_HEAD: i32 = 0;
pub const Z_BODY: i32 = 1;
pub const Z_BAR: i32 = 2;
pub const Z_FULL: i32 = 3;
pub const Z_QUEUE: i32 = 4;

/// Quelle von Titeln für Aktionen: fertige Liste oder ein Behälter (Album, Künstler, Liste), der erst geladen wird.
#[derive(Clone, Debug)]
pub enum Src {
    Items(Vec<BaseItem>),
    Container(Box<BaseItem>),
}

/// Eine Menüaktion.
#[derive(Clone, Debug)]
pub enum Act {
    Play {
        src: Src,
        shuffle: bool,
        start: usize,
    },
    PlayNext(Src),
    Enqueue(Src),
    /// Zweites Menü: Wiedergabelisten zur Wahl.
    ChooseList(Src),
    ToList {
        list: String,
        src: Src,
    },
    NewList(Src),
    MoveEntry {
        list: String,
        entry: String,
        to: u32,
    },
    RemoveEntry {
        list: String,
        entry: String,
    },
    Mix(String),
    DeleteItem(String),
    RenameList(String, String),
    CancelTimer(String),
    DeleteRecording(String),
    PlayItem(Box<BaseItem>),
    Record(String),
    QueueRemove(usize),
}

/// Geöffnetes Menü.
pub struct MenuCtl {
    pub title: String,
    pub entries: Vec<(String, Act)>,
    pub index: usize,
}

/// Offene Namensabfrage.
pub enum PromptKind {
    NewList(Src),
    Rename(String),
}

/// Was die Inhaltsfläche zeigt.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    #[default]
    Cards,
    Tracks,
    Guide,
    Book,
}

/// Zustand der Medienseiten.
#[derive(Default)]
pub struct MediaCtl {
    pub epoch: u64,
    pub route: Option<MediaRoute>,
    pub zone: i32,
    pub index: usize,
    /// Spalte im Programmführer (-1 = Sender).
    pub col: i32,
    pub cols: usize,
    pub view: View,
    pub items: Vec<BaseItem>,
    pub head: Option<BaseItem>,
    pub timers: Vec<TimerInfo>,
    pub guide: GuideData,
    pub menu: Option<MenuCtl>,
    pub prompt: Option<PromptKind>,
    pub viewer: Option<Viewer>,
    pub audio: AudioCtl,
    pub ensure_tick: i32,
    /// Dateien, die „Öffnen mit dem System“ verlangt hat (Automation merkt sich den Pfad statt zu starten).
    pub opened_files: Vec<std::path::PathBuf>,
    pub book_busy: bool,
    pub lib_view_id: String,
}

/// Ergebnis eines Ladevorgangs.
pub enum Loaded {
    Cards {
        head: Option<BaseItem>,
        items: Vec<BaseItem>,
        w: f32,
        h: f32,
        empty: &'static str,
    },
    Tracks {
        head: Option<BaseItem>,
        items: Vec<BaseItem>,
        empty: &'static str,
    },
    Guide(GuideData),
    Timers(Vec<TimerInfo>),
    Book(BaseItem),
}

pub fn is_media_route(r: &Route) -> bool {
    matches!(r, Route::Media(_))
}

/// Bildquelle einer Karte (eigenes Bild, sonst Albumbild).
pub fn cover_pick(item: &BaseItem) -> Option<ImagePick> {
    own_image(item, ImageType::Primary).or_else(|| {
        let id = item.album_id.as_deref().filter(|s| !s.is_empty())?;
        let tag = item
            .album_primary_image_tag
            .as_deref()
            .filter(|s| !s.is_empty())?;
        Some(ImagePick {
            id: id.to_owned(),
            kind: ImageType::Primary,
            tag: tag.to_owned(),
        })
    })
}

fn artists_text(item: &BaseItem) -> String {
    if !item.artists.is_empty() {
        item.artists.join(", ")
    } else {
        item.album_artist.clone().unwrap_or_default()
    }
}

pub fn duration_text(item: &BaseItem) -> String {
    item.run_time_ticks
        .filter(|t| *t > 0)
        .map(|t| crate::play::format_time(t as f64 / 10_000_000.0))
        .unwrap_or_default()
}

fn card_of(item: &BaseItem) -> MediaCard {
    let mut c = MediaCard {
        title: item.name.as_str().into(),
        ..MediaCard::default()
    };
    let sub = match item.kind {
        ItemKind::MusicAlbum => {
            let a = item.album_artist.clone().unwrap_or_default();
            match item.production_year {
                Some(y) if !a.is_empty() => format!("{a} · {y}"),
                Some(y) => y.to_string(),
                None => a,
            }
        }
        ItemKind::MusicArtist => item
            .album_count
            .filter(|n| *n > 0)
            .map(|n| format!("{n} Alben"))
            .unwrap_or_default(),
        ItemKind::Playlist => item
            .child_count
            .map(|n| format!("{n} Titel"))
            .unwrap_or_default(),
        ItemKind::MusicGenre | ItemKind::Genre => "Genre".to_owned(),
        ItemKind::Book | ItemKind::AudioBook => artists_text(item),
        ItemKind::PhotoAlbum => {
            c.badge = "Album".into();
            item.child_count
                .map(|n| format!("{n} Fotos"))
                .unwrap_or_default()
        }
        ItemKind::TvChannel | ItemKind::LiveTvChannel => {
            let num = item.channel_number.clone().unwrap_or_default();
            c.title = format!("{num} {}", item.name).trim().into();
            item.current_program
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_default()
        }
        ItemKind::Recording => item.channel_name.clone().unwrap_or_default(),
        _ => String::new(),
    };
    c.subtitle = sub.into();
    c.round = item.kind == ItemKind::MusicArtist;
    if item.id == "__new" {
        c.badge = "+".into();
    }
    c
}

fn track_of(item: &BaseItem, idx: usize, playing: bool) -> MediaTrack {
    let sub = if item.kind == ItemKind::Audio {
        let a = artists_text(item);
        match item.album.as_deref() {
            Some(al) if !al.is_empty() => format!("{a} · {al}"),
            _ => a,
        }
    } else {
        String::new()
    };
    MediaTrack {
        num: item
            .index_number
            .map_or_else(|| (idx + 1).to_string(), |n| n.to_string())
            .into(),
        title: item.name.as_str().into(),
        sub: sub.into(),
        right: duration_text(item).into(),
        playing,
    }
}

fn timer_row(t: &TimerInfo) -> MediaTrack {
    let start = t.start_date.clone().unwrap_or_default();
    MediaTrack {
        num: "".into(),
        title: t.name.clone().unwrap_or_default().into(),
        sub: format!(
            "{} · {}",
            t.channel_name.clone().unwrap_or_default(),
            crate::media_extra::when_text(&start)
        )
        .into(),
        right: t.status.clone().unwrap_or_default().into(),
        playing: false,
    }
}

impl App {
    pub fn with_media<R>(&self, f: impl FnOnce(&MediaUi) -> R) -> R {
        let ui = self.win();
        let g = ui.global::<MediaUi>();
        f(&g)
    }

    pub fn media_client(&self) -> Option<lfx_api::Client> {
        self.session.active().map(|a| a.client.clone())
    }

    // ------------------------------------------------------------ Einstieg

    /// Hook von `render_route`: aufräumen beim Verlassen, Audio bei Video/TV stoppen.
    pub fn media_on_route(self: &Rc<Self>, route: &Route) {
        if matches!(route, Route::Player { .. } | Route::TvMode) {
            self.audio_stop();
        }
        if !is_media_route(route) {
            self.viewer_close();
            let mut st = self.st.borrow_mut();
            st.media.epoch += 1;
            st.media.menu = None;
            st.media.prompt = None;
        }
    }

    /// Öffnet die Medienseite einer Bibliotheksansicht (Hook aus `lib_collection_ready`).
    pub fn media_open_view(self: &Rc<Self>, id: &str, name: &str, ctype: &str) -> bool {
        let Some(kind) = crate::media_model::kind_for_collection(ctype) else {
            return false;
        };
        self.go_replace(Route::Media(MediaRoute::new(kind, id, name)));
        true
    }

    pub fn render_media_route(self: &Rc<Self>, m: &MediaRoute) {
        let ui = self.win();
        ui.set_page(Page::Media);
        self.st.borrow_mut().graph = lfx_nav::FocusGraph::new();
        let epoch = {
            let mut st = self.st.borrow_mut();
            let keep_view = if m.kind == MediaKind::Music
                || m.kind == MediaKind::Books
                || m.kind == MediaKind::Photos
                || m.kind == MediaKind::LiveTv
            {
                m.id.clone()
            } else {
                st.media.lib_view_id.clone()
            };
            st.media.epoch += 1;
            st.media.route = Some(m.clone());
            st.media.items.clear();
            st.media.head = None;
            st.media.guide = GuideData::default();
            st.media.view = View::Cards;
            st.media.zone = Z_BODY;
            st.media.index = 0;
            st.media.col = -1;
            st.media.menu = None;
            st.media.prompt = None;
            st.media.lib_view_id = keep_view;
            st.media.epoch
        };
        let chips: Vec<&str> = match m.kind {
            MediaKind::Music => MUSIC_TABS.to_vec(),
            MediaKind::LiveTv => LIVE_TABS.to_vec(),
            MediaKind::Album | MediaKind::Artist => vec!["Abspielen", "Zufall", "Instant Mix"],
            MediaKind::Playlist => vec!["Abspielen", "Zufall", "Umbenennen", "Löschen"],
            _ => vec![],
        };
        let tabbed = matches!(m.kind, MediaKind::Music | MediaKind::LiveTv);
        let sub = match m.kind {
            MediaKind::Music => "Musik",
            MediaKind::Album => "Album",
            MediaKind::Artist => "Künstler",
            MediaKind::Playlist => "Wiedergabeliste",
            MediaKind::Books | MediaKind::Book => "Bücher",
            MediaKind::Photos => "Fotos",
            MediaKind::LiveTv => "Live-TV",
        };
        self.with_media(|g| {
            g.set_title(m.name.as_str().into());
            g.set_subtitle(if sub == m.name { "".into() } else { sub.into() });
            g.set_chips(ModelRc::new(VecModel::from(
                chips
                    .iter()
                    .map(|s| SharedString::from(*s))
                    .collect::<Vec<_>>(),
            )));
            g.set_chip_current(if tabbed { m.tab as i32 } else { -1 });
            g.set_view(0);
            g.set_cards(ModelRc::new(VecModel::<MediaCard>::default()));
            g.set_tracks(ModelRc::new(VecModel::<MediaTrack>::default()));
            g.set_head_has_image(false);
            g.set_loading(true);
            g.set_error("".into());
            g.set_empty("".into());
            g.set_menu_open(false);
            g.set_prompt_open(false);
        });
        self.media_show_focus();
        self.media_load(epoch, m.clone());
    }

    // ------------------------------------------------------------ Laden

    fn media_load(self: &Rc<Self>, epoch: u64, m: MediaRoute) {
        let Some(client) = self.media_client() else {
            return;
        };
        let view_id = self.st.borrow().media.lib_view_id.clone();
        self.rt.spawn(async move {
            let res = load_media(&client, &m, &view_id).await;
            post(move |app| match res {
                Ok(l) => app.media_loaded(epoch, l),
                Err(e) => app.media_failed(epoch, &e),
            });
        });
    }

    fn media_failed(self: &Rc<Self>, epoch: u64, text: &str) {
        if self.st.borrow().media.epoch != epoch {
            return;
        }
        tracing::warn!("Medienseite nicht geladen: {text}");
        self.with_media(|g| {
            g.set_loading(false);
            g.set_error(
                "Die Inhalte konnten nicht geladen werden. Ist der Server erreichbar?".into(),
            );
        });
    }

    fn media_loaded(self: &Rc<Self>, epoch: u64, l: Loaded) {
        if self.st.borrow().media.epoch != epoch {
            return;
        }
        match l {
            Loaded::Cards {
                head,
                items,
                w,
                h,
                empty,
            } => {
                let cards: Vec<MediaCard> = items.iter().map(card_of).collect();
                {
                    let mut st = self.st.borrow_mut();
                    st.media.view = View::Cards;
                    st.media.items = items.clone();
                    st.media.head = head.clone();
                }
                self.with_media(|g| {
                    g.set_loading(false);
                    g.set_view(0);
                    g.set_card_w(w);
                    g.set_card_h(h);
                    g.set_cards(ModelRc::new(VecModel::from(cards)));
                    g.set_empty(if items.is_empty() {
                        empty.into()
                    } else {
                        "".into()
                    });
                });
                self.media_head(head.as_ref());
                for (i, it) in items.iter().enumerate() {
                    if let Some(pick) = cover_pick(it) {
                        self.fetch_image(
                            &ImageRef {
                                pick,
                                max_width: 420,
                                quality: 80,
                            },
                            Priority::Visible,
                            move |app, img| app.media_card_image(epoch, i, img),
                        );
                    }
                }
            }
            Loaded::Tracks { head, items, empty } => {
                let playing = self.audio_current_id();
                let rows: Vec<MediaTrack> = items
                    .iter()
                    .enumerate()
                    .map(|(i, t)| track_of(t, i, playing.as_deref() == Some(t.id.as_str())))
                    .collect();
                {
                    let mut st = self.st.borrow_mut();
                    st.media.view = View::Tracks;
                    st.media.items = items.clone();
                    st.media.head = head.clone();
                }
                self.with_media(|g| {
                    g.set_loading(false);
                    g.set_view(1);
                    g.set_tracks(ModelRc::new(VecModel::from(rows)));
                    g.set_empty(if items.is_empty() {
                        empty.into()
                    } else {
                        "".into()
                    });
                });
                self.media_head(head.as_ref());
            }
            Loaded::Timers(list) => {
                let rows: Vec<MediaTrack> = list.iter().map(timer_row).collect();
                {
                    let mut st = self.st.borrow_mut();
                    st.media.view = View::Tracks;
                    st.media.items = Vec::new();
                    st.media.timers = list.clone();
                }
                self.with_media(|g| {
                    g.set_loading(false);
                    g.set_view(1);
                    g.set_tracks(ModelRc::new(VecModel::from(rows)));
                    g.set_empty(if list.is_empty() {
                        "Keine geplanten Aufnahmen.".into()
                    } else {
                        "".into()
                    });
                });
            }
            Loaded::Guide(g) => self.guide_loaded(g),
            Loaded::Book(b) => self.book_loaded(b),
        }
        {
            let mut st = self.st.borrow_mut();
            st.media.zone = if st.media.items.is_empty()
                && st.media.view != View::Guide
                && st.media.view != View::Book
                && st.media.timers.is_empty()
            {
                Z_HEAD
            } else {
                Z_BODY
            };
            st.media.index = 0;
        }
        self.media_show_focus();
    }

    /// Kopfbild (Album-, Künstler-, Listencover) und Untertitel.
    fn media_head(self: &Rc<Self>, head: Option<&BaseItem>) {
        let Some(h) = head else { return };
        let epoch = self.st.borrow().media.epoch;
        let mut sub = match h.kind {
            ItemKind::MusicAlbum => artists_text(h),
            ItemKind::Playlist => h
                .child_count
                .map(|n| format!("{n} Titel"))
                .unwrap_or_default(),
            _ => String::new(),
        };
        if let Some(y) = h.production_year.filter(|_| h.kind == ItemKind::MusicAlbum) {
            sub = format!("{sub} · {y}");
        }
        self.with_media(|g| {
            g.set_title(h.name.as_str().into());
            if !sub.is_empty() {
                g.set_subtitle(sub.as_str().into());
            }
        });
        if let Some(pick) = cover_pick(h) {
            self.fetch_image(
                &ImageRef {
                    pick,
                    max_width: 400,
                    quality: 80,
                },
                Priority::Visible,
                move |app, img| {
                    if app.st.borrow().media.epoch != epoch {
                        return;
                    }
                    app.with_media(|g| {
                        g.set_head_image(img);
                        g.set_head_has_image(true);
                    });
                },
            );
        }
    }

    fn media_card_image(self: &Rc<Self>, epoch: u64, i: usize, img: Image) {
        if self.st.borrow().media.epoch != epoch {
            return;
        }
        self.with_media(|g| {
            let m = g.get_cards();
            if let Some(mut c) = m.row_data(i) {
                c.image = img;
                c.has_image = true;
                m.set_row_data(i, c);
            }
        });
    }

    /// Markiert in Titellisten den laufenden Titel.
    pub fn media_refresh_playing(self: &Rc<Self>) {
        let playing = self.audio_current_id();
        let (view, items) = {
            let st = self.st.borrow();
            (st.media.view, st.media.items.clone())
        };
        if view != View::Tracks || items.is_empty() {
            return;
        }
        self.with_media(|g| {
            let m = g.get_tracks();
            for (i, it) in items.iter().enumerate() {
                if let Some(mut r) = m.row_data(i) {
                    let p = playing.as_deref() == Some(it.id.as_str());
                    if r.playing != p {
                        r.playing = p;
                        m.set_row_data(i, r);
                    }
                }
            }
        });
    }

    // ------------------------------------------------------------ Fokus

    fn body_len(&self) -> usize {
        let st = self.st.borrow();
        match st.media.view {
            View::Cards | View::Tracks if st.media.items.is_empty() => st.media.timers.len(),
            View::Cards | View::Tracks => st.media.items.len(),
            View::Guide => st.media.guide.rows.len(),
            View::Book => 1,
        }
    }

    fn head_len(&self) -> usize {
        1 + self.with_media(|g| g.get_chips().row_count())
    }

    /// Schreibt Zone/Index in die Oberfläche und sorgt dafür, dass das Fokusziel sichtbar ist.
    pub fn media_show_focus(self: &Rc<Self>) {
        let (zone, index, col, cols, view, pitch) = {
            let st = self.st.borrow();
            let h = self.with_media(|g| g.get_card_h());
            (
                st.media.zone,
                st.media.index,
                st.media.col,
                st.media.cols.max(1),
                st.media.view,
                h + 64.0 + 24.0,
            )
        };
        let mut top = 0.0f32;
        let mut hh = 56.0f32;
        let mut x = 0.0f32;
        let mut w = 0.0f32;
        if zone == Z_BODY {
            match view {
                View::Cards => {
                    top = (index / cols) as f32 * pitch;
                    hh = pitch - 16.0;
                }
                View::Tracks => top = index as f32 * 56.0,
                View::Guide => {
                    top = 44.0 + index as f32 * 60.0;
                    if col >= 0
                        && let Some(c) = self
                            .st
                            .borrow()
                            .media
                            .guide
                            .rows
                            .get(index)
                            .and_then(|r| r.get(col as usize))
                    {
                        x = c.x as f32;
                        w = c.w as f32;
                    }
                }
                View::Book => {}
            }
        }
        let (tick, chip) = {
            let mut st = self.st.borrow_mut();
            st.media.ensure_tick += 1;
            (st.media.ensure_tick, 0)
        };
        let _ = chip;
        self.with_media(|g| {
            g.set_focus_zone(zone);
            g.set_focus_index(index as i32);
            g.set_focus_col(col);
            g.set_ensure_top(top);
            g.set_ensure_h(hh);
            g.set_ensure_x(x);
            g.set_ensure_w(w);
            g.set_ensure_tick(tick);
        });
    }

    /// Navigationsaktion; `true` = verbraucht. Hook in `apply_action`.
    pub fn media_handle_action(self: &Rc<Self>, action: NavAction) -> bool {
        // Medientasten wirken überall, solange Audio läuft (nicht im Video-Player).
        let route = self.st.borrow().router.current().clone();
        let on_media = is_media_route(&route);
        let audio = self.audio_active();
        if audio && !matches!(route, Route::Player { .. } | Route::TvMode) {
            match action {
                NavAction::PlayPause => {
                    self.audio_toggle();
                    return true;
                }
                NavAction::Next => {
                    self.audio_next(false);
                    return true;
                }
                NavAction::Previous => {
                    self.audio_prev();
                    return true;
                }
                NavAction::Stop => {
                    self.audio_stop();
                    return true;
                }
                _ => {}
            }
        }
        if !on_media {
            return false;
        }
        if self.st.borrow().media.prompt.is_some() {
            match action {
                NavAction::Back => self.prompt_cancel(),
                NavAction::Activate => self.prompt_submit(),
                _ => {}
            }
            return true;
        }
        if self.st.borrow().media.menu.is_some() {
            self.media_menu_action(action);
            return true;
        }
        if self.st.borrow().media.viewer.is_some() {
            return self.viewer_action(action);
        }
        if self.st.borrow().media.audio.full {
            return self.full_action(action);
        }
        match action {
            NavAction::Back => {
                self.go_back();
                true
            }
            NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right => {
                self.media_move(action);
                true
            }
            NavAction::Activate => {
                self.media_activate();
                true
            }
            NavAction::Menu | NavAction::Info => {
                self.media_context();
                true
            }
            _ => false,
        }
    }

    fn media_move(self: &Rc<Self>, a: NavAction) {
        let (zone, idx, cols, view, col) = {
            let st = self.st.borrow();
            (
                st.media.zone,
                st.media.index,
                st.media.cols.max(1),
                st.media.view,
                st.media.col,
            )
        };
        let n = self.body_len();
        let hl = self.head_len();
        let bar = self.audio_active();
        let (mut nz, mut ni, mut ncol) = (zone, idx, col);
        match (zone, a) {
            (Z_HEAD, NavAction::Left) => ni = idx.saturating_sub(1),
            (Z_HEAD, NavAction::Right) => ni = (idx + 1).min(hl - 1),
            (Z_HEAD, NavAction::Down) => {
                if n > 0 {
                    nz = Z_BODY;
                    ni = 0;
                    ncol = -1;
                } else if bar {
                    nz = Z_BAR;
                    ni = 1;
                }
            }
            (Z_BODY, NavAction::Left) => match view {
                View::Cards => ni = idx.saturating_sub(1),
                View::Guide => ncol = (col - 1).max(-1),
                _ => {}
            },
            (Z_BODY, NavAction::Right) => match view {
                View::Cards => ni = (idx + 1).min(n.saturating_sub(1)),
                View::Guide => {
                    let len = self
                        .st
                        .borrow()
                        .media
                        .guide
                        .rows
                        .get(idx)
                        .map_or(0, Vec::len) as i32;
                    ncol = (col + 1).min(len - 1);
                }
                _ => {}
            },
            (Z_BODY, NavAction::Up) => match view {
                View::Cards if idx >= cols => ni = idx - cols,
                View::Tracks | View::Guide if idx > 0 => ni = idx - 1,
                _ => {
                    nz = Z_HEAD;
                    ni = 0;
                    if view != View::Book {
                        let cur = self.with_media(|g| g.get_chip_current());
                        if cur >= 0 {
                            ni = cur as usize + 1;
                        }
                    }
                }
            },
            (Z_BODY, NavAction::Down) => match view {
                View::Cards => {
                    if idx + cols < n {
                        ni = idx + cols;
                    } else if idx / cols < (n - 1) / cols {
                        ni = n - 1;
                    } else if bar {
                        nz = Z_BAR;
                        ni = 1;
                    }
                }
                View::Tracks | View::Guide => {
                    if idx + 1 < n {
                        ni = idx + 1;
                    } else if bar {
                        nz = Z_BAR;
                        ni = 1;
                    }
                }
                View::Book => {
                    if bar {
                        nz = Z_BAR;
                        ni = 1;
                    }
                }
            },
            (Z_BAR, NavAction::Left) => ni = idx.saturating_sub(1),
            (Z_BAR, NavAction::Right) => ni = (idx + 1).min(5),
            (Z_BAR, NavAction::Up) => {
                nz = if n > 0 { Z_BODY } else { Z_HEAD };
                ni = 0;
                ni = ni.min(n.saturating_sub(1));
                ncol = -1;
            }
            _ => {}
        }
        if view == View::Guide && nz == Z_BODY && ni != idx {
            let len = self
                .st
                .borrow()
                .media
                .guide
                .rows
                .get(ni)
                .map_or(0, Vec::len) as i32;
            ncol = ncol.min(len - 1);
        }
        {
            let mut st = self.st.borrow_mut();
            st.media.zone = nz;
            st.media.index = ni;
            st.media.col = ncol;
        }
        self.media_show_focus();
    }

    fn media_activate(self: &Rc<Self>) {
        let (zone, idx) = {
            let st = self.st.borrow();
            (st.media.zone, st.media.index)
        };
        match zone {
            Z_HEAD => {
                if idx == 0 {
                    self.go_back();
                } else {
                    self.media_chip(idx - 1);
                }
            }
            Z_BODY => self.media_body_activate(idx),
            Z_BAR => self.bar_press(idx as i32),
            _ => {}
        }
    }

    fn media_context(self: &Rc<Self>) {
        let (zone, idx) = {
            let st = self.st.borrow();
            (st.media.zone, st.media.index)
        };
        if zone == Z_BODY {
            self.media_item_menu(idx);
        }
    }

    // ------------------------------------------------------------ Ereignisse aus Slint

    pub fn media_event(self: &Rc<Self>, kind: &str, a: i32, b: i32) {
        self.player_activity();
        let idx = a.max(0) as usize;
        match kind {
            "cols" => {
                self.st.borrow_mut().media.cols = (a.max(1)) as usize;
            }
            "back" => self.apply_action(NavAction::Back),
            "chip" => self.media_chip(idx),
            "card" | "track" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.media.zone = Z_BODY;
                    st.media.index = idx;
                }
                self.media_body_activate(idx);
            }
            "card-menu" | "track-menu" => self.media_item_menu(idx),
            "guide-chan" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.media.zone = Z_BODY;
                    st.media.index = idx;
                    st.media.col = -1;
                }
                self.media_show_focus();
                self.guide_activate();
            }
            "guide-cell" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.media.zone = Z_BODY;
                    st.media.index = idx;
                    st.media.col = b;
                }
                self.media_show_focus();
                self.guide_activate();
            }
            "book-open" => self.book_open(),
            "bar" => self.bar_press(a),
            "np-open" => self.audio_set_full(a == 1),
            "full" => self.full_press(a),
            "queue" => self.audio_jump(idx),
            "queue-menu" => self.queue_menu(idx),
            "menu-pick" => {
                if let Some(m) = self.st.borrow_mut().media.menu.as_mut() {
                    m.index = idx;
                }
                self.media_menu_activate();
            }
            "menu-close" => self.media_menu_close(),
            "prompt-ok" => self.prompt_submit(),
            "prompt-cancel" => self.prompt_cancel(),
            "viewer" => self.viewer_press(a),
            "viewer-zoom" => self.viewer_zoom(a),
            _ => {}
        }
    }

    pub fn media_event_f(self: &Rc<Self>, kind: &str, v: f32) {
        if kind == "seek" {
            self.audio_seek_fraction(v);
        }
    }

    fn media_chip(self: &Rc<Self>, i: usize) {
        let Some(r) = self.st.borrow().media.route.clone() else {
            return;
        };
        match r.kind {
            MediaKind::Music | MediaKind::LiveTv => {
                if i != r.tab {
                    let mut n = r.clone();
                    n.tab = i;
                    self.go_replace(Route::Media(n));
                }
            }
            MediaKind::Album | MediaKind::Artist | MediaKind::Playlist => {
                self.media_head_action(&r, i);
            }
            _ => {}
        }
    }

    /// Aktionen im Kopf von Album, Künstler und Wiedergabeliste.
    fn media_head_action(self: &Rc<Self>, r: &MediaRoute, i: usize) {
        let head = self.st.borrow().media.head.clone();
        let src = match &head {
            Some(h) => Src::Container(Box::new(h.clone())),
            None => return,
        };
        match (r.kind, i) {
            (_, 0) => self.run_act(Act::Play {
                src,
                shuffle: false,
                start: 0,
            }),
            (_, 1) => self.run_act(Act::Play {
                src,
                shuffle: true,
                start: 0,
            }),
            (MediaKind::Album | MediaKind::Artist, 2) => self.run_act(Act::Mix(r.id.clone())),
            (MediaKind::Playlist, 2) => {
                let name = head.map(|h| h.name).unwrap_or_default();
                self.open_prompt(
                    PromptKind::Rename(r.id.clone()),
                    "Wiedergabeliste umbenennen",
                    &name,
                );
            }
            (MediaKind::Playlist, 3) => {
                let name = head.map(|h| h.name).unwrap_or_default();
                self.media_open_menu(
                    &format!("„{name}“ löschen?"),
                    vec![
                        ("Löschen".into(), Act::DeleteItem(r.id.clone())),
                        ("Abbrechen".into(), Act::QueueRemove(usize::MAX)),
                    ],
                );
            }
            _ => {}
        }
    }

    fn media_body_activate(self: &Rc<Self>, idx: usize) {
        let Some(r) = self.st.borrow().media.route.clone() else {
            return;
        };
        let item = self.st.borrow().media.items.get(idx).cloned();
        match r.kind {
            MediaKind::Music => match (r.tab, item) {
                (0, Some(it)) | (1, Some(it)) => {
                    let kind = if r.tab == 0 {
                        MediaKind::Album
                    } else {
                        MediaKind::Artist
                    };
                    self.go(Route::Media(MediaRoute::new(kind, it.id, it.name)));
                }
                (2, Some(_)) => self.play_tracks_from(idx),
                (3, Some(it)) => self.run_act(Act::Mix(it.id)),
                (4, Some(it)) => {
                    if it.id == "__new" {
                        self.open_prompt(
                            PromptKind::NewList(Src::Items(Vec::new())),
                            "Neue Wiedergabeliste",
                            "",
                        );
                    } else {
                        self.go(Route::Media(MediaRoute::new(
                            MediaKind::Playlist,
                            it.id,
                            it.name,
                        )));
                    }
                }
                _ => {}
            },
            MediaKind::Artist => {
                if let Some(it) = item {
                    self.go(Route::Media(MediaRoute::new(
                        MediaKind::Album,
                        it.id,
                        it.name,
                    )));
                }
            }
            MediaKind::Album | MediaKind::Playlist => self.play_tracks_from(idx),
            MediaKind::Books => {
                if let Some(it) = item {
                    self.go(Route::Media(MediaRoute::new(
                        MediaKind::Book,
                        it.id,
                        it.name,
                    )));
                }
            }
            MediaKind::Book => self.book_open(),
            MediaKind::Photos => {
                if let Some(it) = item {
                    if it.kind == ItemKind::Photo {
                        self.viewer_open(idx);
                    } else {
                        self.go(Route::Media(MediaRoute::new(
                            MediaKind::Photos,
                            it.id,
                            it.name,
                        )));
                    }
                }
            }
            MediaKind::LiveTv => match r.tab {
                0 => {
                    if let Some(it) = item {
                        self.run_act(Act::PlayItem(Box::new(it)));
                    }
                }
                1 => self.guide_activate(),
                2 => {
                    if let Some(it) = item {
                        self.run_act(Act::PlayItem(Box::new(it)));
                    }
                }
                _ => self.media_item_menu(idx),
            },
        }
    }

    /// Spielt die Titel der Liste ab `idx` (nur Audio).
    fn play_tracks_from(self: &Rc<Self>, idx: usize) {
        let items = self.st.borrow().media.items.clone();
        let Some(cur) = items.get(idx) else { return };
        if cur.kind != ItemKind::Audio {
            return;
        }
        let audio: Vec<BaseItem> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Audio)
            .cloned()
            .collect();
        let start = audio.iter().position(|i| i.id == cur.id).unwrap_or(0);
        self.run_act(Act::Play {
            src: Src::Items(audio),
            shuffle: false,
            start,
        });
    }

    // ------------------------------------------------------------ Kontextmenüs

    fn media_item_menu(self: &Rc<Self>, idx: usize) {
        let Some(r) = self.st.borrow().media.route.clone() else {
            return;
        };
        let items = self.st.borrow().media.items.clone();
        let Some(it) = items.get(idx).cloned() else {
            if r.kind == MediaKind::LiveTv && r.tab == 3 {
                let t = self.st.borrow().media.timers.get(idx).cloned();
                if let Some(t) = t {
                    self.media_open_menu(
                        &t.name.clone().unwrap_or_default(),
                        vec![("Timer löschen".into(), Act::CancelTimer(t.id))],
                    );
                }
            }
            return;
        };
        let mut e: Vec<(String, Act)> = Vec::new();
        match it.kind {
            ItemKind::Audio => {
                let src = Src::Items(vec![it.clone()]);
                e.push((
                    "Abspielen".into(),
                    Act::Play {
                        src: Src::Items(
                            items
                                .iter()
                                .filter(|i| i.kind == ItemKind::Audio)
                                .cloned()
                                .collect(),
                        ),
                        shuffle: false,
                        start: items
                            .iter()
                            .filter(|i| i.kind == ItemKind::Audio)
                            .position(|i| i.id == it.id)
                            .unwrap_or(0),
                    },
                ));
                e.push(("Als Nächstes spielen".into(), Act::PlayNext(src.clone())));
                e.push((
                    "An Warteschlange anhängen".into(),
                    Act::Enqueue(src.clone()),
                ));
                e.push((
                    "Zur Wiedergabeliste hinzufügen …".into(),
                    Act::ChooseList(src),
                ));
                e.push(("Instant Mix".into(), Act::Mix(it.id.clone())));
                if r.kind == MediaKind::Playlist
                    && let Some(entry) = it.playlist_item_id.clone()
                {
                    if idx > 0 {
                        e.push((
                            "Nach oben".into(),
                            Act::MoveEntry {
                                list: r.id.clone(),
                                entry: entry.clone(),
                                to: idx as u32 - 1,
                            },
                        ));
                    }
                    if idx + 1 < items.len() {
                        e.push((
                            "Nach unten".into(),
                            Act::MoveEntry {
                                list: r.id.clone(),
                                entry: entry.clone(),
                                to: idx as u32 + 1,
                            },
                        ));
                    }
                    e.push((
                        "Aus der Liste entfernen".into(),
                        Act::RemoveEntry {
                            list: r.id.clone(),
                            entry,
                        },
                    ));
                }
            }
            ItemKind::MusicAlbum | ItemKind::MusicArtist | ItemKind::Playlist => {
                let src = Src::Container(Box::new(it.clone()));
                e.push((
                    "Abspielen".into(),
                    Act::Play {
                        src: src.clone(),
                        shuffle: false,
                        start: 0,
                    },
                ));
                e.push((
                    "Zufällig abspielen".into(),
                    Act::Play {
                        src: src.clone(),
                        shuffle: true,
                        start: 0,
                    },
                ));
                e.push(("Als Nächstes spielen".into(), Act::PlayNext(src.clone())));
                e.push((
                    "An Warteschlange anhängen".into(),
                    Act::Enqueue(src.clone()),
                ));
                if it.kind != ItemKind::Playlist {
                    e.push((
                        "Zur Wiedergabeliste hinzufügen …".into(),
                        Act::ChooseList(src),
                    ));
                }
                e.push(("Instant Mix".into(), Act::Mix(it.id.clone())));
                if it.kind == ItemKind::Playlist {
                    e.push(("Löschen".into(), Act::DeleteItem(it.id.clone())));
                }
            }
            ItemKind::Recording => {
                e.push(("Abspielen".into(), Act::PlayItem(Box::new(it.clone()))));
                e.push((
                    "Aufnahme löschen".into(),
                    Act::DeleteRecording(it.id.clone()),
                ));
            }
            ItemKind::TvChannel | ItemKind::LiveTvChannel => {
                e.push(("Ansehen".into(), Act::PlayItem(Box::new(it.clone()))));
            }
            _ => return,
        }
        self.media_open_menu(&it.name, e);
    }

    pub fn media_open_menu(self: &Rc<Self>, title: &str, entries: Vec<(String, Act)>) {
        let labels: Vec<SharedString> = entries.iter().map(|(l, _)| l.as_str().into()).collect();
        self.st.borrow_mut().media.menu = Some(MenuCtl {
            title: title.to_owned(),
            entries,
            index: 0,
        });
        self.with_media(|g| {
            g.set_menu_title(title.into());
            g.set_menu_items(ModelRc::new(VecModel::from(labels)));
            g.set_menu_index(0);
            g.set_menu_open(true);
        });
    }

    fn media_menu_close(self: &Rc<Self>) {
        self.st.borrow_mut().media.menu = None;
        self.with_media(|g| g.set_menu_open(false));
    }

    fn media_menu_action(self: &Rc<Self>, a: NavAction) {
        let n = self
            .st
            .borrow()
            .media
            .menu
            .as_ref()
            .map_or(0, |m| m.entries.len());
        match a {
            NavAction::Back | NavAction::Menu | NavAction::Left => self.media_menu_close(),
            NavAction::Up | NavAction::Down => {
                let mut st = self.st.borrow_mut();
                if let Some(m) = st.media.menu.as_mut() {
                    m.index = if a == NavAction::Up {
                        m.index.saturating_sub(1)
                    } else {
                        (m.index + 1).min(n.saturating_sub(1))
                    };
                    let i = m.index as i32;
                    drop(st);
                    self.with_media(|g| g.set_menu_index(i));
                }
            }
            NavAction::Activate => self.media_menu_activate(),
            _ => {}
        }
    }

    fn media_menu_activate(self: &Rc<Self>) {
        let act = {
            let mut st = self.st.borrow_mut();
            let Some(m) = st.media.menu.take() else {
                return;
            };
            m.entries.get(m.index).map(|(_, a)| a.clone())
        };
        self.with_media(|g| g.set_menu_open(false));
        if let Some(a) = act {
            self.run_act(a);
        }
    }

    // ------------------------------------------------------------ Namensabfrage

    fn open_prompt(self: &Rc<Self>, kind: PromptKind, title: &str, text: &str) {
        {
            let mut st = self.st.borrow_mut();
            st.media.prompt = Some(kind);
            st.ctx = lfx_nav::InputContext::TextInput;
        }
        self.with_media(|g| {
            g.set_prompt_title(title.into());
            g.set_prompt_text(text.into());
            g.set_prompt_open(true);
            g.set_prompt_tick(g.get_prompt_tick() + 1);
        });
    }

    fn prompt_close(self: &Rc<Self>) -> Option<PromptKind> {
        let k = self.st.borrow_mut().media.prompt.take();
        self.st.borrow_mut().ctx = lfx_nav::InputContext::Browse;
        self.with_media(|g| g.set_prompt_open(false));
        self.win().invoke_focus_root();
        k
    }

    fn prompt_cancel(self: &Rc<Self>) {
        let _ = self.prompt_close();
    }

    fn prompt_submit(self: &Rc<Self>) {
        let text = self.with_media(|g| g.get_prompt_text().to_string());
        let name = text.trim().to_owned();
        if self.st.borrow().media.prompt.is_none() {
            return;
        }
        if name.is_empty() {
            return;
        }
        let Some(kind) = self.prompt_close() else {
            return;
        };
        match kind {
            PromptKind::NewList(src) => self.create_playlist(name, src),
            PromptKind::Rename(id) => self.run_act(Act::RenameList(id, name)),
        }
    }

    // ------------------------------------------------------------ Aktionen ausführen

    pub fn run_act(self: &Rc<Self>, act: Act) {
        match act {
            Act::Play {
                src,
                shuffle,
                start,
            } => self.with_src(src, move |app, items| {
                app.audio_play(items, start, Some(shuffle));
            }),
            Act::PlayNext(src) => self.with_src(src, |app, items| app.audio_play_next(items)),
            Act::Enqueue(src) => self.with_src(src, |app, items| app.audio_enqueue(items)),
            Act::ChooseList(src) => self.choose_list(src),
            Act::ToList { list, src } => self.with_src(src, move |app, items| {
                app.add_to_playlist(&list, &items);
            }),
            Act::NewList(src) => {
                self.open_prompt(PromptKind::NewList(src), "Neue Wiedergabeliste", "");
            }
            Act::MoveEntry { list, entry, to } => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.playlist_move(&list, &entry, to).await;
                    post(move |app| app.after_edit(r.is_ok(), "Reihenfolge geändert."));
                });
            }
            Act::RemoveEntry { list, entry } => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.playlist_remove(&list, &[entry.as_str()]).await;
                    post(move |app| app.after_edit(r.is_ok(), "Aus der Liste entfernt."));
                });
            }
            Act::Mix(id) => self.audio_mix(&id),
            Act::DeleteItem(id) => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.delete_item(&id).await;
                    post(move |app| {
                        let back = app
                            .st
                            .borrow()
                            .media
                            .route
                            .as_ref()
                            .is_some_and(|r| r.kind == MediaKind::Playlist);
                        if r.is_ok() && back {
                            app.go_back();
                            app.toast("Wiedergabeliste gelöscht.", ToastKind::Success);
                        } else {
                            app.after_edit(r.is_ok(), "Wiedergabeliste gelöscht.");
                        }
                    });
                });
            }
            Act::RenameList(id, name) => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.rename_playlist(&id, &name).await;
                    post(move |app| app.after_edit(r.is_ok(), "Wiedergabeliste umbenannt."));
                });
            }
            Act::CancelTimer(id) => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.cancel_timer(&id).await;
                    post(move |app| app.after_edit(r.is_ok(), "Timer gelöscht."));
                });
            }
            Act::DeleteRecording(id) => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.delete_recording(&id).await;
                    post(move |app| app.after_edit(r.is_ok(), "Aufnahme gelöscht."));
                });
            }
            Act::PlayItem(item) => {
                self.audio_stop();
                self.play_item_with(&item, crate::play::ResumeMode::Restart);
            }
            Act::Record(program) => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let r = client.record_program(&program).await;
                    post(move |app| app.after_edit(r.is_ok(), "Aufnahme geplant."));
                });
            }
            Act::QueueRemove(i) => {
                if i != usize::MAX {
                    self.audio_remove(i);
                }
            }
        }
    }

    /// Meldung nach einer Änderung auf dem Server und Seite neu laden.
    pub fn after_edit(self: &Rc<Self>, ok: bool, text: &str) {
        if ok {
            self.toast(text, ToastKind::Success);
            let route = self.st.borrow().media.route.clone();
            if let Some(r) = route
                && is_media_route(self.st.borrow().router.current())
            {
                let (zone, index) = {
                    let st = self.st.borrow();
                    (st.media.zone, st.media.index)
                };
                self.render_media_route(&r);
                let mut st = self.st.borrow_mut();
                st.media.zone = zone;
                st.media.index = index;
            }
        } else {
            self.toast("Das hat nicht geklappt.", ToastKind::Error);
        }
    }

    /// Löst eine Quelle in Titel auf (Behälter werden vom Server geladen) und ruft dann `f` im UI-Thread.
    fn with_src(
        self: &Rc<Self>,
        src: Src,
        f: impl FnOnce(&Rc<App>, Vec<BaseItem>) + Send + 'static,
    ) {
        match src {
            Src::Items(items) => f(self, items),
            Src::Container(c) => {
                let Some(client) = self.media_client() else {
                    return;
                };
                self.rt.spawn(async move {
                    let items = container_tracks(&client, &c).await.unwrap_or_default();
                    post(move |app| {
                        if items.is_empty() {
                            app.toast("Keine Titel gefunden.", ToastKind::Info);
                        } else {
                            f(app, items);
                        }
                    });
                });
            }
        }
    }

    fn choose_list(self: &Rc<Self>, src: Src) {
        let Some(client) = self.media_client() else {
            return;
        };
        self.rt.spawn(async move {
            let lists = client
                .items(
                    &ItemsQuery::new()
                        .include_types(&[ItemKind::Playlist])
                        .recursive(true)
                        .sort_by(&[SortBy::SortName])
                        .limit(100),
                )
                .await
                .map(|r| r.items)
                .unwrap_or_default();
            post(move |app| {
                let mut e: Vec<(String, Act)> = lists
                    .into_iter()
                    .map(|l| {
                        (
                            l.name.clone(),
                            Act::ToList {
                                list: l.id,
                                src: src.clone(),
                            },
                        )
                    })
                    .collect();
                e.push(("Neue Wiedergabeliste …".into(), Act::NewList(src)));
                app.media_open_menu("Zur Wiedergabeliste hinzufügen", e);
            });
        });
    }

    fn create_playlist(self: &Rc<Self>, name: String, src: Src) {
        self.with_src_or_empty(src, move |app, items| {
            let Some(client) = app.media_client() else {
                return;
            };
            app.rt.spawn(async move {
                let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
                let r = client.create_playlist(&name, &ids, Some("Audio")).await;
                post(move |app| app.after_edit(r.is_ok(), "Wiedergabeliste angelegt."));
            });
        });
    }

    fn with_src_or_empty(
        self: &Rc<Self>,
        src: Src,
        f: impl FnOnce(&Rc<App>, Vec<BaseItem>) + Send + 'static,
    ) {
        match src {
            Src::Items(items) => f(self, items),
            other => self.with_src(other, f),
        }
    }

    fn add_to_playlist(self: &Rc<Self>, list: &str, items: &[BaseItem]) {
        let Some(client) = self.media_client() else {
            return;
        };
        let list = list.to_owned();
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        self.rt.spawn(async move {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let r = client.playlist_add(&list, &refs, None).await;
            post(move |app| {
                app.after_edit(r.is_ok(), "Zur Wiedergabeliste hinzugefügt.");
            });
        });
    }
}

/// Titel eines Behälters (Album, Künstler, Wiedergabeliste) oder der Titel selbst.
pub async fn container_tracks(
    client: &lfx_api::Client,
    c: &BaseItem,
) -> Result<Vec<BaseItem>, String> {
    let r = match c.kind {
        ItemKind::MusicAlbum => client.album_tracks(&c.id).await,
        ItemKind::Playlist => client.playlist_items(&c.id, 500, 0, &[]).await,
        ItemKind::MusicArtist => {
            client
                .tracks(
                    &ItemsQuery::new()
                        .artist_ids(&[c.id.as_str()])
                        .sort_by(&[SortBy::SortName])
                        .limit(500),
                )
                .await
        }
        _ => return Ok(vec![c.clone()]),
    }
    .map_err(|e| e.to_string())?;
    Ok(r.items
        .into_iter()
        .filter(|i| i.kind == ItemKind::Audio)
        .collect())
}

async fn load_media(
    client: &lfx_api::Client,
    m: &MediaRoute,
    view_id: &str,
) -> Result<Loaded, String> {
    let e = |e: lfx_api::ApiError| e.to_string();
    Ok(match m.kind {
        MediaKind::Music => match m.tab {
            0 => Loaded::Cards {
                head: None,
                items: client
                    .albums(None, Some(view_id), 300, 0)
                    .await
                    .map_err(e)?
                    .items,
                w: 190.0,
                h: 190.0,
                empty: "Keine Alben gefunden.",
            },
            1 => Loaded::Cards {
                head: None,
                items: client
                    .album_artists(
                        &ItemsQuery::new()
                            .parent_id(view_id)
                            .sort_by(&[SortBy::SortName])
                            .limit(300),
                    )
                    .await
                    .map_err(e)?
                    .items,
                w: 170.0,
                h: 170.0,
                empty: "Keine Künstler gefunden.",
            },
            2 => Loaded::Tracks {
                head: None,
                items: client
                    .tracks(
                        &ItemsQuery::new()
                            .parent_id(view_id)
                            .sort_by(&[SortBy::SortName])
                            .limit(500),
                    )
                    .await
                    .map_err(e)?
                    .items,
                empty: "Keine Titel gefunden.",
            },
            3 => Loaded::Cards {
                head: None,
                items: client
                    .music_genres(&ItemsQuery::new().parent_id(view_id).limit(200))
                    .await
                    .map_err(e)?
                    .items,
                w: 190.0,
                h: 120.0,
                empty: "Keine Genres gefunden.",
            },
            _ => {
                let mut items = vec![BaseItem {
                    id: "__new".into(),
                    name: "Neue Wiedergabeliste".into(),
                    kind: ItemKind::Playlist,
                    ..BaseItem::default()
                }];
                items.extend(
                    client
                        .items(
                            &ItemsQuery::new()
                                .include_types(&[ItemKind::Playlist])
                                .recursive(true)
                                .sort_by(&[SortBy::SortName])
                                .order(SortOrder::Ascending)
                                .limit(200),
                        )
                        .await
                        .map_err(e)?
                        .items,
                );
                Loaded::Cards {
                    head: None,
                    items,
                    w: 190.0,
                    h: 190.0,
                    empty: "",
                }
            }
        },
        MediaKind::Album => Loaded::Tracks {
            head: Some(client.item(&m.id, &[]).await.map_err(e)?),
            items: client.album_tracks(&m.id).await.map_err(e)?.items,
            empty: "Dieses Album hat keine Titel.",
        },
        MediaKind::Artist => Loaded::Cards {
            head: Some(client.item(&m.id, &[]).await.map_err(e)?),
            items: client
                .albums(Some(&m.id), None, 200, 0)
                .await
                .map_err(e)?
                .items,
            w: 190.0,
            h: 190.0,
            empty: "Keine Alben gefunden.",
        },
        MediaKind::Playlist => Loaded::Tracks {
            head: Some(client.item(&m.id, &[]).await.map_err(e)?),
            items: client
                .playlist_items(&m.id, 500, 0, &[])
                .await
                .map_err(e)?
                .items,
            empty: "Diese Wiedergabeliste ist leer.",
        },
        MediaKind::Books => Loaded::Cards {
            head: None,
            items: client.books(Some(view_id), 300, 0).await.map_err(e)?.items,
            w: 180.0,
            h: 270.0,
            empty: "Keine Bücher gefunden.",
        },
        MediaKind::Book => Loaded::Book(
            client
                .item(&m.id, &["Overview", "MediaSources"])
                .await
                .map_err(e)?,
        ),
        MediaKind::Photos => Loaded::Cards {
            head: None,
            items: client
                .photo_items(Some(&m.id), 300, 0)
                .await
                .map_err(e)?
                .items,
            w: 240.0,
            h: 180.0,
            empty: "Keine Fotos gefunden.",
        },
        MediaKind::LiveTv => crate::media_extra::load_live(client, m.tab).await?,
    })
}
