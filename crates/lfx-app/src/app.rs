//! Der Kern der App: Zustand, Router, Seitenaufbau und Eingabe (Tastatur, Gamepad, Automation).
//!
//! Alles hier läuft im UI-Thread. Netzwerk und Bilder laufen auf der Tokio-Laufzeit und melden ihr Ergebnis
//! über [`post`] zurück in den UI-Thread.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use lfx_api::BaseItem;
use lfx_images::ImageLoader;
use lfx_nav::{FocusGraph, InputContext, NavAction, NavOutcome, Target};
use lfx_platform::AppDirs;
use lfx_session::{SessionManager, UserChoice};
use lfx_shell::{HitRegions, Rect, ShellHandle};
use lfx_store::Store;
use lfx_ui::{Design, MainWindow, Page, Theme};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, VecModel};

use crate::cli::{DesignId, Options};
use crate::focus;
use crate::home::HomeState;
use crate::play::PlayState;
use crate::route::{Route, Router};

/// Beschriftung der Navigationsleiste (Reihenfolge = Fokus-Index).
pub const NAV_LABELS: [&str; 8] = [
    "Start",
    "Serien",
    "Filme",
    "Neu und beliebt",
    "Meine Liste",
    "Suche",
    "TV",
    "Einstellungen",
];

/// Veränderlicher Zustand (nur UI-Thread).
pub struct State {
    pub router: Router,
    pub graph: FocusGraph,
    pub ctx: InputContext,
    pub design: DesignId,
    /// Server, auf den sich die Anmeldeseiten beziehen.
    pub server_id: Option<String>,
    pub users: Vec<UserChoice>,
    pub login_user: Option<String>,
    pub qc_task: Option<tokio::task::AbortHandle>,
    pub home: HomeState,
    pub play: PlayState,
    pub geo: Option<focus::HomeGeo>,
    pub images: Option<ImageLoader>,
    pub toast_gen: u64,
}

/// Zeitgeber, die leben müssen, solange die App läuft.
#[derive(Default)]
pub struct Timers {
    pub osd: Timer,
    pub hero: Timer,
    pub toast: Timer,
    pub pad: Timer,
}

/// Die App.
pub struct App {
    pub ui: slint::Weak<MainWindow>,
    pub shell: ShellHandle,
    pub rt: tokio::runtime::Handle,
    pub opts: Options,
    pub dirs: AppDirs,
    pub session: SessionManager,
    pub store: Store,
    pub st: RefCell<State>,
    pub timers: Timers,
    pub gamepad: RefCell<Option<(lfx_nav::gilrs_backend::GilrsPump, lfx_nav::GamepadMapper)>>,
    pub started: Instant,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Setzt die App für [`post`].
pub fn install(app: Rc<App>) {
    APP.with(|a| *a.borrow_mut() = Some(app));
}

/// Entfernt die App (beim Beenden).
pub fn uninstall() {
    APP.with(|a| *a.borrow_mut() = None);
}

/// Die App im UI-Thread, falls installiert.
pub fn current() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

/// Führt `f` später im UI-Thread aus (von beliebigen Threads aufrufbar).
pub fn post(f: impl FnOnce(&Rc<App>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = current() {
            f(&app);
        }
    });
}

fn design_to_slint(d: DesignId) -> Design {
    match d {
        DesignId::Flix => Design::Flix,
        DesignId::Plus => Design::Plus,
        DesignId::Prime => Design::Prime,
        DesignId::Retro => Design::Retro,
    }
}

impl App {
    /// Das Fenster (nur im UI-Thread aufrufen, solange es lebt).
    pub fn win(&self) -> MainWindow {
        self.ui.upgrade().expect("Hauptfenster lebt")
    }

    pub fn now_ms() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_millis() as f64)
    }

    // ---------------------------------------------------------------- Start

    /// Erstes Bild: Design, Titelleiste, Startroute.
    pub fn start(self: &Rc<Self>) {
        let design = self.st.borrow().design;
        self.apply_design(design);
        let ui = self.win();
        ui.set_mac(cfg!(target_os = "macos"));
        ui.set_version(env!("CARGO_PKG_VERSION").into());
        ui.set_notices(crate::notices().into());
        ui.set_nav_labels(ModelRc::new(VecModel::from(
            NAV_LABELS
                .iter()
                .map(|s| SharedString::from(*s))
                .collect::<Vec<_>>(),
        )));
        if let Some(url) = &self.opts.server {
            ui.set_server_url(url.as_str().into());
        }
        if let Some(active) = self.session.active() {
            tracing::info!("gespeicherte Sitzung wiederhergestellt");
            self.on_session_ready(active.server_id.clone(), active.user_id.clone());
            self.go_reset(Route::Home);
            return;
        }
        let servers = self.session.servers();
        let pick = self
            .session
            .last_selection()
            .map(|s| s.server_id)
            .or_else(|| servers.first().map(|s| s.id.clone()));
        match pick {
            Some(id) if self.opts.server.is_none() => self.show_profiles(id),
            _ => self.go_reset(Route::AddServer),
        }
    }

    pub fn apply_design(&self, d: DesignId) {
        self.st.borrow_mut().design = d;
        self.win().global::<Theme>().set_design(design_to_slint(d));
        self.win().set_design(d.index());
    }

    // ---------------------------------------------------------------- Routen

    pub fn go(self: &Rc<Self>, route: Route) {
        self.leave_route_hooks();
        self.st.borrow_mut().router.push(route);
        self.render_route();
    }

    pub fn go_reset(self: &Rc<Self>, route: Route) {
        self.leave_route_hooks();
        self.st.borrow_mut().router.reset(route);
        self.render_route();
    }

    pub fn go_replace(self: &Rc<Self>, route: Route) {
        self.leave_route_hooks();
        self.st.borrow_mut().router.replace(route);
        self.render_route();
    }

    /// Zurück im Verlauf (Wurzel: nichts).
    pub fn go_back(self: &Rc<Self>) {
        self.leave_route_hooks();
        let _ = self.st.borrow_mut().router.back();
        self.render_route();
    }

    /// Aufräumen beim Verlassen einer Seite (Quick Connect abbrechen).
    fn leave_route_hooks(self: &Rc<Self>) {
        self.stop_trailer(); // W2-home: Trailer endet bei jedem Seitenwechsel und beim Wiedergabe-Start
        let task = self.st.borrow_mut().qc_task.take();
        if let Some(t) = task {
            t.abort();
        }
        let ui = self.win();
        ui.set_qc_code("".into());
        ui.set_login_busy(false);
        ui.set_login_error("".into());
        ui.set_add_error("".into());
        ui.set_profile_error("".into());
    }

    /// Setzt Seite, Fokuszonen und Anzeige passend zur aktuellen Route.
    pub fn render_route(self: &Rc<Self>) {
        let route = self.st.borrow().router.current().clone();
        let ui = self.win();
        tracing::info!(route = route.name(), "Seite");
        let mut ctx = InputContext::Browse;
        match &route {
            Route::AddServer => {
                ui.set_page(Page::AddServer);
                self.set_graph(focus::column_graph(2), Some(0));
                ui.set_add_slot(0);
                ctx = InputContext::TextInput;
                self.focus_field_later();
            }
            Route::Profiles { .. } => {
                ui.set_page(Page::Profiles);
                self.render_profiles_graph();
            }
            Route::Login { user, .. } => {
                ui.set_page(Page::Login);
                let first = usize::from(user.is_some());
                self.set_graph(focus::column_graph(5), Some(first));
                ui.set_login_slot(first as i32);
                ctx = InputContext::TextInput;
                self.focus_field_later();
            }
            Route::Home => {
                ui.set_page(Page::Home);
                self.st.borrow_mut().graph = FocusGraph::new();
                self.rebuild_home_graph();
                self.enter_home();
            }
            Route::Player { .. } => {
                ui.set_page(Page::Player);
                ctx = InputContext::Player;
                self.st.borrow_mut().graph = FocusGraph::new();
            }
            Route::Settings => {
                ui.set_page(Page::Settings);
                self.set_graph(focus::settings_graph(), None);
                self.st.borrow_mut().graph.focus_zone(lfx_nav::ZoneId(0));
                self.show_focus();
            }
            Route::About => {
                ui.set_page(Page::About);
                self.set_graph(focus::column_graph(1), Some(0));
            }
            other => {
                ui.set_page(Page::Soon);
                ui.set_soon_title(other.soon_title().unwrap_or("Folgt").into());
                self.set_graph(focus::column_graph(1), Some(0));
            }
        }
        self.st.borrow_mut().ctx = ctx;
        ui.invoke_focus_root();
        self.show_focus();
    }

    fn focus_field_later(self: &Rc<Self>) {
        let weak = self.ui.clone();
        slint::Timer::single_shot(Duration::from_millis(30), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_field_focus_tick(ui.get_field_focus_tick() + 1);
            }
        });
    }

    fn set_graph(&self, mut g: FocusGraph, focus_index: Option<usize>) {
        if let Some(i) = focus_index {
            g.set_focus(Target {
                zone: lfx_nav::ZoneId(0),
                index: i,
            });
        }
        self.st.borrow_mut().graph = g;
    }

    // ---------------------------------------------------------------- Fokusanzeige

    /// Schreibt den Fokus des Graphen in die Properties der aktuellen Seite.
    pub fn show_focus(self: &Rc<Self>) {
        let ui = self.win();
        let st = self.st.borrow();
        let f = st.graph.focused();
        match st.router.current() {
            Route::AddServer => ui.set_add_slot(focus::slot_of(f, 0)),
            Route::Login { .. } => ui.set_login_slot(focus::slot_of(f, 0)),
            Route::Profiles { .. } => {
                let n = st.users.len();
                ui.set_profile_slot(focus::slot_of(f, n));
            }
            Route::Home => {
                let (row, idx) = focus::home_props(f);
                ui.set_focus_row(row);
                ui.set_focus_index(idx);
            }
            Route::Settings => ui.set_simple_slot(focus::slot_of(f, 4)),
            Route::Player { .. } => ui.set_player_slot(-1),
            _ => ui.set_simple_slot(focus::slot_of(f, 0)),
        }
    }

    // ---------------------------------------------------------------- Eingabe

    /// Eine Navigationsaktion (Tastatur, Gamepad, Automation).
    pub fn apply_action(self: &Rc<Self>, action: NavAction) {
        self.player_activity();
        if self.home_menu_action(action) {
            return; // W2-home: offenes Menü der Startseite nimmt die Taste
        }
        let route = self.st.borrow().router.current().clone();
        if matches!(route, Route::Player { .. }) {
            self.player_action(action);
            return;
        }
        match action {
            NavAction::Back => {
                self.back_action(&route);
                return;
            }
            NavAction::Home => {
                if self.session.active().is_some() && route != Route::Home {
                    self.go_reset(Route::Home);
                }
                return;
            }
            NavAction::Search => {
                if self.session.active().is_some() {
                    self.go(Route::Search);
                }
                return;
            }
            _ => {}
        }
        let outcome = self.st.borrow_mut().graph.apply(action);
        match outcome {
            NavOutcome::Moved { to, .. } => {
                self.on_focus_moved(to);
                self.show_focus();
            }
            NavOutcome::Activated(t) => self.activate(t),
            NavOutcome::Back => self.back_action(&route),
            NavOutcome::Unhandled => {}
        }
    }

    fn on_focus_moved(self: &Rc<Self>, to: Target) {
        // Textfelder bekommen beim Betreten den Textcursor, damit sofort getippt werden kann.
        let route = self.st.borrow().router.current().clone();
        let text_slot = match route {
            Route::AddServer => to.index == 0,
            Route::Login { .. } => to.index <= 1,
            _ => false,
        };
        self.st.borrow_mut().ctx = if text_slot {
            InputContext::TextInput
        } else {
            InputContext::Browse
        };
        if text_slot {
            self.focus_field_later();
        } else {
            self.win().invoke_focus_root();
        }
    }

    fn back_action(self: &Rc<Self>, route: &Route) {
        match route {
            Route::Login { server, .. } => self.show_profiles(server.clone()),
            Route::Profiles { .. } => self.go_reset(Route::AddServer),
            Route::Home | Route::AddServer => {}
            _ => {
                let can_back = self.st.borrow().router.depth() > 1;
                if can_back {
                    self.go_back();
                } else if self.session.active().is_some() {
                    self.go_reset(Route::Home);
                }
            }
        }
    }

    /// Aktiviert das fokussierte Ziel der aktuellen Seite.
    pub fn activate(self: &Rc<Self>, t: Target) {
        let route = self.st.borrow().router.current().clone();
        match route {
            Route::AddServer => {
                if t.index == 1 || t.index == 0 {
                    let url = self.win().get_server_url().to_string();
                    self.submit_server(url);
                }
            }
            Route::Profiles { .. } => {
                let n = self.st.borrow().users.len();
                let slot = if t.zone.0 == 0 { t.index } else { n + t.index };
                self.profile_slot_activated(slot);
            }
            Route::Login { .. } => match t.index {
                0 | 1 => self.focus_field_later(),
                2 => {
                    let ui = self.win();
                    self.submit_login(
                        ui.get_login_user().to_string(),
                        ui.get_login_password().to_string(),
                    );
                }
                3 => self.start_quick_connect(),
                _ => {
                    if let Route::Login { server, .. } = &route {
                        self.show_profiles(server.clone());
                    }
                }
            },
            Route::Home => self.home_activate(t),
            Route::Settings => {
                let slot = focus::slot_of(Some(t), 4);
                match slot {
                    0..=3 => self.choose_design(DesignId::from_index(slot)),
                    4 => self.go(Route::About),
                    5 => self.sign_out(),
                    _ => self.go_back(),
                }
            }
            _ => self.apply_action(NavAction::Back),
        }
    }

    /// Eine Taste aus dem Fenster.
    pub fn key_event(self: &Rc<Self>, text: &str, ctrl: bool, shift: bool, alt: bool) {
        let Some(key) = crate::keys::from_slint(text) else {
            return;
        };
        let ctx = self.st.borrow().ctx;
        if let Some(action) = lfx_nav::map_key(key, crate::keys::modifiers(ctrl, shift, alt), ctx) {
            // Aus einem Textfeld hinaus: Escape/Enter wirken auf die Seite, der Fokus kehrt zur Wurzel zurück.
            if ctx == InputContext::TextInput {
                self.win().invoke_focus_root();
                self.st.borrow_mut().ctx = InputContext::Browse;
            }
            self.apply_action(action);
        }
    }

    // ---------------------------------------------------------------- Gamepad

    /// Startet die Gamepad-Abfrage (alle 16 ms).
    pub fn start_gamepad(self: &Rc<Self>) {
        let Some(pump) = lfx_nav::gilrs_backend::GilrsPump::new() else {
            tracing::info!("keine Gamepad-Schnittstelle");
            return;
        };
        *self.gamepad.borrow_mut() = Some((
            pump,
            lfx_nav::GamepadMapper::new(lfx_nav::GamepadConfig::default()),
        ));
        let weak = Rc::downgrade(self);
        self.timers.pad.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.poll_gamepad();
                }
            },
        );
    }

    fn poll_gamepad(self: &Rc<Self>) {
        let now = self.started.elapsed().as_millis() as u64;
        let ctx = self.st.borrow().ctx;
        let mut actions = Vec::new();
        if let Some((pump, mapper)) = self.gamepad.borrow_mut().as_mut() {
            while let Some(ev) = pump.next_event() {
                if let Some(i) = mapper.handle(ev, now, ctx) {
                    actions.push(i.action);
                }
            }
            if let Some(i) = mapper.poll(now) {
                actions.push(i.action);
            }
        }
        for a in actions {
            self.apply_action(a);
        }
    }

    // ---------------------------------------------------------------- Titelleiste

    pub fn set_hit_regions(&self, h: &lfx_ui::TitleBarHit) {
        let r = |x: &lfx_ui::HitRect| Rect::new(x.x, x.y, x.width, x.height);
        self.shell.set_hit_regions(HitRegions {
            caption: vec![r(&h.bar)],
            interactive: vec![r(&h.minimize), r(&h.close)],
            maximize_button: Some(r(&h.maximize)),
            ..HitRegions::default()
        });
    }

    // ---------------------------------------------------------------- Hinweise

    /// Kurzer Hinweistext unten im Fenster.
    pub fn toast(self: &Rc<Self>, text: &str, kind: lfx_ui::ToastKind) {
        let ui = self.win();
        ui.set_toast_kind(kind);
        ui.set_toast(text.into());
        let weak = self.ui.clone();
        self.timers.toast.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(4),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_toast("".into());
                }
            },
        );
    }

    // ---------------------------------------------------------------- Einstellungen

    pub fn choose_design(self: &Rc<Self>, d: DesignId) {
        self.apply_design(d);
        self.save_setting("ui.design", serde_json::Value::String(d.slug().to_owned()));
    }

    pub fn save_setting(&self, key: &str, value: serde_json::Value) {
        if let Some(a) = self.session.active()
            && let Err(e) = self
                .store
                .set_setting(&a.server_id, &a.user_id, key, &value)
        {
            tracing::warn!("Einstellung nicht gespeichert: {e}");
        }
    }

    /// Lädt die Einstellungen des Benutzers nach der Anmeldung.
    pub fn on_session_ready(self: &Rc<Self>, server_id: String, user_id: String) {
        if self.opts.design.is_none()
            && let Ok(Some(v)) = self.store.get_setting(&server_id, &user_id, "ui.design")
            && let Some(d) = v.as_str().and_then(DesignId::parse)
        {
            self.apply_design(d);
        }
        if let Some(a) = self.session.active() {
            self.win().set_user_name(a.user_name.as_str().into());
        }
        self.session.start_heartbeat();
    }

    pub fn sign_out(self: &Rc<Self>) {
        let session = self.session.clone();
        self.rt.spawn(async move {
            let _ = session.logout().await;
            post(move |a| {
                a.reset_home();
                let server = a.st.borrow().server_id.clone();
                match server {
                    Some(id) => a.show_profiles(id),
                    None => a.go_reset(Route::AddServer),
                }
            });
        });
    }

    /// Welche Items die Hero-Zone gerade zeigt (für Automation).
    pub fn focused_item(&self) -> Option<BaseItem> {
        let st = self.st.borrow();
        let t = st.graph.focused()?;
        if !matches!(st.router.current(), Route::Home) {
            return None;
        }
        let r = focus::zone_row(t.zone)?;
        st.home.rows.get(r)?.items.get(t.index).cloned()
    }

    /// Anzahl Karten je Zeile.
    pub fn row_counts(&self) -> Vec<(String, usize)> {
        self.st
            .borrow()
            .home
            .rows
            .iter()
            .map(|r| (r.title.clone(), r.items.len()))
            .collect()
    }

    /// Nach Enter im Benutzerfeld: Textcursor ins Passwortfeld.
    pub fn focus_after_accept(self: &Rc<Self>) {
        self.st.borrow_mut().ctx = InputContext::TextInput;
        self.show_focus();
        self.focus_field_later();
    }
}
