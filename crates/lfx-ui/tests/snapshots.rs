//! Snapshot-Tests der Komponenten-Galerie mit dem Slint-Software-Renderer (ohne Fenster, fester Takt).
//!
//! - Szenen (`Gallery.scene`) 0..4 in allen vier Designs bei 1280×720 logisch, DPR 1; DPR 2 für ausgewählte
//!   Szenen von LENZFLIX. Alle Inhalte sind synthetisch (`lfx_ui::demo`).
//! - Referenzen liegen unter `tests/snapshots/*.png`. `LFX_UPDATE_SNAPSHOTS=1` schreibt sie neu.
//! - Vergleich mit Toleranz: ein Pixel zählt als abweichend, wenn ein Kanal um mehr als `CHANNEL_TOL` differiert;
//!   erlaubt ist ein Anteil von `MAX_DIFF_FRACTION` (Glyph-Rasterung kann je CPU-Architektur minimal abweichen).
//! - Auf Windows wird verglichen. Auf anderen Systemen (macOS-CI, Schriftdarstellung ungeprüft) wird nur gerendert
//!   und auf „nicht leer / nicht abgestürzt“ geprüft, außer `LFX_SNAPSHOT_COMPARE=1` erzwingt den Vergleich.
//! - Die Uhr der Plattform steht still (Animationen eingefroren), damit Bilder reproduzierbar sind.

use lfx_ui::{Design, Gallery, Theme, demo};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Rgb8Pixel};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const STAGE: (u32, u32) = (1280, 720);
const CHANNEL_TOL: i32 = 12;
const MAX_DIFF_FRACTION: f64 = 0.003;

thread_local! {
    static WINDOW: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct TestPlatform;

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
        Ok(window)
    }

    fn duration_since_start(&self) -> Duration {
        Duration::ZERO
    }
}

fn init_platform() {
    // Die Plattform ist in Slint je Thread gespeichert (Fenster sind threadgebunden): pro Test-Thread setzen.
    thread_local! { static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if !DONE.with(|d| d.replace(true)) {
        slint::platform::set_platform(Box::new(TestPlatform)).expect("Plattform setzen");
    }
}

/// Gerendertes Bild (RGB8, Zeile für Zeile).
struct Shot {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

fn render(design: Design, scene: i32, scale: u32) -> Shot {
    init_platform();
    let ui = Gallery::new().expect("Galerie laden");
    ui.global::<Theme>().set_design(design);
    ui.set_scene(scene);
    ui.set_frozen(true);
    demo::fill_gallery(&ui);

    // ui.window() erzwingt das Anlegen des Fensters über die Plattform
    let _ = ui.window().scale_factor();
    let window = WINDOW.with(|w| w.borrow().clone().expect("Fenster"));
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale as f32,
    });
    let (w, h) = (STAGE.0 * scale, STAGE.1 * scale);
    window.set_size(PhysicalSize::new(w, h));
    ui.show().expect("anzeigen");
    slint::platform::update_timers_and_animations();

    let mut buf = vec![Rgb8Pixel::default(); (w * h) as usize];
    let drawn = window.draw_if_needed(|renderer| {
        renderer.render(&mut buf, w as usize);
    });
    assert!(drawn, "nichts gezeichnet");
    let rgb = buf.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    ui.hide().expect("ausblenden");
    Shot {
        width: w,
        height: h,
        rgb,
    }
}

fn design_name(d: Design) -> &'static str {
    match d {
        Design::Flix => "flix",
        Design::Plus => "plus",
        Design::Prime => "prime",
        Design::Retro => "retro",
    }
}

const SCENES: [&str; 5] = ["buttons", "cards", "rows", "chrome", "overlays"];

fn snap_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn write_png(path: &Path, shot: &Shot) {
    std::fs::create_dir_all(path.parent().expect("Elternordner")).expect("Ordner anlegen");
    let file = std::fs::File::create(path).expect("PNG anlegen");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), shot.width, shot.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Best);
    enc.write_header()
        .expect("Header")
        .write_image_data(&shot.rgb)
        .expect("Daten");
}

fn read_png(path: &Path) -> Option<Shot> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .ok()?;
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).ok()?;
    assert_eq!(
        info.color_type,
        png::ColorType::Rgb,
        "Referenz muss RGB sein: {}",
        path.display()
    );
    data.truncate(info.buffer_size());
    Some(Shot {
        width: info.width,
        height: info.height,
        rgb: data,
    })
}

/// Anteil abweichender Pixel (Kanal-Differenz über der Toleranz).
fn diff_fraction(a: &Shot, b: &Shot) -> f64 {
    let bad = a
        .rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(b.rgb.as_chunks::<3>().0.iter())
        .filter(|(p, q)| {
            p.iter()
                .zip(q.iter())
                .any(|(x, y)| (i32::from(*x) - i32::from(*y)).abs() > CHANNEL_TOL)
        })
        .count();
    bad as f64 / (a.width as f64 * a.height as f64)
}

fn is_blank(shot: &Shot) -> bool {
    let first = &[shot.rgb[0], shot.rgb[1], shot.rgb[2]];
    shot.rgb.as_chunks::<3>().0.iter().all(|p| p == first)
}

fn compare_enabled() -> bool {
    cfg!(windows) || std::env::var_os("LFX_SNAPSHOT_COMPARE").is_some()
}

/// Prüft eine Szene gegen ihre Referenz; liefert eine Fehlermeldung oder `None`.
fn check(name: &str, shot: &Shot) -> Option<String> {
    if is_blank(shot) {
        return Some(format!("{name}: Bild ist einfarbig (nichts gerendert)"));
    }
    let reference = snap_dir().join(format!("{name}.png"));
    if std::env::var_os("LFX_UPDATE_SNAPSHOTS").is_some() {
        write_png(&reference, shot);
        return None;
    }
    if !compare_enabled() {
        return None;
    }
    let Some(expected) = read_png(&reference) else {
        return Some(format!(
            "{name}: Referenz fehlt ({}); mit LFX_UPDATE_SNAPSHOTS=1 erzeugen",
            reference.display()
        ));
    };
    if (expected.width, expected.height) != (shot.width, shot.height) {
        return Some(format!(
            "{name}: Größe {}x{} statt {}x{}",
            shot.width, shot.height, expected.width, expected.height
        ));
    }
    let frac = diff_fraction(shot, &expected);
    if frac > MAX_DIFF_FRACTION {
        let actual = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("snapshot-actual")
            .join(format!("{name}.png"));
        write_png(&actual, shot);
        return Some(format!(
            "{name}: {:.2} % Pixel weichen ab (erlaubt {:.2} %), Ist-Bild: {}",
            frac * 100.0,
            MAX_DIFF_FRACTION * 100.0,
            actual.display()
        ));
    }
    None
}

#[test]
fn gallery_snapshots() {
    let designs = [Design::Flix, Design::Plus, Design::Prime, Design::Retro];
    let mut failures = Vec::new();
    for design in designs {
        for (scene, label) in SCENES.iter().enumerate() {
            let name = format!("{}-{}@1x", design_name(design), label);
            if let Some(f) = check(&name, &render(design, scene as i32, 1)) {
                failures.push(f);
            }
        }
    }
    // DPR 2: LENZFLIX Karten und Leisten (Referenz), übrige Szenen nur Rendern ohne Absturz.
    for (scene, label) in SCENES.iter().enumerate() {
        let shot = render(Design::Flix, scene as i32, 2);
        if scene == 1 || scene == 3 {
            if let Some(f) = check(&format!("flix-{label}@2x"), &shot) {
                failures.push(f);
            }
        } else {
            assert!(!is_blank(&shot), "flix-{label}@2x leer");
        }
    }
    assert!(
        failures.is_empty(),
        "Snapshot-Abweichungen:\n{}",
        failures.join("\n")
    );
}

#[test]
fn designs_differ_in_breakpoints_and_render() {
    init_platform();
    {
        let ui = Gallery::new().expect("Galerie");
        let theme = ui.global::<Theme>();
        theme.set_design(Design::Flix);
        let flix = theme.invoke_cards_visible_at(1900.0);
        theme.set_design(Design::Plus);
        let plus = theme.invoke_cards_visible_at(1900.0);
        // Web: LENZ+ zeigt ab 1800 px 5 statt 6 Karten
        assert_eq!(flix, 6);
        assert_eq!(plus, 5);
        theme.set_design(Design::Flix);
        assert_eq!(theme.invoke_cards_visible_at(700.0), 2);
    }
    // Zwei Designs liefern verschiedene Bilder derselben Szene
    let a = render(Design::Flix, 0, 1);
    let b = render(Design::Retro, 0, 1);
    assert!(diff_fraction(&a, &b) > 0.05);
}
