//! Klang-Synthesizer ohne Dateien (Port von `sound.js`): Oszillatoren, Rauschen, exponentielle
//! Hüllkurven, Biquad-Filter, Hauptlautstärke und Kompressor. Alle Parameter (Frequenzen,
//! Wellenformen, Zeiten, Lautstärken, Filter) entsprechen dem Web. Das Modul rechnet nur; das
//! Audiogerät steckt in [`crate::output`].
//!
//! Abweichungen zum Web: Oszillatoren sind mit PolyBLEP entschärft statt bandbegrenzten
//! Wellentabellen, der Kompressor hat harte Kante und keinen automatischen Ausgleich (die
//! Hauptlautstärke 0,08 liegt ohnehin meist unter der Schwelle).

use crate::rng::Rng;
use std::f32::consts::PI;

/// Hauptlautstärke (Web: `MASTER_GAIN`).
pub const MASTER_GAIN: f32 = 0.08;
/// Mindestabstand zweier Blip-Klänge in Millisekunden (Web: `HOVER_GAP_MS`).
pub const HOVER_GAP_MS: f64 = 60.0;
/// Untere Hüllkurvengrenze (Web: 0.0001).
const FLOOR: f32 = 0.0001;
/// Oszillatoren laufen nach der Hüllkurve noch 20 ms (Web: `stop(t + dur + 0.02)`).
const OSC_TAIL: f32 = 0.02;
/// Höchstzahl gleichzeitiger Stimmen (Schutz vor Dauerfeuer).
const MAX_VOICES: usize = 96;

/// Die Klänge der LENZ//84-Oberfläche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxSound {
    Blip,
    Click,
    Type,
    Open,
    Close,
    Nav,
    Error,
    Boot,
    Jingle,
}

/// Wellenform eines Oszillators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

/// Parameter eines Tons (Web: `tone(c, {...})`). Zeiten in Sekunden.
#[derive(Debug, Clone, Copy)]
pub struct ToneSpec {
    pub wave: Wave,
    pub f0: f32,
    pub f1: f32,
    pub t0: f32,
    pub dur: f32,
    pub vol: f32,
    pub lp: f32,
    pub attack: f32,
}

impl ToneSpec {
    fn new(wave: Wave, f0: f32) -> Self {
        Self {
            wave,
            f0,
            f1: f0,
            t0: 0.0,
            dur: 0.1,
            vol: 0.5,
            lp: 4000.0,
            attack: 0.005,
        }
    }
}

/// Parameter eines Rauschstoßes (Web: `noise(c, {...})`).
#[derive(Debug, Clone, Copy)]
pub struct NoiseSpec {
    pub t0: f32,
    pub dur: f32,
    pub vol: f32,
    pub hp: f32,
}

/// Frequenz einer MIDI-Note (Web: `NOTE`).
pub fn note(n: f32) -> f32 {
    440.0 * 2.0_f32.powf((n - 69.0) / 12.0)
}

/// Hüllkurve eines Tons zum Zeitpunkt `t` (ab Tonbeginn): exponentiell von 0,0001 auf `vol`
/// in `attack`, dann exponentiell zurück auf 0,0001 bis `dur`; danach 0,0001 (Web: drei
/// `gain`-Rampen).
pub fn tone_envelope(t: f32, vol: f32, attack: f32, dur: f32) -> f32 {
    if t < 0.0 {
        return 0.0;
    }
    if t < attack {
        return FLOOR * (vol / FLOOR).powf(t / attack);
    }
    if t < dur {
        let span = (dur - attack).max(1e-6);
        return vol * (FLOOR / vol).powf((t - attack) / span);
    }
    FLOOR
}

/// Hüllkurve eines Rauschstoßes: sofort `vol`, exponentiell auf 0,0001 in `dur`.
pub fn noise_envelope(t: f32, vol: f32, dur: f32) -> f32 {
    if t < 0.0 || t >= dur {
        return 0.0;
    }
    vol * (FLOOR / vol).powf(t / dur)
}

// ------------------------------------------------------------------ Filter

#[derive(Debug, Clone, Copy)]
enum FilterKind {
    LowPass,
    HighPass,
}

/// Biquad (Direct Form I) mit WebAudio-Q (Voreinstellung 1 dB).
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(kind: FilterKind, freq: f32, sample_rate: f32) -> Self {
        let f = freq.clamp(10.0, sample_rate * 0.49);
        let w0 = 2.0 * PI * f / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let q = 10.0_f32.powf(1.0 / 20.0); // Q = 1 dB
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match kind {
            FilterKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            FilterKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

// ------------------------------------------------------------------ Stimmen

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

fn oscillator(wave: Wave, phase: f32, dt: f32) -> f32 {
    match wave {
        Wave::Sine => (2.0 * PI * phase).sin(),
        Wave::Square => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            naive + poly_blep(phase, dt) - poly_blep((phase + 0.5).fract(), dt)
        }
        Wave::Sawtooth => 2.0 * phase - 1.0 - poly_blep(phase, dt),
        Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
    }
}

#[derive(Debug, Clone)]
enum Source {
    Tone { spec: ToneSpec, phase: f32 },
    Noise { spec: NoiseSpec },
}

#[derive(Debug, Clone)]
struct Voice {
    source: Source,
    filter: Biquad,
    /// Noch zu wartende Frames bis zum Start (Web: `t0`).
    delay: u64,
    /// Seit dem Start gerechnete Frames.
    age: u64,
    /// Gesamtlänge in Frames.
    len: u64,
}

impl Voice {
    fn tone(spec: ToneSpec, sr: f32) -> Self {
        Self {
            filter: Biquad::new(FilterKind::LowPass, spec.lp, sr),
            delay: (spec.t0 * sr) as u64,
            age: 0,
            len: ((spec.dur + OSC_TAIL) * sr) as u64,
            source: Source::Tone { spec, phase: 0.0 },
        }
    }

    fn noise(spec: NoiseSpec, sr: f32) -> Self {
        Self {
            filter: Biquad::new(FilterKind::HighPass, spec.hp, sr),
            delay: (spec.t0 * sr) as u64,
            age: 0,
            len: ((spec.dur * sr) as u64).max(1),
            source: Source::Noise { spec },
        }
    }

    fn finished(&self) -> bool {
        self.delay == 0 && self.age >= self.len
    }

    fn next(&mut self, sr: f32, rng: &mut Rng) -> f32 {
        if self.delay > 0 {
            self.delay -= 1;
            return 0.0;
        }
        if self.age >= self.len {
            return 0.0;
        }
        let t = self.age as f32 / sr;
        self.age += 1;
        match &mut self.source {
            Source::Tone { spec, phase } => {
                let f = if (spec.f1 - spec.f0).abs() > f32::EPSILON && spec.dur > 0.0 {
                    let r = (t / spec.dur).min(1.0);
                    spec.f0 * (spec.f1.max(1.0) / spec.f0).powf(r)
                } else {
                    spec.f0
                };
                let dt = (f / sr).clamp(1e-6, 0.5);
                let s = oscillator(spec.wave, *phase, dt);
                *phase = (*phase + dt).fract();
                self.filter.process(s) * tone_envelope(t, spec.vol, spec.attack, spec.dur)
            }
            Source::Noise { spec } => {
                let s = rng.next_f32() * 2.0 - 1.0;
                self.filter.process(s) * noise_envelope(t, spec.vol, spec.dur)
            }
        }
    }
}

/// Dauerhaftes leises Rauschen (Code-Regen): Rauschpuffer 2 s in Schleife, Tiefpass 700 Hz, 0,18.
#[derive(Debug, Clone)]
struct Rain {
    buf: Vec<f32>,
    pos: usize,
    filter: Biquad,
}

const RAIN_GAIN: f32 = 0.18;
const RAIN_LP: f32 = 700.0;

// ------------------------------------------------------------------ Kompressor

/// Einfacher Kompressor hinter der Hauptlautstärke (Web: Schwelle -18 dB, Verhältnis 6).
#[derive(Debug, Clone)]
struct Compressor {
    env_db: f32,
    attack: f32,
    release: f32,
}

impl Compressor {
    fn new(sr: f32) -> Self {
        let coef = |sec: f32| (-1.0 / (sec * sr)).exp();
        Self {
            env_db: -120.0,
            attack: coef(0.003),
            release: coef(0.25),
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let level_db = 20.0 * x.abs().max(1e-6).log10();
        let c = if level_db > self.env_db {
            self.attack
        } else {
            self.release
        };
        self.env_db = c * self.env_db + (1.0 - c) * level_db;
        let over = self.env_db - (-18.0);
        if over <= 0.0 {
            return x;
        }
        let reduction_db = over - over / 6.0;
        x * 10.0_f32.powf(-reduction_db / 20.0)
    }
}

// ------------------------------------------------------------------ Synthesizer

/// Mischer mit Stimmen. Mono; die Ausgabe dupliziert auf alle Kanäle.
#[derive(Debug, Clone)]
pub struct Synth {
    sample_rate: f32,
    enabled: bool,
    voices: Vec<Voice>,
    rain: Option<Rain>,
    comp: Compressor,
    rng: Rng,
    /// Gerenderte Frames (Zeitbasis für den Blip-Abstand).
    clock: u64,
    last_blip: Option<u64>,
}

impl Synth {
    /// Neuer Synthesizer. Sound ist standardmäßig AUS ([`Synth::set_enabled`]).
    pub fn new(sample_rate: u32, seed: u32) -> Self {
        let sr = sample_rate.max(8000) as f32;
        Self {
            sample_rate: sr,
            enabled: false,
            voices: Vec::new(),
            rain: None,
            comp: Compressor::new(sr),
            rng: Rng::new(seed),
            clock: 0,
            last_blip: None,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate as u32
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Sound an/aus. Ausschalten beendet alle laufenden Stimmen und den Regen sofort.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.voices.clear();
            self.rain = None;
        }
    }

    /// Anzahl laufender Stimmen (ohne Regen).
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    pub fn rain_active(&self) -> bool {
        self.rain.is_some()
    }

    fn add_tone(&mut self, spec: ToneSpec) {
        if self.voices.len() < MAX_VOICES {
            self.voices.push(Voice::tone(spec, self.sample_rate));
        }
    }

    fn add_noise(&mut self, spec: NoiseSpec) {
        if self.voices.len() < MAX_VOICES {
            self.voices.push(Voice::noise(spec, self.sample_rate));
        }
    }

    /// Klang abspielen. `false`, wenn Sound aus ist oder der Blip-Abstand (60 ms) unterschritten wird.
    pub fn play(&mut self, sound: FxSound) -> bool {
        if !self.enabled {
            return false;
        }
        if sound == FxSound::Blip {
            let gap = (HOVER_GAP_MS / 1000.0 * f64::from(self.sample_rate)) as u64;
            if let Some(last) = self.last_blip
                && self.clock.saturating_sub(last) < gap
            {
                return false;
            }
            self.last_blip = Some(self.clock);
        }
        for t in sound_tones(sound) {
            self.add_tone(t);
        }
        for n in sound_noises(sound, &mut self.rng) {
            self.add_noise(n);
        }
        true
    }

    /// Leises Rauschen starten (Code-Regen). Nur bei aktivem Sound.
    pub fn start_rain(&mut self) {
        if !self.enabled || self.rain.is_some() {
            return;
        }
        let len = (self.sample_rate * 2.0) as usize;
        let buf = (0..len).map(|_| self.rng.next_f32() * 2.0 - 1.0).collect();
        self.rain = Some(Rain {
            buf,
            pos: 0,
            filter: Biquad::new(FilterKind::LowPass, RAIN_LP, self.sample_rate),
        });
    }

    pub fn stop_rain(&mut self) {
        self.rain = None;
    }

    /// Mono-Puffer füllen (überschreibt).
    pub fn render(&mut self, out: &mut [f32]) {
        let sr = self.sample_rate;
        for slot in out.iter_mut() {
            let mut acc = 0.0;
            for v in &mut self.voices {
                acc += v.next(sr, &mut self.rng);
            }
            if let Some(r) = &mut self.rain {
                let s = r.buf[r.pos];
                r.pos = (r.pos + 1) % r.buf.len();
                acc += r.filter.process(s) * RAIN_GAIN;
            }
            *slot = self.comp.process(acc * MASTER_GAIN);
        }
        self.clock += out.len() as u64;
        self.voices.retain(|v| !v.finished());
    }

    /// Bequem: Mono-Puffer mit `seconds` Länge rendern.
    pub fn render_seconds(&mut self, seconds: f32) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * self.sample_rate) as usize];
        self.render(&mut out);
        out
    }
}

// ------------------------------------------------------------------ Klangtabelle

fn tone(wave: Wave, f0: f32, f1: f32, dur: f32, vol: f32, lp: f32) -> ToneSpec {
    ToneSpec {
        f1,
        dur,
        vol,
        lp,
        ..ToneSpec::new(wave, f0)
    }
}

/// Töne eines Klangs (Web: `SOUNDS`).
pub fn sound_tones(sound: FxSound) -> Vec<ToneSpec> {
    use Wave::{Sawtooth, Square, Triangle};
    match sound {
        FxSound::Blip => vec![tone(Square, 1320.0, 1500.0, 0.035, 0.12, 3000.0)],
        FxSound::Click => vec![tone(Square, 880.0, 440.0, 0.06, 0.4, 2600.0)],
        FxSound::Type => vec![],
        FxSound::Open => vec![tone(Sawtooth, 220.0, 880.0, 0.18, 0.3, 2400.0)],
        FxSound::Close => vec![tone(Sawtooth, 880.0, 200.0, 0.16, 0.28, 2400.0)],
        FxSound::Nav => vec![
            tone(Square, 660.0, 660.0, 0.05, 0.3, 3000.0),
            ToneSpec {
                t0: 0.05,
                ..tone(Square, 990.0, 990.0, 0.07, 0.3, 3000.0)
            },
        ],
        FxSound::Error => vec![
            tone(Sawtooth, 160.0, 160.0, 0.14, 0.4, 1200.0),
            ToneSpec {
                t0: 0.15,
                ..tone(Sawtooth, 120.0, 120.0, 0.22, 0.4, 1200.0)
            },
        ],
        FxSound::Boot => {
            let mut v: Vec<ToneSpec> = [48.0, 55.0, 60.0, 64.0, 67.0, 72.0]
                .iter()
                .enumerate()
                .map(|(i, &n)| ToneSpec {
                    t0: i as f32 * 0.11,
                    ..tone(
                        if i % 2 == 1 { Square } else { Sawtooth },
                        note(n),
                        note(n),
                        0.5 - i as f32 * 0.04,
                        0.28,
                        1800.0,
                    )
                })
                .collect();
            v.push(tone(Sawtooth, note(36.0), note(36.0), 1.1, 0.3, 600.0));
            v
        }
        FxSound::Jingle => {
            // 8-Bit-Fanfare, ca. 2 s
            let melody = [
                67.0, 67.0, 72.0, 76.0, 79.0, 76.0, 79.0, 84.0, 83.0, 79.0, 76.0, 79.0, 84.0,
            ];
            let len = [
                0.1, 0.1, 0.1, 0.1, 0.2, 0.1, 0.1, 0.3, 0.1, 0.1, 0.1, 0.1, 0.5,
            ];
            let mut out = Vec::new();
            let mut t = 0.0;
            for (n, l) in melody.iter().zip(len) {
                out.push(ToneSpec {
                    t0: t,
                    ..tone(Square, note(*n), note(*n), l * 0.95, 0.3, 5000.0)
                });
                out.push(ToneSpec {
                    t0: t,
                    ..tone(
                        Triangle,
                        note(n - 12.0),
                        note(n - 12.0),
                        l * 0.95,
                        0.3,
                        2000.0,
                    )
                });
                t += l;
            }
            out
        }
    }
}

/// Rauschstöße eines Klangs (Click, Type); `Type` würfelt die Filterfrequenz (1800–3300 Hz).
pub fn sound_noises(sound: FxSound, rng: &mut Rng) -> Vec<NoiseSpec> {
    match sound {
        FxSound::Click => vec![NoiseSpec {
            t0: 0.0,
            dur: 0.015,
            vol: 0.2,
            hp: 3000.0,
        }],
        FxSound::Type => vec![NoiseSpec {
            t0: 0.0,
            dur: 0.022,
            vol: 0.28,
            hp: 1800.0 + rng.next_f32() * 1500.0,
        }],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn rms(x: &[f32]) -> f32 {
        if x.is_empty() {
            return 0.0;
        }
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    fn ms_range(x: &[f32], from: f32, to: f32) -> &[f32] {
        let a = (from * SR as f32 / 1000.0) as usize;
        let b = ((to * SR as f32 / 1000.0) as usize).min(x.len());
        &x[a.min(b)..b]
    }

    /// Zeitpunkt des letzten Samples über der Schwelle, in Sekunden.
    fn last_audible(x: &[f32], thr: f32) -> f32 {
        x.iter()
            .rposition(|s| s.abs() > thr)
            .map_or(0.0, |i| i as f32 / SR as f32)
    }

    fn on() -> Synth {
        let mut s = Synth::new(SR, 1);
        s.set_enabled(true);
        s
    }

    #[test]
    fn sound_ist_standardmaessig_aus() {
        let mut s = Synth::new(SR, 1);
        assert!(!s.is_enabled());
        assert!(!s.play(FxSound::Click));
        s.start_rain();
        assert!(!s.rain_active());
        let out = s.render_seconds(0.1);
        assert_eq!(peak(&out), 0.0);
    }

    #[test]
    fn huellkurve_tone() {
        assert!((tone_envelope(0.0, 0.4, 0.005, 0.06) - FLOOR).abs() < 1e-6);
        assert!((tone_envelope(0.005, 0.4, 0.005, 0.06) - 0.4).abs() < 1e-4);
        assert!(tone_envelope(0.03, 0.4, 0.005, 0.06) < 0.4);
        assert!(tone_envelope(0.03, 0.4, 0.005, 0.06) > tone_envelope(0.05, 0.4, 0.005, 0.06));
        assert!((tone_envelope(0.06, 0.4, 0.005, 0.06) - FLOOR).abs() < 1e-6);
        // Exponentiell: Mitte des Abklingens = geometrisches Mittel
        let mid = tone_envelope(0.0325, 0.4, 0.005, 0.06);
        assert!((mid - (0.4_f32 * FLOOR).sqrt()).abs() < 1e-3);
    }

    #[test]
    fn huellkurve_rauschen() {
        assert!((noise_envelope(0.0, 0.2, 0.015) - 0.2).abs() < 1e-6);
        assert_eq!(noise_envelope(0.015, 0.2, 0.015), 0.0);
        assert!(noise_envelope(0.0075, 0.2, 0.015) < 0.2);
    }

    #[test]
    fn click_dauer_und_abklingen() {
        let mut s = on();
        assert!(s.play(FxSound::Click));
        let out = s.render_seconds(0.3);
        // Ton 60 ms + 20 ms Nachlauf; danach absolute Stille
        assert!(
            last_audible(&out, 1e-9) <= 0.082,
            "Ende {}",
            last_audible(&out, 1e-9)
        );
        assert!(peak(&out) > 0.01, "hörbar");
        let early = rms(ms_range(&out, 2.0, 15.0));
        let late = rms(ms_range(&out, 50.0, 60.0));
        assert!(early > 5.0 * late, "{early} vs {late}");
        // Lautstärke begrenzt: vol 0,4 * Master 0,08 (+ Filterüberschwinger)
        assert!(peak(&out) < 0.4 * MASTER_GAIN * 1.5);
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn nav_zwei_toene_nacheinander() {
        let mut s = on();
        s.play(FxSound::Nav);
        let out = s.render_seconds(0.3);
        assert!(rms(ms_range(&out, 5.0, 45.0)) > 1e-3);
        assert!(rms(ms_range(&out, 55.0, 115.0)) > 1e-3);
        assert!(last_audible(&out, 1e-9) <= 0.05 + 0.07 + 0.021);
        assert_eq!(peak(ms_range(&out, 145.0, 300.0)), 0.0);
    }

    #[test]
    fn error_laenge_und_tonhoehe() {
        let mut s = on();
        s.play(FxSound::Error);
        let out = s.render_seconds(0.8);
        // zweiter Ton beginnt bei 150 ms, endet 150+220+20 ms
        assert!(last_audible(&out, 1e-9) <= 0.39);
        assert!(last_audible(&out, 1e-5) > 0.30);
        // Nulldurchgänge im ersten Ton: ca. 160 Hz
        let seg = ms_range(&out, 10.0, 100.0);
        let zc = seg.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let hz = zc as f32 / 0.09;
        assert!((130.0..200.0).contains(&hz), "{hz}");
    }

    #[test]
    fn boot_und_jingle_dauer() {
        let mut s = on();
        s.play(FxSound::Boot);
        let boot = s.render_seconds(2.0);
        // Bass 1,1 s + 20 ms
        let end = last_audible(&boot, 1e-9);
        assert!((1.0..=1.13).contains(&end), "{end}");
        assert!(peak(&boot) > 0.01);

        let mut s = on();
        s.play(FxSound::Jingle);
        let j = s.render_seconds(3.0);
        // Melodie 1,9 s lang, letzter Ton 0,5 s * 0,95 + 20 ms
        let end = last_audible(&j, 1e-9);
        assert!((1.9..=2.0).contains(&end), "{end}");
    }

    #[test]
    fn blip_abstand_60ms() {
        let mut s = on();
        assert!(s.play(FxSound::Blip));
        assert!(!s.play(FxSound::Blip), "zu schnell");
        let _ = s.render_seconds(0.07);
        assert!(s.play(FxSound::Blip));
        // andere Klänge sind nicht begrenzt
        assert!(s.play(FxSound::Click));
        assert!(s.play(FxSound::Click));
    }

    #[test]
    fn frequenzrampe_open() {
        // Open: 220 -> 880 Hz, zählbar über Nulldurchgänge
        let mut s = on();
        s.play(FxSound::Open);
        let out = s.render_seconds(0.3);
        let a = ms_range(&out, 10.0, 50.0);
        let b = ms_range(&out, 130.0, 170.0);
        let zc = |x: &[f32]| x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!(zc(b) > zc(a) * 2, "{} {}", zc(a), zc(b));
        assert!(last_audible(&out, 1e-9) <= 0.2);
    }

    #[test]
    fn rauschen_hochpass_und_tippen_deterministisch() {
        let run = |seed| {
            let mut s = Synth::new(SR, seed);
            s.set_enabled(true);
            s.play(FxSound::Type);
            s.render_seconds(0.05)
        };
        let a = run(5);
        assert_eq!(a, run(5));
        assert_ne!(a, run(6));
        assert!(last_audible(&a, 1e-9) <= 0.0221);
        assert!(peak(&a) > 1e-3);
    }

    #[test]
    fn regen_dauerhaft_bis_stop() {
        let mut s = on();
        s.start_rain();
        assert!(s.rain_active());
        let out = s.render_seconds(3.0); // länger als der 2-s-Puffer: Schleife
        assert!(rms(ms_range(&out, 2500.0, 3000.0)) > 1e-3);
        s.stop_rain();
        let out = s.render_seconds(0.1);
        assert_eq!(peak(&out), 0.0);
    }

    #[test]
    fn ausschalten_beendet_stimmen() {
        let mut s = on();
        s.play(FxSound::Boot);
        s.start_rain();
        s.set_enabled(false);
        assert_eq!(s.active_voices(), 0);
        assert!(!s.rain_active());
        assert_eq!(peak(&s.render_seconds(0.1)), 0.0);
    }

    #[test]
    fn stimmenzahl_begrenzt() {
        let mut s = on();
        for _ in 0..40 {
            s.play(FxSound::Jingle);
        }
        assert!(s.active_voices() <= MAX_VOICES);
    }

    #[test]
    fn noten_frequenz() {
        assert!((note(69.0) - 440.0).abs() < 1e-3);
        assert!((note(57.0) - 220.0).abs() < 1e-2);
    }
}
