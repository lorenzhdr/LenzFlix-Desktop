//! W3-retro: LENZ//84 Effekte in der App (Web: `features/retro/fx/*`).
//!
//! Zeitpläne und Zustandsmaschinen kommen aus `lfx-retro-fx` (Boot, Glitch, Cheat-Code, Code-Regen, Terminal);
//! hier laufen sie auf einem 50-ms-Zeitgeber und schreiben in das Slint-Global `RetroFx`. Die Effekte sind nur im
//! Design LENZ//84 aktiv und nie im Player, im TV-Modus, in der Anmeldung und in der Verwaltung. Der Klang bleibt
//! standardmäßig aus und öffnet kein Audiogerät, solange er aus ist.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use lfx_retro_fx::boot::BootPlan;
use lfx_retro_fx::cheat::{CheatSequence, OVERDRIVE_MS, TAP_COUNT, TAP_GAP_MS, TapCounter};
use lfx_retro_fx::glitch::{GlitchEvent, GlitchSchedule};
use lfx_retro_fx::rain::{IdleTracker, RainField, TitleFlash};
use lfx_retro_fx::synth::FxSound;
use lfx_retro_fx::terminal::{History, TermStep};
use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer,
    VecModel,
};

use crate::app::App;
use crate::route::Route;
use lfx_ui::{RainCol, RetroFx, RetroLine};

/// Zeitgeberschritt in Millisekunden.
const TICK_MS: u64 = 50;
/// Länge der Streifen-Störung.
const STRIP_MS: u64 = 180;
/// Wackeln nach dem Cheat-Code.
const SHAKE_MS: u64 = 700;
/// Anzahl der Regenzeilen je Spur.
const TRAIL_LEN: usize = 14;
/// Höchstzahl der Terminal-Zeilen.
pub const TERM_MAX_LINES: usize = 60;

/// Läuft der Boot-Bildschirm.
pub struct BootRun {
    pub start_ms: u64,
    pub skipped_at: Option<u32>,
    pub plan: BootPlan,
}

/// Laufender Code-Regen.
pub struct RainRun {
    pub field: RainField,
    /// Spur je Spalte (neueste zuletzt).
    pub hist: Vec<Vec<char>>,
    pub last_y: Vec<f32>,
    pub next_frame_ms: u64,
    pub flash: TitleFlash,
    pub flash_clear_ms: u64,
    pub font: u32,
}

/// Zustand der Retro-Effekte (Teil von `State`).
pub struct RetroState {
    pub tick: Timer,
    pub term_timer: Timer,
    pub fx_active: bool,
    pub booted: bool,
    pub prefs_loaded: bool,
    pub crt: bool,
    pub sound: bool,
    pub boot: Option<BootRun>,
    pub cheat: CheatSequence,
    pub taps: TapCounter,
    pub od_start: Option<u64>,
    pub glitch: Option<GlitchSchedule>,
    pub glitch_next: Option<GlitchEvent>,
    pub glitch_origin: u64,
    pub strip_until: u64,
    pub shake_start: Option<u64>,
    pub rain: Option<RainRun>,
    pub idle: Option<IdleTracker>,
    pub term_open: bool,
    pub term_input: String,
    pub term_lines: Vec<(String, i32)>,
    pub term_pending: VecDeque<TermStep>,
    pub term_epoch: u64,
    pub history: History,
    pub caret_ms: u64,
    pub last_hero: String,
    pub last_clock: (u32, u32),
    pub last_flicker: i32,
    pub last_roll: i32,
    pub seed: u32,
    pub cursor: Option<(f32, f32)>,
    pub audio: Option<lfx_retro_fx::output::FxAudio>,
}

impl Default for RetroState {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(84, |d| d.subsec_nanos() ^ (d.as_secs() as u32));
        Self {
            tick: Timer::default(),
            term_timer: Timer::default(),
            fx_active: false,
            booted: false,
            prefs_loaded: false,
            crt: true,
            sound: false,
            boot: None,
            cheat: CheatSequence::new(),
            taps: TapCounter::new(),
            od_start: None,
            glitch: None,
            glitch_next: None,
            glitch_origin: 0,
            strip_until: 0,
            shake_start: None,
            rain: None,
            idle: None,
            term_open: false,
            term_input: String::new(),
            term_lines: Vec::new(),
            term_pending: VecDeque::new(),
            term_epoch: 0,
            history: History::new(),
            caret_ms: 0,
            last_hero: String::new(),
            last_clock: (99, 99),
            last_flicker: -1,
            last_roll: -2,
            seed,
            cursor: None,
            audio: None,
        }
    }
}

/// Dateiname im DOS-Stil („NEON_LAEUFER.MOV“), wie `dosName` im Web.
pub fn dos_name(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        match c {
            'ä' | 'Ä' => s.push_str("AE"),
            'ö' | 'Ö' => s.push_str("OE"),
            'ü' | 'Ü' => s.push_str("UE"),
            'ß' => s.push_str("SS"),
            c => s.extend(c.to_uppercase()),
        }
    }
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    if out.is_empty() {
        out.push_str("UNBEKANNT");
    }
    let cut: String = out.chars().take(22).collect();
    cut.trim_end_matches('_').to_owned()
}

/// Flackern (Web: Schlüsselbilder über 6,3 s), Deckkraft 0..1.
pub fn flicker_value(now_ms: u64) -> f32 {
    const KF: [(f32, f32); 12] = [
        (0.0, 0.97),
        (7.0, 0.5),
        (9.0, 1.0),
        (23.0, 0.7),
        (24.0, 0.99),
        (41.0, 0.55),
        (44.0, 0.98),
        (63.0, 0.8),
        (64.0, 0.4),
        (66.0, 1.0),
        (87.0, 0.65),
        (100.0, 0.97),
    ];
    let pct = (now_ms % 6300) as f32 / 63.0;
    let mut v = KF[0].1;
    for (p, val) in KF {
        if pct >= p {
            v = val;
        }
    }
    v
}

/// Rollbalken: 0..1 während der ersten 38 % von 12 s (ab 3 s), sonst -1.
pub fn roll_value(now_ms: u64) -> f32 {
    if now_ms < 3000 {
        return -1.0;
    }
    let ph = (now_ms - 3000) % 12_000;
    let run = 12_000.0 * 0.38;
    if (ph as f32) < run {
        ph as f32 / run
    } else {
        -1.0
    }
}

/// Deckkraft der Overdrive-Einblendung nach `t` ms (0 = fertig).
pub fn od_toast_opacity(t: u64) -> Option<f32> {
    Some(match t {
        0..=143 => 0.0,
        144..=359 => 1.0,
        360..=503 => 0.4,
        504..=1439 => 1.0,
        1440..=1511 => 0.6,
        1512..=3167 => 1.0,
        3168..=3311 => 0.3,
        3312..=3599 => 1.0,
        _ => return None,
    })
}

fn shake_offset(t: u64) -> (f32, f32) {
    const T: [(f32, f32); 10] = [
        (0.0, 0.0),
        (-6.0, 3.0),
        (5.0, -4.0),
        (-4.0, -2.0),
        (6.0, 2.0),
        (-3.0, 4.0),
        (4.0, -3.0),
        (-5.0, 1.0),
        (3.0, 3.0),
        (-2.0, -2.0),
    ];
    let i = ((t * 10) / SHAKE_MS) as usize;
    T[i.min(9)]
}

/// 1x4-Bild der Scanlines (Web: `--lfx-retro-scan`, 4-px-Raster, Alpha 0,22).
pub fn scan_image() -> Image {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(1, 4);
    let px = buf.make_mut_slice();
    for (i, p) in px.iter_mut().enumerate() {
        *p = Rgba8Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: if i == 3 {
                56
            } else if i == 2 {
                14
            } else {
                0
            },
        };
    }
    Image::from_rgba8_premultiplied(buf)
}

impl App {
    fn retro_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn retro_fx(&self) -> RetroFxHandle {
        RetroFxHandle(self.win())
    }

    /// Gleicht die Ebene mit Design und Seite ab (aufgerufen bei Designwechsel und Seitenwechsel).
    pub fn retro_sync(self: &Rc<Self>) {
        let (is_retro, route_ok) = {
            let st = self.st.borrow();
            let ok = !matches!(
                st.router.current(),
                Route::AddServer
                    | Route::Profiles { .. }
                    | Route::Login { .. }
                    | Route::Player { .. }
                    | Route::TvMode
                    | Route::Admin
            );
            (st.design == crate::cli::DesignId::Retro, ok)
        };
        let want = is_retro && route_ok && self.session.active().is_some();
        let was = self.st.borrow().retro.fx_active;
        if want {
            self.retro_load_prefs();
        }
        let h = self.retro_fx();
        let g = h.g();
        g.set_fx_on(want);
        if want && !was {
            let now = self.retro_ms();
            {
                let mut st = self.st.borrow_mut();
                let r = &mut st.retro;
                r.fx_active = true;
                g.set_scan(scan_image());
                g.set_crt_on(r.crt);
                g.set_sound_on(r.sound);
                let mut sched = GlitchSchedule::new(r.seed);
                r.glitch_next = Some(sched.next_event());
                r.glitch = Some(sched);
                r.glitch_origin = now;
                r.idle = Some(IdleTracker::new(now));
                r.last_clock = (99, 99);
            }
            self.retro_start_timer();
            if !self.st.borrow().retro.booted && std::env::var_os("LFX_RETRO_NO_BOOT").is_none() {
                self.retro_boot_start(false);
            }
        } else if !want && was {
            self.retro_stop_all();
        }
    }

    fn retro_start_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.st.borrow().retro.tick.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(TICK_MS),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.retro_tick();
                }
            },
        );
    }

    /// Alles beenden (Design gewechselt, Player geöffnet, Abmeldung).
    fn retro_stop_all(self: &Rc<Self>) {
        let g_win = self.win();
        let g = g_win.global::<RetroFx>();
        {
            let mut st = self.st.borrow_mut();
            let r = &mut st.retro;
            r.fx_active = false;
            r.tick.stop();
            r.term_timer.stop();
            r.boot = None;
            r.rain = None;
            r.od_start = None;
            r.term_open = false;
            r.term_pending.clear();
            r.term_epoch += 1;
            r.shake_start = None;
        }
        g.set_boot_show(false);
        g.set_boot_open(false);
        g.set_term_open(false);
        g.set_rain_show(false);
        g.set_od_toast(false);
        g.set_od_hue(-1.0);
        g.set_strip(false);
        g.set_shake_x(0.0);
        g.set_shake_y(0.0);
        g.set_fx_on(false);
    }

    // ---------------------------------------------------------------- Vorlieben

    fn retro_load_prefs(self: &Rc<Self>) {
        if self.st.borrow().retro.prefs_loaded {
            return;
        }
        let Some(a) = self.session.active() else {
            return;
        };
        let get = |k: &str, def: bool| {
            self.store
                .get_setting(&a.server_id, &a.user_id, k)
                .ok()
                .flatten()
                .and_then(|v| v.as_bool())
                .unwrap_or(def)
        };
        let crt = get("retro.crt", true);
        let sound = get("retro.sound", false);
        let mut st = self.st.borrow_mut();
        st.retro.prefs_loaded = true;
        st.retro.crt = crt;
        st.retro.sound = sound;
    }

    /// Setzt CRT oder Klang (`"crt"` / `"sound"`) und speichert pro Benutzer.
    pub fn retro_set_pref(self: &Rc<Self>, name: &str, on: bool) {
        {
            let mut st = self.st.borrow_mut();
            match name {
                "crt" => st.retro.crt = on,
                "sound" => st.retro.sound = on,
                _ => return,
            }
        }
        let g_win = self.win();
        let g = g_win.global::<RetroFx>();
        match name {
            "crt" => g.set_crt_on(on),
            _ => {
                g.set_sound_on(on);
                self.retro_audio_enable(on);
            }
        }
        self.save_setting(&format!("retro.{name}"), serde_json::Value::Bool(on));
    }

    pub fn retro_toggle_pref(self: &Rc<Self>, name: &str) {
        let cur = {
            let st = self.st.borrow();
            if name == "crt" {
                st.retro.crt
            } else {
                st.retro.sound
            }
        };
        self.retro_set_pref(name, !cur);
    }

    fn retro_audio_enable(self: &Rc<Self>, on: bool) {
        let mut st = self.st.borrow_mut();
        if on {
            let seed = st.retro.seed;
            let a = st
                .retro
                .audio
                .get_or_insert_with(|| lfx_retro_fx::output::FxAudio::new(seed));
            a.set_enabled(true);
            let _ = a.unlock();
            a.play(FxSound::Click);
        } else if let Some(a) = st.retro.audio.as_mut() {
            a.set_enabled(false);
            a.suspend();
        }
    }

    pub fn retro_play(&self, s: FxSound) {
        let st = self.st.borrow();
        if st.retro.sound
            && let Some(a) = st.retro.audio.as_ref()
        {
            a.play(s);
        }
    }

    // ---------------------------------------------------------------- Boot

    /// Startet den Boot-Bildschirm (einmal je App-Lauf; `reboot` im Terminal ruft ihn erneut).
    pub fn retro_boot_start(self: &Rc<Self>, _again: bool) {
        let now = self.retro_ms();
        let name = self.session.active().map(|a| a.user_name);
        {
            let mut st = self.st.borrow_mut();
            st.retro.booted = true;
            st.retro.boot = Some(BootRun {
                start_ms: now,
                skipped_at: None,
                plan: BootPlan::new(name.as_deref(), false),
            });
        }
        let win = self.win();
        let g = win.global::<RetroFx>();
        g.set_boot_hint(lfx_retro_fx::boot::SKIP_HINT.into());
        g.set_boot_lines(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
        g.set_boot_open(false);
        g.set_boot_line_phase(0);
        g.set_boot_show(true);
        self.retro_play(FxSound::Boot);
        self.retro_boot_frame();
    }

    /// Überspringen (Taste, Klick, Automation).
    pub fn retro_boot_skip(self: &Rc<Self>) {
        let now = self.retro_ms();
        let mut st = self.st.borrow_mut();
        if let Some(b) = st.retro.boot.as_mut()
            && b.skipped_at.is_none()
        {
            b.skipped_at = Some(now.saturating_sub(b.start_ms) as u32);
        }
    }

    fn retro_boot_frame(self: &Rc<Self>) {
        let now = self.retro_ms();
        let (frame, t, open_at, done) = {
            let st = self.st.borrow();
            let Some(b) = st.retro.boot.as_ref() else {
                return;
            };
            let t = now.saturating_sub(b.start_ms) as u32;
            let f = b.plan.frame(t, b.skipped_at);
            let (open_at, _) = b.plan.timeline(b.skipped_at);
            let fin = f.finished;
            (f, t, open_at, fin)
        };
        let win = self.win();
        let g = win.global::<RetroFx>();
        let lines: Vec<SharedString> = frame
            .lines
            .iter()
            .map(|s| SharedString::from(s.as_str()))
            .collect();
        g.set_boot_lines(ModelRc::new(VecModel::from(lines)));
        g.set_boot_caret(frame.caret && (now / 400).is_multiple_of(2));
        g.set_boot_open(frame.open);
        g.set_boot_line_phase(if !frame.open {
            0
        } else if t < open_at + 330 {
            1
        } else {
            2
        });
        if done {
            g.set_boot_show(false);
            g.set_boot_open(false);
            self.st.borrow_mut().retro.boot = None;
        }
    }

    // ---------------------------------------------------------------- Overdrive

    /// Cheat-Code oder 7 Tipps aufs Logo: 20 s Farbzyklus, Wackeln, Einblendung.
    pub fn retro_overdrive(self: &Rc<Self>) {
        let now = self.retro_ms();
        {
            let mut st = self.st.borrow_mut();
            st.retro.od_start = Some(now);
            st.retro.shake_start = Some(now);
        }
        self.retro_play(FxSound::Jingle);
        let win = self.win();
        let g = win.global::<RetroFx>();
        g.set_od_toast(true);
        g.set_od_toast_opacity(0.0);
        g.set_od_hue(41.0);
    }

    // ---------------------------------------------------------------- Eingaben

    /// Logo getippt: 7 schnelle Tipps lösen Overdrive aus.
    pub fn retro_logo_tap(self: &Rc<Self>) {
        let now = self.retro_ms();
        let fire = {
            let mut st = self.st.borrow_mut();
            if !st.retro.fx_active {
                return;
            }
            let _ = (TAP_COUNT, TAP_GAP_MS);
            st.retro.taps.tap_triggers(now)
        };
        if fire {
            self.retro_overdrive();
        }
    }

    /// Eine Taste. `true` = verbraucht (Boot, Terminal, Regen).
    pub fn retro_key(self: &Rc<Self>, text: &str) -> bool {
        use slint::platform::Key as K;
        if !self.st.borrow().retro.fx_active {
            return false;
        }
        let now = self.retro_ms();
        if let Some(i) = self.st.borrow_mut().retro.idle.as_mut() {
            i.note_input(now);
        }
        if self.st.borrow().retro.rain.is_some() {
            self.retro_rain_stop();
            return true;
        }
        if self.st.borrow().retro.boot.is_some() {
            self.retro_boot_skip();
            return true;
        }
        if self.st.borrow().retro.term_open {
            self.retro_term_key(text);
            return true;
        }
        // Tasten im Textfeld, Player oder Modal gehören der Seite.
        let blocked = {
            let st = self.st.borrow();
            st.ctx == lfx_nav::InputContext::TextInput || st.ctx == lfx_nav::InputContext::Player
        } || self.win().global::<lfx_ui::DetailState>().get_modal_open();
        if blocked {
            return false;
        }
        if text == "`" || text == "^" || text == "°" {
            self.retro_term_open();
            return true;
        }
        let name = {
            let is = |k: K| {
                let c: char = k.into();
                text.starts_with(c) && text.chars().count() == 1
            };
            if is(K::UpArrow) {
                Some("ArrowUp".to_owned())
            } else if is(K::DownArrow) {
                Some("ArrowDown".to_owned())
            } else if is(K::LeftArrow) {
                Some("ArrowLeft".to_owned())
            } else if is(K::RightArrow) {
                Some("ArrowRight".to_owned())
            } else if text.chars().count() == 1 {
                Some(text.to_lowercase())
            } else {
                None
            }
        };
        if let Some(n) = name {
            let hit = self.st.borrow_mut().retro.cheat.feed(&n);
            if hit {
                self.retro_overdrive();
            }
        }
        false
    }

    // ---------------------------------------------------------------- Code-Regen

    pub fn retro_rain_start(self: &Rc<Self>) {
        let win = self.win();
        let (w, h) = {
            let s = win.window().size();
            let sc = win.window().scale_factor();
            ((s.width as f32 / sc) as u32, (s.height as f32 / sc) as u32)
        };
        let font = lfx_retro_fx::rain::font_px(w as f32, 1.0).max(12);
        let now = self.retro_ms();
        {
            let mut st = self.st.borrow_mut();
            if st.retro.term_open || st.retro.boot.is_some() {
                return;
            }
            let seed = st.retro.seed ^ 0x5eed;
            let field = RainField::new(seed, w, h, font, false);
            let cols = field.columns();
            st.retro.rain = Some(RainRun {
                field,
                hist: vec![Vec::new(); cols],
                last_y: vec![-1.0; cols],
                next_frame_ms: now,
                flash: TitleFlash::new(seed ^ 1),
                flash_clear_ms: 0,
                font,
            });
        }
        let g = win.global::<RetroFx>();
        g.set_rain_font(font as f32);
        g.set_rain_cols(ModelRc::new(VecModel::from(Vec::<RainCol>::new())));
        g.set_rain_flash("".into());
        g.set_rain_show(true);
        self.retro_play(FxSound::Open);
    }

    pub fn retro_rain_stop(self: &Rc<Self>) {
        let was = self.st.borrow_mut().retro.rain.take().is_some();
        if was {
            let win = self.win();
            let g = win.global::<RetroFx>();
            g.set_rain_show(false);
            g.set_rain_flash("".into());
            let now = self.retro_ms();
            if let Some(i) = self.st.borrow_mut().retro.idle.as_mut() {
                i.note_input(now);
            }
        }
    }

    fn retro_rain_frame(self: &Rc<Self>, now: u64) {
        let titles: Vec<String> = {
            let st = self.st.borrow();
            st.home
                .rows
                .iter()
                .flat_map(|r| r.items.iter())
                .map(|i| i.name.to_uppercase())
                .take(20)
                .collect()
        };
        let (cols, flash) = {
            let mut st = self.st.borrow_mut();
            let Some(r) = st.retro.rain.as_mut() else {
                return;
            };
            if now < r.next_frame_ms {
                return;
            }
            r.next_frame_ms = now + 33;
            let cells = r.field.step();
            let n = r.field.columns();
            r.hist.resize(n, Vec::new());
            r.last_y.resize(n, -1.0);
            for c in cells {
                let i = c.col as usize;
                if i < n {
                    if c.y < r.last_y[i] {
                        r.hist[i].clear();
                    }
                    r.last_y[i] = c.y;
                    r.hist[i].push(c.head);
                    if r.hist[i].len() > TRAIL_LEN + 1 {
                        r.hist[i].remove(0);
                    }
                }
            }
            let font = r.font as f32;
            let mut out = Vec::new();
            for i in 0..n {
                if r.last_y[i] < 0.0 || r.hist[i].is_empty() {
                    continue;
                }
                let h = &r.hist[i];
                let head = h[h.len() - 1];
                let trail: Vec<String> = h[..h.len() - 1].iter().map(char::to_string).collect();
                out.push(RainCol {
                    x: i as f32 * font,
                    y: (r.last_y[i] + 1.0) * font,
                    head: head.to_string().into(),
                    trail: trail.join("\n").into(),
                });
            }
            let flash = r.flash.poll(now, &titles);
            if flash.is_some() {
                r.flash_clear_ms = now + 1800;
            }
            let clear = r.flash_clear_ms != 0 && now >= r.flash_clear_ms;
            if clear {
                r.flash_clear_ms = 0;
            }
            (
                out,
                flash.or(if clear { Some(String::new()) } else { None }),
            )
        };
        let win = self.win();
        let g = win.global::<RetroFx>();
        g.set_rain_cols(ModelRc::new(VecModel::from(cols)));
        if let Some(f) = flash {
            g.set_rain_flash(f.into());
        }
    }

    // ---------------------------------------------------------------- Zeitgeber

    fn retro_tick(self: &Rc<Self>) {
        if !self.st.borrow().retro.fx_active {
            return;
        }
        let now = self.retro_ms();
        let win = self.win();
        let g = win.global::<RetroFx>();

        // Uhr
        let (hh, mm) = local_hm();
        {
            let mut st = self.st.borrow_mut();
            if st.retro.last_clock != (hh, mm) {
                st.retro.last_clock = (hh, mm);
                g.set_clock_h(format!("{hh:02}").into());
                g.set_clock_m(format!("{mm:02}").into());
            }
        }
        g.set_clock_colon(now % 2000 < 1100);

        // Hero-Dateiname
        let hero_title = win.get_hero().title.to_string();
        let file = {
            let st = self.st.borrow();
            if st.retro.last_hero == hero_title {
                None
            } else {
                let series = st
                    .home
                    .pool
                    .get(st.home.hero_idx)
                    .is_some_and(|i| format!("{:?}", i.kind) == "Series");
                Some(format!(
                    "{}.{}",
                    dos_name(&hero_title),
                    if series { "SER" } else { "MOV" }
                ))
            }
        };
        if let Some(f) = file {
            self.st.borrow_mut().retro.last_hero = hero_title.clone();
            g.set_hero_file(if hero_title.is_empty() {
                "".into()
            } else {
                f.into()
            });
        }

        // Boot
        if self.st.borrow().retro.boot.is_some() {
            self.retro_boot_frame();
        }

        // CRT: Flackern und Rollbalken (nur bei Änderung schreiben)
        if self.st.borrow().retro.crt {
            let f = flicker_value(now);
            let fk = (f * 100.0) as i32;
            if self.st.borrow().retro.last_flicker != fk {
                self.st.borrow_mut().retro.last_flicker = fk;
                g.set_flicker(f);
            }
            let r = roll_value(now);
            let rk = if r < 0.0 { -1 } else { (r * 200.0) as i32 };
            if self.st.borrow().retro.last_roll != rk {
                self.st.borrow_mut().retro.last_roll = rk;
                g.set_roll(r);
            }
        }

        // Glitch-Streifen
        let fire = {
            let mut st = self.st.borrow_mut();
            let origin = st.retro.glitch_origin;
            let due = st
                .retro
                .glitch_next
                .is_some_and(|e| now >= origin + e.at_ms);
            if due {
                let ev = st.retro.glitch_next.take();
                if let Some(s) = st.retro.glitch.as_mut() {
                    st.retro.glitch_next = Some(s.next_event());
                }
                st.retro.strip_until = now + STRIP_MS;
                ev
            } else {
                None
            }
        };
        if let Some(ev) = fire {
            g.set_strip_y(0.15 + (ev.pick as f32) * 0.7);
            g.set_strip(true);
        } else if g.get_strip() && now >= self.st.borrow().retro.strip_until {
            g.set_strip(false);
        }

        // Overdrive
        let od = self.st.borrow().retro.od_start;
        if let Some(start) = od {
            let t = now - start;
            if t >= u64::from(OVERDRIVE_MS) {
                self.st.borrow_mut().retro.od_start = None;
                g.set_od_hue(-1.0);
                g.set_od_toast(false);
            } else {
                g.set_od_hue(((t % 6000) / 200 * 12) as f32);
                match od_toast_opacity(t) {
                    Some(o) => g.set_od_toast_opacity(o),
                    None => g.set_od_toast(false),
                }
            }
        }
        let shake = self.st.borrow().retro.shake_start;
        if let Some(s) = shake {
            let t = now - s;
            if t >= SHAKE_MS {
                self.st.borrow_mut().retro.shake_start = None;
                g.set_shake_x(0.0);
                g.set_shake_y(0.0);
            } else {
                let (x, y) = shake_offset(t);
                g.set_shake_x(x);
                g.set_shake_y(y);
            }
        }

        // Terminal-Cursor
        if self.st.borrow().retro.term_open && now >= self.st.borrow().retro.caret_ms {
            self.st.borrow_mut().retro.caret_ms = now + 500;
            g.set_term_caret(!g.get_term_caret());
        }

        // Leerlauf und Regen
        #[cfg(windows)]
        self.retro_poll_cursor(now);
        let start_rain = {
            let st = self.st.borrow();
            let blocked = st.retro.rain.is_some()
                || st.retro.term_open
                || st.retro.boot.is_some()
                || win.global::<lfx_ui::DetailState>().get_modal_open();
            st.retro
                .idle
                .as_ref()
                .is_some_and(|i| i.should_start(now, blocked))
        };
        if start_rain {
            self.retro_rain_start();
        }
        if self.st.borrow().retro.rain.is_some() {
            self.retro_rain_frame(now);
        }
    }

    #[cfg(windows)]
    fn retro_poll_cursor(self: &Rc<Self>, now: u64) {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut p = POINT::default();
        // SAFETY: Zeiger auf lokale Struktur.
        if unsafe { GetCursorPos(&mut p) }.is_ok() {
            let pos = (p.x as f32, p.y as f32);
            let mut st = self.st.borrow_mut();
            let r = &mut st.retro;
            if r.cursor != Some(pos) {
                r.cursor = Some(pos);
                if let Some(i) = r.idle.as_mut() {
                    i.note_move(pos.0, pos.1, now);
                }
            }
        }
    }

    /// Zustand für Tests.
    pub fn retro_state_json(&self) -> serde_json::Value {
        let st = self.st.borrow();
        let r = &st.retro;
        let win = self.win();
        let g = win.global::<RetroFx>();
        serde_json::json!({
            "active": r.fx_active,
            "booted": r.booted,
            "boot": r.boot.is_some(),
            "boot_lines": g.get_boot_lines().row_count(),
            "crt": r.crt,
            "sound": r.sound,
            "overdrive": r.od_start.is_some(),
            "term_open": r.term_open,
            "term_lines": r.term_lines.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
            "term_input": r.term_input,
            "rain": r.rain.is_some(),
            "rain_cols": g.get_rain_cols().row_count(),
            "hero_file": g.get_hero_file().to_string(),
            "clock": format!("{}:{}", g.get_clock_h(), g.get_clock_m()),
        })
    }

    /// Terminal-Zeilen in die Oberfläche schreiben.
    pub(crate) fn retro_push_lines(&self) {
        let lines: Vec<RetroLine> = self
            .st
            .borrow()
            .retro
            .term_lines
            .iter()
            .map(|(t, k)| RetroLine {
                text: t.as_str().into(),
                kind: *k,
            })
            .collect();
        let win = self.win();
        let g = win.global::<RetroFx>();
        g.set_term_lines(ModelRc::new(VecModel::from(lines)));
    }
}

struct RetroFxHandle(lfx_ui::MainWindow);
impl RetroFxHandle {
    fn g(&self) -> RetroFx<'_> {
        self.0.global::<RetroFx>()
    }
}

/// Ortszeit (Stunde, Minute).
fn local_hm() -> (u32, u32) {
    use chrono::Timelike;
    let n = chrono::Local::now();
    (n.hour(), n.minute())
}

/// Verdrahtet die Rückrufe des Globals.
pub fn wire(app: &Rc<App>, ui: &lfx_ui::MainWindow) {
    let g = ui.global::<RetroFx>();
    let a = app.clone();
    g.on_boot_skip(move || a.retro_boot_skip());
    let a = app.clone();
    g.on_term_close(move || a.retro_term_close());
    let a = app.clone();
    g.on_rain_stop(move || a.retro_rain_stop());
    let a = app.clone();
    g.on_switch(move |n| a.retro_toggle_pref(&n));
    let a = app.clone();
    g.on_logo_tap(move || a.retro_logo_tap());
    let a = app.clone();
    g.on_logo_long(move || {
        if a.st.borrow().retro.fx_active {
            a.retro_term_open();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_names_follow_the_web_rules() {
        assert_eq!(dos_name("Neon Läufer: Teil 2"), "NEON_LAEUFER_TEIL_2");
        assert_eq!(dos_name("   "), "UNBEKANNT");
        assert!(
            dos_name("Ein sehr sehr langer Titel der abgeschnitten wird")
                .chars()
                .count()
                <= 22
        );
        assert!(!dos_name("Abc ").ends_with('_'));
    }

    #[test]
    fn roll_and_flicker_are_periodic() {
        assert_eq!(roll_value(1000), -1.0);
        assert!(roll_value(3000) >= 0.0 && roll_value(3000) < 0.01);
        assert_eq!(roll_value(3000 + 5000), -1.0);
        assert_eq!(flicker_value(0), flicker_value(6300));
    }

    #[test]
    fn overdrive_toast_ends_after_3_6_seconds() {
        assert_eq!(od_toast_opacity(200), Some(1.0));
        assert_eq!(od_toast_opacity(3600), None);
    }
}
