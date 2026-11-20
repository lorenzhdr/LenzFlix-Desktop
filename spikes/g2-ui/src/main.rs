//! Spike G2: Startseiten-Attrappe mit Slint. Optionen: `--bench`, `--seconds N`, `--out DIR`,
//! `--shot DATEI.png`, `--design flix|plus|prime|retro`.

mod art;
#[cfg(feature = "skia")]
mod stl_shim;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, Image, Model, ModelRc, Timer, TimerMode, VecModel};

slint::include_modules!();

const ROWS: usize = 10;
const COUNT: usize = 20;
const BUFFER: usize = 2;
const WIDE: (f32, f32) = (240.0, 135.0);
const POSTER: (f32, f32) = (150.0, 225.0);
const ROW_PAD: f32 = 44.0 + 24.0;
const TITLEBAR: f32 = 40.0;

static DECODED: AtomicU64 = AtomicU64::new(0);
type Key = (usize, usize);

struct RowState {
    wide: bool,
    row_h: f32,
    scroll: f32,
    target: f32,
    cards: Rc<VecModel<CardData>>,
    first: usize,
    end: usize,
}

struct State {
    rows: Vec<RowState>,
    scrolls: Rc<VecModel<f32>>,
    images: HashMap<Key, Image>,
    wanted: Arc<Mutex<HashSet<Key>>>,
    jobs: Sender<(usize, usize)>,
    scroll_y: f32,
    target_y: f32,
    focus: (usize, usize),
    vw: f32,
    vh: f32,
    gutter: f32,
    gap: f32,
}

thread_local! {
    static STATE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

impl State {
    fn card_size(&self, ri: usize) -> (f32, f32) {
        if self.rows[ri].wide { WIDE } else { POSTER }
    }
    fn card_x(&self, ri: usize, idx: usize) -> f32 {
        self.gutter + idx as f32 * (self.card_size(ri).0 + self.gap)
    }
    fn max_x(&self, ri: usize) -> f32 {
        let w = self.card_size(ri).0;
        (2.0 * self.gutter + COUNT as f32 * (w + self.gap) - self.gap - self.vw).max(0.0)
    }
    fn max_y(&self) -> f32 {
        (self.rows.iter().map(|r| r.row_h).sum::<f32>() - (self.vh - TITLEBAR)).max(0.0)
    }
    fn row_top(&self, ri: usize) -> f32 {
        self.rows[..ri].iter().map(|r| r.row_h).sum()
    }

    fn wanted_range(&self, ri: usize) -> (usize, usize) {
        let (w, _) = self.card_size(ri);
        let step = w + self.gap;
        let s = self.rows[ri].scroll;
        let first = (((s - self.gutter) / step).floor() as i64 - BUFFER as i64).max(0) as usize;
        let last = ((((s + self.vw - self.gutter) / step).floor() as i64) + BUFFER as i64)
            .clamp(0, COUNT as i64 - 1) as usize;
        (first.min(COUNT - 1), last + 1)
    }

    fn card(&self, ri: usize, idx: usize) -> CardData {
        match self.images.get(&(ri, idx)) {
            Some(img) => CardData {
                idx: idx as i32,
                x: self.card_x(ri, idx),
                art: img.clone(),
                loaded: true,
            },
            None => CardData {
                idx: idx as i32,
                x: self.card_x(ri, idx),
                art: Image::default(),
                loaded: false,
            },
        }
    }

    fn drop_card(&mut self, ri: usize, idx: usize) {
        self.images.remove(&(ri, idx));
        self.wanted.lock().unwrap().remove(&(ri, idx));
    }

    fn add_card(&mut self, ri: usize, idx: usize) -> CardData {
        if !self.images.contains_key(&(ri, idx)) && self.wanted.lock().unwrap().insert((ri, idx)) {
            let _ = self.jobs.send((ri, idx));
        }
        self.card(ri, idx)
    }

    /// Hält das Kartenmodell der Zeile auf dem sichtbaren Bereich plus Puffer (inkrementell).
    fn update_range(&mut self, ri: usize) {
        let (nf, ne) = self.wanted_range(ri);
        let (f, e) = (self.rows[ri].first, self.rows[ri].end);
        if (nf, ne) == (f, e) {
            return;
        }
        let model = self.rows[ri].cards.clone();
        if e <= f || ne <= f || nf >= e {
            // leer oder disjunkt: neu aufbauen
            for i in f..e {
                self.drop_card(ri, i);
            }
            model.set_vec(Vec::new());
            for i in nf..ne {
                let c = self.add_card(ri, i);
                model.push(c);
            }
        } else {
            for i in f..nf {
                model.remove(0);
                self.drop_card(ri, i);
            }
            for i in ne..e {
                model.remove(model.row_count() - 1);
                self.drop_card(ri, i);
            }
            for i in (nf..f).rev() {
                let c = self.add_card(ri, i);
                model.insert(0, c);
            }
            for i in e..ne {
                let c = self.add_card(ri, i);
                model.push(c);
            }
        }
        self.rows[ri].first = nf;
        self.rows[ri].end = ne;
    }

    fn set_scroll(&mut self, ri: usize, x: f32) {
        let x = x.clamp(0.0, self.max_x(ri));
        self.rows[ri].scroll = x;
        self.scrolls.set_row_data(ri, x);
        self.update_range(ri);
    }
}

fn deliver(key: Key, buf: slint::SharedPixelBuffer<slint::Rgba8Pixel>) {
    DECODED.fetch_add(1, Ordering::Relaxed);
    STATE.with(|s| {
        let Some(st) = s.borrow().clone() else { return };
        let mut st = st.borrow_mut();
        if !st.wanted.lock().unwrap().contains(&key) {
            return;
        }
        let img = Image::from_rgba8(buf);
        st.images.insert(key, img);
        let (ri, idx) = key;
        let (f, e) = (st.rows[ri].first, st.rows[ri].end);
        if idx >= f && idx < e {
            let data = st.card(ri, idx);
            st.rows[ri].cards.set_row_data(idx - f, data);
        }
    });
}

fn spawn_workers(
    wanted: Arc<Mutex<HashSet<Key>>>,
    rx: std::sync::mpsc::Receiver<(usize, usize)>,
    wide: Vec<bool>,
) {
    let rx = Arc::new(Mutex::new(rx));
    let wide = Arc::new(wide);
    for _ in 0..3 {
        let (rx, wanted, wide) = (rx.clone(), wanted.clone(), wide.clone());
        std::thread::spawn(move || {
            loop {
                let job = rx.lock().unwrap().recv();
                let Ok((ri, idx)) = job else { return };
                if !wanted.lock().unwrap().contains(&(ri, idx)) {
                    continue;
                }
                let buf = art::render((ri * COUNT + idx + 1) as u32, wide[ri]);
                if slint::invoke_from_event_loop(move || deliver((ri, idx), buf)).is_err() {
                    return;
                }
            }
        });
    }
}

#[derive(Default)]
struct Args {
    bench: bool,
    seconds: f32,
    out: String,
    shot: Option<String>,
    design: Option<String>,
    variant: String,
}

fn parse_args() -> Args {
    let mut a = Args {
        seconds: 15.0,
        out: ".".into(),
        ..Default::default()
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--bench" => a.bench = true,
            "--seconds" => a.seconds = it.next().and_then(|v| v.parse().ok()).unwrap_or(15.0),
            "--out" => a.out = it.next().unwrap_or_else(|| ".".into()),
            "--shot" => a.shot = it.next(),
            "--design" => a.design = it.next(),
            "--variant" => a.variant = it.next().unwrap_or_default(),
            _ => {}
        }
    }
    a
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q).floor() as usize]
}

fn renderer_name() -> String {
    std::env::var("SLINT_BACKEND")
        .map(|v| v.trim_start_matches("winit-").to_string())
        .unwrap_or_else(|_| "default".into())
}

fn tri(phase: f32) -> f32 {
    1.0 - (2.0 * phase.fract() - 1.0).abs()
}

struct Stats {
    frames: usize,
    fps: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    over_20: usize,
}

fn stats(frames: &[Instant]) -> Stats {
    let skip = 30.min(frames.len());
    let frames = &frames[skip..];
    let mut d: Vec<f64> = frames
        .windows(2)
        .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
        .collect();
    let span = frames
        .last()
        .zip(frames.first())
        .map(|(l, f)| (*l - *f).as_secs_f64())
        .unwrap_or(0.0);
    let fps = if span > 0.0 {
        d.len() as f64 / span
    } else {
        0.0
    };
    let over_20 = d.iter().filter(|v| **v > 20.0).count();
    d.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Stats {
        frames: d.len(),
        fps,
        p50: percentile(&d, 0.5),
        p95: percentile(&d, 0.95),
        p99: percentile(&d, 0.99),
        max: d.last().copied().unwrap_or(0.0),
        over_20,
    }
}

fn stats_json(s: &Stats) -> String {
    format!(
        "{{ \"frames\": {}, \"fps\": {:.2}, \"p50_ms\": {:.3}, \"p95_ms\": {:.3}, \"p99_ms\": {:.3}, \"max_ms\": {:.3}, \"frames_over_20ms\": {} }}",
        s.frames, s.fps, s.p50, s.p95, s.p99, s.max, s.over_20
    )
}

/// Primär: Abstand der winit-`RedrawRequested`-Ereignisse (für alle Renderer verfügbar, enthält
/// Zeichnen + Present/VSync-Wartezeit). Zusätzlich, falls der Renderer es kann: Abstand der
/// `AfterRendering`-Meldungen des Rendering-Notifiers.
fn write_report(args: &Args, ui: &App, redraws: &[Instant], notifier: &[Instant]) -> bool {
    let p = stats(redraws);
    let n = stats(notifier);
    let pass = p.fps >= 85.0 && p.p95 <= 11.7;
    let size = ui.window().size();
    let name = renderer_name();
    let json = format!(
        "{{
  \"renderer\": \"{name}\",
  \"window_px\": [{}, {}],
  \"scale_factor\": {},
  \"seconds\": {},
  \"variant\": \"{}\",
  \"redraw_events\": {},
  \"notifier_after_rendering\": {},
  \"images_decoded\": {},
  \"pass_85fps_p95_11_7ms\": {pass}
}}
",
        size.width,
        size.height,
        ui.window().scale_factor(),
        args.seconds,
        args.variant,
        stats_json(&p),
        stats_json(&n),
        DECODED.load(Ordering::Relaxed),
    );
    let suffix = if args.variant.is_empty() {
        String::new()
    } else {
        format!("-{}", args.variant)
    };
    let path = format!("{}/g2-bench-{name}{suffix}.json", args.out);
    std::fs::write(&path, &json).expect("Bericht schreiben");
    println!("{json}");
    pass
}

fn main() {
    let args = parse_args();
    let ui = App::new().expect("Fenster anlegen");
    if let Some(d) = &args.design {
        ui.global::<Theme>().set_design(match d.as_str() {
            "plus" => Design::Plus,
            "prime" => Design::Prime,
            "retro" => Design::Retro,
            _ => Design::Flix,
        });
    }
    let t = ui.global::<Theme>().get_t();
    ui.global::<Nav>()
        .set_shadows(!args.variant.contains("noshadow"));
    ui.set_renderer_label(format!("Renderer: {}", renderer_name()).into());

    // Zeilen (gerade = 16:9, ungerade = 2:3) und Zustand
    let (tx, rx) = channel();
    let wanted: Arc<Mutex<HashSet<Key>>> = Arc::default();
    let wide: Vec<bool> = (0..ROWS).map(|r| r % 2 == 0).collect();
    spawn_workers(wanted.clone(), rx, wide.clone());
    let row_data = Rc::new(VecModel::<RowData>::default());
    let scrolls = Rc::new(VecModel::from(vec![0.0f32; ROWS]));
    let mut rows = Vec::new();
    for (ri, w) in wide.iter().enumerate() {
        let cards = Rc::new(VecModel::<CardData>::default());
        let row_h = if *w { WIDE.1 } else { POSTER.1 } + ROW_PAD;
        row_data.push(RowData {
            title: format!("Reihe {}", ri + 1).into(),
            wide: *w,
            row_h,
            cards: ModelRc::from(cards.clone()),
        });
        rows.push(RowState {
            wide: *w,
            row_h,
            scroll: 0.0,
            target: 0.0,
            cards,
            first: 0,
            end: 0,
        });
    }
    let st = Rc::new(RefCell::new(State {
        rows,
        scrolls: scrolls.clone(),
        images: HashMap::new(),
        wanted,
        jobs: tx,
        scroll_y: 0.0,
        target_y: 0.0,
        focus: (0, 0),
        vw: 1280.0,
        vh: 800.0,
        gutter: t.gutter,
        gap: t.card_gap,
    }));
    STATE.with(|s| *s.borrow_mut() = Some(st.clone()));
    {
        let mut s = st.borrow_mut();
        for ri in 0..ROWS {
            s.update_range(ri);
        }
    }
    ui.set_rows(ModelRc::from(row_data));
    ui.set_row_scrolls(ModelRc::from(scrolls));

    // Fensteraktionen (eigene Titelleiste)
    let weak = ui.as_weak();
    ui.on_minimize({
        let w = weak.clone();
        move || w.upgrade().unwrap().window().set_minimized(true)
    });
    let toggle = {
        let w = weak.clone();
        move || {
            let ui = w.upgrade().unwrap();
            let m = !ui.window().is_maximized();
            ui.window().set_maximized(m);
            ui.set_is_max(m);
        }
    };
    ui.on_toggle_max(toggle.clone());
    ui.on_close_window({
        let w = weak.clone();
        move || {
            let _ = w.upgrade().unwrap().hide();
        }
    });
    let last_down: Rc<RefCell<Option<Instant>>> = Rc::default();
    ui.on_drag({
        let w = weak.clone();
        move || {
            // Doppelklick selbst erkennen: drag_window schluckt das Loslassen.
            let now = Instant::now();
            let dbl = last_down
                .borrow()
                .is_some_and(|p| now - p < Duration::from_millis(400));
            if dbl {
                *last_down.borrow_mut() = None;
                toggle();
            } else {
                *last_down.borrow_mut() = Some(now);
                w.upgrade().unwrap().window().with_winit_window(|win| {
                    let _ = win.drag_window();
                });
            }
        }
    });
    ui.on_resize({
        let w = weak.clone();
        move |dir| {
            use winit::window::ResizeDirection as D;
            let d = [
                D::North,
                D::South,
                D::East,
                D::West,
                D::NorthEast,
                D::NorthWest,
                D::SouthEast,
                D::SouthWest,
            ][dir.clamp(0, 7) as usize];
            w.upgrade().unwrap().window().with_winit_window(|win| {
                let _ = win.drag_resize_window(d);
            });
        }
    });

    // Navigation
    ui.on_nav({
        let (st, w) = (st.clone(), weak.clone());
        move |dx, dy| {
            let ui = w.upgrade().unwrap();
            let mut s = st.borrow_mut();
            let (r, c) = s.focus;
            let r = (r as i32 + dy).clamp(0, ROWS as i32 - 1) as usize;
            let c = (c as i32 + dx).clamp(0, COUNT as i32 - 1) as usize;
            s.focus = (r, c);
            let nav = ui.global::<Nav>();
            nav.set_frow(r as i32);
            nav.set_fcol(c as i32);
            nav.set_kbd(true);
            let (cw, _) = s.card_size(r);
            let x = s.card_x(r, c);
            let (g, vw) = (s.gutter, s.vw);
            let cur = s.rows[r].target;
            let t = if x - cur < g {
                x - g
            } else if x + cw - cur > vw - g {
                x + cw - vw + g
            } else {
                cur
            };
            s.rows[r].target = t.clamp(0.0, s.max_x(r));
            let (top, h) = (s.row_top(r), s.rows[r].row_h);
            let view = s.vh - TITLEBAR;
            if top < s.target_y {
                s.target_y = top;
            } else if top + h > s.target_y + view {
                s.target_y = top + h - view;
            }
            s.target_y = s.target_y.clamp(0.0, s.max_y());
        }
    });
    ui.on_card_clicked({
        let (st, w) = (st.clone(), weak.clone());
        move |r, c| {
            let ui = w.upgrade().unwrap();
            st.borrow_mut().focus = (r as usize, c as usize);
            let nav = ui.global::<Nav>();
            nav.set_frow(r);
            nav.set_fcol(c);
            nav.set_kbd(false);
        }
    });
    ui.on_wheel({
        let st = st.clone();
        move |r, dx, dy| {
            let mut s = st.borrow_mut();
            let r = r as usize;
            if dx != 0.0 {
                s.rows[r].target = (s.rows[r].target - dx).clamp(0.0, s.max_x(r));
            }
            if dy != 0.0 {
                s.target_y = (s.target_y - dy).clamp(0.0, s.max_y());
            }
        }
    });

    // Frametime-Messung über den Rendering-Notifier
    let frames: Rc<RefCell<Vec<Instant>>> = Rc::default();
    let redraws: Rc<RefCell<Vec<Instant>>> = Rc::default();
    let recording = Rc::new(std::cell::Cell::new(false));
    let notifier_ok = {
        let (frames, recording) = (frames.clone(), recording.clone());
        ui.window().set_rendering_notifier(move |state, _| {
            if matches!(state, slint::RenderingState::AfterRendering) && recording.get() {
                frames.borrow_mut().push(Instant::now());
            }
        })
    };
    {
        let (redraws, recording) = (redraws.clone(), recording.clone());
        ui.window().on_winit_window_event(move |_, ev| {
            if matches!(ev, winit::event::WindowEvent::RedrawRequested) && recording.get() {
                redraws.borrow_mut().push(Instant::now());
            }
            slint::winit_030::EventResult::Propagate
        });
    }
    if let Err(e) = &notifier_ok {
        eprintln!("Rendering-Notifier nicht verfügbar: {e}");
    }

    // Takt: Größe, Glättung, Bench-Fahrplan
    let started = Instant::now();
    let bench_start: Rc<std::cell::Cell<Option<Instant>>> = Rc::default();
    let maxed = Rc::new(std::cell::Cell::new(false));
    let shot_done = Rc::new(std::cell::Cell::new(false));
    let last_tick = Rc::new(std::cell::Cell::new(Instant::now()));
    let max_tick = Rc::new(std::cell::Cell::new(0.0f64));
    let timer = Timer::default();
    {
        let (st, w, frames, recording, redraws) = (
            st.clone(),
            weak.clone(),
            frames.clone(),
            recording.clone(),
            redraws.clone(),
        );
        let variant = args.variant.clone();
        let args_seconds = args.seconds;
        let is_bench = args.bench;
        let shot = args.shot.clone();
        let out = args.out.clone();
        let (max_tick, maxed) = (max_tick.clone(), maxed.clone());
        timer.start(TimerMode::Repeated, Duration::from_millis(4), move || {
            let t0 = Instant::now();
            let ui = w.upgrade().unwrap();
            let dt = (t0 - last_tick.get()).as_secs_f32();
            last_tick.set(t0);
            let mut s = st.borrow_mut();
            let size = ui.window().size();
            let sf = ui.window().scale_factor();
            let (vw, vh) = (size.width as f32 / sf, size.height as f32 / sf);
            if (vw - s.vw).abs() > 0.5 || (vh - s.vh).abs() > 0.5 {
                s.vw = vw;
                s.vh = vh;
                for ri in 0..ROWS {
                    let x = s.rows[ri].scroll;
                    s.set_scroll(ri, x);
                }
            }
            ui.set_is_max(ui.window().is_maximized());

            if is_bench && !maxed.get() && started.elapsed().as_secs_f32() > 0.4 {
                maxed.set(true);
                ui.window().set_maximized(true);
            }
            let bs = bench_start.get();
            if let Some(bs) = bs {
                let t = bs.elapsed().as_secs_f32();
                if t >= args_seconds {
                    recording.set(false);
                    bench_start.set(None);
                    let rep = Args {
                        seconds: args_seconds,
                        out: out.clone(),
                        variant: variant.clone(),
                        ..Default::default()
                    };
                    write_report(&rep, &ui, &redraws.borrow(), &frames.borrow());
                    let _ = slint::quit_event_loop();
                    return;
                }
                let y = if variant.contains("still") {
                    0.0
                } else {
                    s.max_y() * tri(t / 10.0)
                };
                s.scroll_y = y;
                ui.set_scroll_y(y);
                for ri in 0..ROWS {
                    let m = s.max_x(ri);
                    if m <= 0.0 || variant.contains("still") || variant.contains("yonly") {
                        continue;
                    }
                    let phase = t * 700.0 / (2.0 * m) + ri as f32 * 0.07;
                    let x = m * tri(phase);
                    s.set_scroll(ri, if ri % 2 == 0 { x } else { m - x });
                }
                // Fokus-Zoom wechselt alle 0,6 s
                let k = (t / 0.6) as usize;
                let r0 = (0..ROWS)
                    .find(|r| s.row_top(*r) + s.rows[*r].row_h > y)
                    .unwrap_or(0);
                let r = (r0 + k % 3).min(ROWS - 1);
                let c = (s.rows[r].first + BUFFER + k % 3).min(COUNT - 1);
                let nav = ui.global::<Nav>();
                if !variant.contains("nofocus") {
                    nav.set_frow(r as i32);
                    nav.set_fcol(c as i32);
                    nav.set_kbd(k.is_multiple_of(2));
                }
            } else {
                if is_bench && started.elapsed().as_secs_f32() >= 3.0 {
                    bench_start.set(Some(Instant::now()));
                    recording.set(true);
                }
                // Glättung der Tastatur-/Mausrad-Scrolls
                let k = 1.0 - (-dt / 0.07).exp();
                let ty = s.target_y;
                if (ty - s.scroll_y).abs() > 0.3 {
                    s.scroll_y += (ty - s.scroll_y) * k;
                    ui.set_scroll_y(s.scroll_y);
                } else if s.scroll_y != ty {
                    s.scroll_y = ty;
                    ui.set_scroll_y(ty);
                }
                for ri in 0..ROWS {
                    let (cur, tg) = (s.rows[ri].scroll, s.rows[ri].target);
                    if (tg - cur).abs() > 0.3 {
                        s.set_scroll(ri, cur + (tg - cur) * k);
                    } else if cur != tg {
                        s.set_scroll(ri, tg);
                    }
                }
                if let Some(path) = &shot {
                    let el = started.elapsed().as_secs_f32();
                    if el > 1.5 && !shot_done.get() && el < 2.0 {
                        let nav = ui.global::<Nav>();
                        nav.set_frow(1);
                        nav.set_fcol(2);
                        nav.set_kbd(true);
                    }
                    if el > 3.2 && !shot_done.get() {
                        shot_done.set(true);
                        match ui.window().take_snapshot() {
                            Ok(buf) => {
                                let file = std::fs::File::create(path).expect("Bilddatei");
                                let mut enc = png::Encoder::new(
                                    std::io::BufWriter::new(file),
                                    buf.width(),
                                    buf.height(),
                                );
                                enc.set_color(png::ColorType::Rgba);
                                enc.set_depth(png::BitDepth::Eight);
                                let mut wr = enc.write_header().expect("PNG-Kopf");
                                wr.write_image_data(buf.as_bytes()).expect("PNG-Daten");
                                println!(
                                    "Schnappschuss {}x{} nach {path}",
                                    buf.width(),
                                    buf.height()
                                );
                            }
                            Err(e) => eprintln!("Schnappschuss fehlgeschlagen: {e}"),
                        }
                        let _ = slint::quit_event_loop();
                    }
                }
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            if ms > max_tick.get() {
                max_tick.set(ms);
            }
        });
    }
    ui.run().expect("Ereignisschleife");
    println!("Längster Takt (UI-Thread): {:.2} ms", max_tick.get());
}
