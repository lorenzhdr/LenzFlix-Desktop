//! Trailer im Billboard (W2-home): der lokale Trailer des Billboard-Titels startet nach einer Verzögerung stumm
//! im Rechteck des Billboards (Video-Ebene der Shell mit `set_rect`, die Seite hält dort ein Loch im
//! Hintergrund), das Bild blendet über. Ende, Fehler, Wegscrollen, Seitenwechsel und Wiedergabe-Start stoppen
//! ihn. Es gibt keine Fortschrittsmeldung (keine Spuren in „Weiterschauen“).

use std::rc::Rc;
use std::time::Duration;

use lfx_core::billboard::{
    TRAILER_MIN_IMAGE_MS, TrailerPlan, TrailerPolicy, choose_trailer, local_trailer_stream,
};
use lfx_player::{PlayRequest, Player, PlayerConfig};
use lfx_shell::Rect;
use slint::Timer;

use crate::app::{App, post};
use crate::route::Route;
use crate::video::VideoPipe;

/// Zustand des Trailers.
pub struct TrailerState {
    /// Der Player spielt (oder lädt) gerade einen Trailer.
    pub active: bool,
    /// Das Video scheint durch (erstes Bild gezeichnet).
    pub visible: bool,
    /// Stumm (Standard).
    pub muted: bool,
    /// Zählt Starts; späte Antworten alter Starts werden verworfen.
    pub epoch: u64,
    /// Rechteck des Billboards in logischen Fensterpixeln (zuletzt gemeldet).
    pub rect: Option<(f32, f32, f32, f32)>,
    /// Billboard ist weggescrollt: kein Trailer, bis es wieder zu sehen ist.
    pub suppressed: bool,
    /// Test-Schalter: ohne die Wartezeit starten.
    pub immediate: bool,
    pub delay: Timer,
    pub tick: Timer,
}

impl Default for TrailerState {
    fn default() -> Self {
        Self {
            active: false,
            visible: false,
            muted: true,
            epoch: 0,
            rect: None,
            suppressed: false,
            immediate: false,
            delay: Timer::default(),
            tick: Timer::default(),
        }
    }
}

/// Höhe der Titelleiste plus Kopfleiste: darunter ist das Billboard verdeckt.
const COVERED_TOP: f32 = 32.0 + 80.0;

impl App {
    /// Plant den Trailer für den aktuellen Billboard-Titel (nach der Wartezeit; Web: Bild mindestens 3 s).
    pub fn schedule_trailer(self: &Rc<Self>) {
        self.stop_trailer();
        let (item, immediate) = {
            let st = self.st.borrow();
            if !matches!(st.router.current(), Route::Home)
                || st.design == crate::cli::DesignId::Retro // W3-retro: das Retro-Hero kennt keinen Trailer
                || st.play.opened
                || st.home.x.trailer.suppressed
                || st.design == crate::cli::DesignId::Plus
            // W3-plus: kein Trailer im Hero
            {
                return;
            }
            (
                st.home.pool.get(st.home.hero_idx).cloned(),
                st.home.x.trailer.immediate,
            )
        };
        let Some(item) = item else { return };
        let policy = TrailerPolicy {
            datasaver_trailer: !self.effects().auto_trailer,
            reduced_motion: false,
            tv_background_playing: false,
        };
        let hero = lfx_core::billboard::HeroItem {
            local_trailer_count: item.local_trailer_count,
            remote_trailer_urls: Vec::new(),
            item: item.clone(),
        };
        if choose_trailer(&hero, false, &policy) != TrailerPlan::Local {
            return;
        }
        let epoch = {
            let mut st = self.st.borrow_mut();
            st.home.x.trailer.epoch += 1;
            st.home.x.trailer.epoch
        };
        let wait = if immediate {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(TRAILER_MIN_IMAGE_MS)
        };
        let weak = Rc::downgrade(self);
        let id = item.id.clone();
        self.st.borrow().home.x.trailer.delay.start(
            slint::TimerMode::SingleShot,
            wait,
            move || {
                if let Some(app) = weak.upgrade() {
                    app.resolve_trailer(epoch, id.clone());
                }
            },
        );
    }

    /// Fragt den lokalen Trailer ab und startet ihn.
    fn resolve_trailer(self: &Rc<Self>, epoch: u64, item_id: String) {
        if self.st.borrow().home.x.trailer.epoch != epoch {
            return;
        }
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let list = client.local_trailers(&item_id).await.unwrap_or_default();
            let Some(trailer) = list.into_iter().find(|t| !t.id.is_empty()) else {
                return;
            };
            let (path, _) = local_trailer_stream(&trailer.id);
            let Ok(url) = client.absolute_url(&format!("{path}?static=true")) else {
                return;
            };
            let mut req = PlayRequest::new(trailer.id.clone(), trailer.id.clone(), url);
            req.http_headers = vec![format!("Authorization: {}", client.auth_header_value())];
            post(move |app| app.begin_trailer(epoch, req));
        });
    }

    fn begin_trailer(self: &Rc<Self>, epoch: u64, req: PlayRequest) {
        let (ok, rect) = {
            let st = self.st.borrow();
            (
                st.home.x.trailer.epoch == epoch
                    && matches!(st.router.current(), Route::Home)
                    && !st.play.opened
                    && !st.home.x.trailer.suppressed,
                st.home.x.trailer.rect,
            )
        };
        let Some((x, y, w, h)) = rect.filter(|_| ok) else {
            return;
        };
        let Some(layer) = self.shell.video_layer() else {
            return;
        };
        if self.st.borrow().play.player.is_none() {
            match Player::new(PlayerConfig::default()) {
                Ok(p) => self.st.borrow_mut().play.player = Some(p),
                Err(e) => {
                    tracing::debug!("Trailer: Player nicht startbar: {}", e.text);
                    return;
                }
            }
        }
        let Some(player) = self.st.borrow().play.player.clone() else {
            return;
        };
        if self.st.borrow().play.pipe.is_none() {
            match VideoPipe::start(&player, layer.clone()) {
                Ok(p) => self.st.borrow_mut().play.pipe = Some(p),
                Err(e) => {
                    tracing::debug!("Trailer: Videoausgabe nicht startbar: {e}");
                    return;
                }
            }
        }
        player.set_muted(self.st.borrow().home.x.trailer.muted);
        layer.set_rect(Some(Rect::new(x, y, w, h)));
        // Unsichtbar, bis das erste Bild kommt (sonst stünde ein schwarzes Loch im Billboard).
        layer.set_visible(false);
        self.st.borrow_mut().home.x.trailer.active = true;
        player.open(req);
        let weak = Rc::downgrade(self);
        self.st.borrow().home.x.trailer.tick.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(200),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.trailer_tick(epoch);
                }
            },
        );
        tracing::info!("Trailer gestartet");
    }

    fn trailer_tick(self: &Rc<Self>, epoch: u64) {
        let (player, frames, visible) = {
            let st = self.st.borrow();
            if st.home.x.trailer.epoch != epoch || !st.home.x.trailer.active {
                return;
            }
            (
                st.play.player.clone(),
                st.play.pipe.as_ref().map_or(0, |p| {
                    p.stats.frames.load(std::sync::atomic::Ordering::Relaxed)
                }),
                st.home.x.trailer.visible,
            )
        };
        let Some(player) = player else {
            self.stop_trailer();
            return;
        };
        let s = player.state();
        if s.error.is_some() || (s.eof && s.loaded) {
            // Ende oder Fehler: zurück zum Bild (die Rotation läuft weiter).
            self.stop_trailer();
            return;
        }
        if !visible && s.loaded && frames > 0 {
            if let Some(layer) = self.shell.video_layer() {
                layer.set_visible(true);
            }
            self.st.borrow_mut().home.x.trailer.visible = true;
            self.push_home_ui();
        }
    }

    /// Stoppt den Trailer (falls einer läuft) und blendet das Bild wieder ein.
    pub fn stop_trailer(self: &Rc<Self>) {
        let (was_active, player) = {
            let mut st = self.st.borrow_mut();
            let t = &mut st.home.x.trailer;
            t.epoch += 1;
            t.delay.stop();
            t.tick.stop();
            let was = t.active;
            t.active = false;
            let was_visible = t.visible;
            t.visible = false;
            (was || was_visible, st.play.player.clone())
        };
        if !was_active {
            return;
        }
        if let Some(p) = player {
            p.stop();
            // Der nächste Film soll nicht stumm starten.
            p.set_muted(false);
        }
        if let Some(layer) = self.shell.video_layer() {
            layer.set_visible(false);
            layer.set_rect(None);
        }
        self.push_home_ui();
    }

    pub fn trailer_toggle_mute(self: &Rc<Self>) {
        let muted = {
            let mut st = self.st.borrow_mut();
            st.home.x.trailer.muted = !st.home.x.trailer.muted;
            st.home.x.trailer.muted
        };
        if let Some(p) = self.st.borrow().play.player.clone() {
            p.set_muted(muted);
        }
        self.push_home_ui();
    }

    /// Das Billboard hat ein neues Rechteck (Fensterkoordinaten): Video-Ebene nachführen, bei Wegscrollen stoppen.
    pub fn on_trailer_rect(self: &Rc<Self>, x: f32, y: f32, w: f32, h: f32) {
        // W3-picker: im Couch-Look zeichnet die Oberfläche mit Zoom, Fensterkoordinaten sind größer
        let z = self.couch_zoom();
        let (x, y, w, h) = (x * z, y * z, w * z, h * z);
        let (active, suppressed) = {
            let mut st = self.st.borrow_mut();
            st.home.x.trailer.rect = Some((x, y, w, h));
            (st.home.x.trailer.active, st.home.x.trailer.suppressed)
        };
        let covered = (y + h - COVERED_TOP) < h * 0.5;
        if active {
            if covered {
                self.st.borrow_mut().home.x.trailer.suppressed = true;
                self.stop_trailer();
            } else if let Some(layer) = self.shell.video_layer() {
                layer.set_rect(Some(Rect::new(x, y, w, h)));
            }
        } else if suppressed && !covered {
            self.st.borrow_mut().home.x.trailer.suppressed = false;
            self.schedule_trailer();
        }
    }
}
