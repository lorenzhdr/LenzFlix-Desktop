//! Player-OSD: reine Zustandslogik ohne UI und ohne Player (alles testbar).
//!
//! Sichtbarkeit und Mauszeiger, Fokus der Bedienelemente, Menüs (Ton, Untertitel, Geschwindigkeit, Qualität, Info),
//! Standard-Spuren nach Benutzer-Einstellung, Überspringen, „Nächste Folge“-Countdown, Trickplay-Kacheln,
//! Untertitelstil und Fehlercodes.

use std::time::{Duration, Instant};

use lfx_player::segments::SegmentKind;
use lfx_player::{PlayerState, Track, TrackChoice, TrackKind};

/// Wie lange das OSD nach der letzten Eingabe sichtbar bleibt.
pub const OSD_HIDE_AFTER: Duration = Duration::from_secs(3);

/// Soll das OSD sichtbar sein? Es bleibt bei Pause, offenem Menü, Fehler, Ladezustand und frischer Eingabe.
pub fn osd_visible(
    now: Instant,
    last_activity: Option<Instant>,
    paused: bool,
    menu_open: bool,
    blocking: bool,
) -> bool {
    if paused || menu_open || blocking {
        return true;
    }
    last_activity.is_some_and(|t| now.saturating_duration_since(t) < OSD_HIDE_AFTER)
}

// ------------------------------------------------------------------ Fokus

/// Bedienelemente, die sich per Tastatur/Gamepad ansteuern lassen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Back,
    Rewind,
    PlayPause,
    Forward,
    Seek,
    Volume,
    Next,
    Subtitles,
    Audio,
    Speed,
    Quality,
    Together, // W5-remote
    Fullscreen,
}

/// Anordnung: Zeile 0 oben (Zurück), 1 Mitte, 2 Zeitleiste, 3 untere Knopfreihe.
pub const ROWS: [&[Control]; 4] = [
    &[Control::Back],
    &[Control::Rewind, Control::PlayPause, Control::Forward],
    &[Control::Seek],
    &[
        Control::Volume,
        Control::Next,
        Control::Subtitles,
        Control::Audio,
        Control::Speed,
        Control::Quality,
        Control::Together, // W5-remote
        Control::Fullscreen,
    ],
];

/// Slot-Nummer für die Oberfläche (`-1` = kein Fokus).
pub fn control_slot(c: Option<Control>) -> i32 {
    match c {
        None => -1,
        Some(Control::Back) => 0,
        Some(Control::Rewind) => 1,
        Some(Control::PlayPause) => 2,
        Some(Control::Forward) => 3,
        Some(Control::Seek) => 4,
        Some(Control::Volume) => 5,
        Some(Control::Next) => 6,
        Some(Control::Subtitles) => 7,
        Some(Control::Audio) => 8,
        Some(Control::Speed) => 9,
        Some(Control::Quality) => 10,
        Some(Control::Fullscreen) => 11,
        Some(Control::Together) => 12, // W5-remote
    }
}

/// Richtung für die Fokussuche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

fn locate(c: Control) -> (usize, usize) {
    for (r, row) in ROWS.iter().enumerate() {
        if let Some(i) = row.iter().position(|x| *x == c) {
            return (r, i);
        }
    }
    (1, 1)
}

/// Nächster Fokus; `has_next` blendet „Nächste Folge“ aus, wenn es keine gibt.
pub fn step_focus(cur: Control, dir: Dir, has_next: bool) -> Control {
    let usable = |c: &Control| has_next || *c != Control::Next;
    let (r, i) = locate(cur);
    match dir {
        Dir::Left | Dir::Right => {
            let row: Vec<Control> = ROWS[r].iter().copied().filter(usable).collect();
            let pos = row.iter().position(|c| *c == cur).unwrap_or(0) as i32;
            let n = pos + if dir == Dir::Left { -1 } else { 1 };
            row.get(n.max(0) as usize).copied().unwrap_or(cur)
        }
        Dir::Up | Dir::Down => {
            let nr = if dir == Dir::Up {
                r.saturating_sub(1)
            } else {
                (r + 1).min(ROWS.len() - 1)
            };
            let row: Vec<Control> = ROWS[nr].iter().copied().filter(usable).collect();
            if row.is_empty() {
                return cur;
            }
            // Mitte bleibt Mitte, sonst gleiche relative Position.
            if nr == 1 {
                return Control::PlayPause;
            }
            row[i.min(row.len() - 1)]
        }
    }
}

// ------------------------------------------------------------------ Menüs

/// Art des geöffneten Menüs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Audio,
    Subtitles,
    Speed,
    Quality,
    Info,
    Together, // W5-remote
}

impl MenuKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Audio => "Tonspur",
            Self::Subtitles => "Untertitel",
            Self::Speed => "Geschwindigkeit",
            Self::Quality => "Qualität",
            Self::Info => "Wiedergabe-Info",
            Self::Together => "Gemeinsam schauen", // W5-remote
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "audio" => Self::Audio,
            "subtitles" => Self::Subtitles,
            "speed" => Self::Speed,
            "quality" => Self::Quality,
            "info" => Self::Info,
            "together" => Self::Together, // W5-remote
            _ => return None,
        })
    }

    pub fn control(self) -> Control {
        match self {
            Self::Audio => Control::Audio,
            Self::Subtitles => Control::Subtitles,
            Self::Speed => Control::Speed,
            Self::Quality | Self::Info => Control::Quality,
            Self::Together => Control::Together, // W5-remote
        }
    }
}

/// Was ein Menüeintrag bewirkt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuAction {
    Track(TrackKind, TrackChoice),
    Speed(f64),
    /// Qualitätsstufe (0 = Original/Automatisch).
    Quality(usize),
    /// Untertitelgröße weiterschalten.
    SubSize,
    /// Untertitelhintergrund umschalten.
    SubBackground,
    /// W5-remote: Eintrag des Menüs „Gemeinsam schauen“.
    Together(TogetherOp),
    /// Nur Anzeige.
    None,
}

/// W5-remote: Aktionen des Menüs „Gemeinsam schauen“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TogetherOp {
    /// Gruppe mit dem aktuellen Titel starten.
    New,
    /// Der n-ten Gruppe der Liste beitreten.
    Join(usize),
    /// Gruppe verlassen.
    Leave,
    /// Liste neu laden.
    Refresh,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub detail: String,
    pub selected: bool,
    pub action: MenuAction,
}

fn item(
    label: impl Into<String>,
    detail: impl Into<String>,
    selected: bool,
    action: MenuAction,
) -> MenuItem {
    MenuItem {
        label: label.into(),
        detail: detail.into(),
        selected,
        action,
    }
}

/// Deutscher Sprachname zu einem ISO-639-Code (2 oder 3 Buchstaben).
pub fn language_name(code: &str) -> Option<&'static str> {
    Some(match code.to_ascii_lowercase().as_str() {
        "de" | "deu" | "ger" => "Deutsch",
        "en" | "eng" => "Englisch",
        "fr" | "fra" | "fre" => "Französisch",
        "es" | "spa" => "Spanisch",
        "it" | "ita" => "Italienisch",
        "ja" | "jpn" => "Japanisch",
        "ko" | "kor" => "Koreanisch",
        "zh" | "zho" | "chi" => "Chinesisch",
        "pt" | "por" => "Portugiesisch",
        "ru" | "rus" => "Russisch",
        "nl" | "nld" | "dut" => "Niederländisch",
        "pl" | "pol" => "Polnisch",
        "tr" | "tur" => "Türkisch",
        "sv" | "swe" => "Schwedisch",
        "da" | "dan" => "Dänisch",
        "no" | "nor" | "nob" => "Norwegisch",
        "fi" | "fin" => "Finnisch",
        "cs" | "ces" | "cze" => "Tschechisch",
        "hu" | "hun" => "Ungarisch",
        "ar" | "ara" => "Arabisch",
        "hi" | "hin" => "Hindi",
        "und" => "Unbekannt",
        _ => return None,
    })
}

fn same_language(a: &str, b: &str) -> bool {
    match (language_name(a), language_name(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// Anzeigename einer Spur: „Deutsch · Titel“, sonst „Spur n“.
pub fn track_label(t: &Track, index_in_kind: usize) -> String {
    let lang = t
        .lang
        .as_deref()
        .map(|l| language_name(l).map_or_else(|| l.to_owned(), str::to_owned));
    let mut s = match (lang, t.title.as_deref().filter(|x| !x.is_empty())) {
        (Some(l), Some(title)) if !title.eq_ignore_ascii_case(&l) => format!("{l} · {title}"),
        (Some(l), _) => l,
        (None, Some(title)) => title.to_owned(),
        (None, None) => format!("Spur {}", index_in_kind + 1),
    };
    if t.forced {
        s.push_str(" (erzwungen)");
    }
    s
}

fn track_detail(t: &Track) -> String {
    let mut parts = Vec::new();
    if let Some(c) = t.codec.as_deref().filter(|c| !c.is_empty()) {
        parts.push(c.to_ascii_uppercase());
    }
    if t.external {
        parts.push("extern".to_owned());
    }
    parts.join(" · ")
}

/// Tonspuren-Menü.
pub fn audio_menu(tracks: &[Track]) -> Vec<MenuItem> {
    tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .enumerate()
        .map(|(i, t)| {
            item(
                track_label(t, i),
                track_detail(t),
                t.selected,
                MenuAction::Track(TrackKind::Audio, TrackChoice::Id(t.id)),
            )
        })
        .collect()
}

/// Stil der Untertitel (Benutzer-Einstellung).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubStyle {
    /// Größe in Prozent (50..=200).
    pub size_pct: u32,
    /// Halbdurchsichtiger Hintergrundkasten.
    pub background: bool,
}

impl Default for SubStyle {
    fn default() -> Self {
        Self {
            size_pct: 100,
            background: false,
        }
    }
}

const SUB_SIZES: [u32; 6] = [75, 100, 125, 150, 175, 200];

impl SubStyle {
    /// Nächste Größe (läuft im Kreis).
    pub fn next_size(self) -> Self {
        let i = SUB_SIZES.iter().position(|s| *s == self.size_pct);
        let n = match i {
            Some(i) => SUB_SIZES[(i + 1) % SUB_SIZES.len()],
            None => 100,
        };
        Self {
            size_pct: n,
            ..self
        }
    }

    pub fn toggle_background(self) -> Self {
        Self {
            background: !self.background,
            ..self
        }
    }

    /// mpv-Wert für `sub-scale`.
    pub fn mpv_scale(self) -> f64 {
        f64::from(self.size_pct.clamp(50, 200)) / 100.0
    }

    /// mpv-Optionen (Name, Wert) für den Stil.
    pub fn mpv_options(self) -> Vec<(&'static str, String)> {
        vec![
            ("sub-scale", format!("{:.2}", self.mpv_scale())),
            (
                "sub-border-style",
                if self.background {
                    "background-box"
                } else {
                    "outline-and-shadow"
                }
                .to_owned(),
            ),
            (
                "sub-back-color",
                if self.background {
                    "#99000000"
                } else {
                    "#00000000"
                }
                .to_owned(),
            ),
        ]
    }

    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({"size": self.size_pct, "background": self.background})
    }

    pub fn from_json(v: &serde_json::Value) -> Self {
        let d = Self::default();
        Self {
            size_pct: v["size"]
                .as_u64()
                .map_or(d.size_pct, |s| (s as u32).clamp(50, 200)),
            background: v["background"].as_bool().unwrap_or(d.background),
        }
    }
}

/// Untertitel-Menü: „Aus“, alle Spuren, dann Stilzeilen.
pub fn subtitle_menu(tracks: &[Track], style: SubStyle) -> Vec<MenuItem> {
    let subs: Vec<&Track> = tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Subtitle)
        .collect();
    let any = subs.iter().any(|t| t.selected);
    let mut v = vec![item(
        "Aus",
        "",
        !any,
        MenuAction::Track(TrackKind::Subtitle, TrackChoice::Off),
    )];
    for (i, t) in subs.iter().enumerate() {
        v.push(item(
            track_label(t, i),
            track_detail(t),
            t.selected,
            MenuAction::Track(TrackKind::Subtitle, TrackChoice::Id(t.id)),
        ));
    }
    v.push(item(
        "Größe",
        format!("{} %", style.size_pct),
        false,
        MenuAction::SubSize,
    ));
    v.push(item(
        "Hintergrund",
        if style.background { "An" } else { "Aus" },
        false,
        MenuAction::SubBackground,
    ));
    v
}

/// Geschwindigkeiten des Menüs.
pub const SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

pub fn speed_label(s: f64) -> String {
    if (s - 1.0).abs() < 1e-9 {
        "Normal".to_owned()
    } else {
        format!("{s}×").replace('.', ",")
    }
}

pub fn speed_menu(current: f64) -> Vec<MenuItem> {
    SPEEDS
        .iter()
        .map(|s| {
            item(
                speed_label(*s),
                "",
                (current - s).abs() < 1e-6,
                MenuAction::Speed(*s),
            )
        })
        .collect()
}

/// Qualitätsstufen: (Anzeige, Höchstbitrate in Bit/s; `None` = Original).
pub const QUALITIES: [(&str, Option<u64>); 5] = [
    ("Original (Direktwiedergabe)", None),
    ("Hoch · 20 Mbit/s", Some(20_000_000)),
    ("Mittel · 8 Mbit/s", Some(8_000_000)),
    ("Niedrig · 4 Mbit/s", Some(4_000_000)),
    ("Sparsam · 1,5 Mbit/s", Some(1_500_000)),
];

pub fn quality_menu(current: usize) -> Vec<MenuItem> {
    QUALITIES
        .iter()
        .enumerate()
        .map(|(i, (l, _))| item(*l, "", i == current, MenuAction::Quality(i)))
        .collect()
}

/// Info-Zeilen (Bezeichnung, Wert) zur laufenden Wiedergabe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InfoFacts {
    pub method: String,
    pub hdr_state: String,
}

pub fn info_menu(s: &PlayerState, facts: &InfoFacts) -> Vec<MenuItem> {
    let mut rows: Vec<(String, String)> = Vec::new();
    rows.push(("Wiedergabeart".into(), facts.method.clone()));
    if s.video.width > 0 {
        rows.push((
            "Video".into(),
            format!("{} × {}", s.video.width, s.video.height),
        ));
    }
    rows.push((
        "Decoder".into(),
        s.hwdec
            .clone()
            .map_or_else(|| "Software".to_owned(), |h| format!("Hardware ({h})")),
    ));
    rows.push(("HDR".into(), facts.hdr_state.clone()));
    if let Some(a) = s.selected(TrackKind::Audio) {
        rows.push((
            "Ton".into(),
            format!(
                "{}{}",
                track_label(a, 0),
                a.codec
                    .as_deref()
                    .map(|c| format!(" ({})", c.to_ascii_uppercase()))
                    .unwrap_or_default()
            ),
        ));
    }
    rows.push(("Geschwindigkeit".into(), speed_label(s.speed)));
    rows.into_iter()
        .map(|(l, d)| item(l, d, false, MenuAction::None))
        .collect()
}

// ------------------------------------------------------------------ Standard-Spuren

/// Untertitelmodus aus der Benutzerkonfiguration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubMode {
    #[default]
    Default,
    Smart,
    Always,
    OnlyForced,
    None,
}

impl SubMode {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("Smart") => Self::Smart,
            Some("Always") => Self::Always,
            Some("OnlyForced") => Self::OnlyForced,
            Some("None") => Self::None,
            _ => Self::Default,
        }
    }
}

/// Standard-Sprachen des Benutzers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackPrefs {
    pub audio_lang: Option<String>,
    pub sub_lang: Option<String>,
    pub sub_mode: SubMode,
}

/// Wählt nach dem Laden Ton und Untertitel. `None` = nichts ändern.
pub fn pick_default_tracks(
    tracks: &[Track],
    prefs: &TrackPrefs,
) -> (Option<TrackChoice>, Option<TrackChoice>) {
    let audio: Vec<&Track> = tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .collect();
    let subs: Vec<&Track> = tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Subtitle)
        .collect();
    let audio_pick = prefs.audio_lang.as_deref().and_then(|l| {
        let t = audio
            .iter()
            .find(|t| t.lang.as_deref().is_some_and(|x| same_language(x, l)))?;
        (!t.selected).then_some(TrackChoice::Id(t.id))
    });
    let audio_lang_now = audio_pick
        .and_then(|c| match c {
            TrackChoice::Id(id) => audio.iter().find(|t| t.id == id),
            _ => None,
        })
        .or_else(|| audio.iter().find(|t| t.selected))
        .and_then(|t| t.lang.clone());
    let preferred_sub = |forced_only: bool| {
        subs.iter()
            .filter(|t| !forced_only || t.forced)
            .find(|t| {
                prefs
                    .sub_lang
                    .as_deref()
                    .is_none_or(|l| t.lang.as_deref().is_some_and(|x| same_language(x, l)))
            })
            .map(|t| TrackChoice::Id(t.id))
    };
    let sub_pick = match prefs.sub_mode {
        SubMode::None => Some(TrackChoice::Off),
        SubMode::Always => preferred_sub(false).or(Some(TrackChoice::Off)),
        SubMode::OnlyForced => preferred_sub(true).or(Some(TrackChoice::Off)),
        SubMode::Smart => {
            let differs = match (&prefs.sub_lang, &audio_lang_now) {
                (Some(s), Some(a)) => !same_language(s, a),
                (Some(_), None) => true,
                _ => false,
            };
            if differs {
                preferred_sub(false).or(Some(TrackChoice::Off))
            } else {
                Some(TrackChoice::Off)
            }
        }
        SubMode::Default => subs
            .iter()
            .find(|t| t.default || t.forced)
            .map(|t| TrackChoice::Id(t.id)),
    };
    (audio_pick, sub_pick)
}

// ------------------------------------------------------------------ Überspringen, Folge, Resume

pub fn skip_label(kind: SegmentKind) -> &'static str {
    match kind {
        SegmentKind::Intro => "Intro überspringen",
        SegmentKind::Outro => "Abspann überspringen",
        SegmentKind::Recap => "Rückblick überspringen",
        SegmentKind::Preview => "Vorschau überspringen",
        SegmentKind::Commercial => "Werbung überspringen",
    }
}

/// Dauer des „Nächste Folge“-Countdowns.
pub const UP_NEXT_SECS: u64 = 10;

/// Zustand der „Nächste Folge“-Karte.
#[derive(Debug, Clone, Copy, Default)]
pub struct UpNext {
    shown_at: Option<Instant>,
    dismissed: bool,
}

impl UpNext {
    pub fn show(&mut self, now: Instant) {
        if self.shown_at.is_none() && !self.dismissed {
            self.shown_at = Some(now);
        }
    }

    pub fn dismiss(&mut self) {
        self.shown_at = None;
        self.dismissed = true;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn visible(&self) -> bool {
        self.shown_at.is_some()
    }

    pub fn dismissed(&self) -> bool {
        self.dismissed
    }

    /// Restsekunden (aufgerundet), `None` solange die Karte nicht steht.
    pub fn remaining(&self, now: Instant) -> Option<u64> {
        let t = self.shown_at?;
        let left =
            Duration::from_secs(UP_NEXT_SECS).saturating_sub(now.saturating_duration_since(t));
        Some(left.as_secs() + u64::from(left.subsec_nanos() > 0))
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.remaining(now) == Some(0)
    }
}

/// Fortsetzen anbieten? Nur bei nennenswerter Position, nicht kurz vor dem Ende.
pub fn resume_offer(position_secs: f64, runtime_secs: f64) -> Option<f64> {
    if position_secs < 15.0 {
        return None;
    }
    if runtime_secs > 0.0 && position_secs >= runtime_secs * 0.95 {
        return None;
    }
    Some(position_secs)
}

// ------------------------------------------------------------------ Trickplay

/// Eine Vorschau in einem Kachelbild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrickCell {
    /// Nummer des Kachelbilds (ab 0).
    pub tile: u32,
    /// Ausschnitt in Pixeln innerhalb des Kachelbilds.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Parameter des Trickplay (aus `TrickplayInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrickGeom {
    pub width: u32,
    pub height: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    pub count: u32,
    pub interval_ms: u32,
}

/// Zelle zur Zeit `secs`; `None` ohne gültige Angaben.
pub fn trick_cell(g: &TrickGeom, secs: f64) -> Option<TrickCell> {
    if g.tile_w == 0 || g.tile_h == 0 || g.interval_ms == 0 || g.count == 0 {
        return None;
    }
    let idx = ((secs.max(0.0) * 1000.0) / f64::from(g.interval_ms)).floor() as u32;
    let idx = idx.min(g.count - 1);
    let per_tile = g.tile_w * g.tile_h;
    let tile = idx / per_tile;
    let within = idx % per_tile;
    Some(TrickCell {
        tile,
        x: (within % g.tile_w) * g.width,
        y: (within / g.tile_w) * g.height,
        w: g.width,
        h: g.height,
    })
}

// ------------------------------------------------------------------ Fehlercodes

/// Stufe, in der die Wiedergabe scheiterte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailStage {
    /// Vorbereitung (PlaybackInfo, keine Quelle).
    Prepare,
    /// Player/mpv.
    Engine,
    /// Videoausgabe.
    Output,
}

/// Kurzcode für den Support, z. B. `P0`, `M-7`, `V0`. Enthält nie Adressen oder Tokens.
pub fn error_code(stage: FailStage, mpv_code: i32) -> String {
    match stage {
        FailStage::Prepare => "P0".to_owned(),
        FailStage::Engine => format!("M{mpv_code}"),
        FailStage::Output => "V0".to_owned(),
    }
}

/// `S1:F3 Titel` für Folgen, sonst der Titel.
pub fn episode_label(season: Option<i32>, episode: Option<i32>, name: &str) -> String {
    match (season, episode) {
        (Some(s), Some(e)) => format!("S{s}:F{e} {name}"),
        (None, Some(e)) => format!("F{e} {name}"),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: i64, kind: TrackKind, lang: &str, selected: bool) -> Track {
        Track {
            id,
            kind,
            lang: Some(lang.into()),
            title: None,
            codec: Some("aac".into()),
            default: false,
            forced: false,
            external: false,
            selected,
        }
    }

    #[test]
    fn osd_hides_after_inactivity_but_not_when_paused_or_menu() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(4);
        assert!(osd_visible(t0, Some(t0), false, false, false));
        assert!(!osd_visible(later, Some(t0), false, false, false));
        assert!(osd_visible(later, Some(t0), true, false, false));
        assert!(osd_visible(later, Some(t0), false, true, false));
        assert!(osd_visible(later, None, false, false, true));
        assert!(!osd_visible(later, None, false, false, false));
    }

    #[test]
    fn focus_moves_through_rows_and_skips_missing_next() {
        assert_eq!(
            step_focus(Control::PlayPause, Dir::Right, true),
            Control::Forward
        );
        assert_eq!(
            step_focus(Control::Forward, Dir::Right, true),
            Control::Forward
        );
        assert_eq!(
            step_focus(Control::PlayPause, Dir::Down, true),
            Control::Seek
        );
        assert_eq!(step_focus(Control::Seek, Dir::Down, true), Control::Volume);
        assert_eq!(
            step_focus(Control::Volume, Dir::Right, false),
            Control::Subtitles
        );
        assert_eq!(step_focus(Control::Volume, Dir::Right, true), Control::Next);
        assert_eq!(step_focus(Control::Seek, Dir::Up, true), Control::PlayPause);
        assert_eq!(step_focus(Control::PlayPause, Dir::Up, true), Control::Back);
        assert_eq!(step_focus(Control::Back, Dir::Up, true), Control::Back);
        assert_eq!(control_slot(None), -1);
        assert_eq!(control_slot(Some(Control::Fullscreen)), 11);
    }

    #[test]
    fn labels_use_german_language_names() {
        let t = track(1, TrackKind::Audio, "ger", true);
        assert_eq!(track_label(&t, 0), "Deutsch");
        let mut t2 = track(2, TrackKind::Audio, "eng", false);
        t2.title = Some("Kommentar".into());
        assert_eq!(track_label(&t2, 1), "Englisch · Kommentar");
        let mut t3 = track(3, TrackKind::Subtitle, "xx", false);
        t3.lang = None;
        assert_eq!(track_label(&t3, 2), "Spur 3");
        t3.forced = true;
        assert!(track_label(&t3, 0).ends_with("(erzwungen)"));
    }

    #[test]
    fn subtitle_menu_has_off_tracks_and_style_rows() {
        let tracks = [
            track(1, TrackKind::Audio, "ger", true),
            track(1, TrackKind::Subtitle, "ger", false),
        ];
        let m = subtitle_menu(&tracks, SubStyle::default());
        assert_eq!(m.len(), 4);
        assert!(m[0].selected);
        assert_eq!(m[2].action, MenuAction::SubSize);
        assert_eq!(m[2].detail, "100 %");
        let tracks2 = [track(1, TrackKind::Subtitle, "ger", true)];
        let m2 = subtitle_menu(&tracks2, SubStyle::default());
        assert!(!m2[0].selected && m2[1].selected);
        assert_eq!(audio_menu(&tracks).len(), 1);
    }

    #[test]
    fn sub_style_cycles_and_roundtrips() {
        let s = SubStyle::default().next_size();
        assert_eq!(s.size_pct, 125);
        let mut s = SubStyle {
            size_pct: 200,
            background: false,
        };
        s = s.next_size();
        assert_eq!(s.size_pct, 75);
        let s = s.toggle_background();
        assert!(s.background);
        assert_eq!(SubStyle::from_json(&s.to_json()), s);
        assert_eq!(
            SubStyle::from_json(&serde_json::json!({"size": 9999})).size_pct,
            200
        );
        assert!(
            s.mpv_options()
                .iter()
                .any(|(k, v)| *k == "sub-border-style" && v == "background-box")
        );
    }

    #[test]
    fn speed_menu_marks_current() {
        let m = speed_menu(1.5);
        assert_eq!(m.iter().filter(|i| i.selected).count(), 1);
        assert_eq!(speed_label(1.0), "Normal");
        assert_eq!(speed_label(1.25), "1,25×");
        assert!(quality_menu(0)[0].selected);
    }

    #[test]
    fn default_tracks_follow_user_preferences() {
        let tracks = [
            track(1, TrackKind::Audio, "eng", true),
            track(2, TrackKind::Audio, "ger", false),
            track(1, TrackKind::Subtitle, "ger", false),
        ];
        let prefs = TrackPrefs {
            audio_lang: Some("deu".into()),
            sub_lang: Some("ger".into()),
            sub_mode: SubMode::Smart,
        };
        let (a, s) = pick_default_tracks(&tracks, &prefs);
        assert_eq!(a, Some(TrackChoice::Id(2)));
        // Ton ist bereits Deutsch -> Smart zeigt keine Untertitel.
        assert_eq!(s, Some(TrackChoice::Off));
        let prefs = TrackPrefs {
            audio_lang: None,
            sub_lang: Some("ger".into()),
            sub_mode: SubMode::Smart,
        };
        // Ton Englisch, Untertitel Deutsch gewünscht -> Untertitel an.
        let (a, s) = pick_default_tracks(&tracks, &prefs);
        assert_eq!(a, None);
        assert_eq!(s, Some(TrackChoice::Id(1)));
        let always = TrackPrefs {
            sub_mode: SubMode::Always,
            ..prefs.clone()
        };
        assert_eq!(
            pick_default_tracks(&tracks, &always).1,
            Some(TrackChoice::Id(1))
        );
        let none = TrackPrefs {
            sub_mode: SubMode::None,
            ..prefs.clone()
        };
        assert_eq!(
            pick_default_tracks(&tracks, &none).1,
            Some(TrackChoice::Off)
        );
        let forced = TrackPrefs {
            sub_mode: SubMode::OnlyForced,
            ..prefs
        };
        assert_eq!(
            pick_default_tracks(&tracks, &forced).1,
            Some(TrackChoice::Off)
        );
        assert_eq!(SubMode::parse(Some("Smart")), SubMode::Smart);
        assert_eq!(SubMode::parse(None), SubMode::Default);
    }

    #[test]
    fn up_next_counts_down_once() {
        let t0 = Instant::now();
        let mut u = UpNext::default();
        assert!(!u.visible());
        assert_eq!(u.remaining(t0), None);
        u.show(t0);
        assert_eq!(u.remaining(t0), Some(10));
        assert_eq!(u.remaining(t0 + Duration::from_millis(2500)), Some(8));
        assert!(!u.expired(t0 + Duration::from_secs(9)));
        assert!(u.expired(t0 + Duration::from_secs(10)));
        u.dismiss();
        assert!(!u.visible());
        u.show(t0);
        assert!(!u.visible(), "nach Abbruch nicht erneut");
        u.reset();
        u.show(t0);
        assert!(u.visible());
    }

    #[test]
    fn resume_offer_rules() {
        assert_eq!(resume_offer(5.0, 3000.0), None);
        assert_eq!(resume_offer(754.0, 3000.0), Some(754.0));
        assert_eq!(resume_offer(2900.0, 3000.0), None);
        assert_eq!(resume_offer(100.0, 0.0), Some(100.0));
    }

    #[test]
    fn trickplay_cell_math() {
        let g = TrickGeom {
            width: 320,
            height: 180,
            tile_w: 10,
            tile_h: 10,
            count: 360,
            interval_ms: 10_000,
        };
        let c = trick_cell(&g, 0.0).unwrap();
        assert_eq!((c.tile, c.x, c.y), (0, 0, 0));
        let c = trick_cell(&g, 95.0).unwrap(); // Index 9
        assert_eq!((c.tile, c.x, c.y), (0, 9 * 320, 0));
        let c = trick_cell(&g, 100.0).unwrap(); // Index 10: zweite Zeile
        assert_eq!((c.tile, c.x, c.y), (0, 0, 180));
        let c = trick_cell(&g, 1000.0).unwrap(); // Index 100: zweites Kachelbild
        assert_eq!((c.tile, c.x, c.y), (1, 0, 0));
        let c = trick_cell(&g, 99999.0).unwrap(); // geklemmt auf Index 359
        assert_eq!(c.tile, 3);
        assert!(
            trick_cell(
                &TrickGeom {
                    interval_ms: 0,
                    ..g
                },
                1.0
            )
            .is_none()
        );
    }

    #[test]
    fn labels_codes_and_episode_text() {
        assert_eq!(skip_label(SegmentKind::Intro), "Intro überspringen");
        assert_eq!(error_code(FailStage::Engine, -7), "M-7");
        assert_eq!(error_code(FailStage::Prepare, 0), "P0");
        assert_eq!(
            episode_label(Some(1), Some(3), "Der Nebel"),
            "S1:F3 Der Nebel"
        );
        assert_eq!(episode_label(None, None, "Film"), "Film");
        assert_eq!(MenuKind::from_name("subtitles"), Some(MenuKind::Subtitles));
        assert_eq!(MenuKind::Quality.control(), Control::Quality);
    }
}
