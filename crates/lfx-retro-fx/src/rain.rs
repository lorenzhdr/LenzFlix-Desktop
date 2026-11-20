//! Code-Regen-Bildschirmschoner (Port der Spaltenlogik aus `screensaver.js`): fallende Zeichen in
//! Spalten, Kopf hell und Spur Bernstein. Die UI zeichnet die gelieferten Zellen; hier steckt nur
//! Zustand, Zufall (Seed) und Zeitplan.

use crate::rng::Rng;

/// Leerlaufzeit bis zum Start (3 min).
pub const IDLE_MS: u64 = 3 * 60 * 1000;
/// Bilddauer (30 fps); bei reduzierter Bewegung doppelt so lang.
pub const FRAME_MS: f64 = 1000.0 / 30.0;
/// Zeichenvorrat (Web: `CHARS`).
pub const CHARS: &str = "0123456789ABCDEF0123456789ABCDEFGHIJKLMNOPRSTUVWXYZ#%&$+=<>/\\|:;▓▒░█▌▐";
/// Farben für die UI (Hintergrund, Kopf, Spur).
pub const BG_COLOR: &str = "#0a0603";
pub const HEAD_COLOR: &str = "#ffe2a8";
pub const TRAIL_COLOR: &str = "rgba(255, 176, 0, 0.9)";
/// Verblassen je Bild (Alpha der Hintergrundfläche).
pub const FADE_ALPHA: f32 = 0.14;

/// Dauer eines Bildes in ms.
pub fn frame_ms(reduced: bool) -> f64 {
    if reduced { FRAME_MS * 2.0 } else { FRAME_MS }
}

/// Schriftgröße in physischen Pixeln (Web: 16 bzw. 20 CSS-Pixel mal DPR bis 2).
pub fn font_px(width_css: f32, dpr: f32) -> u32 {
    let d = dpr.clamp(1.0, 2.0);
    ((if width_css < 800.0 { 16.0 } else { 20.0 }) * d).round() as u32
}

/// Eine zu zeichnende Zelle: Spalte, Zeile (kann gebrochen sein), Kopf- und Spurzeichen.
/// Der Kopf steht bei `y`, die Spur eine Zeile darüber.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RainCell {
    pub col: u32,
    pub y: f32,
    pub head: char,
    pub trail: char,
}

/// Spaltenfeld des Regens.
#[derive(Debug, Clone)]
pub struct RainField {
    rng: Rng,
    chars: Vec<char>,
    drops: Vec<f32>,
    height_px: u32,
    font_px: u32,
    reduced: bool,
}

impl RainField {
    pub fn new(seed: u32, width_px: u32, height_px: u32, font_px: u32, reduced: bool) -> Self {
        let mut f = Self {
            rng: Rng::new(seed),
            chars: CHARS.chars().collect(),
            drops: Vec::new(),
            height_px: height_px.max(1),
            font_px: font_px.max(1),
            reduced,
        };
        f.resize(width_px, height_px, font_px);
        f
    }

    /// Größe ändern: bestehende Spalten bleiben, neue starten bei -[0,40).
    pub fn resize(&mut self, width_px: u32, height_px: u32, font_px: u32) {
        self.font_px = font_px.max(1);
        self.height_px = height_px.max(1);
        let n = width_px.max(1).div_ceil(self.font_px) as usize;
        self.drops.truncate(n);
        while self.drops.len() < n {
            let start = -(self.rng.next_f64() * 40.0).floor() as f32;
            self.drops.push(start);
        }
    }

    pub fn columns(&self) -> usize {
        self.drops.len()
    }

    fn glyph(&mut self) -> char {
        self.chars[self.rng.below(self.chars.len())]
    }

    /// Ein Bild: Zellen aller Spalten mit `y >= 0`, danach rücken die Tropfen vor (unten
    /// angekommene setzen mit 2,5 % Wahrscheinlichkeit neu oben an).
    pub fn step(&mut self) -> Vec<RainCell> {
        let mut cells = Vec::new();
        for i in 0..self.drops.len() {
            let y = self.drops[i];
            if y >= 0.0 {
                let head = self.glyph();
                let trail = self.glyph();
                cells.push(RainCell {
                    col: i as u32,
                    y,
                    head,
                    trail,
                });
            }
            let below = y * self.font_px as f32 > self.height_px as f32;
            self.drops[i] = if below && self.rng.next_f64() > 0.975 {
                -(self.rng.next_f64() * 20.0).floor() as f32
            } else {
                y + if self.reduced { 0.5 } else { 1.0 }
            };
        }
        cells
    }
}

/// Aufblitzender Titel im Regen (alle 5–9 s; 40 % „LENZ//84“, sonst einer der ersten 20 Titel).
#[derive(Debug, Clone)]
pub struct TitleFlash {
    rng: Rng,
    next_ms: u64,
}

impl TitleFlash {
    pub fn new(seed: u32) -> Self {
        Self {
            rng: Rng::new(seed),
            next_ms: 0,
        }
    }

    /// Neuer Text, wenn es Zeit ist (Animation neu starten), sonst `None`.
    pub fn poll(&mut self, now_ms: u64, titles: &[String]) -> Option<String> {
        if now_ms < self.next_ms {
            return None;
        }
        let mut pool: Vec<&str> = vec!["LENZ//84"];
        pool.extend(titles.iter().take(20).map(String::as_str));
        let pick = if self.rng.next_f64() < 0.4 {
            0
        } else {
            self.rng.below(pool.len())
        };
        self.next_ms = now_ms + 5000 + (self.rng.next_f64() * 4000.0) as u64;
        Some(pool[pick].to_string())
    }
}

/// Leerlauferkennung: Start nach [`IDLE_MS`] ohne Eingabe, wenn nichts blockiert.
#[derive(Debug, Clone)]
pub struct IdleTracker {
    idle_ms: u64,
    last_input: u64,
    last_move: Option<(f32, f32)>,
}

impl IdleTracker {
    pub fn new(now_ms: u64) -> Self {
        Self::with_idle(now_ms, IDLE_MS)
    }

    pub fn with_idle(now_ms: u64, idle_ms: u64) -> Self {
        Self {
            idle_ms,
            last_input: now_ms,
            last_move: None,
        }
    }

    /// Eingabe (Taste, Tippen, Rad).
    pub fn note_input(&mut self, now_ms: u64) {
        self.last_input = now_ms;
    }

    /// Zeigerbewegung; winzige Bewegungen (< 3 px Summe, Sensorrauschen) zählen nicht.
    pub fn note_move(&mut self, x: f32, y: f32, now_ms: u64) {
        if let Some((lx, ly)) = self.last_move
            && (x - lx).abs() + (y - ly).abs() < 3.0
        {
            return;
        }
        self.last_move = Some((x, y));
        self.last_input = now_ms;
    }

    /// Soll der Schoner jetzt starten? `blocked`: Player, Dialog, Terminal, Video läuft, Fenster verdeckt.
    pub fn should_start(&self, now_ms: u64, blocked: bool) -> bool {
        !blocked && now_ms.saturating_sub(self.last_input) >= self.idle_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaltenzahl_und_schrift() {
        assert_eq!(font_px(1200.0, 1.0), 20);
        assert_eq!(font_px(700.0, 2.0), 32);
        assert_eq!(font_px(1200.0, 3.0), 40, "DPR bis 2");
        let f = RainField::new(1, 1000, 600, 20, false);
        assert_eq!(f.columns(), 50);
        let g = RainField::new(1, 1001, 600, 20, false);
        assert_eq!(g.columns(), 51);
    }

    #[test]
    fn start_versetzt_und_deterministisch() {
        let mut a = RainField::new(5, 400, 600, 20, false);
        let mut b = RainField::new(5, 400, 600, 20, false);
        for _ in 0..50 {
            assert_eq!(a.step(), b.step());
        }
        assert_ne!(
            RainField::new(5, 400, 600, 20, false).drops,
            RainField::new(6, 400, 600, 20, false).drops
        );
        // Startwerte in (-40, 0]
        assert!(
            RainField::new(1, 400, 600, 20, false)
                .drops
                .iter()
                .all(|d| (-40.0..=0.0).contains(d))
        );
    }

    #[test]
    fn tropfen_fallen_und_setzen_neu_an() {
        let mut f = RainField::new(2, 100, 100, 20, false);
        let first = f.step();
        let before: Vec<f32> = f.drops.clone();
        let _ = f.step();
        for (b, a) in before.iter().zip(&f.drops) {
            assert!((a - b - 1.0).abs() < 1e-6 || *a < 0.0, "{b} -> {a}");
        }
        assert!(first.iter().all(|c| c.y >= 0.0));
        // Nach langer Laufzeit gibt es weiter Zellen (Neustart oben)
        let mut total = 0;
        for _ in 0..2000 {
            total += f.step().len();
        }
        assert!(total > 100);
    }

    #[test]
    fn reduziert_halbe_schritte() {
        let mut f = RainField::new(2, 40, 100, 20, true);
        f.drops = vec![0.0, 3.0];
        let cells = f.step();
        assert_eq!(cells.len(), 2);
        assert_eq!(f.drops, vec![0.5, 3.5]);
        assert!((frame_ms(true) - 2.0 * frame_ms(false)).abs() < 1e-9);
    }

    #[test]
    fn zeichen_aus_vorrat() {
        let mut f = RainField::new(3, 200, 100, 20, false);
        for _ in 0..60 {
            for c in f.step() {
                assert!(CHARS.contains(c.head) && CHARS.contains(c.trail));
            }
        }
    }

    #[test]
    fn groesse_aendern_behaelt_spalten() {
        let mut f = RainField::new(4, 200, 100, 20, false);
        let old = f.drops.clone();
        f.resize(400, 100, 20);
        assert_eq!(&f.drops[..old.len()], &old[..]);
        assert_eq!(f.columns(), 20);
        f.resize(100, 100, 20);
        assert_eq!(f.columns(), 5);
    }

    #[test]
    fn titel_aufblitzen() {
        let mut t = TitleFlash::new(1);
        let titles: Vec<String> = (0..30).map(|i| format!("TITEL {i}")).collect();
        let first = t.poll(0, &titles).expect("sofort");
        assert!(first == "LENZ//84" || titles[..20].contains(&first));
        assert!(t.poll(4999, &titles).is_none());
        let mut now = 0;
        for _ in 0..30 {
            now += 9001;
            let s = t.poll(now, &titles).expect("nach 9 s immer fällig");
            assert!(s == "LENZ//84" || titles[..20].contains(&s));
        }
    }

    #[test]
    fn leerlauf() {
        let mut i = IdleTracker::new(0);
        assert!(!i.should_start(IDLE_MS - 1, false));
        assert!(i.should_start(IDLE_MS, false));
        assert!(!i.should_start(IDLE_MS, true));
        i.note_input(IDLE_MS);
        assert!(!i.should_start(IDLE_MS + 1000, false));
        // Sensorrauschen zählt nicht, echte Bewegung schon
        i.note_move(10.0, 10.0, 0);
        i.note_move(11.0, 10.0, IDLE_MS * 2);
        assert!(i.should_start(IDLE_MS * 2, false));
        i.note_move(30.0, 10.0, IDLE_MS * 2);
        assert!(!i.should_start(IDLE_MS * 2, false));
    }
}
