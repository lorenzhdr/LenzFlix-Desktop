//! Player-OSD in der App: Ansicht zusammenbauen, Zeitgeber, Befehle der Oberfläche, Tasten und Menüs.

use std::rc::Rc;
use std::time::{Duration, Instant};

use lfx_api::PlayMethod;
use lfx_nav::NavAction;
use lfx_player::segments::{SegmentKind, SkipAction, should_show_up_next, skippable_at};
use lfx_player::{PlayerState, TrackChoice, TrackKind};
use lfx_ui::{MenuRow, PlayerView};
use slint::{ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::play::{MenuState, format_time};
use crate::play_osd::{
    Control, Dir, InfoFacts, MenuAction, MenuKind, SubStyle, audio_menu, control_slot, info_menu,
    osd_visible, pick_default_tracks, quality_menu, skip_label, speed_menu, step_focus,
    subtitle_menu, trick_cell,
};
use crate::route::Route;

/// Zeitgeber-Takt der Oberfläche.
const TICK: Duration = Duration::from_millis(100);
/// Ab dieser Restzeit (ohne Abspann-Segment) erscheint „Nächste Folge“.
const UP_NEXT_FALLBACK_SECS: f64 = 30.0;
/// Standard-Spulschritte.
const SEEK_BACK: f64 = -10.0;
const SEEK_FWD: f64 = 30.0;

fn rows(items: &[crate::play_osd::MenuItem]) -> ModelRc<MenuRow> {
    ModelRc::new(VecModel::from(
        items
            .iter()
            .map(|i| MenuRow {
                label: i.label.as_str().into(),
                detail: i.detail.as_str().into(),
                selected: i.selected,
            })
            .collect::<Vec<_>>(),
    ))
}

impl App {
    // ---------------------------------------------------------------- Zeitgeber

    pub(crate) fn start_osd_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.timers
            .osd
            .start(slint::TimerMode::Repeated, TICK, move || {
                if let Some(app) = weak.upgrade() {
                    app.osd_tick();
                }
            });
    }

    fn osd_tick(self: &Rc<Self>) {
        if !matches!(self.st.borrow().router.current(), Route::Player { .. }) {
            self.timers.osd.stop();
            return;
        }
        let (player, opened) = {
            let st = self.st.borrow();
            (st.play.player.clone(), st.play.opened)
        };
        if let (Some(player), true) = (player, opened) {
            let s = player.state();
            self.tick_loaded(&player, &s);
            self.tick_up_next(&s);
            self.tick_media(&s);
            if let Some(e) = &s.error {
                let mut st = self.st.borrow_mut();
                if st.play.error.is_none() {
                    st.play.error = Some((
                        crate::play_osd::error_code(crate::play_osd::FailStage::Engine, e.code),
                        e.text.clone(),
                    ));
                }
            }
            if s.eof && s.loaded {
                tracing::info!("Ende erreicht");
                let (next, auto) = {
                    let st = self.st.borrow();
                    (st.play.next_item.is_some(), st.play.autoplay_next)
                };
                if next && auto {
                    self.play_next();
                } else {
                    self.leave_player();
                }
                return;
            }
        }
        self.refresh_view();
    }

    /// Arbeiten, die erst nach dem Laden möglich sind: Standard-Spuren, Untertitelstil, Lautstärke, HDR.
    fn tick_loaded(self: &Rc<Self>, player: &lfx_player::Player, s: &PlayerState) {
        if !s.loaded {
            return;
        }
        let (tracks_done, style_done, vol_done) = {
            let st = self.st.borrow();
            (
                st.play.tracks_applied,
                st.play.style_applied,
                st.play.volume_applied,
            )
        };
        if !tracks_done && !s.tracks.is_empty() {
            let prefs = self.st.borrow().play.prefs.clone();
            let (a, sub) = pick_default_tracks(&s.tracks, &prefs);
            if let Some(a) = a {
                player.set_audio_track(a);
            }
            if let Some(sub) = sub {
                player.set_subtitle_track(sub);
            }
            self.st.borrow_mut().play.tracks_applied = true;
        }
        if !style_done {
            let style = self.st.borrow().play.sub_style;
            self.apply_sub_style(player, style);
            self.st.borrow_mut().play.style_applied = true;
        }
        if !vol_done {
            let (v, m) = self.load_volume();
            player.set_volume(v);
            player.set_muted(m);
            self.st.borrow_mut().play.volume_applied = true;
        }
        self.tick_hdr(player, s);
    }

    fn tick_hdr(self: &Rc<Self>, player: &lfx_player::Player, s: &PlayerState) {
        if s.video.width <= 0 {
            return;
        }
        let Some(layer) = self.shell.video_layer() else {
            return;
        };
        let out = layer.output_hdr_info();
        let key = format!(
            "{}x{}|{:?}|{:?}|{:?}|{}",
            s.video.width,
            s.video.height,
            s.video.gamma,
            s.video.max_cll.map(|c| c as i64),
            s.video.sig_peak.map(|c| (c * 100.0) as i64),
            out.hdr_enabled
        );
        if self.st.borrow().play.hdr_key.as_deref() == Some(&key) {
            return;
        }
        let plan = crate::play_hdr::plan(&s.video, &out, false);
        crate::play_hdr::apply(&plan, &layer, player);
        let mut st = self.st.borrow_mut();
        st.play.hdr_key = Some(key);
        st.play.hdr_status = plan.status;
    }

    fn tick_up_next(self: &Rc<Self>, s: &PlayerState) {
        if !s.loaded {
            return;
        }
        let now = Instant::now();
        let mut st = self.st.borrow_mut();
        let dur = s.duration.unwrap_or(0.0);
        if st.play.next_item.is_some()
            && !st.play.up_next.dismissed()
            && dur > 60.0
            && should_show_up_next(
                &st.play.segments,
                s.position,
                s.duration,
                UP_NEXT_FALLBACK_SECS,
            )
        {
            st.play.up_next.show(now);
        }
        // Zurückgespult hinter den Beginn der Karte: wieder zulassen.
        if st.play.up_next.visible()
            && !should_show_up_next(
                &st.play.segments,
                s.position,
                s.duration,
                UP_NEXT_FALLBACK_SECS,
            )
        {
            st.play.up_next.reset();
        }
        let expired = st.play.up_next.expired(now) && st.play.autoplay_next;
        drop(st);
        if expired {
            self.play_next();
        }
    }

    // ---------------------------------------------------------------- Ansicht

    /// Setzt die Player-Ansicht aus dem Zustand.
    pub fn refresh_view(&self) {
        let now = Instant::now();
        let st = self.st.borrow();
        let p = &st.play;
        let s = p
            .player
            .as_ref()
            .filter(|_| p.opened)
            .map(lfx_player::Player::state)
            .unwrap_or_default();
        let dur = s.duration.unwrap_or(0.0);
        let shown_frac = p.scrub.map(f64::from);
        let pos = shown_frac.map_or(s.position, |f| f * dur);
        let error = p.error.clone();
        let loading = error.is_none() && (!p.opened || !s.loaded);
        let menu_open = p.menu.is_some();
        let blocking = loading || error.is_some() || p.resume.is_some();
        let osd = osd_visible(
            now,
            p.last_activity,
            s.paused && s.loaded,
            menu_open || p.focus.is_some(),
            blocking,
        );
        let has_next = p.next_item.is_some();
        let buffered = p
            .player
            .as_ref()
            .filter(|_| p.opened && dur > 0.0)
            .and_then(|pl| pl.mpv().get_property_f64("demuxer-cache-time").ok())
            .map_or(0.0, |t| (t / dur).clamp(0.0, 1.0) as f32);
        let skip = if s.loaded && !p.up_next.visible() {
            skippable_at(&p.segments, s.position, s.duration).map(|(seg, _)| skip_label(seg.kind))
        } else {
            None
        };
        // Trickplay-Vorschau
        let mut trick = (
            false,
            0.0f32,
            String::new(),
            false,
            slint::Image::default(),
            0,
            0,
            0,
            0,
        );
        if let Some(frac) = p.trick.hover.filter(|_| dur > 0.0) {
            let secs = f64::from(frac) * dur;
            trick.0 = true;
            trick.1 = frac;
            trick.2 = format_time(secs);
            if let Some(geom) = &p.trick.geom
                && let Some(cell) = trick_cell(geom, secs)
                && let Some(img) = p.trick.tiles.get(&cell.tile)
            {
                trick.3 = true;
                trick.4 = img.clone();
                trick.5 = cell.x as i32;
                trick.6 = cell.y as i32;
                trick.7 = cell.w as i32;
                trick.8 = cell.h as i32;
            }
        }
        let chapter_at = |t: f64| {
            p.chapters
                .iter()
                .rev()
                .find(|(start, _)| *start <= t + 0.5)
                .map(|(_, n)| n.clone())
                .unwrap_or_default()
        };
        let chapter_title = chapter_at(p.trick.hover.map_or(s.position, |f| f64::from(f) * dur));
        let marks: Vec<f32> = if dur > 0.0 {
            p.chapters
                .iter()
                .filter(|(start, _)| *start > 1.0)
                .map(|(start, _)| (start / dur).clamp(0.0, 1.0) as f32)
                .collect()
        } else {
            Vec::new()
        };
        let (menu_title, menu_items, menu_focus) = match &p.menu {
            Some(m) => (m.kind.title(), rows(&m.items), m.focus as i32),
            None => ("", rows(&[]), -1),
        };
        let (up_secs, up_title, up_sub) = match (&p.next_item, p.up_next.remaining(now)) {
            (Some(n), Some(r)) => (
                r as i32,
                n.name.clone(),
                crate::play_osd::episode_label(n.parent_index_number, n.index_number, &n.name),
            ),
            _ => (0, String::new(), String::new()),
        };
        let view = PlayerView {
            title: p.title.as_str().into(),
            subtitle: p.subtitle.as_str().into(),
            pos_text: format_time(pos).into(),
            dur_text: format_time(dur).into(),
            rest_text: format_time((dur - pos).max(0.0)).into(),
            progress: if dur > 0.0 {
                (pos / dur).clamp(0.0, 1.0) as f32
            } else {
                0.0
            },
            buffered,
            paused: s.paused,
            osd,
            buffering: s.buffering && s.loaded,
            loading,
            cover: p.cover.clone().unwrap_or_default(),
            has_cover: p.cover.is_some(),
            note: error.as_ref().map(|e| e.1.as_str()).unwrap_or("").into(),
            error_code: error.as_ref().map(|e| e.0.as_str()).unwrap_or("").into(),
            has_next,
            volume: (s.volume / 100.0).clamp(0.0, 1.0) as f32,
            muted: s.muted,
            fullscreen: p.fullscreen,
            focus_slot: control_slot(p.focus),
            skip_label: skip.unwrap_or("").into(),
            up_next_visible: p.up_next.visible(),
            up_next_title: up_title.into(),
            up_next_sub: up_sub.into(),
            up_next_secs: up_secs,
            up_next_focus: p.up_next_focus,
            menu_open,
            menu_title: menu_title.into(),
            menu_items,
            menu_focus,
            resume_open: p.resume.is_some(),
            resume_text: p.resume.as_ref().map_or(SharedString::new(), |r| {
                format!("Fortsetzen bei {}", format_time(r.position)).into()
            }),
            resume_focus: p.resume.as_ref().map_or(0, |r| r.focus),
            trick_visible: trick.0,
            trick_frac: trick.1,
            trick_time: trick.2.into(),
            trick_has_image: trick.3,
            trick_image: trick.4,
            trick_clip_x: trick.5,
            trick_clip_y: trick.6,
            trick_clip_w: trick.7,
            trick_clip_h: trick.8,
            chapter_marks: ModelRc::new(VecModel::from(marks)),
            chapter_title: chapter_title.into(),
        };
        drop(st);
        self.win().set_pv(view);
    }

    // ---------------------------------------------------------------- Befehle der Oberfläche

    /// Alle Rückrufe der Player-Seite.
    pub fn player_cmd(self: &Rc<Self>, name: &str, value: f32) {
        if name != "activity" && name != "scrub" && name != "scrub-end" {
            self.player_activity();
        }
        match name {
            "activity" => self.player_activity(),
            "toggle" => self.toggle_pause(),
            "seek-by" => self.seek_by(value),
            "seek-to" => self.seek_to_fraction(value),
            "scrub" => self.scrub(value),
            "scrub-end" => {
                self.st.borrow_mut().play.trick.hover = None;
                self.refresh_view();
            }
            "leave" => self.leave_player(),
            "menu-close" => self.close_menu(),
            "menu-pick" => self.menu_pick(value.max(0.0) as usize),
            "skip" => self.skip_segment(),
            "next" => self.play_next(),
            "up-next-now" => self.play_next(),
            "up-next-cancel" => {
                self.st.borrow_mut().play.up_next.dismiss();
                self.refresh_view();
            }
            "resume-pick" => self.resume_chosen(value >= 0.5),
            "volume" => self.set_volume_fraction(value),
            "mute" => self.toggle_mute(),
            "fullscreen" => self.toggle_fullscreen(),
            other => {
                if let Some(kind) = other.strip_prefix("menu:").and_then(MenuKind::from_name) {
                    self.toggle_menu(kind);
                }
            }
        }
    }

    /// Bewegung oder Eingabe: OSD zeigen.
    pub fn player_activity(self: &Rc<Self>) {
        self.st.borrow_mut().play.last_activity = Some(Instant::now());
    }

    pub fn toggle_pause(self: &Rc<Self>) {
        self.player_activity();
        if let Some(p) = self.st.borrow().play.player.clone() {
            p.toggle_pause();
        }
    }

    pub fn seek_by(self: &Rc<Self>, secs: f32) {
        self.player_activity();
        if let Some(p) = self.st.borrow().play.player.clone() {
            p.seek_by(f64::from(secs));
        }
    }

    pub fn seek_to_fraction(self: &Rc<Self>, frac: f32) {
        self.player_activity();
        if let Some(p) = self.st.borrow().play.player.clone()
            && let Some(d) = p.state().duration
        {
            p.seek_to(d * f64::from(frac.clamp(0.0, 1.0)));
        }
    }

    fn set_volume_fraction(self: &Rc<Self>, frac: f32) {
        let v = f64::from(frac.clamp(0.0, 1.0)) * 100.0;
        if let Some(p) = self.st.borrow().play.player.clone() {
            p.set_volume(v);
            if v > 0.0 {
                p.set_muted(false);
            }
        }
        self.save_volume(v, false);
    }

    fn toggle_mute(self: &Rc<Self>) {
        if let Some(p) = self.st.borrow().play.player.clone() {
            let s = p.state();
            p.set_muted(!s.muted);
            self.save_volume(s.volume, !s.muted);
        }
    }

    fn step_volume(self: &Rc<Self>, delta: f64) {
        if let Some(p) = self.st.borrow().play.player.clone() {
            let v = (p.state().volume + delta).clamp(0.0, 100.0);
            p.set_volume(v);
            p.set_muted(false);
            self.save_volume(v, false);
        }
    }

    fn skip_segment(self: &Rc<Self>) {
        let Some(player) = self.st.borrow().play.player.clone() else {
            return;
        };
        let s = player.state();
        let action = skippable_at(&self.st.borrow().play.segments, s.position, s.duration);
        match action {
            Some((_, SkipAction::SeekTo(t))) => player.seek_to(t),
            Some((_, SkipAction::NextItem)) => {
                if self.st.borrow().play.next_item.is_some() {
                    self.play_next();
                } else {
                    self.leave_player();
                }
            }
            None => {}
        }
    }

    /// Ist ein Überspringen-Knopf sichtbar (für Aktivieren per Taste)?
    fn skip_visible(&self) -> Option<SegmentKind> {
        let st = self.st.borrow();
        let s = st.play.player.as_ref()?.state();
        if st.play.up_next.visible() {
            return None;
        }
        skippable_at(&st.play.segments, s.position, s.duration).map(|(seg, _)| seg.kind)
    }

    // ---------------------------------------------------------------- Menüs

    fn build_menu(&self, kind: MenuKind) -> Vec<crate::play_osd::MenuItem> {
        let st = self.st.borrow();
        let s = st
            .play
            .player
            .as_ref()
            .map(lfx_player::Player::state)
            .unwrap_or_default();
        match kind {
            MenuKind::Audio => audio_menu(&s.tracks),
            MenuKind::Subtitles => subtitle_menu(&s.tracks, st.play.sub_style),
            MenuKind::Speed => speed_menu(s.speed),
            MenuKind::Quality => quality_menu(st.play.quality),
            MenuKind::Info => {
                let method = match st.play.request.as_ref().map(|r| r.play_method) {
                    Some(PlayMethod::DirectPlay) => "Direktwiedergabe",
                    Some(PlayMethod::DirectStream) => "Direkt (umgepackt)",
                    Some(PlayMethod::Transcode) => "Server-Umwandlung",
                    None => "–",
                };
                info_menu(
                    &s,
                    &InfoFacts {
                        method: method.to_owned(),
                        hdr_state: if st.play.hdr_status.is_empty() {
                            "–".to_owned()
                        } else {
                            st.play.hdr_status.clone()
                        },
                    },
                )
            }
        }
    }

    pub fn open_menu(self: &Rc<Self>, kind: MenuKind) {
        let items = self.build_menu(kind);
        let focus = items.iter().position(|i| i.selected).unwrap_or(0);
        let mut st = self.st.borrow_mut();
        st.play.menu = Some(MenuState { kind, items, focus });
        drop(st);
        self.refresh_view();
    }

    fn toggle_menu(self: &Rc<Self>, kind: MenuKind) {
        let same = self.st.borrow().play.menu.as_ref().map(|m| m.kind) == Some(kind);
        if same {
            self.close_menu();
        } else {
            self.open_menu(kind);
        }
    }

    pub fn close_menu(self: &Rc<Self>) {
        self.st.borrow_mut().play.menu = None;
        self.refresh_view();
    }

    fn menu_pick(self: &Rc<Self>, index: usize) {
        let (kind, action) = {
            let st = self.st.borrow();
            let Some(m) = &st.play.menu else { return };
            let Some(item) = m.items.get(index) else {
                return;
            };
            (m.kind, item.action)
        };
        let player = self.st.borrow().play.player.clone();
        let mut keep_open = false;
        match action {
            MenuAction::Track(TrackKind::Audio, c) => {
                if let Some(p) = &player {
                    p.set_audio_track(c);
                }
            }
            MenuAction::Track(_, c) => {
                if let Some(p) = &player {
                    p.set_subtitle_track(c);
                }
            }
            MenuAction::Speed(v) => {
                if let Some(p) = &player {
                    p.set_speed(v);
                }
            }
            MenuAction::Quality(i) => {
                self.close_menu();
                self.change_quality(i);
                return;
            }
            MenuAction::SubSize => {
                let s = self.st.borrow().play.sub_style.next_size();
                self.set_sub_style(s);
                keep_open = true;
            }
            MenuAction::SubBackground => {
                let s = self.st.borrow().play.sub_style.toggle_background();
                self.set_sub_style(s);
                keep_open = true;
            }
            MenuAction::None => keep_open = true,
        }
        if keep_open {
            let items = self.build_menu(kind);
            if let Some(m) = self.st.borrow_mut().play.menu.as_mut() {
                m.items = items;
            }
            self.refresh_view();
        } else {
            self.close_menu();
        }
    }

    fn set_sub_style(self: &Rc<Self>, style: SubStyle) {
        self.st.borrow_mut().play.sub_style = style;
        if let Some(p) = self.st.borrow().play.player.clone() {
            self.apply_sub_style(&p, style);
        }
        self.save_setting("player.sub_style", style.to_json());
    }

    pub(crate) fn apply_sub_style(&self, player: &lfx_player::Player, style: SubStyle) {
        let m = player.mpv();
        for (k, v) in style.mpv_options() {
            let _ = m.set_property_string(k, &v);
        }
    }

    pub(crate) fn load_sub_style(&self) -> SubStyle {
        self.session
            .active()
            .and_then(|a| {
                self.store
                    .get_setting(&a.server_id, &a.user_id, "player.sub_style")
                    .ok()
                    .flatten()
            })
            .map_or_else(SubStyle::default, |v| SubStyle::from_json(&v))
    }

    fn load_volume(&self) -> (f64, bool) {
        let v = self.session.active().and_then(|a| {
            self.store
                .get_setting(&a.server_id, &a.user_id, "player.volume")
                .ok()
                .flatten()
        });
        match v {
            Some(v) => (
                v["volume"].as_f64().unwrap_or(100.0).clamp(0.0, 100.0),
                v["muted"].as_bool().unwrap_or(false),
            ),
            None => (100.0, false),
        }
    }

    fn save_volume(&self, volume: f64, muted: bool) {
        self.save_setting(
            "player.volume",
            serde_json::json!({"volume": volume, "muted": muted}),
        );
    }

    // ---------------------------------------------------------------- Tasten und Gamepad

    /// Aktionen im Player-Kontext (Tastatur, Gamepad, Automation).
    pub fn player_action(self: &Rc<Self>, action: NavAction) {
        // 1. Fortsetzen-Abfrage
        if self.st.borrow().play.resume.is_some() {
            match action {
                NavAction::Back | NavAction::Stop => self.leave_player(),
                NavAction::Activate | NavAction::PlayPause => {
                    let f = self.st.borrow().play.resume.as_ref().map_or(0, |r| r.focus);
                    self.resume_chosen(f == 1);
                }
                NavAction::VolumeUp | NavAction::SeekBackward | NavAction::Up | NavAction::Left => {
                    self.set_resume_focus(0)
                }
                NavAction::VolumeDown
                | NavAction::SeekForward
                | NavAction::Down
                | NavAction::Right => self.set_resume_focus(1),
                _ => {}
            }
            self.refresh_view();
            return;
        }
        // 2. Menü
        if self.st.borrow().play.menu.is_some() {
            match action {
                NavAction::Back | NavAction::Menu | NavAction::Stop => self.close_menu(),
                NavAction::Activate => {
                    let f = self.st.borrow().play.menu.as_ref().map_or(0, |m| m.focus);
                    self.menu_pick(f);
                }
                NavAction::VolumeUp | NavAction::Up => self.move_menu(-1),
                NavAction::VolumeDown | NavAction::Down => self.move_menu(1),
                _ => {}
            }
            self.refresh_view();
            return;
        }
        let Some(player) = self.st.borrow().play.player.clone() else {
            if action == NavAction::Back {
                self.leave_player();
            }
            return;
        };
        // 3. „Nächste Folge“-Karte
        if self.st.borrow().play.up_next.visible() {
            match action {
                NavAction::Back => {
                    self.st.borrow_mut().play.up_next.dismiss();
                    self.refresh_view();
                    return;
                }
                NavAction::Activate => {
                    if self.st.borrow().play.up_next_focus == 1 {
                        self.st.borrow_mut().play.up_next.dismiss();
                        self.refresh_view();
                    } else {
                        self.play_next();
                    }
                    return;
                }
                NavAction::SeekBackward | NavAction::Left => {
                    self.st.borrow_mut().play.up_next_focus = 0;
                    return;
                }
                NavAction::SeekForward | NavAction::Right => {
                    self.st.borrow_mut().play.up_next_focus = 1;
                    return;
                }
                _ => {}
            }
        }
        // 4. Bedienfokus (Tab/Menü-Taste): Pfeile bewegen den Fokus
        let focus = self.st.borrow().play.focus;
        if let Some(c) = focus {
            let has_next = self.st.borrow().play.next_item.is_some();
            let moved = match action {
                NavAction::SeekBackward | NavAction::Left if c == Control::Seek => {
                    player.seek_by(SEEK_BACK);
                    return;
                }
                NavAction::SeekForward | NavAction::Right if c == Control::Seek => {
                    player.seek_by(10.0);
                    return;
                }
                NavAction::SeekBackward | NavAction::Left => {
                    Some(step_focus(c, Dir::Left, has_next))
                }
                NavAction::SeekForward | NavAction::Right => {
                    Some(step_focus(c, Dir::Right, has_next))
                }
                NavAction::VolumeUp | NavAction::Up => Some(step_focus(c, Dir::Up, has_next)),
                NavAction::VolumeDown | NavAction::Down => Some(step_focus(c, Dir::Down, has_next)),
                _ => None,
            };
            if let Some(n) = moved {
                self.st.borrow_mut().play.focus = Some(n);
                self.refresh_view();
                return;
            }
            match action {
                NavAction::Activate => {
                    self.activate_control(c);
                    return;
                }
                NavAction::Back | NavAction::Menu => {
                    self.st.borrow_mut().play.focus = None;
                    self.refresh_view();
                    return;
                }
                _ => {}
            }
        }
        let s = player.state();
        match action {
            NavAction::Back | NavAction::Stop => self.leave_player(),
            NavAction::PlayPause => player.toggle_pause(),
            NavAction::Activate => {
                if self.skip_visible().is_some() {
                    self.skip_segment();
                } else {
                    player.toggle_pause();
                }
            }
            NavAction::Play => player.play(),
            NavAction::Pause => player.pause(),
            NavAction::SeekForward => player.seek_by(10.0),
            NavAction::SeekBackward => player.seek_by(SEEK_BACK),
            NavAction::VolumeUp => self.step_volume(5.0),
            NavAction::VolumeDown => self.step_volume(-5.0),
            NavAction::Mute => self.toggle_mute(),
            NavAction::Fullscreen => self.toggle_fullscreen(),
            NavAction::Next => self.play_next(),
            NavAction::Previous => player.seek_to(0.0),
            NavAction::Info => self.open_menu(MenuKind::Info),
            NavAction::Menu => {
                self.st.borrow_mut().play.focus = Some(Control::PlayPause);
            }
            _ => {
                let _ = s;
            }
        }
        self.refresh_view();
    }

    fn set_resume_focus(&self, f: i32) {
        if let Some(r) = self.st.borrow_mut().play.resume.as_mut() {
            r.focus = f;
        }
    }

    fn move_menu(&self, delta: i32) {
        if let Some(m) = self.st.borrow_mut().play.menu.as_mut() {
            let n = m.items.len() as i32;
            if n > 0 {
                m.focus = (m.focus as i32 + delta).clamp(0, n - 1) as usize;
            }
        }
    }

    fn activate_control(self: &Rc<Self>, c: Control) {
        match c {
            Control::Back => self.leave_player(),
            Control::Rewind => self.seek_by(SEEK_BACK as f32),
            Control::PlayPause => self.toggle_pause(),
            Control::Forward => self.seek_by(SEEK_FWD as f32),
            Control::Seek => self.toggle_pause(),
            Control::Volume => self.toggle_mute(),
            Control::Next => self.play_next(),
            Control::Subtitles => self.toggle_menu(MenuKind::Subtitles),
            Control::Audio => self.toggle_menu(MenuKind::Audio),
            Control::Speed => self.toggle_menu(MenuKind::Speed),
            Control::Quality => self.toggle_menu(MenuKind::Quality),
            Control::Fullscreen => self.toggle_fullscreen(),
        }
    }

    // ---------------------------------------------------------------- Trickplay

    fn scrub(self: &Rc<Self>, frac: f32) {
        self.player_activity();
        let tile = {
            let mut st = self.st.borrow_mut();
            st.play.trick.hover = Some(frac);
            let dur = st
                .play
                .player
                .as_ref()
                .and_then(|p| p.state().duration)
                .unwrap_or(0.0);
            st.play
                .trick
                .geom
                .and_then(|g| trick_cell(&g, f64::from(frac) * dur))
                .map(|c| c.tile)
        };
        if let Some(t) = tile {
            self.request_trick_tile(t);
        }
        self.refresh_view();
    }

    /// Spurwahl für Tests: wählt die n-te Spur der Art.
    pub fn choose_track_for_test(&self, kind: TrackKind, n: usize) -> bool {
        let st = self.st.borrow();
        let Some(p) = &st.play.player else {
            return false;
        };
        let s = p.state();
        let Some(t) = s.tracks.iter().filter(|t| t.kind == kind).nth(n) else {
            return false;
        };
        match kind {
            TrackKind::Audio => p.set_audio_track(TrackChoice::Id(t.id)),
            TrackKind::Subtitle => p.set_subtitle_track(TrackChoice::Id(t.id)),
            TrackKind::Video => return false,
        }
        true
    }
}
