//! W2-detail: Hovercard (Maus). Nach 500 ms Hover wächst die Karte zur Hovercard (Bild, Aktionen, Metazeile, Genres,
//! Folgeninfo); sie bleibt offen, solange die Maus auf Karte oder Hovercard ist (150 ms Gnadenfrist). Tastatur und
//! Gamepad öffnen nie die Hovercard, dort öffnet die Info-Taste das Modal (siehe `detail.rs`).
//!
//! Maße und Zeiten nach `src/css/js-ui/hovercard.css` und `features/hovercard.js` der Web-Vorlage.

use std::rc::Rc;
use std::time::{Duration, Instant};

use lfx_api::{BaseItem, ItemKind};
use lfx_core::billboard::format_rating;
use lfx_core::images::{CardKind, card_images, display_title};
use lfx_images::Priority;
use lfx_ui::{DetailState as Ui, HoverData};
use slint::{Image, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app::{App, post};
use crate::detail::ITEM_TTL;
use crate::detail_logic::{
    self as logic, HC_CLOSE_ANIM_MS, HC_CLOSE_GRACE_MS, HC_DELAY_MS, RectF, popup_rect,
};
use crate::route::Route;

/// Zustand der Hovercard (hängt in `DetailCtl::hover`).
#[derive(Default)]
pub struct HoverCtl {
    /// Angezeigter Titel.
    pub item: Option<BaseItem>,
    /// (Zeile, Index) der Karte, zu der die Hovercard gehört.
    pub key: Option<(i32, i32)>,
    pub card_hover: bool,
    pub hc_hover: bool,
    pub visible: bool,
    pub image: Option<Image>,
    pub logo: Option<Image>,
    /// Zählt Öffnungen; späte Antworten und Zeitgeber alter Öffnungen werden verworfen.
    pub token: u64,
    show: Timer,
    close: Timer,
    anim: Timer,
    hide: Timer,
}

fn ss(s: impl Into<String>) -> SharedString {
    SharedString::from(s.into())
}

/// Anzeigedaten der Hovercard aus einem Titel (Bilder werden getrennt nachgereicht).
pub fn hover_data(
    item: &BaseItem,
    image: Option<&Image>,
    logo: Option<&Image>,
    can_download: bool,
) -> HoverData {
    let ud = item.user_data.clone().unwrap_or_default();
    let pct = logic::progress_percent(item);
    let playable = logic::is_playable(item);
    HoverData {
        title: ss(display_title(item)),
        has_image: image.is_some(),
        image: image.cloned().unwrap_or_default(),
        has_logo: logo.is_some(),
        logo: logo.cloned().unwrap_or_default(),
        rating: ss(format_rating(item.official_rating.as_deref())),
        year: ss(item
            .production_year
            .map(|y| y.to_string())
            .unwrap_or_default()),
        length: ss(logic::duration_text(item)),
        quality: ss(logic::quality_label(item, 3800)),
        genres: ModelRc::new(VecModel::from(
            item.genres
                .iter()
                .filter(|g| !g.is_empty())
                .take(3)
                .map(|g| ss(g.clone()))
                .collect::<Vec<_>>(),
        )),
        episode: ss(if item.kind == ItemKind::Episode {
            logic::episode_label(item)
        } else {
            String::new()
        }),
        progress: if ud.played || pct <= 0.0 {
            0.0
        } else {
            (pct / 100.0) as f32
        },
        remain: ss(if ud.played {
            String::new()
        } else {
            logic::remaining_text(item)
        }),
        fav: ud.is_favorite,
        like: ud.likes == Some(true),
        played: ud.played,
        can_like: playable || item.kind == ItemKind::BoxSet,
        can_download: can_download && matches!(item.kind, ItemKind::Movie | ItemKind::Episode),
        can_played: logic::can_mark_played(item),
    }
}

impl App {
    /// Titel der Karte (Zeile, Index) auf der aktuellen Seite.
    fn hover_item_at(&self, row: i32, idx: i32) -> Option<BaseItem> {
        let st = self.st.borrow();
        if !matches!(st.router.current(), Route::Home) || row < 0 || idx < 0 {
            return None;
        }
        st.home
            .rows
            .get(row as usize)?
            .items
            .get(idx as usize)
            .cloned()
    }

    /// Maus-Hover einer Karte: Rechteck in Fensterkoordinaten.
    pub fn on_card_hover(self: &Rc<Self>, row: i32, idx: i32, hovered: bool, card: RectF) {
        let blocked = {
            let st = self.st.borrow();
            st.detail.surface != crate::detail::Surface::None || st.detail.menu.is_some()
        };
        if blocked {
            return;
        }
        let key = (row, idx);
        if !hovered {
            let relevant = {
                let mut st = self.st.borrow_mut();
                let h = &mut st.detail.hover;
                if h.key == Some(key) || h.card_hover {
                    h.card_hover = false;
                }
                h.show.stop();
                h.visible && h.key == Some(key)
            };
            if relevant {
                self.start_close_timer();
            }
            return;
        }
        let (visible, same) = {
            let mut st = self.st.borrow_mut();
            let h = &mut st.detail.hover;
            h.card_hover = true;
            (h.visible, h.key == Some(key))
        };
        if same {
            self.st.borrow_mut().detail.hover.close.stop();
            return;
        }
        if visible {
            // Wechsel auf eine andere Karte: die offene Hovercard weicht sofort, die neue öffnet ohne Verzögerung.
            self.close_hovercard_now();
            self.open_hovercard(row, idx, card);
            return;
        }
        let weak = Rc::downgrade(self);
        let mut st = self.st.borrow_mut();
        let h = &mut st.detail.hover;
        h.card_hover = true;
        h.show.start(
            TimerMode::SingleShot,
            Duration::from_millis(HC_DELAY_MS),
            move || {
                if let Some(app) = weak.upgrade() {
                    let still = app.st.borrow().detail.hover.card_hover;
                    if still {
                        app.open_hovercard(row, idx, card);
                    }
                }
            },
        );
    }

    fn open_hovercard(self: &Rc<Self>, row: i32, idx: i32, card: RectF) {
        let Some(item) = self.hover_item_at(row, idx) else {
            return;
        };
        let (vw, vh) = self.window_logical();
        let r = popup_rect(card, vw, vh);
        let token = {
            let mut st = self.st.borrow_mut();
            let h = &mut st.detail.hover;
            h.token += 1;
            h.visible = true;
            h.key = Some((row, idx));
            h.item = Some(item.clone());
            h.image = None;
            h.logo = None;
            h.hc_hover = false;
            h.close.stop();
            h.hide.stop();
            h.token
        };
        let can_dl = self.st.borrow().detail.can_download.unwrap_or(false);
        self.with_ui(|_, ui| {
            ui.set_hc(hover_data(&item, None, None, can_dl));
            ui.set_hc_from_x(card.x);
            ui.set_hc_from_y(card.y);
            ui.set_hc_from_w(card.w);
            ui.set_hc_to_x(r.left);
            ui.set_hc_to_y(r.top);
            ui.set_hc_to_w(r.width);
            ui.set_hc_open(false);
            ui.set_hc_visible(true);
        });
        // Beim nächsten Takt auf das Zielrechteck wachsen (Slint animiert den Wechsel).
        let weak = Rc::downgrade(self);
        self.st.borrow_mut().detail.hover.anim.start(
            TimerMode::SingleShot,
            Duration::from_millis(30),
            move || {
                if let Some(app) = weak.upgrade()
                    && app.st.borrow().detail.hover.token == token
                {
                    app.with_ui(|_, ui| ui.set_hc_open(true));
                }
            },
        );
        self.load_hover_images(&item, token);
        self.load_hover_details(&item, token);
    }

    fn load_hover_images(self: &Rc<Self>, item: &BaseItem, token: u64) {
        let imgs = card_images(item, CardKind::Landscape, false, 2.0, 1.0);
        if let Some(r) = imgs.main {
            self.fetch_image(&r, Priority::Visible, move |app, img| {
                if app.st.borrow().detail.hover.token != token {
                    return;
                }
                app.st.borrow_mut().detail.hover.image = Some(img);
                app.refresh_hovercard_flags();
            });
        }
        if let Some(r) = imgs.logo {
            self.fetch_image(&r, Priority::Visible, move |app, img| {
                if app.st.borrow().detail.hover.token != token {
                    return;
                }
                app.st.borrow_mut().detail.hover.logo = Some(img);
                app.refresh_hovercard_flags();
            });
        }
    }

    /// Volle Angaben (Genres, Qualität, Freigabe) nachladen; der Zwischenspeicher gilt 5 Minuten.
    fn load_hover_details(self: &Rc<Self>, item: &BaseItem, token: u64) {
        let cached = {
            let st = self.st.borrow();
            st.detail
                .cache
                .get(&item.id)
                .filter(|(t, _)| t.elapsed() < ITEM_TTL)
                .map(|(_, i)| i.clone())
        };
        if let Some(full) = cached {
            self.st.borrow_mut().detail.hover.item = Some(full);
            self.refresh_hovercard_flags();
            return;
        }
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        let id = item.id.clone();
        self.rt.spawn(async move {
            let res = client
                .item(
                    &id,
                    &[
                        "Genres",
                        "OfficialRating",
                        "ProductionYear",
                        "RunTimeTicks",
                        "ChildCount",
                        "RecursiveItemCount",
                        "Width",
                        "MediaSources",
                        "UserData",
                    ],
                )
                .await;
            post(move |app| {
                if let Ok(full) = res {
                    app.st
                        .borrow_mut()
                        .detail
                        .cache
                        .insert(full.id.clone(), (Instant::now(), full.clone()));
                    if app.st.borrow().detail.hover.token == token {
                        app.st.borrow_mut().detail.hover.item = Some(full);
                        app.refresh_hovercard_flags();
                    }
                }
            });
        });
    }

    /// Schreibt die Anzeigedaten der offenen Hovercard neu (Favorit, Gesehen, Bilder, Details).
    pub fn refresh_hovercard_flags(self: &Rc<Self>) {
        let data = {
            let st = self.st.borrow();
            let h = &st.detail.hover;
            if !h.visible {
                return;
            }
            let Some(item) = &h.item else { return };
            hover_data(
                item,
                h.image.as_ref(),
                h.logo.as_ref(),
                st.detail.can_download.unwrap_or(false),
            )
        };
        self.with_ui(|_, ui| ui.set_hc(data));
    }

    fn start_close_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.st.borrow_mut().detail.hover.close.start(
            TimerMode::SingleShot,
            Duration::from_millis(HC_CLOSE_GRACE_MS),
            move || {
                if let Some(app) = weak.upgrade() {
                    let keep = {
                        let st = app.st.borrow();
                        st.detail.hover.card_hover || st.detail.hover.hc_hover
                    };
                    if !keep {
                        app.close_hovercard();
                    }
                }
            },
        );
    }

    /// Schließt mit Animation (Rückweg auf das Kartenrechteck, 200 ms).
    pub fn close_hovercard(self: &Rc<Self>) {
        let token = {
            let st = self.st.borrow();
            if !st.detail.hover.visible {
                return;
            }
            st.detail.hover.token
        };
        self.with_ui(|_, ui| ui.set_hc_open(false));
        let weak = Rc::downgrade(self);
        self.st.borrow_mut().detail.hover.hide.start(
            TimerMode::SingleShot,
            Duration::from_millis(HC_CLOSE_ANIM_MS),
            move || {
                if let Some(app) = weak.upgrade()
                    && app.st.borrow().detail.hover.token == token
                {
                    app.close_hovercard_now();
                }
            },
        );
    }

    /// Schließt sofort (Seitenwechsel, Modal, Tastatur).
    pub fn close_hovercard_now(self: &Rc<Self>) {
        let was = {
            let mut st = self.st.borrow_mut();
            let h = &mut st.detail.hover;
            h.show.stop();
            h.close.stop();
            h.anim.stop();
            h.hide.stop();
            let was = h.visible;
            h.visible = false;
            h.key = None;
            h.item = None;
            h.hc_hover = false;
            h.token += 1;
            was
        };
        if was {
            self.with_ui(|_, ui| {
                ui.set_hc_visible(false);
                ui.set_hc_open(false);
            });
        }
    }

    fn hover_item(&self) -> Option<BaseItem> {
        self.st.borrow().detail.hover.item.clone()
    }
}

/// Rückrufe der Hovercard und der Karten (Hover, Rechtsklick).
pub fn wire(app: &Rc<App>, g: &Ui<'_>) {
    let a = app.clone();
    g.on_card_hover(move |row, idx, hovered, x, y, w, h| {
        a.on_card_hover(row, idx, hovered, RectF { x, y, w, h });
    });
    let a = app.clone();
    g.on_card_context(move |row, idx, x, y| {
        if let Some(item) = a.hover_item_at(row, idx) {
            a.open_menu_for(item, x, y, false);
        }
    });
    let a = app.clone();
    g.on_hc_hovered(move |h| {
        {
            let mut st = a.st.borrow_mut();
            let hv = &mut st.detail.hover;
            hv.hc_hover = h;
            if h {
                hv.close.stop();
            }
        }
        if !h && a.st.borrow().detail.hover.visible && !a.st.borrow().detail.hover.card_hover {
            a.start_close_timer();
        }
    });
    let a = app.clone();
    g.on_hc_play(move || {
        if let Some(item) = a.hover_item() {
            a.play_any(&item, false);
        }
    });
    let a = app.clone();
    g.on_hc_fav(move || {
        if let Some(item) = a.hover_item() {
            a.toggle_fav(&item.id);
        }
    });
    let a = app.clone();
    g.on_hc_like(move || {
        if let Some(item) = a.hover_item() {
            a.toggle_like(&item.id);
        }
    });
    let a = app.clone();
    g.on_hc_played(move || {
        if let Some(item) = a.hover_item() {
            a.toggle_played(&item.id);
        }
    });
    let a = app.clone();
    g.on_hc_download(move || {
        a.toast(
            "Herunterladen folgt in einer späteren Version.",
            lfx_ui::ToastKind::Info,
        );
    });
    let a = app.clone();
    g.on_hc_more(move || {
        if let Some(item) = a.hover_item() {
            a.open_modal(&item.id.clone(), Some(item));
        }
    });
}
