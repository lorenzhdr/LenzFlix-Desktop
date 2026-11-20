//! Ausgabe des Synthesizers über das Audiogerät (cpal; WASAPI, CoreAudio, ...).
//!
//! Der Stream wird erst bei [`FxAudio::unlock`] angelegt (nach einer Benutzergeste und nur bei
//! eingeschaltetem Sound). Befehle gehen über einen Kanal in den Audio-Rückruf, der den
//! [`Synth`] besitzt. Fehler werden geschluckt und als `false` gemeldet (Sound ist Beiwerk).
//! `FxAudio` ist nicht `Send` (cpal-Streams sind es auf manchen Plattformen nicht): im
//! UI-Thread halten.

use crate::events::sound_for_event;
use crate::synth::{FxSound, Synth};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

enum Cmd {
    Play(FxSound),
    StartRain,
    StopRain,
    Enabled(bool),
}

/// Audioausgabe der Effekte. Sound ist standardmäßig AUS.
pub struct FxAudio {
    enabled: Arc<AtomicBool>,
    tx: Option<Sender<Cmd>>,
    stream: Option<Stream>,
    seed: u32,
}

impl FxAudio {
    pub fn new(seed: u32) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            tx: None,
            stream: None,
            seed,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Läuft der Ausgabe-Stream?
    pub fn is_running(&self) -> bool {
        self.stream.is_some()
    }

    /// Sound an/aus. Ausschalten stoppt laufende Klänge sofort; der Stream bleibt (leise)
    /// bestehen, bis [`FxAudio::suspend`] ihn schließt.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        self.send(Cmd::Enabled(on));
    }

    fn send(&self, c: Cmd) -> bool {
        self.tx.as_ref().is_some_and(|t| t.send(c).is_ok())
    }

    /// Nach einer Benutzergeste: Stream anlegen (nur bei eingeschaltetem Sound).
    pub fn unlock(&mut self) -> bool {
        if !self.is_enabled() {
            return false;
        }
        if self.stream.is_some() {
            return true;
        }
        match build_stream(self.seed) {
            Some((stream, tx)) => {
                let _ = tx.send(Cmd::Enabled(true));
                self.tx = Some(tx);
                self.stream = Some(stream);
                true
            }
            None => false,
        }
    }

    /// Klang spielen; `false`, wenn Sound aus ist oder noch nicht entsperrt wurde.
    pub fn play(&self, sound: FxSound) -> bool {
        self.is_enabled() && self.send(Cmd::Play(sound))
    }

    /// Seiten-Ereignis (`hover`, `click`, `type`, ...) abspielen.
    pub fn play_event(&self, name: &str) -> bool {
        sound_for_event(name).is_some_and(|s| self.play(s))
    }

    pub fn start_rain(&self) {
        if self.is_enabled() {
            self.send(Cmd::StartRain);
        }
    }

    pub fn stop_rain(&self) {
        self.send(Cmd::StopRain);
    }

    /// Regen stoppen und den Stream schließen (z. B. bei Design-Wechsel oder im Player).
    pub fn suspend(&mut self) {
        self.stop_rain();
        self.stream = None;
        self.tx = None;
    }
}

fn build_stream(seed: u32) -> Option<(Stream, Sender<Cmd>)> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let (tx, rx) = channel();
    let rx = Arc::new(Mutex::new(rx));
    let stream = match format {
        SampleFormat::F32 => run::<f32>(&device, &config, rx, seed),
        SampleFormat::I16 => run::<i16>(&device, &config, rx, seed),
        SampleFormat::U16 => run::<u16>(&device, &config, rx, seed),
        SampleFormat::I32 => run::<i32>(&device, &config, rx, seed),
        _ => None,
    }?;
    stream.play().ok()?;
    Some((stream, tx))
}

fn run<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    rx: Arc<Mutex<Receiver<Cmd>>>,
    seed: u32,
) -> Option<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let mut synth = Synth::new(config.sample_rate, seed);
    let mut mono: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                if let Ok(rx) = rx.try_lock() {
                    while let Ok(c) = rx.try_recv() {
                        match c {
                            Cmd::Play(s) => {
                                synth.play(s);
                            }
                            Cmd::StartRain => synth.start_rain(),
                            Cmd::StopRain => synth.stop_rain(),
                            Cmd::Enabled(on) => synth.set_enabled(on),
                        }
                    }
                }
                let frames = data.len() / channels;
                mono.resize(frames, 0.0);
                synth.render(&mut mono);
                for (frame, s) in data.chunks_mut(channels).zip(&mono) {
                    let v = T::from_sample(s.clamp(-1.0, 1.0));
                    for slot in frame.iter_mut() {
                        *slot = v;
                    }
                }
                // Reste (unvollständiger Frame) stumm
                let rest = data.len() % channels;
                if rest > 0 {
                    let n = data.len();
                    for slot in &mut data[n - rest..] {
                        *slot = T::EQUILIBRIUM;
                    }
                }
            },
            |e| eprintln!("Audio-Fehler: {e}"),
            None,
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohne_entsperren_stumm_und_aus() {
        let mut a = FxAudio::new(1);
        assert!(!a.is_enabled(), "Sound standardmäßig aus");
        assert!(!a.play(FxSound::Click));
        assert!(!a.play_event("click"));
        assert!(!a.unlock(), "aus: kein Stream");
        assert!(!a.is_running());
        a.set_enabled(true);
        // Ohne Stream (nicht entsperrt) wird nichts gesendet
        assert!(!a.play(FxSound::Click));
        a.suspend();
    }
}
