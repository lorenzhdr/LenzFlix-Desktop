//! W5-tray: Infobereich-Symbol (Windows) bzw. Menüleisten-Symbol (macOS) und Schließen-Verhalten.
//!
//! Das Symbol gibt es nur, solange „Beim Schließen im Infobereich weiterlaufen“ an ist (Standard aus). Dann
//! versteckt das Schließen des Fensters nur das Fenster; das Symbol-Menü bringt es zurück (`Öffnen`), pausiert die
//! Wiedergabe oder beendet die App wirklich. Ohne die Einstellung beendet das Schließen die App wie bisher.
//! Auf Systemen ohne Symbol-Unterstützung ([`native`] ohne `tray-icon`) bleibt es beim Beenden.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};

use crate::app::App;
use crate::tray_logic::{CloseDecision, TrayAction, TrayPrefs, close_decision, pause_label};

/// Zustand des Symbols (nur UI-Thread).
#[derive(Default)]
struct TrayCtl {
    prefs: TrayPrefs,
    icon: Option<native::Native>,
    /// Zuletzt ausgeführter Menüeintrag (für Tests).
    last_action: Option<TrayAction>,
    /// Aktualisiert die Pause-Beschriftung, solange das Symbol lebt.
    sync: Timer,
}

thread_local! {
    static TRAY: RefCell<TrayCtl> = RefCell::new(TrayCtl::default());
}

/// Symbol-Anbindung: echte Umsetzung mit `tray-icon` (Windows, macOS), sonst ein Stummel.
#[cfg(any(windows, target_os = "macos"))]
mod native {
    use std::sync::Once;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use crate::tray_logic::{TOOLTIP, TrayAction};

    const ICON_PNG: &[u8] = include_bytes!("../../../assets/app/lenzflix-tray.png");

    pub struct Native {
        _icon: TrayIcon,
        pause: MenuItem,
    }

    fn icon() -> Result<Icon, String> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(ICON_PNG))
            .read_info()
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err("Symbol ist kein RGBA8-Bild".into());
        }
        buf.truncate(info.buffer_size());
        Icon::from_rgba(buf, info.width, info.height).map_err(|e| e.to_string())
    }

    /// Die Ereignis-Handler der Bibliothek lassen sich nur einmal setzen: einmal verdrahten, danach läuft jedes
    /// neu angelegte Symbol über dieselbe Weiterleitung in den UI-Thread.
    fn wire_handlers() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            MenuEvent::set_event_handler(Some(|ev: MenuEvent| {
                if let Some(a) = TrayAction::from_id(ev.id().as_ref()) {
                    crate::app::post(move |app| app.tray_action(a));
                }
            }));
            TrayIconEvent::set_event_handler(Some(|ev: TrayIconEvent| match ev {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => {
                    // macOS öffnet bei Linksklick das Menü; dort würde das Fenster sonst nebenbei aufspringen.
                    if cfg!(windows) {
                        crate::app::post(|app| app.tray_action(TrayAction::Open));
                    }
                }
                _ => {}
            }));
        });
    }

    pub fn create() -> Result<Native, String> {
        wire_handlers();
        let menu = Menu::new();
        let open = MenuItem::with_id(TrayAction::Open.id(), "LENZFLIX öffnen", true, None);
        let pause = MenuItem::with_id(
            TrayAction::TogglePause.id(),
            "Wiedergabe pausieren",
            false,
            None,
        );
        let quit = MenuItem::with_id(TrayAction::Quit.id(), "Beenden", true, None);
        menu.append_items(&[&open, &pause, &quit])
            .map_err(|e| e.to_string())?;
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(TOOLTIP)
            .with_icon(icon()?);
        // Windows: Linksklick öffnet das Fenster, Rechtsklick das Menü (so kennt man es).
        #[cfg(windows)]
        let builder = builder.with_menu_on_left_click(false);
        let tray = builder.build().map_err(|e| e.to_string())?;
        Ok(Native { _icon: tray, pause })
    }

    impl Native {
        pub fn set_pause(&self, label: &str, enabled: bool) {
            self.pause.set_text(label);
            self.pause.set_enabled(enabled);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod native {
    pub struct Native;

    pub fn create() -> Result<Native, String> {
        Err("Infobereich-Symbol wird auf diesem System nicht unterstützt".into())
    }

    impl Native {
        pub fn set_pause(&self, _label: &str, _enabled: bool) {}
    }
}

impl App {
    /// Einstellungen laden, Schließen-Rückruf einhängen, Symbol anlegen (sobald die Ereignisschleife läuft).
    pub fn tray_start(self: &Rc<Self>) {
        let prefs = TrayPrefs::load(&TrayPrefs::path(&self.dirs.config));
        TRAY.with(|t| t.borrow_mut().prefs = prefs);
        let a = self.clone();
        self.win().window().on_close_requested(move || {
            match a.close_decision() {
                CloseDecision::HideToTray => {
                    a.tray_hide();
                    CloseRequestResponse::KeepWindowShown
                }
                // Standardweg: Fenster schließen, die Ereignisschleife endet.
                CloseDecision::Quit => CloseRequestResponse::HideWindow,
            }
        });
        if prefs.close_to_tray {
            // macOS verlangt, dass das Symbol erst nach dem Start der Ereignisschleife entsteht.
            let a = self.clone();
            Timer::single_shot(Duration::from_millis(0), move || a.tray_ensure());
        }
    }

    /// Einstellung „Beim Schließen im Infobereich weiterlaufen“.
    pub fn tray_close_enabled(&self) -> bool {
        TRAY.with(|t| t.borrow().prefs.close_to_tray)
    }

    /// Schaltet die Einstellung um, speichert sie und legt das Symbol an bzw. entfernt es.
    pub fn tray_set_close_enabled(self: &Rc<Self>, on: bool) {
        let prefs = TrayPrefs { close_to_tray: on };
        TRAY.with(|t| t.borrow_mut().prefs = prefs);
        if on {
            self.tray_ensure();
            // Ohne Symbol (nicht unterstützt) wäre die Einstellung wirkungslos: ehrlich zurücknehmen.
            if !self.tray_ready() {
                TRAY.with(|t| t.borrow_mut().prefs = TrayPrefs::default());
                self.tray_save();
                return;
            }
        } else {
            self.tray_remove();
        }
        self.tray_save();
    }

    fn tray_save(&self) {
        let prefs = TRAY.with(|t| t.borrow().prefs);
        if let Err(e) = prefs.save(&TrayPrefs::path(&self.dirs.config)) {
            tracing::warn!("Fenster-Einstellung nicht gespeichert: {e}");
        }
    }

    fn tray_ready(&self) -> bool {
        TRAY.with(|t| t.borrow().icon.is_some())
    }

    fn close_decision(&self) -> CloseDecision {
        TRAY.with(|t| {
            let t = t.borrow();
            close_decision(t.prefs, t.icon.is_some())
        })
    }

    /// Legt das Symbol an, falls die Einstellung an ist und noch keins existiert.
    fn tray_ensure(self: &Rc<Self>) {
        if self.tray_ready() || !self.tray_close_enabled() {
            return;
        }
        match native::create() {
            Ok(n) => {
                let weak = Rc::downgrade(self);
                TRAY.with(|t| {
                    let mut t = t.borrow_mut();
                    t.icon = Some(n);
                    t.sync.start(
                        TimerMode::Repeated,
                        Duration::from_millis(1000),
                        move || {
                            if let Some(app) = weak.upgrade() {
                                app.tray_sync();
                            }
                        },
                    );
                });
                self.tray_sync();
            }
            Err(e) => tracing::warn!("Infobereich-Symbol nicht anlegbar: {e}"),
        }
    }

    /// Entfernt das Symbol (Einstellung aus).
    fn tray_remove(&self) {
        self.tray_shutdown();
        // Ein verstecktes Fenster darf nie ohne Symbol zurückbleiben.
        if !self.shell.is_window_visible() {
            self.shell.show_window();
        }
    }

    /// Aufräumen beim Beenden (vor dem Abbau der Laufzeit).
    pub fn tray_shutdown(&self) {
        TRAY.with(|t| {
            let mut t = t.borrow_mut();
            t.sync.stop();
            t.icon = None;
        });
    }

    /// Pause-Eintrag an die Wiedergabe anpassen (Beschriftung, nur bei laufender Wiedergabe bedienbar).
    fn tray_sync(&self) {
        let (playing, paused) = self.playback_flags();
        TRAY.with(|t| {
            if let Some(n) = &t.borrow().icon {
                n.set_pause(pause_label(playing, paused), playing);
            }
        });
    }

    /// (läuft ein Titel, ist er pausiert)
    fn playback_flags(&self) -> (bool, bool) {
        let st = self.st.borrow();
        let p = &st.play;
        match p.player.as_ref().filter(|_| p.opened) {
            Some(pl) => {
                let s = pl.state();
                (s.loaded, s.paused)
            }
            None => (false, false),
        }
    }

    /// Fenster verstecken; die App läuft im Infobereich weiter.
    pub fn tray_hide(&self) {
        self.shell.hide_window();
    }

    /// Fenster zeigen und nach vorn holen (Symbol-Menü, zweiter Programmstart).
    pub fn tray_show(&self) {
        self.shell.show_window();
    }

    /// Schließen-Wunsch (X in der Titelleiste): verstecken oder beenden, je nach Einstellung.
    pub fn request_close(self: &Rc<Self>) {
        match self.close_decision() {
            CloseDecision::HideToTray => self.tray_hide(),
            CloseDecision::Quit => self.quit_app(),
        }
    }

    /// Beendet die App wirklich (am Verstecken vorbei).
    pub fn quit_app(&self) {
        let _ = slint::quit_event_loop();
    }

    /// Menüeintrag des Symbols.
    pub fn tray_action(self: &Rc<Self>, action: TrayAction) {
        TRAY.with(|t| t.borrow_mut().last_action = Some(action));
        match action {
            TrayAction::Open => self.tray_show(),
            TrayAction::TogglePause => {
                let (playing, _) = self.playback_flags();
                if playing {
                    self.toggle_pause();
                }
                self.tray_sync();
            }
            TrayAction::Quit => self.quit_app(),
        }
    }

    /// Zustand für Tests und Diagnose.
    pub fn tray_state_json(&self) -> serde_json::Value {
        let (playing, paused) = self.playback_flags();
        TRAY.with(|t| {
            let t = t.borrow();
            serde_json::json!({
                "close_to_tray": t.prefs.close_to_tray,
                "icon": t.icon.is_some(),
                "window_visible": self.shell.is_window_visible(),
                "last_action": t.last_action.map(TrayAction::id),
                "pause_label": pause_label(playing, paused),
            })
        })
    }
}
