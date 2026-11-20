//! Demo und Messlauf der Shell.
//!
//! Aufruf (im Repo-Wurzelordner):
//!   cargo run -p lfx-shell --example shell_demo --release -- [--bench] [--software] [--seconds N]
//!     [--billboard] [--out <datei.json>] [--hdr]
//!
//! Normalmodus: rahmenloses Fenster mit Slint-Titelleiste (Minimieren, Maximieren mit Snap-Flyout,
//! Schließen, Ziehen), halbtransparentem Overlay über dem Video-Layer (Testmuster per D3D11).
//! Taste B oder Knopf wechselt das Billboard-Rechteck.
//! `--bench`: 15 s Scroll-/Hover-Animation in einer Slint-Liste, Bildzeiten als JSON.

use std::cell::RefCell;
use std::rc::Rc;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lfx_shell::{HitRegions, Rect, Shell, ShellHandle, ShellOptions, UiBackend, UiBackendPref};

slint::slint! {
    #[edition = "1.19"]
    component Card inherits Rectangle {
        in property <int> idx;
        in property <bool> big;
        in property <bool> hot;
        width: big ? 240px : 150px;
        height: big ? 135px : 225px;
        border-radius: 8px;
        background: @linear-gradient(135deg, Colors.hsv(mod(idx * 37, 360), 0.55, 0.60, 1.0), Colors.hsv(mod(idx * 37 + 60, 360), 0.65, 0.35, 1.0));
        Text {
            text: "Film " + root.idx;
            color: white;
            font-size: 15px;
            font-weight: 600;
        }
        // Hover-/Fokus-Effekt nur für die eine betroffene Karte (eigene Ebene mit Skalierung und Schatten).
        if hot: Rectangle {
            border-radius: 8px;
            background: @linear-gradient(135deg, Colors.hsv(mod(idx * 37, 360), 0.65, 0.75, 1.0), Colors.hsv(mod(idx * 37 + 60, 360), 0.75, 0.45, 1.0));
            transform-scale: 1.08;
            drop-shadow-blur: 24px;
            drop-shadow-color: #000000b0;
            drop-shadow-offset-y: 8px;
            Text {
                text: "Film " + root.idx;
                color: white;
                font-size: 15px;
                font-weight: 700;
            }
        }
    }

    component TitleButton inherits Rectangle {
        in property <string> glyph;
        in property <color> hover-color: #ffffff22;
        callback clicked;
        width: 46px;
        background: ta.has-hover ? hover-color : transparent;
        Text { text: glyph; color: white; font-size: 14px; }
        ta := TouchArea { clicked => { root.clicked(); } }
    }

    export component Demo inherits Window {
        no-frame: true;
        background: transparent;
        preferred-width: 1280px;
        preferred-height: 800px;
        in property <bool> bench;
        in property <bool> is-max;
        in property <bool> billboard;
        in property <float> t;
        in property <string> info;
        in property <string> note;
        callback minimize();
        callback toggle-max();
        callback request-close();
        callback toggle-billboard();
        callback layout-changed(length, length);
        changed width => { root.layout-changed(root.width, root.height); }
        changed height => { root.layout-changed(root.width, root.height); }

        fs := FocusScope {
            key-pressed(e) => {
                if (e.text == "b" || e.text == "B") { root.toggle-billboard(); return accept; }
                reject
            }

            // Inhalt unter der Titelleiste
            Rectangle {
                y: 40px;
                height: parent.height - 40px;
                background: root.bench ? #0d0d12 : transparent;
                clip: true;

                if root.bench: Rectangle {
                    for row in 10: Rectangle {
                        property <bool> even: mod(row, 2) == 0;
                        property <int> slots: even ? 8 : 11;
                        property <length> stride: even ? 250px : 160px;
                        property <length> total: slots * stride;
                        y: (floor(row / 2) * 410px + mod(row, 2) * 160px) - (1 - abs(mod(root.t, 10) / 5 - 1)) * 1100px;
                        for i in (even ? 8 : 11): Card {
                            idx: row * 20 + i;
                            big: even;
                            hot: row == mod(floor(root.t / 0.6), 10) && i == mod(floor(root.t / 0.6), 6) + 2;
                            x: mod(i * stride - (even ? 1 : -1) * root.t * 700px, total) - stride + 40px;
                        }
                    }
                }

                if !root.bench: Rectangle {
                    // Halbtransparentes Overlay über dem Video
                    Rectangle {
                        x: 24px; y: 20px; width: 420px; height: 150px;
                        background: #000000a8; border-radius: 12px;
                        Text {
                            x: 18px; y: 14px;
                            text: "LENZFLIX Shell-Demo\n" + root.info + "\n" + root.note + "\nTaste B: Billboard umschalten";
                            color: white; font-size: 14px;
                        }
                    }
                    Rectangle {
                        x: 0px; y: parent.height - 72px; width: parent.width; height: 72px;
                        background: #000000c0;
                        Rectangle {
                            x: 24px; y: 30px; width: parent.width - 48px; height: 6px;
                            background: #ffffff30; border-radius: 3px;
                            Rectangle {
                                width: parent.width * mod(root.t / 20, 1.0);
                                background: #e5a00d; border-radius: 3px;
                            }
                        }
                    }
                }
            }

            // Titelleiste
            Rectangle {
                x: 0px; y: 0px; width: parent.width; height: 40px;
                background: #14141ce8;
                Text { x: 16px; y: 0px; height: 40px; text: "LENZFLIX"; color: white; font-size: 14px; font-weight: 700; vertical-alignment: center; }
                Rectangle {
                    x: parent.width - 3 * 46px; y: 0px;
                    width: 3 * 46px; height: 40px;
                    TitleButton { x: 0px; y: 0px; height: 40px; glyph: "–"; clicked => { root.minimize(); } }
                    TitleButton { x: 46px; y: 0px; height: 40px; glyph: root.is-max ? "❐" : "☐"; clicked => { root.toggle-max(); } }
                    TitleButton { x: 92px; y: 0px; height: 40px; glyph: "✕"; hover-color: #c42b1c; clicked => { root.request-close(); } }
                }
            }
        }
    }
}

const TITLE_H: f32 = 40.0;
const BTN_W: f32 = 46.0;

struct Args {
    bench: bool,
    software: bool,
    billboard: bool,
    seconds: Option<f64>,
    out: Option<String>,
    hdr: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        bench: false,
        software: false,
        billboard: false,
        seconds: None,
        out: None,
        hdr: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--bench" => a.bench = true,
            "--software" => a.software = true,
            "--billboard" => a.billboard = true,
            "--hdr" => a.hdr = true,
            "--seconds" => a.seconds = it.next().and_then(|v| v.parse().ok()),
            "--out" => a.out = it.next(),
            other => eprintln!("unbekanntes Argument: {other}"),
        }
    }
    a
}

/// Trefferflächen aus dem festen Layout der Demo-Titelleiste.
fn regions_for(width: f32) -> HitRegions {
    HitRegions {
        caption: vec![Rect::new(0.0, 0.0, width, TITLE_H)],
        interactive: vec![
            Rect::new(width - 3.0 * BTN_W, 0.0, BTN_W, TITLE_H),
            Rect::new(width - BTN_W, 0.0, BTN_W, TITLE_H),
        ],
        maximize_button: Some(Rect::new(width - 2.0 * BTN_W, 0.0, BTN_W, TITLE_H)),
        ..HitRegions::default()
    }
}

fn billboard_rect(w: f32, h: f32) -> Rect {
    Rect::new(
        w * 0.35,
        TITLE_H + (h - TITLE_H) * 0.08,
        w * 0.60,
        (h - TITLE_H) * 0.5,
    )
}

#[cfg(windows)]
mod pattern {
    //! Testmuster per D3D11: wandernde Farbstreifen, gefüllt in einem eigenen Thread.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use lfx_shell::VideoLayer;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::core::Interface;

    fn hsv(h: f32, s: f32, v: f32) -> [f32; 4] {
        let c = v * s;
        let hp = (h.rem_euclid(360.0)) / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        [r + m, g + m, b + m, 1.0]
    }

    pub fn spawn(layer: VideoLayer, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<u64> {
        std::thread::spawn(move || {
            // SAFETY: der Zeiger gehört dem Video-Layer und bleibt gültig, solange `layer` lebt.
            let raw = layer.d3d11_device_raw();
            let dev: ID3D11Device = unsafe { ID3D11Device::from_raw_borrowed(&raw) }
                .expect("D3D11-Gerät")
                .clone();
            let ctx: ID3D11DeviceContext = unsafe { dev.GetImmediateContext() }.expect("Kontext");
            let ctx1: ID3D11DeviceContext1 = ctx.cast().expect("ID3D11DeviceContext1");
            let start = std::time::Instant::now();
            let mut frames = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let Some(target) = layer.acquire_target() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                // SAFETY: `texture_raw` ist der ausgeliehene Backbuffer bis `present()`.
                unsafe {
                    let Some(tex) = ID3D11Texture2D::from_raw_borrowed(&target.texture_raw) else {
                        continue;
                    };
                    let mut rtv: Option<ID3D11RenderTargetView> = None;
                    if dev
                        .CreateRenderTargetView(tex, None, Some(&mut rtv))
                        .is_ok()
                        && let Some(rtv) = rtv
                    {
                        let t = start.elapsed().as_secs_f32();
                        let n = 48u32;
                        let w = target.width.div_ceil(n);
                        for i in 0..n {
                            let color = hsv(i as f32 * 360.0 / n as f32 + t * 60.0, 0.7, 0.85);
                            let rect = RECT {
                                left: (i * w) as i32,
                                top: 0,
                                right: ((i + 1) * w).min(target.width) as i32,
                                bottom: target.height as i32,
                            };
                            ctx1.ClearView(&rtv, &color, Some(&[rect]));
                        }
                        // Weißer Balken als Bewegungsmarke.
                        let x = ((t * 400.0) as u32 % target.width.max(1)) as i32;
                        let bar = RECT {
                            left: x,
                            top: 0,
                            right: (x + 12).min(target.width as i32),
                            bottom: target.height as i32,
                        };
                        ctx1.ClearView(&rtv, &[1.0, 1.0, 1.0, 1.0], Some(&[bar]));
                    }
                }
                layer.present();
                frames += 1;
            }
            frames
        })
    }
}

fn json_report(
    path: &str,
    handle: &ShellHandle,
    backend: UiBackend,
    s: &lfx_shell::FrameSummary,
    secs: f64,
    verdict: bool,
) {
    let ((w, h), scale) = handle.window_info();
    let hz = handle.refresh_rate_mhz().map_or(0.0, |m| m as f64 / 1000.0);
    let text = format!(
        "{{\n  \"ui_backend\": \"{backend:?}\",\n  \"note\": \"{}\",\n  \"window_px\": [{w}, {h}],\n  \"dpr\": {scale},\n  \"refresh_hz\": {hz:.1},\n  \"seconds\": {secs:.1},\n  \"frames\": {},\n  \"fps\": {:.2},\n  \"p50_ms\": {:.2},\n  \"p95_ms\": {:.2},\n  \"p99_ms\": {:.2},\n  \"max_ms\": {:.2},\n  \"frames_over_20ms\": {},\n  \"target_fps_85_p95_11_7\": {verdict}\n}}\n",
        handle.ui_backend_note().replace('"', "'"),
        s.frames,
        s.fps,
        s.p50_ms,
        s.p95_ms,
        s.p99_ms,
        s.max_ms,
        s.over_20ms,
    );
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("JSON konnte nicht geschrieben werden: {e}");
    }
}

fn main() {
    let args = parse_args();
    let opts = ShellOptions {
        maximized: args.bench,
        ui_backend: if args.software {
            UiBackendPref::Software
        } else {
            UiBackendPref::Auto
        },
        ..ShellOptions::default()
    };
    let shell = Shell::new(opts).expect("Shell");
    let handle = shell.handle();
    let ui = Demo::new().expect("UI");
    shell.attach(&ui);
    println!(
        "UI-Pfad: {:?} ({})",
        handle.ui_backend(),
        handle.ui_backend_note()
    );

    ui.set_bench(args.bench);
    ui.set_billboard(args.billboard);
    ui.set_note(handle.ui_backend_note().into());

    let state = Rc::new(RefCell::new(args.billboard));
    let video = handle.video_layer();

    // Video-Layer: Testmuster-Thread (nicht im Bench, dort misst nur die UI).
    #[cfg(windows)]
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(windows)]
    let mut video_thread = None;
    #[cfg(windows)]
    if let Some(layer) = &video {
        if args.hdr {
            let info = layer.output_hdr_info();
            println!("Ausgabe-HDR: {info:?}");
            // Nur API-Aufrufe, kein Sichttest: Format umschalten und Metadaten setzen.
            layer.set_format(lfx_shell::VideoFormat::Rgb10A2Pq);
            layer.set_hdr_metadata(lfx_shell::HdrMetadata::bt2020_1000());
        }
        if args.bench {
            layer.set_visible(false);
        } else {
            video_thread = Some(pattern::spawn(layer.clone(), stop.clone()));
        }
    }

    {
        let h = handle.clone();
        ui.on_minimize(move || h.minimize());
    }
    {
        let h = handle.clone();
        let weak = ui.as_weak();
        ui.on_toggle_max(move || {
            h.toggle_maximize();
            if let Some(u) = weak.upgrade() {
                u.set_is_max(h.is_maximized());
            }
        });
    }
    {
        let h = handle.clone();
        ui.on_request_close(move || h.close());
    }
    {
        let h = handle.clone();
        let video = video.clone();
        let state = state.clone();
        let weak = ui.as_weak();
        ui.on_layout_changed(move |w, hgt| {
            h.set_hit_regions(regions_for(w));
            if let Some(layer) = &video
                && !args.bench
            {
                layer.set_rect(state.borrow().then(|| billboard_rect(w, hgt)));
            }
            if let Some(u) = weak.upgrade() {
                u.set_is_max(h.is_maximized());
            }
        });
    }
    {
        let video = video.clone();
        let state = state.clone();
        let weak = ui.as_weak();
        let h = handle.clone();
        ui.on_toggle_billboard(move || {
            let on = !*state.borrow();
            *state.borrow_mut() = on;
            if let Some(u) = weak.upgrade() {
                u.set_billboard(on);
                if let Some(layer) = &video {
                    let ((pw, ph), k) = h.window_info();
                    let (w, hgt) = (pw as f32 / k as f32, ph as f32 / k as f32);
                    layer.set_rect(on.then(|| billboard_rect(w, hgt)));
                }
            }
        });
    }

    // Animationstakt (Slint-Timer): treibt `t`, Bildtakt bleibt der vsync des Present.
    let t0 = Instant::now();
    let timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let h = handle.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(4),
            move || {
                if let Some(u) = weak.upgrade() {
                    u.set_t(t0.elapsed().as_secs_f32());
                    if !args.bench {
                        let ((w, hgt), k) = h.window_info();
                        u.set_info(format!("{w}x{hgt} px, Skalierung {k:.2}").into());
                    }
                }
            },
        );
    }

    // Selbsttest der Hit-Test-Codes über echte WM_NCHITTEST-Nachrichten (Windows).
    #[cfg(windows)]
    let probe_timer = slint::Timer::default();
    #[cfg(windows)]
    {
        let h = handle.clone();
        probe_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(1500),
            move || {
                let ((pw, ph), k) = h.window_info();
                let (w, hh) = (pw as f32 / k as f32, ph as f32 / k as f32);
                let pts = [
                    ("Ziehfläche", 200.0, 20.0),
                    ("Maximieren-Knopf", w - 69.0, 20.0),
                    ("Schließen-Knopf", w - 23.0, 20.0),
                    ("linker Rand", 2.0, hh / 2.0),
                    ("rechter unterer Eckgriff", w - 2.0, hh - 2.0),
                    ("Inhalt", w / 2.0, hh / 2.0),
                ];
                for (name, x, y) in pts {
                    println!(
                        "Hit-Test {name} ({x:.0},{y:.0}) -> {}",
                        h.probe_hit_code(x, y)
                    );
                }
            },
        );
    }

    // Messlauf.
    let result: Rc<RefCell<Option<lfx_shell::FrameSummary>>> = Rc::new(RefCell::new(None));
    let bench_timer = slint::Timer::default();
    let bench_secs = 15.0;
    if args.bench {
        let h = handle.clone();
        let res = result.clone();
        bench_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(3), move || {
            h.start_frame_recording(30);
            let h2 = h.clone();
            let res2 = res.clone();
            slint::Timer::single_shot(Duration::from_secs_f64(bench_secs), move || {
                *res2.borrow_mut() = Some(h2.stop_frame_recording());
                #[cfg(windows)]
                {
                    let p = h2.last_recording_parts();
                    println!(
                        "Teilzeiten p50/p95/max ms: holen {:.2}/{:.2}/{:.2}, rendern {:.2}/{:.2}/{:.2}, praesentieren {:.2}/{:.2}/{:.2}",
                        p[0].p50_ms, p[0].p95_ms, p[0].max_ms, p[1].p50_ms, p[1].p95_ms, p[1].max_ms, p[2].p50_ms, p[2].p95_ms, p[2].max_ms
                    );
                }
                let _ = slint::quit_event_loop();
            });
        });
    }
    let quit_timer = slint::Timer::default();
    if let (Some(s), false) = (args.seconds, args.bench) {
        quit_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs_f64(s),
            || {
                let _ = slint::quit_event_loop();
            },
        );
    }

    ui.show().expect("anzeigen");
    shell.run().expect("Ereignisschleife");

    #[cfg(windows)]
    {
        stop.store(true, Ordering::Relaxed);
        if let Some(t) = video_thread {
            println!("Video-Testmuster: {} Bilder", t.join().unwrap_or(0));
        }
    }
    if let Some(s) = *result.borrow() {
        let verdict = s.fps >= 85.0 && s.p95_ms <= 11.7;
        println!("Bench: {s:?}");
        println!(
            "Ziel (>= 85 fps, p95 <= 11,7 ms): {}",
            if verdict { "erreicht" } else { "verfehlt" }
        );
        if let Some(path) = &args.out {
            json_report(path, &handle, handle.ui_backend(), &s, bench_secs, verdict);
        }
    }
}
