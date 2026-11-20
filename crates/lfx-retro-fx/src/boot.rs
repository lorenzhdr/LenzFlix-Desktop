//! Boot-Sequenz (Port von `boot.js`) als reine Zeitfunktion: zu jedem Zeitpunkt `t` (ms seit
//! Start) liefert [`BootPlan::frame`] die sichtbaren Textzeilen, den Cursor und den Zustand
//! „öffnet“/„fertig“. Die Darstellung (Bernstein-Text, weißer Strich, Hälften fahren auseinander)
//! macht die UI; der Klang `FxSound::Boot` startet zum Zeitpunkt 0.

/// Anzeigename der BIOS-Version (Web: `BOOT_VERSION`).
pub const BOOT_VERSION: &str = "v1.6";
/// Harter Abbruch: nach dieser Zeit ist das Overlay immer weg.
pub const HARD_TIMEOUT_MS: u32 = 4500;
/// Gesamtdauer bei reduzierter Bewegung.
pub const REDUCED_MS: u32 = 800;
/// Hinweiszeile unten im Boot-Bild.
pub const SKIP_HINT: &str = "TASTE / TIPPEN ZUM ÜBERSPRINGEN";

/// Öffnen (Einschalt-Effekt) im Normalablauf.
const OPEN_AT_MS: u32 = 2450;
/// Dauer des Öffnens bis zum Ende: normal / übersprungen.
const OPEN_MS: u32 = 640;
const OPEN_FAST_MS: u32 = 420;

/// Sichtbarer Zustand zu einem Zeitpunkt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFrame {
    /// Bereits vorhandene Zeilen (auch noch leere, die gerade anfangen zu tippen).
    pub lines: Vec<String>,
    /// Blinkender Cursor hinter „BEREIT.“.
    pub caret: bool,
    /// Einschalt-Effekt läuft (CSS-Klasse `is-open`).
    pub open: bool,
    /// Overlay entfernen, `onDone` feuern.
    pub finished: bool,
}

/// Zeitplan der Boot-Sequenz.
#[derive(Debug, Clone)]
pub struct BootPlan {
    user: String,
    reduced: bool,
}

/// Tippende Zeile: Beginn, Gesamtzeit; Rückgabe (Text bei `tt`, fertig getippt).
fn typed(text: &str, start: u32, total_ms: u32, tt: u32) -> Option<(String, bool)> {
    if tt < start {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as u32;
    if n == 0 {
        return Some((String::new(), true));
    }
    let interval = ((total_ms as f64 / n as f64).round() as u32).max(8);
    let shown = ((tt - start) / interval).min(n);
    Some((chars[..shown as usize].iter().collect(), shown >= n))
}

impl BootPlan {
    /// `user`: Anzeigename, falls bis 1,7 s bekannt; sonst GAST.
    pub fn new(user: Option<&str>, reduced: bool) -> Self {
        let who = user
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("GAST");
        Self {
            user: who.to_uppercase(),
            reduced,
        }
    }

    /// Zeitpunkt, ab dem „öffnet“ gilt, und Zeitpunkt des Endes. `skipped_at`: Überspringen per
    /// Taste/Klick/Tipp (nur wirksam, bevor das Öffnen sowieso beginnt).
    pub fn timeline(&self, skipped_at: Option<u32>) -> (u32, u32) {
        let natural_open = if self.reduced {
            REDUCED_MS - 300
        } else {
            OPEN_AT_MS
        };
        let natural_end = if self.reduced {
            REDUCED_MS
        } else {
            OPEN_AT_MS + OPEN_MS
        };
        match skipped_at {
            Some(s) if s < natural_open => (s, (s + OPEN_FAST_MS).min(HARD_TIMEOUT_MS)),
            _ => (natural_open, natural_end.min(HARD_TIMEOUT_MS)),
        }
    }

    /// Gesamtdauer ohne Überspringen.
    pub fn total_ms(&self) -> u32 {
        self.timeline(None).1
    }

    /// Zustand zum Zeitpunkt `t_ms`.
    pub fn frame(&self, t_ms: u32, skipped_at: Option<u32>) -> BootFrame {
        let (open_at, end_at) = self.timeline(skipped_at);
        let open = t_ms >= open_at;
        let finished = t_ms >= end_at || t_ms >= HARD_TIMEOUT_MS;
        if self.reduced {
            let lines = [
                format!("LENZ//84 BIOS {BOOT_VERSION}"),
                "SPEICHERTEST ... 640K OK".to_string(),
                "LADE MEDIATHEK ... OK".to_string(),
                "BEREIT.".to_string(),
            ]
            .to_vec();
            return BootFrame {
                lines,
                caret: false,
                open,
                finished,
            };
        }
        // Beim Öffnen werden alle Zeitgeber gelöscht: der Text friert ein.
        let tt = t_ms.min(open_at);
        let mut lines = Vec::new();
        let mut caret = false;

        if let Some((s, _)) = typed(&format!("LENZ//84 BIOS {BOOT_VERSION}"), 0, 400, tt) {
            lines.push(s);
        }
        if tt >= 500 {
            let kb = (((tt - 500) / 30) * 32).min(640);
            lines.push(format!("SPEICHERTEST ... {}", mem_text(kb)));
        }
        if let Some((s, done)) = typed("LADE MEDIATHEK ...", 1200, 380, tt) {
            lines.push(if done { format!("{s} OK") } else { s });
        }
        if let Some((s, _)) = typed(&format!("BENUTZER: {}", self.user), 1700, 300, tt) {
            lines.push(s);
        }
        if let Some((s, done)) = typed("BEREIT.", 2100, 160, tt) {
            lines.push(s);
            caret = done && !open;
        }
        // Vor dem ersten Tick ist die Speicherzeile leer (nur „SPEICHERTEST ... 0K“ erst nach 30 ms).
        BootFrame {
            lines,
            caret,
            open,
            finished,
        }
    }
}

fn mem_text(kb: u32) -> String {
    if kb >= 640 {
        format!("{kb}K OK")
    } else {
        format!("{kb}K")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> BootPlan {
        BootPlan::new(Some("Anna"), false)
    }

    #[test]
    fn zeitplan_normal() {
        let p = plan();
        assert_eq!(p.timeline(None), (2450, 3090));
        assert_eq!(p.total_ms(), 3090);
        assert!(p.frame(0, None).lines.len() == 1);
        assert!(!p.frame(3089, None).finished);
        assert!(p.frame(3090, None).finished);
    }

    #[test]
    fn bios_zeile_tippt() {
        let p = plan();
        assert_eq!(p.frame(0, None).lines, vec![String::new()]);
        let mid = &p.frame(200, None).lines[0];
        assert!(mid.starts_with("LENZ//") && mid.len() < "LENZ//84 BIOS v1.6".len());
        assert_eq!(p.frame(450, None).lines[0], "LENZ//84 BIOS v1.6");
    }

    #[test]
    fn speichertest_zaehlt_bis_640() {
        let p = plan();
        assert!(p.frame(499, None).lines.len() == 1);
        // 30 ms pro Schritt von 32K: 20 Schritte bis 640K = 600 ms nach 500
        assert_eq!(p.frame(530, None).lines[1], "SPEICHERTEST ... 32K");
        assert_eq!(p.frame(1099, None).lines[1], "SPEICHERTEST ... 608K");
        assert_eq!(p.frame(1100, None).lines[1], "SPEICHERTEST ... 640K OK");
    }

    #[test]
    fn spaetere_zeilen_und_name() {
        let p = plan();
        let f = p.frame(2300, None);
        assert_eq!(f.lines[2], "LADE MEDIATHEK ... OK");
        assert_eq!(f.lines[3], "BENUTZER: ANNA");
        assert_eq!(f.lines[4], "BEREIT.");
        assert!(f.caret);
        assert!(!f.open);
        assert!(BootPlan::new(None, false).frame(2300, None).lines[3].ends_with("GAST"));
        assert!(BootPlan::new(Some("  "), false).frame(2300, None).lines[3].ends_with("GAST"));
    }

    #[test]
    fn oeffnen_nimmt_cursor_und_beendet() {
        let p = plan();
        let f = p.frame(2500, None);
        assert!(f.open && !f.caret && !f.finished);
    }

    #[test]
    fn ueberspringen_friert_text_ein_und_oeffnet_schnell() {
        let p = plan();
        assert_eq!(p.timeline(Some(700)), (700, 1120));
        let f = p.frame(900, Some(700));
        assert!(f.open && !f.finished);
        // eingefroren: gleicher Text wie bei 700 ms
        assert_eq!(f.lines, p.frame(700, Some(700)).lines);
        assert!(p.frame(1120, Some(700)).finished);
        // Überspringen nach dem natürlichen Öffnen ändert nichts
        assert_eq!(p.timeline(Some(3000)), (2450, 3090));
    }

    #[test]
    fn reduziert_statisch_800ms() {
        let p = BootPlan::new(Some("x"), true);
        assert_eq!(p.total_ms(), 800);
        let f = p.frame(0, None);
        assert_eq!(f.lines.len(), 4);
        assert!(!f.open);
        assert!(p.frame(500, None).open);
        assert!(p.frame(800, None).finished);
    }

    #[test]
    fn harter_timeout() {
        assert!(plan().frame(HARD_TIMEOUT_MS, None).finished);
    }
}
