//! Rückfallkette beim Start einer Sendung (Port der Entscheidungen aus `player/index.js`):
//! Direkt (D) → Remux per fMP4-HLS (F) → ts-HLS (T) → ein ts-Neuversuch, höchstens
//! [`MAX_ATTEMPTS`] Starts, nie eine echte Video-Umwandlung. Reine Zustandslogik ohne IO; der
//! Player führt die Schritte aus und meldet die Ergebnisse zurück.
//!
//! Fehlercodes der Kette tragen die Buchstaben: `M3·D>T>T`.

use indexmap::IndexSet;

use crate::stream::FailReason;

/// Höchstzahl der Starts je `play()`.
pub const MAX_ATTEMPTS: u32 = 3;
/// Wartezeit vor dem ts-Neuversuch.
pub const RETRY_DELAY_MS: u64 = 500;
/// Höchstzahl gemerkter Titel je Liste.
pub const REMEMBER_MAX: usize = 200;

/// Art eines Startversuchs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptKind {
    /// Direktwiedergabe.
    Direct,
    /// fMP4-HLS (HEVC angeboten).
    Fmp4,
    /// ts-HLS.
    Ts,
}

impl AttemptKind {
    /// Buchstabe der Kette.
    pub fn letter(self) -> char {
        match self {
            AttemptKind::Direct => 'D',
            AttemptKind::Fmp4 => 'F',
            AttemptKind::Ts => 'T',
        }
    }
}

/// Der aufgelöste Stream eines Versuchs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptInfo {
    /// Direkt (sonst HLS).
    pub direct: bool,
    /// HEVC wurde angeboten (fMP4).
    pub offered_hevc: bool,
    /// Quelle ist h264.
    pub source_h264: bool,
    /// Bildformat-Hinweis der Quelle (`video_format_hint`), vorab berechnet.
    pub format_hint: Option<String>,
}

impl AttemptInfo {
    /// Art des Versuchs.
    pub fn kind(&self) -> AttemptKind {
        if self.direct {
            AttemptKind::Direct
        } else if self.offered_hevc {
            AttemptKind::Fmp4
        } else {
            AttemptKind::Ts
        }
    }
}

/// Nächster Schritt nach einem Fehlschlag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Neu auflösen und noch einmal starten (vorher die alte Sitzung beenden).
    Retry {
        no_direct: bool,
        no_hevc: bool,
        delay_ms: u64,
    },
    /// Aufgeben.
    GiveUp(Failure),
}

/// Endgültiger Fehler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub reason: FailReason,
    /// Code, bei `M<n>`/`B-PLAY` mit Kette (`M3·D>T>T`).
    pub code: Option<String>,
    /// Grund bei ungeeignetem Bildformat.
    pub hint: Option<String>,
}

/// Was nach einem erfolgreichen Start zu merken ist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuccessNotes {
    /// h264 lief mit fMP4.
    pub fmp4_ok: bool,
    /// F scheiterte, T lief: Titel für die Sitzung merken.
    pub remember_fmp4_failed: bool,
    /// ... und dauerhaft (M3/M4 an frisch aufgelöster F-Sitzung).
    pub persist_fmp4_failed: bool,
    /// Direct scheiterte mit M3/M4, Remux lief: dauerhaft merken.
    pub persist_direct_failed: bool,
}

fn is_media_code(code: Option<&str>) -> bool {
    matches!(code, Some("M3" | "M4"))
}

/// `^(M\d+|B-PLAY)$`
fn carries_chain(code: &str) -> bool {
    code == "B-PLAY"
        || (code.len() > 1
            && code.starts_with('M')
            && code[1..].bytes().all(|b| b.is_ascii_digit()))
}

/// Zustand einer Kette (je `play()` eine Instanz).
#[derive(Debug, Clone)]
pub struct FallbackChain {
    chain: Vec<AttemptKind>,
    attempt: u32,
    no_hevc: bool,
    ts_retried: bool,
    from_fmp4: bool,
    fmp4_media_err: bool,
    direct_media_error: bool,
    was_warm_first: bool,
}

impl FallbackChain {
    /// Neue Kette. `no_hevc` = vorgegeben (Titel mit gemerktem fMP4-Fehler),
    /// `first_was_warm` = erster Versuch übernimmt eine Vorwärm-Sitzung.
    pub fn new(no_hevc: bool, first_was_warm: bool) -> Self {
        Self {
            chain: Vec::new(),
            attempt: 0,
            no_hevc,
            ts_retried: false,
            from_fmp4: false,
            fmp4_media_err: false,
            direct_media_error: false,
            was_warm_first: first_was_warm,
        }
    }

    /// Beginnt einen Versuch (nummeriert und trägt den Buchstaben in die Kette ein).
    pub fn begin(&mut self, info: &AttemptInfo) {
        self.attempt += 1;
        self.chain.push(info.kind());
    }

    /// Kette als Text, z. B. `D>T>T`.
    pub fn chain(&self) -> String {
        self.chain
            .iter()
            .map(|k| k.letter().to_string())
            .collect::<Vec<_>>()
            .join(">")
    }

    /// Anzahl der begonnenen Versuche.
    pub fn attempts(&self) -> u32 {
        self.attempt
    }

    /// Erfolg: was gemerkt werden soll.
    pub fn on_success(&self, info: &AttemptInfo) -> SuccessNotes {
        SuccessNotes {
            fmp4_ok: !info.direct && info.offered_hevc && info.source_h264,
            remember_fmp4_failed: self.from_fmp4,
            persist_fmp4_failed: self.from_fmp4 && self.fmp4_media_err,
            persist_direct_failed: self.direct_media_error,
        }
    }

    /// Endgültiger Fehler mit Code samt Kette und Hinweis.
    pub fn finish(
        &self,
        info: &AttemptInfo,
        reason: FailReason,
        code: Option<&str>,
        hint: Option<String>,
    ) -> Failure {
        let code = code.map(|c| {
            if carries_chain(c) {
                format!("{c}·{}", self.chain())
            } else {
                c.to_string()
            }
        });
        let mut hint = hint;
        if hint.is_none()
            && reason == FailReason::Video
            && code.as_deref().is_some_and(|c| c.starts_with("M3"))
        {
            hint.clone_from(&info.format_hint);
        }
        Failure { reason, code, hint }
    }

    /// Entscheidet nach einem Fehlschlag des Versuchs `info`.
    pub fn on_failure(
        &mut self,
        info: &AttemptInfo,
        reason: FailReason,
        code: Option<&str>,
    ) -> Step {
        if !matches!(reason, FailReason::Video | FailReason::Browser)
            || self.attempt >= MAX_ATTEMPTS
        {
            return Step::GiveUp(self.finish(info, reason, code, None));
        }
        let media = reason == FailReason::Video && is_media_code(code);
        if info.direct {
            if media {
                self.direct_media_error = true;
            }
            return Step::Retry {
                no_direct: true,
                no_hevc: self.no_hevc,
                delay_ms: 0,
            };
        }
        if info.offered_hevc && info.source_h264 {
            // M3 mit bekannt ungeeignetem Bildformat: auch ts würde scheitern (oder umwandeln)
            let hint = if reason == FailReason::Video && code == Some("M3") {
                info.format_hint.clone()
            } else {
                None
            };
            if hint.is_some() {
                return Step::GiveUp(self.finish(info, reason, code, hint));
            }
            self.no_hevc = true;
            self.from_fmp4 = true;
            self.fmp4_media_err = !(self.attempt == 1 && self.was_warm_first) && media;
            return Step::Retry {
                no_direct: false,
                no_hevc: true,
                delay_ms: 0,
            };
        }
        if !info.offered_hevc && !self.ts_retried {
            self.ts_retried = true;
            self.no_hevc = true;
            return Step::Retry {
                no_direct: false,
                no_hevc: true,
                delay_ms: RETRY_DELAY_MS,
            };
        }
        Step::GiveUp(self.finish(info, reason, code, None))
    }
}

/// Gemerkte Titel (Sitzung): Direct und fMP4 scheiterten. Reihenfolge deterministisch; läuft die
/// Liste voll, wird sie geleert (wie im Web).
#[derive(Debug, Default)]
pub struct FallbackMemory {
    direct_failed: IndexSet<String>,
    fmp4_failed: IndexSet<String>,
}

impl FallbackMemory {
    /// Leere Merkliste.
    pub fn new() -> Self {
        Self::default()
    }

    /// Titel gleich ohne Direktwiedergabe anfragen?
    pub fn wants_no_direct(&self, item_id: &str, force_hls: bool) -> bool {
        force_hls || self.direct_failed.contains(item_id)
    }

    /// Titel gleich ohne HEVC (ts) anfragen?
    pub fn wants_no_hevc(&self, item_id: &str) -> bool {
        self.fmp4_failed.contains(item_id)
    }

    /// Merkt: Direct scheiterte.
    pub fn remember_direct_failed(&mut self, item_id: &str) {
        if self.direct_failed.len() >= REMEMBER_MAX {
            self.direct_failed.clear();
        }
        self.direct_failed.insert(item_id.to_string());
    }

    /// Merkt: fMP4 scheiterte, ts lief.
    pub fn remember_fmp4_failed(&mut self, item_id: &str) {
        if self.fmp4_failed.len() >= REMEMBER_MAX {
            self.fmp4_failed.clear();
        }
        self.fmp4_failed.insert(item_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> AttemptInfo {
        AttemptInfo {
            direct: true,
            offered_hevc: false,
            source_h264: true,
            format_hint: None,
        }
    }
    fn f() -> AttemptInfo {
        AttemptInfo {
            direct: false,
            offered_hevc: true,
            source_h264: true,
            format_hint: None,
        }
    }
    fn t() -> AttemptInfo {
        AttemptInfo {
            direct: false,
            offered_hevc: false,
            source_h264: true,
            format_hint: None,
        }
    }

    #[test]
    fn kette_direct_dann_ts_dann_ts_neuversuch_mit_code() {
        let mut c = FallbackChain::new(false, false);
        c.begin(&d());
        assert_eq!(
            c.on_failure(&d(), FailReason::Video, Some("M3")),
            Step::Retry {
                no_direct: true,
                no_hevc: false,
                delay_ms: 0
            }
        );
        c.begin(&t());
        assert_eq!(
            c.on_failure(&t(), FailReason::Video, Some("M3")),
            Step::Retry {
                no_direct: false,
                no_hevc: true,
                delay_ms: 500
            }
        );
        c.begin(&t());
        match c.on_failure(&t(), FailReason::Video, Some("M3")) {
            Step::GiveUp(fl) => assert_eq!(fl.code.as_deref(), Some("M3·D>T>T")),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn fmp4_dann_ts_merkt_titel() {
        let mut c = FallbackChain::new(false, false);
        c.begin(&f());
        assert_eq!(
            c.on_failure(&f(), FailReason::Video, Some("M4")),
            Step::Retry {
                no_direct: false,
                no_hevc: true,
                delay_ms: 0
            }
        );
        c.begin(&t());
        let n = c.on_success(&t());
        assert!(n.remember_fmp4_failed && n.persist_fmp4_failed);
        assert_eq!(c.chain(), "F>T");
    }

    #[test]
    fn vorgewaermte_sitzung_wird_nicht_dauerhaft_gemerkt() {
        let mut c = FallbackChain::new(false, true);
        c.begin(&f());
        c.on_failure(&f(), FailReason::Video, Some("M3"));
        c.begin(&t());
        let n = c.on_success(&t());
        assert!(n.remember_fmp4_failed && !n.persist_fmp4_failed);
    }

    #[test]
    fn m3_mit_bildformat_hinweis_bricht_sofort_ab() {
        let mut info = f();
        info.format_hint = Some("Bildformat h264 HDR".into());
        let mut c = FallbackChain::new(false, false);
        c.begin(&info);
        match c.on_failure(&info, FailReason::Video, Some("M3")) {
            Step::GiveUp(fl) => {
                assert_eq!(fl.code.as_deref(), Some("M3·F"));
                assert_eq!(fl.hint.as_deref(), Some("Bildformat h264 HDR"));
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn andere_gruende_geben_sofort_auf_ohne_kette_im_code() {
        let mut c = FallbackChain::new(false, false);
        c.begin(&d());
        match c.on_failure(&d(), FailReason::Network, Some("NET")) {
            Step::GiveUp(fl) => assert_eq!(fl.code.as_deref(), Some("NET")),
            s => panic!("{s:?}"),
        }
        let mut c = FallbackChain::new(false, false);
        c.begin(&d());
        match c.on_failure(&d(), FailReason::Audio, Some("A-CODEC")) {
            Step::GiveUp(fl) => assert_eq!(fl.code.as_deref(), Some("A-CODEC")),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn hoechstens_drei_starts() {
        let mut c = FallbackChain::new(false, false);
        for _ in 0..MAX_ATTEMPTS {
            c.begin(&t());
        }
        assert!(matches!(
            c.on_failure(&t(), FailReason::Video, Some("M3")),
            Step::GiveUp(_)
        ));
    }

    #[test]
    fn direct_m3_wird_nach_remux_erfolg_dauerhaft() {
        let mut c = FallbackChain::new(false, false);
        c.begin(&d());
        c.on_failure(&d(), FailReason::Video, Some("M3"));
        c.begin(&t());
        assert!(c.on_success(&t()).persist_direct_failed);
        let mut c = FallbackChain::new(false, false);
        c.begin(&d());
        c.on_failure(&d(), FailReason::Browser, Some("B-PLAY"));
        c.begin(&t());
        assert!(!c.on_success(&t()).persist_direct_failed);
    }

    #[test]
    fn merkliste_leert_sich_bei_200() {
        let mut m = FallbackMemory::new();
        for i in 0..REMEMBER_MAX {
            m.remember_direct_failed(&format!("i{i}"));
        }
        assert!(m.wants_no_direct("i0", false));
        m.remember_direct_failed("neu");
        assert!(!m.wants_no_direct("i0", false));
        assert!(m.wants_no_direct("neu", false));
        assert!(m.wants_no_direct("egal", true));
        m.remember_fmp4_failed("x");
        assert!(m.wants_no_hevc("x"));
    }
}
