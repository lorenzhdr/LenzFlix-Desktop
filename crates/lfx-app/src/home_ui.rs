//! Startseite (W2-home): Zustand der Kopfleiste, Menüs (Profil, Datensparmodus, Geheimmenü), Ambient-Farbe und
//! Ereignisse der Oberfläche. Die Oberfläche (`HomePage`) meldet alles als `home-event(name, zahl)`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use lfx_core::color::DEFAULT_AMBIENT;
use lfx_core::datasaver::{DataSaver, Effects, ImageScales, Part, available_for_design};
use lfx_core::trend::TrendInfo;
use lfx_nav::NavAction;
use lfx_ui::{HomeLayout, HomeUi, ToastKind};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::home_trailer::TrailerState;
use crate::route::Route;

/// Schlüssel der Einstellungen (eigener App-Speicher, nie die Web-Einstellungen).
const KEY_DS: &str = "ui.datasaver";
const KEY_SCALE: &str = "ui.imgscale";

/// Fünf Klicks in drei Sekunden öffnen das Geheimmenü.
const SECRET_CLICKS: usize = 5;
const SECRET_WINDOW: Duration = Duration::from_secs(3);

/// Zusatzzustand der Startseite.
pub struct HomeExtra {
    pub layout: Option<HomeLayout>,
    /// 0 keins, 1 Profilmenü, 2 Datensparmodus, 3 Geheimmenü.
    pub menu: i32,
    pub menu_index: i32,
    pub ds: DataSaver,
    pub scales: ImageScales,
    pub settings_loaded: bool,
    pub ambient: [u8; 3],
    pub trend: TrendInfo,
    pub secret_clicks: Vec<Instant>,
    pub trailer: TrailerState,
    /// Ungekürzter Pool (der Datensparteil `hero` kürzt `HomeState::pool`).
    pub pool_full: Vec<lfx_api::BaseItem>,
}

impl Default for HomeExtra {
    fn default() -> Self {
        Self {
            layout: None,
            menu: 0,
            menu_index: -1,
            ds: DataSaver::default(),
            scales: ImageScales::default(),
            settings_loaded: false,
            ambient: DEFAULT_AMBIENT,
            trend: TrendInfo::default(),
            secret_clicks: Vec::new(),
            trailer: TrailerState::default(),
            pool_full: Vec::new(),
        }
    }
}

/// Einträge je Menü (für die Tastatursteuerung).
fn menu_len(menu: i32) -> i32 {
    match menu {
        1 => 5,
        2 => 8,
        _ => 0,
    }
}

impl App {
    /// Wirkungen des Datensparmodus für das aktuelle Design.
    pub fn effects(&self) -> Effects {
        let st = self.st.borrow();
        let avail = available_for_design(st.design.slug());
        st.home.x.ds.effects(avail, &st.home.x.scales)
    }

    /// Teil des Datensparmodus wirkt gerade?
    pub fn ds_part_active(&self, part: Part) -> bool {
        let st = self.st.borrow();
        let avail = available_for_design(st.design.slug());
        st.home.x.ds.is_part_active(avail, part)
    }

    /// Lädt Datensparmodus und Bildgrößen des Benutzers (einmal je Sitzung).
    pub fn load_home_settings(&self) {
        if self.st.borrow().home.x.settings_loaded {
            return;
        }
        let Some(a) = self.session.active() else {
            return;
        };
        let ds = self
            .store
            .get_setting_as::<DataSaver>(&a.server_id, &a.user_id, KEY_DS)
            .ok()
            .flatten()
            .unwrap_or_default();
        let scales = self
            .store
            .get_setting_as::<ImageScales>(&a.server_id, &a.user_id, KEY_SCALE)
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut st = self.st.borrow_mut();
        st.home.x.ds = ds;
        st.home.x.scales = scales;
        st.home.x.settings_loaded = true;
    }

    fn save_ds(&self) {
        let ds = self.st.borrow().home.x.ds.clone();
        if let Some(a) = self.session.active()
            && let Err(e) = self
                .store
                .set_setting_as(&a.server_id, &a.user_id, KEY_DS, &ds)
        {
            tracing::warn!("Datensparmodus nicht gespeichert: {e}");
        }
    }

    fn save_scales(&self) {
        let s = self.st.borrow().home.x.scales;
        if let Some(a) = self.session.active()
            && let Err(e) = self
                .store
                .set_setting_as(&a.server_id, &a.user_id, KEY_SCALE, &s)
        {
            tracing::warn!("Bildgrößen nicht gespeichert: {e}");
        }
    }

    /// Bildskala für neu geladene Bilder (Datensparmodus: kleiner).
    pub fn image_scale(&self) -> f64 {
        self.effects().image_scale
    }

    /// Schreibt Kopfleisten-, Menü- und Billboard-Zustand in die Oberfläche.
    pub fn push_home_ui(&self) {
        let ui = self.win();
        let st = self.st.borrow();
        let x = &st.home.x;
        let avail = available_for_design(st.design.slug());
        let parts: Vec<bool> = x.ds.parts_state().iter().map(|(_, on)| *on).collect();
        let labels: Vec<SharedString> = Part::ALL.iter().map(|p| p.label().into()).collect();
        let [r, g, b] = x.ambient;
        let user = self
            .session
            .active()
            .map(|a| a.user_name.clone())
            .unwrap_or_default();
        let initial: String = user
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default();
        let hero_count = st.home.pool.len() as i32;
        let hero_item = st.home.pool.get(st.home.hero_idx);
        let badge = hero_item
            .and_then(|i| lfx_core::billboard::badge_for(i, App::now_ms()))
            .unwrap_or("");
        let primary = if hero_item.is_some_and(lfx_core::billboard::has_progress) {
            "Fortsetzen"
        } else {
            "Abspielen"
        };
        let (width, height) = self.shell.window_info().0;
        let (dpr_w, dpr_h) = (width, height);
        let info = format!(
            "Design: {}\nBildschirm: {} × {} px",
            st.design.slug(),
            dpr_w,
            dpr_h
        );
        let t = &x.trailer;
        let chrome = HomeUi {
            ambient: Color::from_rgb_u8(r, g, b),
            ds_on: x.ds.is_active(avail),
            ds_parts: ModelRc::new(VecModel::from(parts)),
            ds_labels: ModelRc::new(VecModel::from(labels)),
            user_initial: initial.into(),
            user_name: user.into(),
            menu: x.menu,
            menu_index: x.menu_index,
            hero_count,
            hero_index: st.home.hero_idx as i32,
            trailer_on: t.visible,
            trailer_muted: t.muted,
            trailer_btn: t.visible,
            scale_normal: (x.scales.normal() * 100.0).round() as i32,
            scale_saver: (x.scales.saver() * 100.0).round() as i32,
            info_text: info.into(),
            badge: badge.into(),
            primary: primary.into(),
        };
        // W2-integrate: dieselbe Kopfleiste steht auf den Bibliotheksseiten
        ui.global::<lfx_ui::LibraryState>()
            .set_chrome(chrome.clone());
        ui.set_home_ui(chrome);
    }

    pub fn set_ambient(&self, rgb: [u8; 3]) {
        self.st.borrow_mut().home.x.ambient = rgb;
        self.push_home_ui();
    }

    // ---------------------------------------------------------------- Menüs

    pub fn home_open_menu(self: &Rc<Self>, menu: i32) {
        {
            let mut st = self.st.borrow_mut();
            let x = &mut st.home.x;
            x.menu = if x.menu == menu { 0 } else { menu };
            x.menu_index = if x.menu == 0 { -1 } else { 0 };
        }
        self.push_home_ui();
    }

    pub fn home_close_menu(self: &Rc<Self>) {
        let was = self.st.borrow().home.x.menu;
        if was == 0 {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            st.home.x.menu = 0;
            st.home.x.menu_index = -1;
        }
        self.push_home_ui();
        self.win().invoke_focus_root();
    }

    /// Tastaturaktion, solange ein Menü offen ist. `true` = verbraucht.
    pub fn home_menu_action(self: &Rc<Self>, action: NavAction) -> bool {
        let (menu, idx) = {
            let st = self.st.borrow();
            if !matches!(st.router.current(), Route::Home) {
                return false;
            }
            (st.home.x.menu, st.home.x.menu_index)
        };
        if menu == 0 {
            return false;
        }
        let n = menu_len(menu);
        match action {
            NavAction::Back | NavAction::Menu => self.home_close_menu(),
            NavAction::Up | NavAction::Down if n > 0 => {
                let d = if action == NavAction::Up { -1 } else { 1 };
                let next = (idx + d).rem_euclid(n);
                self.st.borrow_mut().home.x.menu_index = next;
                self.push_home_ui();
            }
            NavAction::Activate => self.menu_activate(menu, idx),
            _ => {}
        }
        true
    }

    fn menu_activate(self: &Rc<Self>, menu: i32, idx: i32) {
        match menu {
            1 => self.profile_item(idx),
            2 => match idx {
                0 => self.ds_toggle_main(),
                1..=6 => self.ds_toggle_part((idx - 1) as usize),
                7 => self.ds_reset(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Eintrag des Profilmenüs.
    pub fn profile_item(self: &Rc<Self>, idx: i32) {
        self.home_close_menu();
        match idx {
            0 => {
                // „Wer schaut?“: zur Profilauswahl des Servers, ohne den Benutzer abzumelden.
                let server = self
                    .st
                    .borrow()
                    .server_id
                    .clone()
                    .or_else(|| self.session.active().map(|a| a.server_id.clone()));
                if let Some(id) = server {
                    self.reset_home();
                    self.show_profiles(id);
                }
            }
            1 => self.go(Route::Settings),
            2 => self.toast(
                "Die Statistik folgt in einer späteren Version.",
                ToastKind::Info,
            ),
            3 => self.go(Route::About),
            4 => self.sign_out(),
            _ => {}
        }
    }

    // ---------------------------------------------------------------- Datensparmodus

    pub fn ds_toggle_main(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.home.x.ds.on = !st.home.x.ds.on;
        }
        self.save_ds();
        self.ds_changed();
    }

    pub fn ds_toggle_part(self: &Rc<Self>, i: usize) {
        let Some(part) = Part::ALL.get(i).copied() else {
            return;
        };
        {
            let mut st = self.st.borrow_mut();
            let now_on = !st.home.x.ds.disabled_parts.contains(&part);
            st.home.x.ds.set_part(part, !now_on);
        }
        self.save_ds();
        self.ds_changed();
    }

    pub fn ds_reset(self: &Rc<Self>) {
        self.st.borrow_mut().home.x.ds.reset_parts();
        self.save_ds();
        self.ds_changed();
    }

    /// Reagiert laufende Vorgänge auf den Datensparmodus (Trailer, Pool-Größe, Vorladen).
    fn ds_changed(self: &Rc<Self>) {
        self.apply_pool_cap();
        if !self.effects().auto_trailer {
            self.stop_trailer();
        } else {
            self.schedule_trailer();
        }
        self.push_home_ui();
        self.prefetch_rest();
    }

    // ---------------------------------------------------------------- Ereignisse der Oberfläche

    pub fn on_home_layout(self: &Rc<Self>, l: HomeLayout) {
        if self.st.borrow().home.x.layout.as_ref() == Some(&l) {
            return;
        }
        self.st.borrow_mut().home.x.layout = Some(l);
        if matches!(self.st.borrow().router.current(), Route::Home) {
            self.rebuild_home_graph();
            self.set_default_home_focus_pub();
            self.show_focus();
        }
    }

    pub fn on_home_event(self: &Rc<Self>, name: &str, arg: i32) {
        match name {
            "nav" => self.nav_selected(arg),
            "logo" => self.logo_clicked(),
            "hero-play" => self.hero_clicked(false),
            "hero-info" => self.hero_clicked(true),
            "mute" => self.trailer_toggle_mute(),
            "dot" => self.hero_goto(arg.max(0) as usize),
            "menu-close" => self.home_close_menu(),
            "profile-item" => self.profile_item(arg),
            "ds-main" => self.ds_toggle_main(),
            "ds-part" => self.ds_toggle_part(arg.max(0) as usize),
            "ds-reset" => self.ds_reset(),
            "row-view" => self.row_view(arg),
            "clear-cache" => {
                self.toast(
                    "Der Bild-Cache wird beim nächsten Start neu aufgebaut.",
                    ToastKind::Info,
                );
            }
            _ => {}
        }
    }

    pub fn on_home_event_f(self: &Rc<Self>, name: &str, v: f32) {
        let pct = f64::from(v.round()) / 100.0;
        {
            let mut st = self.st.borrow_mut();
            match name {
                "normal" => st.home.x.scales.set(Some(pct), None),
                "saver" => st.home.x.scales.set(None, Some(pct)),
                _ => return,
            }
        }
        self.save_scales();
        self.push_home_ui();
    }

    /// Klick auf das Logo: fünf Klicks in drei Sekunden öffnen das Geheimmenü, sonst zur Startseite.
    fn logo_clicked(self: &Rc<Self>) {
        let now = Instant::now();
        let open = {
            let mut st = self.st.borrow_mut();
            let c = &mut st.home.x.secret_clicks;
            c.retain(|t| now.duration_since(*t) <= SECRET_WINDOW);
            c.push(now);
            if c.len() >= SECRET_CLICKS {
                c.clear();
                true
            } else {
                false
            }
        };
        if open {
            self.st.borrow_mut().home.x.menu = 3;
            self.st.borrow_mut().home.x.menu_index = -1;
            self.push_home_ui();
        } else if matches!(self.st.borrow().router.current(), Route::Home) {
            // Auf der Startseite: Billboard und Zeilen zurück nach oben (Fokus Start).
            let t = lfx_nav::Target {
                zone: crate::focus::Z_NAV,
                index: 0,
            };
            let _ = self.st.borrow_mut().graph.set_focus(t);
            self.show_focus();
        }
    }
}
