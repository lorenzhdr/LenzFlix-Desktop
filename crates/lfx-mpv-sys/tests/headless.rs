//! Laden der echten libmpv und Kurzwiedergabe ohne Fenster.
//! Ohne vorhandene Bibliothek (kein `vendor/mpv/...`) werden die Tests übersprungen.

use std::sync::Arc;
use std::time::{Duration, Instant};

use lfx_mpv_sys::ffi::{MPV_FORMAT_DOUBLE, make_version};
use lfx_mpv_sys::{Api, Event, LoadError, Mpv, PropValue};

fn load() -> Option<Arc<Api>> {
    match Api::load() {
        Ok(a) => Some(Arc::new(a)),
        Err(LoadError::NotFound(_)) => {
            eprintln!("libmpv nicht vorhanden, Test übersprungen");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn api_version_is_at_least_2_6() {
    let Some(api) = load() else { return };
    assert!(
        api.client_api_version() >= make_version(2, 6),
        "{:#x}",
        api.client_api_version()
    );
}

#[test]
fn plays_lavfi_clip_headless() {
    let Some(api) = load() else { return };
    let mpv = Mpv::new(api).unwrap();
    mpv.set_option_string("vo", "null").unwrap();
    mpv.set_option_string("ao", "null").unwrap();
    mpv.set_option_string("terminal", "no").unwrap();
    mpv.set_option_string("idle", "yes").unwrap();
    mpv.initialize().unwrap();
    mpv.observe_property(1, "time-pos", MPV_FORMAT_DOUBLE)
        .unwrap();
    mpv.command(&["loadfile", "av://lavfi:testsrc2=size=320x240:rate=30"])
        .unwrap();

    let start = Instant::now();
    let mut max_pos = 0.0_f64;
    let mut loaded = false;
    while start.elapsed() < Duration::from_secs(8) && max_pos <= 2.0 {
        match mpv.wait_event(0.1) {
            Event::FileLoaded => loaded = true,
            Event::PropertyChange {
                value: PropValue::Double(d),
                ..
            } => max_pos = max_pos.max(d),
            Event::Shutdown => break,
            _ => {}
        }
    }
    assert!(loaded, "Datei wurde nicht geladen");
    assert!(max_pos > 1.0, "time-pos nur {max_pos}");
    assert!(mpv.get_property_f64("time-pos").unwrap() > 1.0);
}
