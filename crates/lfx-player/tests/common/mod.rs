//! Gemeinsame Hilfen der Integrationstests. Ohne libmpv werden Tests übersprungen.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lfx_mpv_sys::Api;
use lfx_player::{Player, PlayerConfig, PlayerEvent, PlayerState};
use tokio::sync::broadcast::{Receiver, error::TryRecvError};

/// Headless-Player oder `None`, wenn libmpv fehlt.
pub fn headless_player() -> Option<Player> {
    let api = match Api::load() {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("libmpv nicht vorhanden, Test übersprungen: {e}");
            return None;
        }
    };
    Some(Player::with_api(api, PlayerConfig::headless()).expect("Player"))
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_path(ext: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lfx-player-test-{}-{n}.{ext}", std::process::id()))
}

/// Schreibt eine Stille-WAV (8 kHz, 8 Bit, mono) und liefert den Pfad.
pub fn write_wav(seconds: u32) -> PathBuf {
    let rate = 8000u32;
    let data_len = rate * seconds;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes()); // Bytes/s
    v.extend_from_slice(&1u16.to_le_bytes()); // Blockgröße
    v.extend_from_slice(&8u16.to_le_bytes()); // Bits
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v.extend(std::iter::repeat_n(128u8, data_len as usize));
    let p = temp_path("wav");
    std::fs::write(&p, v).unwrap();
    p
}

/// Schreibt einen kleinen SRT-Untertitel.
pub fn write_srt() -> PathBuf {
    let p = temp_path("srt");
    std::fs::write(&p, "1\n00:00:00,500 --> 00:00:02,000\nBeispieltext\n\n").unwrap();
    p
}

/// Wartet, bis `f` für den Zustand wahr ist.
pub fn wait_state(player: &Player, what: &str, f: impl Fn(&PlayerState) -> bool) -> PlayerState {
    let t0 = Instant::now();
    loop {
        let s = player.state();
        if f(&s) {
            return s;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(15),
            "Zeitüberschreitung: {what}; Zustand: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wartet auf ein passendes Ereignis.
pub fn wait_event(
    rx: &mut Receiver<PlayerEvent>,
    what: &str,
    f: impl Fn(&PlayerEvent) -> bool,
) -> PlayerEvent {
    let t0 = Instant::now();
    loop {
        match rx.try_recv() {
            Ok(e) if f(&e) => return e,
            Ok(_) | Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(10)),
            Err(TryRecvError::Closed) => panic!("Ereigniskanal zu: {what}"),
        }
        assert!(
            t0.elapsed() < Duration::from_secs(15),
            "Zeitüberschreitung: {what}"
        );
    }
}
