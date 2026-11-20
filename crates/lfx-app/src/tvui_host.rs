//! W2-tvui: Abläufe der TV-Modus-Seite (Betreten, Katalog, Controller-Ausgaben, Player, Eingabe, Dev-Befehle).
//! Zustand und Hilfen stehen in `tvui.rs`, Umwandlung in Slint-Strukturen in `tvui_view.rs`.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use lfx_api::BaseItem;
use lfx_nav::NavAction;
use lfx_player::{Player, PlayerConfig, PrepareOptions, prepare};
use lfx_shell::Rect;
use lfx_tv::api_source::ApiCatalogSource;
use lfx_tv::catalog::{CatalogCache, RawCatalog, cache_key, fetch_catalog};
use lfx_tv::channel_list::ChannelList;
use lfx_tv::clock::Clock;
use lfx_tv::config::to_planner_config;
use lfx_tv::controller::{Card, Reason, Snapshot, TvController, TvOptions, TvOutput};
use lfx_tv::epg::{self, EpgGrid};
use lfx_tv::info_card::{CardAction, Details, InfoCard, Overview, build_info_card};
use lfx_tv::model::{describe, next_program, title_of};
use lfx_tv::request::{PlayResult, PlayerPhase, Token};
use lfx_tv::stream::FailReason;
use lfx_tv_schedule::time::format_clock;
use lfx_tv_schedule::{PlanEntry, Scheduler, normalize_catalog};
use lfx_ui::{MainWindow, TvState};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, TimerMode, VecModel};

use crate::app::{App, post};
use crate::route::Route;
use crate::tvui::{InfoView, TvClock, TvUi, card_texts, catalog_cache, fail_result, phase_index};
use crate::tvui_view as view;
use crate::video::VideoPipe;

const ZAP_SHOW: Duration = Duration::from_millis(2500);
const CONTROLS_SHOW: Duration = Duration::from_millis(3000);
const ROW_PITCH: f32 = 85.0;

fn empty_view(no: i64) -> InfoView {
    InfoView {
        has: false,
        no: no as i32,
        color: slint::Color::default(),
        ink: slint::Color::default(),
        short: String::new(),
        title: String::new(),
        sub: String::new(),
        time: String::new(),
        badges: Vec::new(),
        next: String::new(),
        progress: 0.0,
    }
}

impl App {
    fn tv_global<R>(&self, f: impl FnOnce(&TvState<'_>) -> R) -> R {
        let ui = self.win();
        let g = ui.global::<TvState>();
        f(&g)
    }

    fn tv_now(&self) -> i64 {
        self.st
            .borrow()
            .tv
            .clock
            .map_or_else(|| TvClock::new().now_ms(), |c| c.now_ms())
    }

    fn tv_ctl<R>(&self, f: impl FnOnce(&mut TvController<Scheduler>) -> R) -> Option<R> {
        let mut st = self.st.borrow_mut();
        st.tv.ctl.as_mut().map(f)
    }

    // ---------------------------------------------------------------- Betreten / Verlassen

    /// Route `TvMode` betreten: Katalog laden, dann Controller starten.
    pub fn tv_enter(self: &Rc<Self>) {
        let Some(active) = self.session.active() else {
            return;
        };
        // Die Video-Ebene gehört jetzt dem TV-Modus: eine laufende Wiedergabe der Titelseiten beenden.
        self.stop_playback();
        {
            let mut st = self.st.borrow_mut();
            st.play.pipe = None;
            st.play.player = None;
        }
        let visit = {
            let mut st = self.st.borrow_mut();
            let visit = st.tv.visit + 1;
            st.tv = TvUi {
                active: true,
                visit,
                clock: Some(TvClock::new()),
                ..TvUi::default()
            };
            visit
        };
        self.tv_global(|g| {
            g.set_loading(true);
            g.set_load_error("".into());
            g.set_load_text("Sendersuchlauf …".into());
            g.set_load_progress(0.0);
            g.set_tab(0);
            g.set_fullscreen(false);
            g.set_phase(0);
            g.set_card_kind(0);
            g.set_banner("".into());
            g.set_digits("".into());
            g.set_has_entry(false);
            g.set_zap_on(false);
            g.set_ic_open(false);
            g.set_toast("".into());
            g.set_controls_on(true);
            g.set_channels(ModelRc::new(VecModel::default()));
            g.set_epg_rows(ModelRc::new(VecModel::default()));
        });
        let client = active.client.clone();
        let user_id = active.user_id.clone();
        let handle = self.rt.clone();
        let now = self.tv_now();
        // `fetch_catalog` ist nicht `Send` (Fortschrittsrückruf): in einem eigenen Thread ausführen.
        self.rt.spawn_blocking(move || {
            handle.block_on(async move {
                let channels = client.branding().await.ok().map(|b| b.tv_channels);
                let key = cache_key(&user_id, now);
                let cached = catalog_cache()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&key);
                let raw: Result<RawCatalog, String> = match cached {
                    Some(r) => Ok(r),
                    None => {
                        let src = ApiCatalogSource::new(&client);
                        let mut on_progress = |f: f64, label: &str| {
                            let label = label.to_owned();
                            post(move |app| {
                                if app.st.borrow().tv.visit == visit {
                                    app.tv_global(|g| {
                                        g.set_load_progress(f as f32);
                                        g.set_load_text(label.as_str().into());
                                    });
                                }
                            });
                        };
                        match fetch_catalog(&src, now, Some(&mut on_progress)).await {
                            Ok(r) => {
                                catalog_cache()
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .put(&user_id, &key, &r);
                                Ok(r)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                };
                post(move |app| app.tv_catalog_ready(visit, raw, channels));
            });
        });
    }

    fn tv_catalog_ready(
        self: &Rc<Self>,
        visit: u64,
        raw: Result<RawCatalog, String>,
        channels: Option<BTreeMap<u8, String>>,
    ) {
        {
            let st = self.st.borrow();
            if st.tv.visit != visit || !st.tv.active {
                return;
            }
        }
        let raw = match raw {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("TV-Katalog nicht ladbar: {e}");
                self.tv_global(|g| {
                    g.set_load_error("Der Sender-Katalog konnte nicht geladen werden.".into());
                });
                return;
            }
        };
        let catalog = Arc::new(normalize_catalog(&raw.to_jsval()));
        let map: BTreeMap<i64, String> = channels
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (i64::from(k), v))
            .collect();
        let sched = Scheduler::new(catalog, &to_planner_config(&map));
        let clock = self.st.borrow().tv.clock.unwrap_or_else(TvClock::new);
        let (lc, vol, muted) = self.session.active().map_or((None, None, None), |a| {
            let get = |k: &str| {
                self.store
                    .get_setting(&a.server_id, &a.user_id, k)
                    .ok()
                    .flatten()
            };
            (
                get("tv.last_channel").and_then(|v| v.as_i64()),
                get("tv.volume").and_then(|v| v.as_f64()),
                get("tv.muted").and_then(|v| v.as_bool()),
            )
        });
        let opts = TvOptions {
            last_channel: lc.unwrap_or(1),
            volume: vol.unwrap_or(1.0),
            muted: muted.unwrap_or(false),
            watch: true,
        };
        let mut ctl = TvController::new(sched, Box::new(clock), opts);
        let now = clock.now_ms();
        let list = ChannelList::build(ctl.plan_mut(), now);
        let grid = EpgGrid::build(ctl.plan_mut(), now, None);
        let list_model = Rc::new(VecModel::from(
            list.rows().iter().map(view::chan_row).collect::<Vec<_>>(),
        ));
        {
            let mut st = self.st.borrow_mut();
            st.tv.ctl = Some(ctl);
            st.tv.list = Some(list);
            st.tv.grid = Some(grid);
            st.tv.list_model = Some(list_model.clone());
            st.tv.details = Some(Details::new(true));
            st.tv.last_epg_ms = now;
        }
        self.tv_global(|g| {
            g.set_channels(ModelRc::from(list_model.clone()));
            g.set_loading(false);
        });
        self.tv_refresh_epg(true);
        self.tv_ctl(|c| c.start());
        self.tv_start_host_timer();
        self.tv_pump();
        self.tv_activity();
    }

    /// Verlässt die Seite: alles stoppen, Video-Ebene freigeben.
    pub fn tv_leave(self: &Rc<Self>) {
        if !self.st.borrow().tv.active {
            return;
        }
        let (player, session) = {
            let mut st = self.st.borrow_mut();
            st.tv.active = false;
            st.tv.visit += 1;
            if let Some(c) = st.tv.ctl.as_mut() {
                c.dispose();
            }
            st.tv.ctl = None;
            st.tv.poll.stop();
            st.tv.host.stop();
            st.tv.zap_hide.stop();
            st.tv.controls_hide.stop();
            st.tv.toast_hide.stop();
            st.tv.pending = None;
            st.tv.want = None;
            st.tv.card = None;
            (st.tv.player.clone(), st.tv.play_session.take())
        };
        if let Some(p) = player {
            p.stop();
        }
        self.tv_delete_encodings(session);
        {
            // Reihenfolge: erst die Ausgabe, dann der Player.
            let mut st = self.st.borrow_mut();
            st.tv.pipe = None;
            st.tv.player = None;
        }
        if let Some(layer) = self.shell.video_layer() {
            layer.set_visible(false);
            layer.set_rect(None);
        }
        self.tv_global(|g| {
            g.set_fullscreen(false);
            g.set_ic_open(false);
        });
    }

    fn tv_delete_encodings(&self, session: Option<String>) {
        let (Some(id), Some(active)) = (session.filter(|s| !s.is_empty()), self.session.active())
        else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let _ = client.delete_active_encodings(&id).await;
        });
    }

    fn tv_start_host_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.st.borrow().tv.host.start(
            TimerMode::Repeated,
            Duration::from_millis(200),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.tv_host_tick();
                }
            },
        );
    }

    // ---------------------------------------------------------------- Ausgaben des Controllers

    /// Arbeitet die Ausgaben des Controllers ab und stellt den nächsten Zeitgeber ein.
    pub fn tv_pump(self: &Rc<Self>) {
        for _ in 0..16 {
            let Some(outs) = self.tv_ctl(|c| c.take_output()) else {
                return;
            };
            if outs.is_empty() {
                break;
            }
            for o in outs {
                self.tv_handle(o);
            }
        }
        let deadline = self.tv_ctl(|c| c.next_deadline()).flatten();
        let now = self.tv_now();
        let weak = Rc::downgrade(self);
        let st = self.st.borrow();
        match deadline {
            Some(d) => st.tv.poll.start(
                TimerMode::SingleShot,
                Duration::from_millis((d - now).clamp(1, 60_000) as u64),
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.tv_ctl(|c| c.poll());
                        app.tv_pump();
                    }
                },
            ),
            None => st.tv.poll.stop(),
        }
    }

    fn tv_handle(self: &Rc<Self>, o: TvOutput) {
        match o {
            TvOutput::Change(snap) => self.tv_apply_snapshot(*snap),
            TvOutput::Message(text) => self.tv_toast(&text),
            TvOutput::Tick(t) => self.tv_tick(t),
            TvOutput::Day(_) => self.tv_refresh_epg(false),
            TvOutput::Zap(_) | TvOutput::WarmNote(_) | TvOutput::Unlock => {}
            TvOutput::Digits(d) => {
                self.tv_global(|g| g.set_digits(d.unwrap_or_default().as_str().into()));
            }
            TvOutput::Play(req) => self.tv_play(req),
            TvOutput::PlayUrl { token, .. } => {
                // Trailer gibt es am Desktop nicht (LoadTrailer wird mit „keiner“ beantwortet).
                self.tv_ctl(|c| {
                    c.on_play_url_result(token, &PlayResult::fail(FailReason::Aborted));
                });
            }
            TvOutput::LoadTrailer { token, .. } => {
                self.tv_ctl(|c| c.on_trailer(token, None));
            }
            TvOutput::StopPlayer => self.tv_stop_player(),
            TvOutput::SyncTo(off_ms) => {
                let player = self.st.borrow().tv.player.clone();
                if let Some(p) = player {
                    let s = p.state();
                    if s.loaded && ((s.position * 1000.0) as i64 - off_ms).abs() > 10_000 {
                        p.seek_to(off_ms as f64 / 1000.0);
                    }
                }
            }
            TvOutput::SetVolume(v) => {
                let player = self.st.borrow().tv.player.clone();
                if let Some(p) = player {
                    p.set_volume((v * 100.0).clamp(0.0, 100.0));
                }
            }
            TvOutput::SetMuted(m) => {
                let player = self.st.borrow().tv.player.clone();
                if let Some(p) = player {
                    p.set_muted(m);
                }
            }
            TvOutput::SetPlayed { item_id } => {
                if let Some(active) = self.session.active() {
                    let client = active.client.clone();
                    self.rt.spawn(async move {
                        let _ = client.set_played(&item_id, true).await;
                    });
                }
            }
            TvOutput::SavePrefs(p) => {
                if let Some(c) = p.last_channel {
                    self.save_setting("tv.last_channel", json!(c));
                }
                if let Some(v) = p.volume {
                    self.save_setting("tv.volume", json!(v));
                }
                if let Some(m) = p.muted {
                    self.save_setting("tv.muted", json!(m));
                }
            }
            // Vorwärmen, Vorauflösen und Tonwechsel gibt es am Desktop (noch) nicht.
            TvOutput::PrefetchTarget { .. }
            | TvOutput::CancelPrefetch
            | TvOutput::Prefetch { .. }
            | TvOutput::Prewarm { .. }
            | TvOutput::ReleaseWarm
            | TvOutput::PrepareDays(_)
            | TvOutput::SetAudio { .. }
            | TvOutput::SetSubtitle(_) => {}
        }
    }

    fn tv_toast(self: &Rc<Self>, text: &str) {
        self.tv_global(|g| g.set_toast(text.into()));
        let weak = self.ui.clone();
        self.st.borrow().tv.toast_hide.start(
            TimerMode::SingleShot,
            Duration::from_secs(3),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.global::<TvState>().set_toast("".into());
                }
            },
        );
    }

    // ---------------------------------------------------------------- Anzeige

    fn tv_describe(&self, snap: &Snapshot) -> InfoView {
        let now = self.tv_now();
        let mut st = self.st.borrow_mut();
        let tv = &mut st.tv;
        let (Some(ctl), Some(list)) = (tv.ctl.as_mut(), tv.list.as_ref()) else {
            return empty_view(snap.channel);
        };
        let (color, ink, short) = list
            .rows()
            .iter()
            .find(|r| r.no == snap.channel)
            .map_or_else(
                || {
                    (
                        slint::Color::default(),
                        slint::Color::default(),
                        String::new(),
                    )
                },
                |r| {
                    (
                        view::hex_color(r.badge.color),
                        view::hex_color(r.badge.ink),
                        r.badge.short.clone(),
                    )
                },
            );
        let details = tv.details.as_ref().map(|d| d.map().clone());
        match &snap.entry {
            Some(e) => {
                let d = describe(ctl.plan_mut(), snap.channel, e, now, details.as_ref());
                let next = next_program(ctl.plan_mut(), snap.channel, e)
                    .map(|n| {
                        view::next_line(&title_of(ctl.plan_mut(), &n).title, &format_clock(n.start))
                    })
                    .unwrap_or_default();
                InfoView {
                    has: true,
                    color,
                    ink,
                    short,
                    title: d.title,
                    sub: d.sub,
                    time: d.time,
                    badges: d.badges,
                    next,
                    progress: d.progress as f32,
                    ..empty_view(snap.channel)
                }
            }
            None => InfoView {
                has: true,
                color,
                ink,
                short,
                title: "Kein Programm".into(),
                ..empty_view(snap.channel)
            },
        }
    }

    fn tv_apply_snapshot(self: &Rc<Self>, snap: Snapshot) {
        let iv = self.tv_describe(&snap);
        let ct = card_texts(&snap);
        let channel_changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.tv.last_channel != snap.channel;
            st.tv.last_channel = snap.channel;
            if let Some(l) = st.tv.list.as_mut() {
                l.set_current(Some(snap.channel));
            }
            st.tv.snap = Some(snap.clone());
            changed
        };
        self.tv_global(|g| {
            g.set_phase(phase_index(snap.phase));
            g.set_card_kind(ct.kind);
            g.set_card_title(ct.title.as_str().into());
            g.set_card_text(ct.text.as_str().into());
            g.set_card_hint(ct.hint.as_str().into());
            g.set_card_code(ct.code.as_str().into());
            g.set_card_button(ct.button.as_str().into());
            g.set_banner(snap.banner.clone().unwrap_or_default().as_str().into());
            g.set_needs_tap(snap.needs_tap);
            g.set_muted_by_policy(snap.muted_by_policy);
            g.set_muted(snap.muted);
            g.set_volume(f32::from(snap.volume) / 100.0);
            g.set_current_channel(snap.channel as i32);
            g.set_has_entry(iv.has);
            g.set_info_no(iv.no);
            g.set_info_color(iv.color);
            g.set_info_ink(iv.ink);
            g.set_info_short(iv.short.as_str().into());
            g.set_info_title(iv.title.as_str().into());
            g.set_info_sub(iv.sub.as_str().into());
            g.set_info_time(iv.time.as_str().into());
            g.set_info_badges(view::strings(&iv.badges));
            g.set_info_next(iv.next.as_str().into());
            g.set_info_progress(iv.progress);
            if channel_changed {
                g.set_zap_on(true);
                g.set_zap_no(iv.no);
                g.set_zap_color(iv.color);
                g.set_zap_ink(iv.ink);
                g.set_zap_short(iv.short.as_str().into());
                g.set_zap_title(iv.title.as_str().into());
                g.set_zap_time(iv.time.as_str().into());
                g.set_zap_progress(iv.progress);
            }
        });
        if channel_changed {
            let weak = self.ui.clone();
            self.st
                .borrow()
                .tv
                .zap_hide
                .start(TimerMode::SingleShot, ZAP_SHOW, move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.global::<TvState>().set_zap_on(false);
                    }
                });
            self.tv_scroll_list(snap.channel);
            self.tv_refresh_epg_current();
        }
    }

    /// Höhe des sichtbaren Listenbereichs in logischen Pixeln (grob, für das Zentrieren).
    fn tv_list_height(&self) -> f32 {
        let ((_, ph), dpr) = self.shell.window_info();
        (ph as f32 / dpr as f32 - 32.0 - 64.0 - 16.0).max(200.0)
    }

    fn tv_scroll_list(&self, no: i64) {
        let max = (18.0 * ROW_PITCH - self.tv_list_height()).max(0.0);
        let want = ((no - 1) as f32 * ROW_PITCH - 140.0).clamp(0.0, max);
        self.tv_global(|g| g.set_list_y(-want));
    }

    fn tv_tick(self: &Rc<Self>, t: i64) {
        let (rebuilt_grid, last_epg) = {
            let mut st = self.st.borrow_mut();
            let tv = &mut st.tv;
            let (Some(ctl), Some(list), Some(grid)) =
                (tv.ctl.as_mut(), tv.list.as_mut(), tv.grid.as_mut())
            else {
                return;
            };
            list.tick(ctl.plan_mut(), t);
            let rebuilt = grid.tick(ctl.plan_mut(), t);
            (rebuilt, tv.last_epg_ms)
        };
        // Senderliste: nur geänderte Zeilen schreiben.
        let (model, rows) = {
            let st = self.st.borrow();
            (
                st.tv.list_model.clone(),
                st.tv
                    .list
                    .as_ref()
                    .map(|l| l.rows().iter().map(view::chan_row).collect::<Vec<_>>()),
            )
        };
        if let (Some(m), Some(rows)) = (model, rows) {
            for (i, r) in rows.into_iter().enumerate() {
                if m.row_data(i).as_ref() != Some(&r) {
                    m.set_row_data(i, r);
                }
            }
        }
        if rebuilt_grid || t - last_epg >= 15_000 {
            self.st.borrow_mut().tv.last_epg_ms = t;
            self.tv_refresh_epg(rebuilt_grid);
        } else {
            self.tv_update_now_line(t);
        }
        // Info und Zapp-Einblendung: Fortschritt.
        let snap = self.st.borrow().tv.snap.clone();
        if let Some(s) = snap {
            let iv = self.tv_describe(&s);
            self.tv_global(|g| {
                g.set_info_progress(iv.progress);
                g.set_zap_progress(iv.progress);
                g.set_info_next(iv.next.as_str().into());
            });
        }
    }

    fn tv_update_now_line(&self, t: i64) {
        let line = {
            let st = self.st.borrow();
            st.tv.grid.as_ref().and_then(|g| g.now_line(t))
        };
        self.tv_global(|g| match line {
            Some(min) => {
                g.set_now_x((min * view::PPM) as f32);
                g.set_now_visible(true);
            }
            None => g.set_now_visible(false),
        });
    }

    /// Programm-Raster neu in die Oberfläche schreiben; `reveal`: zur Uhrzeit scrollen.
    fn tv_refresh_epg(self: &Rc<Self>, reveal: bool) {
        let now = self.tv_now();
        let cur = self.st.borrow().tv.last_channel;
        let data = {
            let mut st = self.st.borrow_mut();
            let Some(grid) = st.tv.grid.as_mut() else {
                return;
            };
            grid.apply_time(now);
            let days: Vec<String> = grid.days().iter().map(|d| d.label.clone()).collect();
            let sel = grid
                .days()
                .iter()
                .position(|d| d.d == grid.selected())
                .unwrap_or(0);
            (
                view::epg_rows(grid, cur),
                view::scale(grid),
                days,
                sel,
                (grid.total_min() * view::PPM) as f32,
                epg::reveal_left(grid, now, view::PPM),
            )
        };
        let (rows, scale, days, sel, total, left) = data;
        self.tv_global(|g| {
            g.set_epg_rows(rows);
            g.set_scale(scale);
            g.set_days(view::strings(&days));
            g.set_day_index(sel as i32);
            g.set_epg_total_w(total);
            if reveal && let Some(l) = left {
                g.set_epg_x(-(l as f32));
            }
        });
        self.tv_update_now_line(now);
    }

    /// Nur die Markierung des laufenden Senders in den Rasterzeilen aktualisieren.
    fn tv_refresh_epg_current(&self) {
        let cur = self.st.borrow().tv.last_channel;
        self.tv_global(|g| {
            let rows = g.get_epg_rows();
            for i in 0..rows.row_count() {
                if let Some(mut r) = rows.row_data(i) {
                    let is = i64::from(r.no) == cur;
                    if r.current != is {
                        r.current = is;
                        rows.set_row_data(i, r);
                    }
                }
            }
        });
    }

    // ---------------------------------------------------------------- Wiedergabe

    fn tv_play(self: &Rc<Self>, req: lfx_tv::TvPlayRequest) {
        let Some(active) = self.session.active() else {
            return;
        };
        let (visit, now) = {
            let mut st = self.st.borrow_mut();
            st.tv.want = Some(req.token);
            st.tv.plays += 1;
            (st.tv.visit, st.tv.clock.map_or(0, |c| c.now_ms()))
        };
        let start = req.live_seconds_at(now).max(0.0);
        let client = active.client.clone();
        let token = req.token;
        self.rt.spawn(async move {
            let opts = PrepareOptions {
                start_seconds: start,
                audio_stream_index: req.audio_index,
                // Nie Untertitel einbrennen lassen: ohne Wahl „aus“.
                subtitle_stream_index: req.subtitle_index.or(Some(-1)),
                allow_transcode: true,
                ..PrepareOptions::default()
            };
            match prepare(&client, &req.item_id, &opts).await {
                Ok(play) => post(move |app| app.tv_open(visit, token, play)),
                Err(e) => {
                    let res = fail_result(&e);
                    tracing::warn!("TV-Start nicht möglich");
                    post(move |app| {
                        let ok = {
                            let st = app.st.borrow();
                            st.tv.visit == visit && st.tv.want == Some(token)
                        };
                        if ok {
                            app.tv_ctl(|c| c.on_play_result(token, &res));
                            app.tv_pump();
                        }
                    });
                }
            }
        });
    }

    fn tv_ensure_player(self: &Rc<Self>) -> Result<Player, String> {
        if let Some(p) = self.st.borrow().tv.player.clone() {
            return Ok(p);
        }
        let layer = self
            .shell
            .video_layer()
            .ok_or_else(|| "Wiedergabe auf macOS folgt".to_owned())?;
        let player = Player::new(PlayerConfig::default())
            .map_err(|e| format!("Der Player konnte nicht gestartet werden: {}", e.text))?;
        let pipe = VideoPipe::start(&player, layer.clone())?;
        let rect = self.st.borrow().tv.rect;
        layer.set_rect(rect);
        layer.set_visible(true);
        let vol = self
            .st
            .borrow()
            .tv
            .snap
            .as_ref()
            .map(|s| (s.volume, s.muted));
        if let Some((v, m)) = vol {
            player.set_volume(f64::from(v));
            player.set_muted(m);
        }
        let mut st = self.st.borrow_mut();
        st.tv.pipe = Some(pipe);
        st.tv.player = Some(player.clone());
        Ok(player)
    }

    fn tv_open(self: &Rc<Self>, visit: u64, token: Token, play: lfx_player::PlayRequest) {
        {
            let st = self.st.borrow();
            if st.tv.visit != visit || st.tv.want != Some(token) {
                return;
            }
        }
        let player = match self.tv_ensure_player() {
            Ok(p) => p,
            Err(text) => {
                tracing::warn!("TV-Player: {text}");
                self.tv_ctl(|c| {
                    c.on_play_result(token, &PlayResult::fail_code(FailReason::Browser, "P1"));
                });
                self.tv_pump();
                return;
            }
        };
        let gen0 = player.state().generation;
        let old_session = {
            let mut st = self.st.borrow_mut();
            st.tv.pending = Some((token, gen0));
            st.tv.playing_gen = None;
            st.tv.last_phase = None;
            st.tv
                .play_session
                .replace(play.play_session_id.clone().unwrap_or_default())
        };
        self.tv_delete_encodings(old_session);
        // Kein ProgressReporter: der TV-Modus meldet nichts („Weiterschauen“ bleibt unberührt).
        player.open(play);
    }

    fn tv_stop_player(&self) {
        let (player, session) = {
            let mut st = self.st.borrow_mut();
            st.tv.pending = None;
            st.tv.want = None;
            st.tv.playing_gen = None;
            (st.tv.player.clone(), st.tv.play_session.take())
        };
        if let Some(p) = player {
            p.stop();
        }
        self.tv_delete_encodings(session);
    }

    /// Rückmeldungen des Players an den Controller (alle 200 ms).
    fn tv_host_tick(self: &Rc<Self>) {
        let (player, pending, playing_gen, last) = {
            let st = self.st.borrow();
            (
                st.tv.player.clone(),
                st.tv.pending,
                st.tv.playing_gen,
                st.tv.last_phase,
            )
        };
        let Some(player) = player else {
            return;
        };
        let s = player.state();
        let mut result: Option<(Token, PlayResult)> = None;
        let mut phase: Option<PlayerPhase> = None;
        if let Some((token, gen0)) = pending {
            if s.generation > gen0 {
                if s.error.is_some() {
                    result = Some((token, PlayResult::fail_code(FailReason::Video, "M1")));
                } else if s.loaded {
                    result = Some((token, PlayResult::ok()));
                }
            }
        } else if let Some(g) = playing_gen
            && s.generation == g
        {
            phase = Some(if s.error.is_some() {
                PlayerPhase::Error
            } else if s.eof {
                PlayerPhase::Ended
            } else if s.buffering {
                PlayerPhase::Stalled
            } else {
                PlayerPhase::Playing
            });
        }
        if let Some((token, res)) = result {
            {
                let mut st = self.st.borrow_mut();
                st.tv.pending = None;
                if res.ok {
                    st.tv.playing_gen = Some(s.generation);
                }
            }
            self.tv_ctl(|c| c.on_play_result(token, &res));
            self.tv_pump();
        } else if let Some(ph) = phase
            && last != Some(ph)
        {
            self.st.borrow_mut().tv.last_phase = Some(ph);
            self.tv_ctl(|c| {
                c.on_player_state(ph);
                if ph == PlayerPhase::Error {
                    c.on_player_error(FailReason::Video, Some("M2"), None);
                }
            });
            self.tv_pump();
        }
    }

    /// Das Videofenster hat Lage oder Größe geändert (Fensterkoordinaten, logische Pixel).
    pub fn tv_set_rect(&self, x: f32, y: f32, w: f32, h: f32) {
        if w < 2.0 || h < 2.0 {
            return;
        }
        let r = Rect::new(x, y, w, h);
        let has_pipe = {
            let mut st = self.st.borrow_mut();
            st.tv.rect = Some(r);
            st.tv.pipe.is_some()
        };
        if has_pipe && let Some(layer) = self.shell.video_layer() {
            layer.set_rect(Some(r));
        }
    }

    // ---------------------------------------------------------------- Eingabe

    /// Aktionen der TV-Seite; `true` = behandelt.
    pub fn tv_action(self: &Rc<Self>, action: NavAction) -> bool {
        if !self.st.borrow().tv.active {
            return false;
        }
        self.tv_activity();
        let card_open = self.st.borrow().tv.card.is_some();
        if card_open {
            match action {
                NavAction::Back | NavAction::Info | NavAction::Menu => self.tv_close_card(),
                NavAction::Activate => self.tv_card_action(0),
                _ => {}
            }
            return true;
        }
        match action {
            NavAction::Back => {
                if self.st.borrow().tv.fullscreen {
                    self.tv_set_fullscreen(false);
                    true
                } else {
                    false
                }
            }
            NavAction::ChannelUp | NavAction::Up => {
                self.tv_zap(1);
                true
            }
            NavAction::ChannelDown | NavAction::Down => {
                self.tv_zap(-1);
                true
            }
            NavAction::Digit(n) => {
                self.tv_ctl(|c| c.push_digit(char::from(b'0' + n.min(9))));
                self.tv_pump();
                true
            }
            NavAction::Mute => {
                self.tv_toggle_mute();
                true
            }
            NavAction::VolumeUp | NavAction::VolumeDown => {
                let v = f64::from(self.st.borrow().tv.snap.as_ref().map_or(100, |s| s.volume));
                let nv = if action == NavAction::VolumeUp {
                    v + 5.0
                } else {
                    v - 5.0
                };
                self.tv_ctl(|c| c.set_volume(nv));
                self.tv_pump();
                true
            }
            NavAction::Fullscreen => {
                let on = !self.st.borrow().tv.fullscreen;
                self.tv_set_fullscreen(on);
                true
            }
            NavAction::Info | NavAction::Activate => {
                self.tv_open_current_card();
                true
            }
            NavAction::Left => {
                self.tv_select_tab(0);
                true
            }
            NavAction::Right => {
                self.tv_select_tab(1);
                true
            }
            NavAction::Menu => {
                let t = 1 - self.st.borrow().tv.tab;
                self.tv_select_tab(t);
                true
            }
            _ => false,
        }
    }

    fn tv_toggle_mute(self: &Rc<Self>) {
        self.tv_ctl(|c| c.toggle_mute());
        self.tv_pump();
    }

    /// Bewegung oder Taste: Bedienleiste einblenden, nach 3 s wieder aus.
    pub fn tv_activity(self: &Rc<Self>) {
        if !self.st.borrow().tv.active {
            return;
        }
        self.tv_global(|g| g.set_controls_on(true));
        let weak = self.ui.clone();
        self.st
            .borrow()
            .tv
            .controls_hide
            .start(TimerMode::SingleShot, CONTROLS_SHOW, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.global::<TvState>().set_controls_on(false);
                }
            });
    }

    pub fn tv_zap(self: &Rc<Self>, delta: i64) {
        self.tv_ctl(|c| c.zap(delta));
        self.tv_pump();
    }

    pub fn tv_tune(self: &Rc<Self>, no: i64) {
        self.tv_ctl(|c| c.tune(no, true));
        self.tv_pump();
    }

    pub fn tv_select_tab(self: &Rc<Self>, tab: i32) {
        self.st.borrow_mut().tv.tab = tab;
        self.tv_global(|g| g.set_tab(tab));
        if tab == 1 {
            self.tv_refresh_epg(true);
        } else {
            let ch = self.st.borrow().tv.last_channel;
            self.tv_scroll_list(ch);
        }
    }

    pub fn tv_select_day(self: &Rc<Self>, i: i32) {
        let now = self.tv_now();
        {
            let mut st = self.st.borrow_mut();
            let tv = &mut st.tv;
            let (Some(ctl), Some(grid)) = (tv.ctl.as_mut(), tv.grid.as_mut()) else {
                return;
            };
            let Some(d) = grid.days().get(i.max(0) as usize).map(|d| d.d) else {
                return;
            };
            grid.select_day(ctl.plan_mut(), d, now);
        }
        self.tv_refresh_epg(true);
    }

    pub fn tv_epg_now(self: &Rc<Self>) {
        let now = self.tv_now();
        {
            let mut st = self.st.borrow_mut();
            let tv = &mut st.tv;
            let (Some(ctl), Some(grid)) = (tv.ctl.as_mut(), tv.grid.as_mut()) else {
                return;
            };
            grid.select_today(ctl.plan_mut(), now);
        }
        self.tv_refresh_epg(true);
    }

    pub fn tv_set_fullscreen(self: &Rc<Self>, on: bool) {
        self.st.borrow_mut().tv.fullscreen = on;
        self.tv_global(|g| g.set_fullscreen(on));
    }

    pub fn tv_cell_clicked(self: &Rc<Self>, row: i32, cell: i32) {
        let now = self.tv_now();
        let found = {
            let st = self.st.borrow();
            let Some(grid) = st.tv.grid.as_ref() else {
                return;
            };
            grid.rows()
                .get(row.max(0) as usize)
                .and_then(|r| r.cells.get(cell.max(0) as usize))
                .map(|c| (c.no, c.entry.clone(), EpgGrid::on_cell(c, now)))
        };
        let Some((no, _entry, action)) = found else {
            return;
        };
        match action {
            epg::CellAction::Tune(_) => self.tv_tune(no),
            epg::CellAction::Info { no, entry } => self.tv_open_card(no, &entry),
        }
    }

    fn tv_open_current_card(self: &Rc<Self>) {
        let cur = self
            .st
            .borrow()
            .tv
            .snap
            .as_ref()
            .map(|s| (s.channel, s.entry.clone()));
        if let Some((no, Some(e))) = cur {
            self.tv_open_card(no, &e);
        }
    }

    fn tv_open_card(self: &Rc<Self>, no: i64, entry: &PlanEntry) {
        let now = self.tv_now();
        let (card, want_id) = {
            let mut st = self.st.borrow_mut();
            let tv = &mut st.tv;
            let (Some(ctl), Some(details)) = (tv.ctl.as_mut(), tv.details.as_mut()) else {
                return;
            };
            let card = build_info_card(ctl.plan_mut(), no, entry, now, Some(details.map()));
            let want = card
                .item_id
                .clone()
                .and_then(|id| details.request(&id, true, now));
            (card, want)
        };
        self.tv_show_card(&card);
        self.st.borrow_mut().tv.card = Some(card);
        if let (Some(id), Some(active)) = (want_id, self.session.active()) {
            let client = active.client.clone();
            let visit = self.st.borrow().tv.visit;
            self.rt.spawn(async move {
                let text = client
                    .item(&id, &["Overview"])
                    .await
                    .ok()
                    .and_then(|i| i.overview)
                    .unwrap_or_default();
                post(move |app| app.tv_details_loaded(visit, &id, &text));
            });
        }
    }

    fn tv_details_loaded(self: &Rc<Self>, visit: u64, id: &str, text: &str) {
        let card = {
            let mut st = self.st.borrow_mut();
            if st.tv.visit != visit {
                return;
            }
            let tv = &mut st.tv;
            let Some(details) = tv.details.as_mut() else {
                return;
            };
            details.provide(id, text);
            let map = details.map().clone();
            tv.card.as_mut().map(|c| {
                c.set_details(Some(&map));
                c.clone()
            })
        };
        if let Some(c) = card {
            self.tv_show_card(&c);
        }
    }

    fn tv_show_card(&self, c: &InfoCard) {
        let (color, ink, short) = {
            let st = self.st.borrow();
            st.tv
                .list
                .as_ref()
                .and_then(|l| l.rows().iter().find(|r| r.no == c.no))
                .map_or_else(
                    || {
                        (
                            slint::Color::default(),
                            slint::Color::default(),
                            String::new(),
                        )
                    },
                    |r| {
                        (
                            view::hex_color(r.badge.color),
                            view::hex_color(r.badge.ink),
                            r.badge.short.clone(),
                        )
                    },
                )
        };
        let overview = match &c.overview {
            Overview::Hidden => String::new(),
            o => o.text().to_owned(),
        };
        let actions: Vec<String> = c.actions.iter().map(|a| a.label().to_owned()).collect();
        self.tv_global(|g| {
            g.set_ic_open(true);
            g.set_ic_channel(c.channel.as_str().into());
            g.set_ic_color(color);
            g.set_ic_ink(ink);
            g.set_ic_short(short.as_str().into());
            g.set_ic_title(c.title.as_str().into());
            g.set_ic_sub(c.sub.as_str().into());
            g.set_ic_time(c.time_line.as_str().into());
            g.set_ic_meta(c.meta.clone().unwrap_or_default().as_str().into());
            g.set_ic_badges(view::strings(&c.badges));
            g.set_ic_overview(overview.as_str().into());
            g.set_ic_actions(view::strings(&actions));
        });
    }

    pub fn tv_close_card(self: &Rc<Self>) {
        self.st.borrow_mut().tv.card = None;
        self.tv_global(|g| g.set_ic_open(false));
    }

    pub fn tv_card_action(self: &Rc<Self>, i: usize) {
        let action = self
            .st
            .borrow()
            .tv
            .card
            .as_ref()
            .and_then(|c| c.actions.get(i).cloned());
        let Some(action) = action else {
            return;
        };
        self.tv_close_card();
        match action {
            CardAction::TuneNow(no) => self.tv_tune(no),
            CardAction::Details(id) => self.go(Route::Detail { id }),
            CardAction::PlayFromStart(id) => {
                let item = BaseItem {
                    id,
                    ..BaseItem::default()
                };
                self.play_item(&item);
            }
        }
    }

    /// Knopf der Zustandskarte (Erneut versuchen / Zum Starten tippen / Ton an).
    pub fn tv_card_button(self: &Rc<Self>) {
        let gesture =
            self.st.borrow().tv.snap.as_ref().is_some_and(|s| {
                s.needs_tap || s.muted_by_policy || s.reason == Some(Reason::Gesture)
            });
        if gesture {
            self.tv_ctl(|c| c.gesture());
        } else {
            self.tv_ctl(|c| c.retry());
        }
        self.tv_pump();
    }

    pub fn tv_set_volume(self: &Rc<Self>, v: f32) {
        self.tv_ctl(|c| c.set_volume(f64::from(v) * 100.0));
        self.tv_pump();
    }

    /// Verdrahtet die Rückrufe der Seite (einmal beim Start).
    pub fn tv_wire(app: &Rc<Self>, ui: &MainWindow) {
        let g = ui.global::<TvState>();
        let a = app.clone();
        g.on_leave(move || a.apply_action(NavAction::Back));
        let a = app.clone();
        g.on_select_tab(move |t| a.tv_select_tab(t));
        let a = app.clone();
        g.on_select_day(move |i| a.tv_select_day(i));
        let a = app.clone();
        g.on_epg_now(move || a.tv_epg_now());
        let a = app.clone();
        g.on_tune(move |n| a.tv_tune(i64::from(n)));
        let a = app.clone();
        g.on_zap(move |d| a.tv_zap(i64::from(d)));
        let a = app.clone();
        g.on_cell_clicked(move |r, c| a.tv_cell_clicked(r, c));
        let a = app.clone();
        g.on_toggle_mute(move || a.tv_toggle_mute());
        let a = app.clone();
        g.on_set_volume(move |v| a.tv_set_volume(v));
        let a = app.clone();
        g.on_toggle_fullscreen(move || {
            let on = !a.st.borrow().tv.fullscreen;
            a.tv_set_fullscreen(on);
        });
        let a = app.clone();
        g.on_open_info(move || a.tv_open_current_card());
        let a = app.clone();
        g.on_close_info(move || a.tv_close_card());
        let a = app.clone();
        g.on_info_action(move |i| a.tv_card_action(i.max(0) as usize));
        let a = app.clone();
        g.on_card_button_clicked(move || a.tv_card_button());
        let a = app.clone();
        g.on_activity(move || a.tv_activity());
        let a = app.clone();
        g.on_player_rect(move |x, y, w, h| a.tv_set_rect(x, y, w, h));
    }

    // ---------------------------------------------------------------- Entwicklung

    /// Zustand für Tests und Fehlersuche (keine Tokens, keine Adressen).
    pub fn tv_state_json(&self) -> Value {
        let (load_error, loading) =
            self.tv_global(|g| (g.get_load_error().to_string(), g.get_loading()));
        let st = self.st.borrow();
        let tv = &st.tv;
        let snap = tv.snap.as_ref();
        let player = tv.player.as_ref().map(|p| {
            let s = p.state();
            let frames = tv.pipe.as_ref().map_or(0, |pipe| {
                pipe.stats.frames.load(std::sync::atomic::Ordering::Relaxed)
            });
            json!({"loaded": s.loaded, "position": s.position, "paused": s.paused,
                   "eof": s.eof, "error": s.error.as_ref().map(|e| e.text.clone()),
                   "frames": frames, "volume": s.volume, "muted": s.muted})
        });
        let card_code = snap.and_then(|s| match &s.card {
            Some(Card::Error { code, .. }) => code.clone(),
            _ => None,
        });
        json!({
            "active": tv.active,
            "tab": tv.tab,
            "fullscreen": tv.fullscreen,
            "controller": tv.ctl.is_some(),
            "channel": snap.map(|s| s.channel),
            "phase": snap.map(|s| format!("{:?}", s.phase)),
            "reason": snap.and_then(|s| s.reason.map(|r| r.as_str())),
            "card": snap.and_then(|s| s.card.as_ref()).map(|c| match c {
                Card::Next { .. } => "next",
                Card::Pause { .. } => "pause",
                Card::Closedown { .. } => "closedown",
                Card::Empty { .. } => "empty",
                Card::Error { .. } => "error",
            }),
            "card_code": card_code,
            "entry_item": snap.and_then(|s| s.entry.as_ref()).and_then(|e| e.item_id.clone()),
            "entry_kind": snap.and_then(|s| s.entry.as_ref()).map(|e| e.kind.as_str()),
            "volume": snap.map(|s| s.volume),
            "muted": snap.map(|s| s.muted),
            "list_rows": tv.list_model.as_ref().map_or(0, |m| m.row_count()),
            "info_card": tv.card.is_some(),
            "plays": tv.plays,
            "rect": tv.rect.map(|r| json!([r.x, r.y, r.w, r.h])),
            "player": player,
            "load_error": load_error,
            "loading": loading,
        })
    }

    /// Dev-Befehl `tv` mit der Operation `op` und Wert `value`.
    pub fn tv_automation(self: &Rc<Self>, msg: &Value) -> Result<Value, String> {
        let op = msg["op"].as_str().unwrap_or("");
        let n = msg["value"].as_i64().unwrap_or(0);
        match op {
            "tab" => self.tv_select_tab(n as i32),
            "zap" => self.tv_zap(n),
            "tune" => self.tv_tune(n),
            "digit" => {
                self.tv_ctl(|c| c.push_digit(char::from(b'0' + (n.clamp(0, 9) as u8))));
                self.tv_pump();
            }
            "info" => self.tv_open_current_card(),
            "closeinfo" => self.tv_close_card(),
            "day" => self.tv_select_day(n as i32),
            "epgnow" => self.tv_epg_now(),
            "fs" => self.tv_set_fullscreen(n != 0),
            "mute" => self.tv_toggle_mute(),
            "retry" => self.tv_card_button(),
            "controls" => self.tv_global(|g| g.set_controls_on(n != 0)),
            "front" => self.tv_dev_topmost(n != 0),
            "cell" => {
                let row = msg["row"].as_i64().unwrap_or(0) as i32;
                self.tv_cell_clicked(row, n as i32);
            }
            _ => return Err("unbekannte TV-Operation".into()),
        }
        Ok(json!({}))
    }

    /// Nur Entwicklung: Fenster ganz nach vorn (Bildschirmfotos lesen den Desktop, fremde Fenster stören sonst).
    fn tv_dev_topmost(&self, on: bool) {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
            };
            let hwnd = HWND(self.shell.hwnd_raw() as *mut std::ffi::c_void);
            let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
            // SAFETY: gültiges Fensterhandle der eigenen Shell, nur Z-Ordnung wird geändert.
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    Some(after),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
        #[cfg(not(windows))]
        let _ = on;
    }
}
