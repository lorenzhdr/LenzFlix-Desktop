//! W5-stats: Statistik-Seite („Meine Statistik“) – Zustand, Laden (Charts-Dienst per Branding-Adresse), Anzeige, Fokus.
//!
//! Die Adresse des Dienstes kommt aus dem Branding-CSS des Servers (`--lfx-charts-url`), das Token geht nur im Header
//! an den Dienst (`lfx_api::charts::ChartsClient`). Rechnen und Formatieren steht in `stats_logic.rs`, die
//! Oberfläche (`pages/stats.slint`) zeigt nur `StatsState`.

use std::collections::HashMap;
use std::rc::Rc;

use lfx_api::ApiError;
use lfx_api::charts::{ChartsClient, MeStats};
use lfx_nav::NavAction;
use lfx_ui::{StBar, StKpi, StShare, StatsState};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::{App, post};
use crate::route::Route;
use crate::stats_logic::{RANGES, ShareRow, StatsView, build_view, norm_id};

/// Anzeigezustand der Seite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Loading,
    Ready,
    /// Der Dienst ist auf diesem Server nicht eingerichtet.
    NoService,
    Error,
}

impl Status {
    fn index(self) -> i32 {
        match self {
            Self::Loading => 0,
            Self::Ready => 1,
            Self::NoService => 2,
            Self::Error => 3,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::NoService => "no-service",
            Self::Error => "error",
        }
    }
}

/// Zustand der Statistik-Seite (nur UI-Thread).
pub struct StatsCtl {
    /// Index in [`RANGES`].
    pub range: usize,
    pub status: Status,
    /// Fokus: 0 Zurück, 1..=4 Zeiträume, 5 Aktion der Meldungsansicht.
    pub focus: usize,
    /// Zählt Ladevorgänge hoch; späte Antworten alter Vorgänge verfallen.
    pub epoch: u64,
    pub view: Option<StatsView>,
    pub message: String,
    pub detail: String,
}

impl Default for StatsCtl {
    fn default() -> Self {
        Self {
            range: 1,
            status: Status::Loading,
            focus: 2,
            epoch: 0,
            view: None,
            message: String::new(),
            detail: String::new(),
        }
    }
}

/// Ergebnis eines Ladevorgangs.
enum Loaded {
    Ok(Box<MeStats>, HashMap<String, String>),
    NoService,
    Failed(String, String),
}

fn failure(e: &ApiError) -> (String, String) {
    match e {
        ApiError::Unauthorized | ApiError::Forbidden => (
            "Anmeldung abgelaufen".into(),
            "Bitte melde dich erneut an, um deine Statistik zu sehen.".into(),
        ),
        ApiError::Network(_) | ApiError::Timeout => (
            "Keine Verbindung".into(),
            "Der Statistik-Dienst ist gerade nicht erreichbar. Prüfe deine Verbindung und versuche es erneut.".into(),
        ),
        _ => (
            "Statistik nicht verfügbar".into(),
            "Der Statistik-Dienst hat nicht wie erwartet geantwortet. Versuche es später erneut.".into(),
        ),
    }
}

async fn load(client: lfx_api::Client, range: lfx_api::charts::StatsRange) -> Loaded {
    let branding = match client.branding().await {
        Ok(b) => b,
        Err(e) => {
            let (m, d) = failure(&e);
            return Loaded::Failed(m, d);
        }
    };
    let charts = match ChartsClient::from_branding(&client, &branding) {
        None => return Loaded::NoService,
        Some(Err(_)) => return Loaded::NoService,
        Some(Ok(c)) => c,
    };
    let stats = match charts.my_stats(range).await {
        Ok(s) => s,
        Err(ApiError::NotFound) => return Loaded::NoService,
        Err(e) => {
            let (m, d) = failure(&e);
            return Loaded::Failed(m, d);
        }
    };
    // Namen der Titel nachladen; Fehler lassen die Seite trotzdem erscheinen (Name „Unbekannter Titel“).
    let ids = crate::stats_logic::title_ids(&stats);
    let mut titles = HashMap::new();
    if !ids.is_empty() {
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        if let Ok(res) = client
            .items(&lfx_core::rows::logic::base_query().ids(&refs))
            .await
        {
            for it in res.items {
                titles.insert(norm_id(&it.id), it.name);
            }
        }
    }
    Loaded::Ok(Box::new(stats), titles)
}

fn share_model(rows: &[ShareRow]) -> ModelRc<StShare> {
    ModelRc::new(VecModel::from(
        rows.iter()
            .map(|r| StShare {
                name: r.name.as_str().into(),
                meta: r.meta.as_str().into(),
                frac: r.frac,
            })
            .collect::<Vec<_>>(),
    ))
}

impl App {
    /// Aufbau beim Betreten der Seite.
    pub fn render_stats(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.stats.focus = st.stats.range + 1;
        }
        self.win().global::<StatsState>().set_scroll_y(0.0);
        self.stats_load();
    }

    /// Beim Verlassen: laufende Antworten verwerfen.
    pub fn stats_leave(self: &Rc<Self>) {
        self.st.borrow_mut().stats.epoch += 1;
    }

    pub fn on_stats_page(&self) -> bool {
        matches!(self.st.borrow().router.current(), Route::Stats)
    }

    /// Lädt die Statistik des gewählten Zeitraums neu.
    pub fn stats_load(self: &Rc<Self>) {
        let (epoch, range) = {
            let mut st = self.st.borrow_mut();
            st.stats.epoch += 1;
            st.stats.status = Status::Loading;
            (st.stats.epoch, RANGES[st.stats.range.min(3)])
        };
        self.stats_push();
        let Some(active) = self.session.active() else {
            self.stats_finish(
                epoch,
                Loaded::Failed(
                    "Nicht angemeldet".into(),
                    "Melde dich an, um deine Statistik zu sehen.".into(),
                ),
            );
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let r = load(client, range).await;
            post(move |app| app.stats_finish(epoch, r));
        });
    }

    fn stats_finish(self: &Rc<Self>, epoch: u64, r: Loaded) {
        if self.st.borrow().stats.epoch != epoch || !self.on_stats_page() {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            let s = &mut st.stats;
            match r {
                Loaded::Ok(stats, titles) => {
                    s.view = Some(build_view(&stats, &titles));
                    s.status = Status::Ready;
                }
                Loaded::NoService => {
                    s.status = Status::NoService;
                    s.message = "Statistik nicht eingerichtet".into();
                    s.detail = "Auf diesem Server ist der Statistik-Dienst nicht eingerichtet. Frage deinen Administrator, ob er ihn aktivieren kann.".into();
                    s.view = None;
                }
                Loaded::Failed(m, d) => {
                    s.status = Status::Error;
                    s.message = m;
                    s.detail = d;
                    s.view = None;
                    s.focus = 5;
                }
            }
            if s.status != Status::Error && s.focus == 5 {
                s.focus = s.range + 1;
            }
        }
        self.stats_push();
    }

    /// Schreibt den Zustand in `StatsState`.
    pub fn stats_push(self: &Rc<Self>) {
        let ui = self.win();
        let g = ui.global::<StatsState>();
        let st = self.st.borrow();
        let s = &st.stats;
        g.set_status(s.status.index());
        g.set_range(s.range as i32);
        g.set_focus(s.focus as i32);
        g.set_message(SharedString::from(s.message.as_str()));
        g.set_detail(SharedString::from(s.detail.as_str()));
        let Some(v) = (s.status == Status::Ready)
            .then_some(s.view.as_ref())
            .flatten()
        else {
            return;
        };
        g.set_hero_value(v.hero_value.as_str().into());
        g.set_hero_sub(v.hero_sub.as_str().into());
        g.set_kpis(ModelRc::new(VecModel::from(
            v.kpis
                .iter()
                .map(|(l, x)| StKpi {
                    label: l.as_str().into(),
                    value: x.as_str().into(),
                })
                .collect::<Vec<_>>(),
        )));
        g.set_notice(v.notice.as_str().into());
        g.set_top_series(share_model(&v.top_series));
        g.set_top_movies(share_model(&v.top_movies));
        g.set_bars(ModelRc::new(VecModel::from(
            v.bars
                .iter()
                .map(|b| StBar {
                    frac: b.frac,
                    label: b.label.as_str().into(),
                    tip: b.tip.as_str().into(),
                    hot: b.hot,
                })
                .collect::<Vec<_>>(),
        )));
        g.set_line(v.line.as_str().into());
        g.set_range_title(v.range_title.as_str().into());
        g.set_heat(ModelRc::new(VecModel::from(v.heat.clone())));
        g.set_heat_best(v.heat_best.as_str().into());
        match &v.binge {
            Some((big, sub, line)) => {
                g.set_has_binge(true);
                g.set_binge_big(big.as_str().into());
                g.set_binge_sub(sub.as_str().into());
                g.set_binge_line(line.as_str().into());
            }
            None => g.set_has_binge(false),
        }
        g.set_devices(share_model(&v.devices));
        g.set_methods(share_model(&v.methods));
    }

    /// Wählt einen Zeitraum und lädt neu.
    pub fn stats_set_range(self: &Rc<Self>, index: usize) {
        {
            let mut st = self.st.borrow_mut();
            if index > 3 {
                return;
            }
            st.stats.range = index;
            st.stats.focus = index + 1;
        }
        self.win().global::<StatsState>().set_scroll_y(0.0);
        self.stats_load();
    }

    /// Ereignis der Oberfläche (Maus).
    pub fn stats_event(self: &Rc<Self>, kind: &str, v: i32) {
        match kind {
            "back" => self.apply_action(NavAction::Back),
            "range" => self.stats_set_range(v.max(0) as usize),
            "retry" => self.stats_load(),
            _ => {}
        }
    }

    /// Tastatur/Gamepad auf der Seite; `true` = verbraucht.
    pub fn stats_handle_action(self: &Rc<Self>, action: NavAction) -> bool {
        let (status, focus) = {
            let st = self.st.borrow();
            (st.stats.status, st.stats.focus)
        };
        let set_focus = |app: &Rc<Self>, f: usize| {
            app.st.borrow_mut().stats.focus = f;
            app.stats_push();
        };
        match action {
            NavAction::Left | NavAction::Right => {
                let right = action == NavAction::Right;
                if status == Status::Error {
                    set_focus(self, if focus == 0 { 5 } else { 0 });
                } else if status == Status::NoService {
                    set_focus(self, 0);
                } else {
                    let next = if right {
                        (focus + 1).min(4)
                    } else {
                        focus.saturating_sub(1)
                    };
                    set_focus(self, next);
                }
                true
            }
            NavAction::Up | NavAction::Down | NavAction::PageUp | NavAction::PageDown => {
                if status != Status::Ready {
                    return true;
                }
                let step = match action {
                    NavAction::Up | NavAction::Down => 120.0,
                    _ => 520.0,
                };
                let dir = if matches!(action, NavAction::Up | NavAction::PageUp) {
                    1.0
                } else {
                    -1.0
                };
                self.stats_scroll(dir * step);
                true
            }
            NavAction::Activate => {
                match focus {
                    0 => self.apply_action(NavAction::Back),
                    1..=4 => {
                        if self.st.borrow().stats.range != focus - 1 || status != Status::Ready {
                            self.stats_set_range(focus - 1);
                        }
                    }
                    _ => self.stats_load(),
                }
                true
            }
            _ => false,
        }
    }

    /// Verschiebt den Inhalt um `delta` Pixel (positiv = nach oben scrollen), begrenzt auf Anfang und Ende.
    fn stats_scroll(self: &Rc<Self>, delta: f32) {
        let ui = self.win();
        let g = ui.global::<StatsState>();
        let cur = g.get_scroll_y();
        let max = g.get_max_scroll().max(0.0);
        let next = (cur + delta).clamp(-max, 0.0);
        g.set_scroll_y(next);
    }

    /// Zustand für Tests und Automation.
    pub fn stats_state_json(&self) -> serde_json::Value {
        let st = self.st.borrow();
        let s = &st.stats;
        let v = s.view.as_ref();
        serde_json::json!({
            "status": s.status.name(),
            "range": s.range,
            "focus": s.focus,
            "message": s.message,
            "kpis": v.map(|v| v.kpis.clone()),
            "hero": v.map(|v| v.hero_value.clone()),
            "bars": v.map_or(0, |v| v.bars.len()),
            "heat_cells": v.map_or(0, |v| v.heat.iter().filter(|c| **c > 0.0).count()),
            "top_series": v.map(|v| v.top_series.iter().map(|r| r.name.clone()).collect::<Vec<_>>()),
            "top_movies": v.map(|v| v.top_movies.iter().map(|r| r.name.clone()).collect::<Vec<_>>()),
            "devices": v.map_or(0, |v| v.devices.len()),
            "methods": v.map_or(0, |v| v.methods.len()),
            "range_title": v.map(|v| v.range_title.clone()),
        })
    }

    /// Befehl `stats_*` der Entwickler-Steuerung.
    #[cfg(feature = "dev-automation")]
    pub fn stats_automation(
        self: &Rc<Self>,
        cmd: &str,
        msg: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match cmd {
            "stats_state" => {}
            "stats_range" => {
                let i = msg["index"].as_u64().ok_or("index fehlt")?;
                self.stats_set_range(i as usize);
            }
            "stats_retry" => self.stats_load(),
            "stats_scroll" => {
                let d = msg["delta"].as_f64().ok_or("delta fehlt")?;
                self.stats_scroll(d as f32);
            }
            _ => return Err("unbekannter Befehl".into()),
        }
        Ok(serde_json::json!({ "stats": self.stats_state_json() }))
    }
}
