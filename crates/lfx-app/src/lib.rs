//! LENZFLIX Desktop: die Anwendung.
//!
//! Verbindet die Fundament-Crates: `lfx-shell` (Fenster, Video-Ebene), `lfx-ui` (Slint-Seiten), `lfx-session`
//! (Server, Benutzer, Anmeldung), `lfx-core` (Zeilen, Billboard), `lfx-images` (Bilder), `lfx-player`
//! (Wiedergabe) und `lfx-nav` (Fokus, Tastatur, Gamepad). Router und Zustand liegen in Rust ([`route`], [`app`]),
//! die Seiten sind Slint-Komponenten in `lfx-ui`.

pub mod app;
pub mod auth;
#[cfg(feature = "dev-automation")]
pub mod automation;
pub mod capture;
pub mod cli;
pub mod detail; // W2-detail
#[cfg(feature = "dev-automation")]
pub mod detail_automation; // W2-detail
pub mod detail_logic; // W2-detail
pub mod focus;
pub mod home;
#[cfg(feature = "dev-automation")]
pub mod home_automation; // W2-home
pub mod home_focus; // W2-home
pub mod home_trailer; // W2-home
pub mod home_ui; // W2-home
pub mod hovercard; // W2-detail
pub mod keys;
pub mod library; // W2-library
mod library_impl; // W2-library
pub mod library_np; // W2-library
pub mod library_search; // W2-library
pub mod logging;
pub mod picker; // W3-picker
#[cfg(feature = "dev-automation")]
pub mod picker_automation; // W3-picker
pub mod picker_couch; // W3-picker
pub mod play;
#[cfg(feature = "dev-automation")]
pub mod play_auto; // W2-player
pub mod play_extras; // W2-player
pub mod play_hdr; // W2-player
pub mod play_osd; // W2-player
pub mod play_ui; // W2-player
pub mod plus; // W3-plus
pub mod prime_gate; // W3-prime
pub mod retro; // W3-retro
#[cfg(feature = "dev-automation")]
pub mod retro_automation; // W3-retro
pub mod retro_term; // W3-retro
pub mod route;
pub mod tvui; // W2-tvui
pub mod tvui_host; // W2-tvui
pub mod tvui_view; // W2-tvui
pub mod video;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use lfx_nav::InputContext;
use lfx_platform::{AppDirs, MemorySecretStore, SecretStore, os_secret_store};
use lfx_session::SessionManager;
use lfx_shell::{Shell, ShellOptions};
use lfx_store::Store;
use lfx_ui::MainWindow;
use slint::ComponentHandle;

use crate::app::{App, State, Timers};
use crate::cli::{DesignId, Options};
use crate::route::Router;

/// Hinweise zu Drittkomponenten für die Über-Seite.
pub fn notices() -> &'static str {
    include_str!("../../../THIRD-PARTY-NOTICES.md")
}

/// Ordner der App je nach Aufruf.
fn resolve_dirs(opts: &Options) -> Result<AppDirs, String> {
    match &opts.home {
        Some(root) => Ok(AppDirs::from_root(root)),
        None => {
            AppDirs::resolve().ok_or_else(|| "kein Ordner für die App-Daten ermittelbar".to_owned())
        }
    }
}

/// Startet die App und kehrt erst beim Beenden zurück.
pub fn run(opts: Options) -> Result<(), String> {
    let dirs = resolve_dirs(&opts)?;
    dirs.create_all()
        .map_err(|e| format!("App-Ordner nicht anlegbar: {e}"))?;
    logging::init(&dirs.data.join("logs").join("lenzflix.log"));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "LENZFLIX startet");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .thread_name("lfx-rt")
        .build()
        .map_err(|e| format!("Laufzeit nicht startbar: {e}"))?;
    let _enter = runtime.enter();

    // Mit isoliertem Ordner (Tests, Entwicklung) bleiben Tokens im Speicher und nie im Schlüsselbund.
    let secrets: Arc<dyn SecretStore> = if opts.home.is_some() && !opts.keyring {
        Arc::new(MemorySecretStore::new())
    } else {
        os_secret_store().map_err(|e| format!("Schlüsselbund nicht verfügbar: {e}"))?
    };
    let store =
        Store::open(&dirs.data.join("lenzflix.db")).map_err(|e| format!("Datenbank: {e}"))?;
    let session =
        SessionManager::new(&dirs.config, secrets, store.clone()).map_err(|e| e.to_string())?;

    let shell = Shell::new(ShellOptions {
        maximized: opts.maximized,
        size: opts.size.unwrap_or((1280.0, 800.0)),
        ..ShellOptions::default()
    })
    .map_err(|e| e.to_string())?;
    let ui = MainWindow::new().map_err(|e| e.to_string())?;
    shell.attach(&ui);

    let app = Rc::new(App {
        ui: ui.as_weak(),
        shell: shell.handle(),
        rt: runtime.handle().clone(),
        opts: opts.clone(),
        dirs: dirs.clone(),
        session,
        store,
        st: RefCell::new(State {
            router: Router::default(),
            graph: lfx_nav::FocusGraph::new(),
            ctx: InputContext::Browse,
            design: opts.design.unwrap_or(DesignId::Flix),
            server_id: None,
            users: Vec::new(),
            login_user: None,
            qc_task: None,
            home: home::HomeState::default(),
            play: play::PlayState::default(),
            geo: None,
            images: None,
            toast_gen: 0,
            retro: retro::RetroState::default(),      // W3-retro
            tv: tvui::TvUi::default(),                // W2-tvui
            lib: library::LibState::default(),        // W2-library
            detail: detail::DetailCtl::default(),     // W2-detail
            picker: picker::PickerCtl::default(),     // W3-picker
            couch: picker_couch::CouchCtl::default(), // W3-picker
        }),
        timers: Timers::default(),
        gamepad: RefCell::new(None),
        started: Instant::now(),
    });
    app::install(app.clone());
    wire(&app, &ui, &shell);
    App::tv_wire(&app, &ui); // W2-tvui
    App::picker_wire(&app, &ui); // W3-picker
    app.start();
    app.start_gamepad();
    app.start_couch_watch(); // W3-picker
    #[cfg(feature = "dev-automation")]
    if let Err(e) = automation::start(&dirs.data) {
        tracing::warn!("Automation nicht gestartet: {e}");
    }

    ui.show().map_err(|e| e.to_string())?;
    let result = shell.run().map_err(|e| e.to_string());

    // Aufräumen: Wiedergabe beenden (Stopped melden), Videoausgabe vor dem Player freigeben.
    app.tv_leave(); // W2-tvui
    app.stop_playback();
    {
        let mut st = app.st.borrow_mut();
        st.play.reporter = None;
        st.play.pipe = None;
        st.play.player = None;
    }
    app::uninstall();
    drop(app);
    drop(ui);
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));
    tracing::info!("LENZFLIX beendet");
    result
}

use std::sync::Arc;

/// Verbindet die Rückrufe der Slint-Seiten mit der App.
fn wire(app: &Rc<App>, ui: &MainWindow, shell: &Shell) {
    let h = shell.handle();
    {
        let h = h.clone();
        ui.on_minimize(move || h.minimize());
    }
    {
        let h = h.clone();
        let weak = ui.as_weak();
        ui.on_toggle_maximize(move || {
            h.toggle_maximize();
            if let Some(u) = weak.upgrade() {
                u.set_is_maximized(h.is_maximized());
            }
        });
    }
    {
        let h = h.clone();
        ui.on_close_window(move || h.close());
    }
    {
        let h = h.clone();
        ui.on_drag_started(move || h.drag_window());
    }
    let a = app.clone();
    ui.on_titlebar_hit(move |hit| a.set_hit_regions(&hit));
    let a = app.clone();
    ui.on_key(move |t, c, s, al| a.key_event(&t, c, s, al));

    let a = app.clone();
    ui.on_add_submit(move |u| a.submit_server(u.to_string()));

    let a = app.clone();
    ui.on_profile_clicked(move |i| a.profile_slot_activated(i.max(0) as usize));
    let a = app.clone();
    ui.on_other_user(move || {
        let n = a.st.borrow().users.len();
        a.profile_slot_activated(n);
    });
    let a = app.clone();
    ui.on_change_server(move || {
        let n = a.st.borrow().users.len();
        a.profile_slot_activated(n + 1);
    });

    let a = app.clone();
    ui.on_login_submit(move |u, p| a.submit_login(u.to_string(), p.to_string()));
    let a = app.clone();
    ui.on_login_quick_connect(move || a.start_quick_connect());
    let a = app.clone();
    ui.on_login_back(move || a.apply_action(lfx_nav::NavAction::Back));
    let a = app.clone();
    ui.on_login_field_accepted(move |slot| {
        if slot == 0 {
            a.focus_slot(0, 1);
            a.focus_after_accept();
        } else {
            let ui = a.win();
            a.submit_login(
                ui.get_login_user().to_string(),
                ui.get_login_password().to_string(),
            );
        }
    });

    let a = app.clone();
    ui.on_nav_selected(move |i| a.nav_selected(i));
    let a = app.clone();
    ui.on_hero_play(move || a.hero_clicked(false));
    let a = app.clone();
    ui.on_hero_info(move || a.hero_clicked(true));
    let a = app.clone();
    ui.on_card_clicked(move |r, i| a.card_clicked(r, i));
    let a = app.clone();
    ui.on_home_geometry(move |g| a.on_home_geometry(g));
    {
        // W2-library
        let lib = ui.global::<lfx_ui::LibraryState>();
        let a1 = app.clone();
        lib.on_event(move |k, x, y| a1.lib_event(&k, x, y));
        let a2 = app.clone();
        lib.on_geometry(move |g| a2.lib_on_geometry(g));
        let a3 = app.clone();
        lib.on_chrome_event(move |k, v| a3.on_home_event(&k, v));
        let a4 = app.clone();
        lib.on_chrome_event_f(move |k, v| a4.on_home_event_f(&k, v));
    }

    detail::wire(app, ui); // W2-detail
    retro::wire(app, ui); // W3-retro
    // W2-home
    let a = app.clone();
    ui.on_home_event(move |k, v| a.on_home_event(&k, v));
    let a = app.clone();
    ui.on_home_event_f(move |k, v| a.on_home_event_f(&k, v));
    let a = app.clone();
    ui.on_home_layout(move |l| a.on_home_layout(l));
    let a = app.clone();
    ui.on_home_trailer_rect(move |x, y, w, h| a.on_trailer_rect(x, y, w, h));

    let a = app.clone();
    ui.on_player_cmd(move |n, v| a.player_cmd(&n, v)); // W2-player

    let a = app.clone();
    ui.on_simple_back(move || a.apply_action(lfx_nav::NavAction::Back));
    let a = app.clone();
    ui.on_design_chosen(move |i| a.choose_design(DesignId::from_index(i)));
    let a = app.clone();
    ui.on_about(move || a.go(route::Route::About));
    let a = app.clone();
    ui.on_sign_out(move || a.sign_out());
}
