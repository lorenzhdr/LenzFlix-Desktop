//! W4-account: reine Logik der Konto-, Einstellungs- und Verwaltungsseiten (Tabellen, Auswahllisten, Formate).
//! Nichts hier berührt Oberfläche oder Netzwerk, damit es sich einfach testen lässt.

use lfx_api::{SessionInfo, TaskInfo, UserConfiguration, UserPolicy};
use serde_json::Value;

/// Art einer Zeile (Reihenfolge = Slint-`AcRow.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Heading = 0,
    Toggle = 1,
    Choice = 2,
    Button = 3,
    Field = 4,
    Info = 5,
    Item = 6,
    Note = 7,
    Preview = 8,
}

impl RowKind {
    /// Kann die Zeile den Fokus bekommen?
    pub fn focusable(self) -> bool {
        matches!(
            self,
            Self::Toggle | Self::Choice | Self::Button | Self::Field | Self::Item
        )
    }
}

/// Bereiche der Benutzer-Einstellungen (Seitenleiste).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetSec {
    Display,
    Home,
    Playback,
    Subtitles,
    Controls,
    Profile,
    QuickConnect,
    Servers,
    Admin,
    About,
    SignOut,
}

impl SetSec {
    pub fn label(self) -> &'static str {
        match self {
            Self::Display => "Anzeige",
            Self::Home => "Startseite",
            Self::Playback => "Wiedergabe",
            Self::Subtitles => "Untertitel",
            Self::Controls => "Steuerung",
            Self::Profile => "Profil",
            Self::QuickConnect => "Quick Connect",
            Self::Servers => "Konten und Server",
            Self::Admin => "Verwaltung",
            Self::About => "Über LENZFLIX",
            Self::SignOut => "Abmelden",
        }
    }

    /// Reihenfolge der Seitenleiste; „Verwaltung“ nur für Administratoren.
    pub fn list(is_admin: bool) -> Vec<Self> {
        let mut v = vec![
            Self::Display,
            Self::Home,
            Self::Playback,
            Self::Subtitles,
            Self::Controls,
            Self::Profile,
            Self::QuickConnect,
            Self::Servers,
        ];
        if is_admin {
            v.push(Self::Admin);
        }
        v.push(Self::About);
        v.push(Self::SignOut);
        v
    }

    /// Hat der Eintrag eine Inhaltsliste (sonst löst Eingabe eine Aktion aus)?
    pub fn has_content(self) -> bool {
        !matches!(self, Self::Admin | Self::About | Self::SignOut)
    }
}

/// Bereiche der Verwaltung.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmSec {
    Overview,
    Users,
    Libraries,
    Tasks,
    Activity,
    Logs,
    Devices,
    Server,
    Back,
}

impl AdmSec {
    pub const ALL: [Self; 9] = [
        Self::Overview,
        Self::Users,
        Self::Libraries,
        Self::Tasks,
        Self::Activity,
        Self::Logs,
        Self::Devices,
        Self::Server,
        Self::Back,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Übersicht",
            Self::Users => "Benutzer",
            Self::Libraries => "Bibliotheken",
            Self::Tasks => "Geplante Aufgaben",
            Self::Activity => "Aktivität",
            Self::Logs => "Protokolle",
            Self::Devices => "Geräte",
            Self::Server => "Server",
            Self::Back => "‹ Einstellungen",
        }
    }
}

// ---------------------------------------------------------------- Lokale Einstellungen

/// Art einer app-lokalen Einstellung.
pub enum LocalKind {
    /// Schalter mit Standardwert.
    Toggle(bool),
    /// Auswahl aus (gespeicherter Wert, Beschriftung); letzter Wert ist der Standardindex.
    Choice(&'static [(&'static str, &'static str)], usize),
}

/// Eine Einstellung, die nur in `lfx-store` liegt (je Server und Benutzer).
pub struct LocalDef {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: LocalKind,
}

pub const KEY_COUCH: &str = "ui.couch";
pub const KEY_SOUND: &str = "ui.retro_sound";
pub const KEY_UPD_AUTO: &str = "update.auto";
pub const KEY_UPD_CHANNEL: &str = "update.channel";
pub const KEY_BITRATE: &str = "playback.max_bitrate";
pub const KEY_SUB_SIZE: &str = "sub.size";
pub const KEY_SUB_COLOR: &str = "sub.color";
pub const KEY_SUB_BG: &str = "sub.bg";
pub const KEY_PAD: &str = "ctl.gamepad";
pub const KEY_SEEK_FWD: &str = "ctl.seek_fwd";
pub const KEY_SEEK_BACK: &str = "ctl.seek_back";

const BITRATES: &[(&str, &str)] = &[
    ("0", "Automatisch"),
    ("120000000", "120 Mbit/s"),
    ("80000000", "80 Mbit/s"),
    ("40000000", "40 Mbit/s"),
    ("20000000", "20 Mbit/s"),
    ("10000000", "10 Mbit/s"),
    ("8000000", "8 Mbit/s"),
    ("4000000", "4 Mbit/s"),
    ("2000000", "2 Mbit/s"),
    ("1000000", "1 Mbit/s"),
];
const SUB_SIZES: &[(&str, &str)] = &[
    ("s", "Klein"),
    ("m", "Normal"),
    ("l", "Groß"),
    ("xl", "Sehr groß"),
];
const SUB_COLORS: &[(&str, &str)] = &[("white", "Weiß"), ("yellow", "Gelb"), ("cyan", "Hellblau")];
const SUB_BGS: &[(&str, &str)] = &[
    ("none", "Keiner"),
    ("shadow", "Schatten"),
    ("box", "Kasten"),
];
const SEEK_FWD: &[(&str, &str)] = &[
    ("10", "10 s"),
    ("15", "15 s"),
    ("30", "30 s"),
    ("60", "60 s"),
];
const SEEK_BACK: &[(&str, &str)] = &[("5", "5 s"), ("10", "10 s"), ("15", "15 s"), ("30", "30 s")];
const CHANNELS: &[(&str, &str)] = &[("stable", "Stabil"), ("pre", "Vorabversionen")];

pub const LOCALS: &[LocalDef] = &[
    LocalDef {
        key: KEY_COUCH,
        label: "Couch-Modus (große Bedienung)",
        kind: LocalKind::Toggle(false),
    },
    LocalDef {
        key: KEY_SOUND,
        label: "Klänge in LENZ//84",
        kind: LocalKind::Toggle(false),
    },
    LocalDef {
        key: KEY_UPD_AUTO,
        label: "Automatisch nach Updates suchen",
        kind: LocalKind::Toggle(true),
    },
    LocalDef {
        key: KEY_UPD_CHANNEL,
        label: "Update-Kanal",
        kind: LocalKind::Choice(CHANNELS, 0),
    },
    LocalDef {
        key: KEY_BITRATE,
        label: "Maximale Bitrate",
        kind: LocalKind::Choice(BITRATES, 0),
    },
    LocalDef {
        key: KEY_SUB_SIZE,
        label: "Schriftgröße",
        kind: LocalKind::Choice(SUB_SIZES, 1),
    },
    LocalDef {
        key: KEY_SUB_COLOR,
        label: "Schriftfarbe",
        kind: LocalKind::Choice(SUB_COLORS, 0),
    },
    LocalDef {
        key: KEY_SUB_BG,
        label: "Hintergrund",
        kind: LocalKind::Choice(SUB_BGS, 1),
    },
    LocalDef {
        key: KEY_PAD,
        label: "Gamepad verwenden",
        kind: LocalKind::Toggle(true),
    },
    LocalDef {
        key: KEY_SEEK_FWD,
        label: "Vorspulen um",
        kind: LocalKind::Choice(SEEK_FWD, 0),
    },
    LocalDef {
        key: KEY_SEEK_BACK,
        label: "Zurückspulen um",
        kind: LocalKind::Choice(SEEK_BACK, 1),
    },
];

pub fn local_def(key: &str) -> Option<&'static LocalDef> {
    LOCALS.iter().find(|d| d.key == key)
}

/// Aktueller Schalterwert (gespeichert oder Standard).
pub fn local_bool(def: &LocalDef, stored: Option<&Value>) -> bool {
    let default = matches!(def.kind, LocalKind::Toggle(true));
    stored.and_then(Value::as_bool).unwrap_or(default)
}

/// Aktueller Auswahlindex (gespeichert oder Standard).
pub fn local_index(def: &LocalDef, stored: Option<&Value>) -> usize {
    match def.kind {
        LocalKind::Choice(opts, default) => stored
            .and_then(Value::as_str)
            .and_then(|s| opts.iter().position(|(v, _)| *v == s))
            .unwrap_or(default),
        LocalKind::Toggle(_) => 0,
    }
}

/// Beschriftung des aktuellen Werts.
pub fn local_label(def: &LocalDef, stored: Option<&Value>) -> &'static str {
    match def.kind {
        LocalKind::Choice(opts, _) => opts[local_index(def, stored)].1,
        LocalKind::Toggle(_) => "",
    }
}

/// Wert nach einem Schritt in Richtung `dir` (umlaufend).
pub fn cycle(idx: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    ((idx as i64 + i64::from(dir)).rem_euclid(len as i64)) as usize
}

/// Neuer gespeicherter Wert nach einem Schritt.
pub fn local_step(def: &LocalDef, stored: Option<&Value>, dir: i32) -> Value {
    match def.kind {
        LocalKind::Toggle(_) => Value::Bool(!local_bool(def, stored)),
        LocalKind::Choice(opts, _) => {
            let i = cycle(local_index(def, stored), opts.len(), dir);
            Value::String(opts[i].0.to_owned())
        }
    }
}

/// Schriftgröße der Vorschau in Pixeln.
pub fn preview_size(code: &str) -> f32 {
    match code {
        "s" => 14.0,
        "l" => 24.0,
        "xl" => 30.0,
        _ => 18.0,
    }
}

/// Farbe der Vorschau (RGB).
pub fn preview_rgb(code: &str) -> (u8, u8, u8) {
    match code {
        "yellow" => (0xff, 0xe0, 0x4a),
        "cyan" => (0x7f, 0xdb, 0xff),
        _ => (0xff, 0xff, 0xff),
    }
}

/// Hintergrund der Vorschau (0 keiner, 1 Schatten, 2 Kasten).
pub fn preview_bg(code: &str) -> i32 {
    match code {
        "none" => 0,
        "box" => 2,
        _ => 1,
    }
}

// ---------------------------------------------------------------- Server-Konfiguration

/// Schalter der Benutzerkonfiguration (Server).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfgToggle {
    HidePlayedInLatest,
    DisplayMissingEpisodes,
    PlayDefaultAudio,
    RememberAudio,
    RememberSubs,
    NextEpisode,
}

impl CfgToggle {
    pub fn label(self) -> &'static str {
        match self {
            Self::HidePlayedInLatest => "Gesehenes in „Neu hinzugefügt“ ausblenden",
            Self::DisplayMissingEpisodes => "Fehlende Folgen anzeigen",
            Self::PlayDefaultAudio => "Standard-Tonspur automatisch wählen",
            Self::RememberAudio => "Tonspur-Auswahl merken",
            Self::RememberSubs => "Untertitel-Auswahl merken",
            Self::NextEpisode => "Nächste Folge automatisch abspielen",
        }
    }
    pub fn get(self, c: &UserConfiguration) -> bool {
        match self {
            Self::HidePlayedInLatest => c.hide_played_in_latest,
            Self::DisplayMissingEpisodes => c.display_missing_episodes,
            Self::PlayDefaultAudio => c.play_default_audio_track,
            Self::RememberAudio => c.remember_audio_selections,
            Self::RememberSubs => c.remember_subtitle_selections,
            Self::NextEpisode => c.enable_next_episode_auto_play,
        }
    }
    pub fn set(self, c: &mut UserConfiguration, on: bool) {
        match self {
            Self::HidePlayedInLatest => c.hide_played_in_latest = on,
            Self::DisplayMissingEpisodes => c.display_missing_episodes = on,
            Self::PlayDefaultAudio => c.play_default_audio_track = on,
            Self::RememberAudio => c.remember_audio_selections = on,
            Self::RememberSubs => c.remember_subtitle_selections = on,
            Self::NextEpisode => c.enable_next_episode_auto_play = on,
        }
    }
}

/// Auswahlfelder der Benutzerkonfiguration (Server).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfgChoice {
    AudioLang,
    SubLang,
    SubMode,
}

pub const LANGS: &[(&str, &str)] = &[
    ("", "Automatisch"),
    ("ger", "Deutsch"),
    ("eng", "Englisch"),
    ("fre", "Französisch"),
    ("spa", "Spanisch"),
    ("ita", "Italienisch"),
    ("jpn", "Japanisch"),
    ("tur", "Türkisch"),
];
pub const SUB_MODES: &[(&str, &str)] = &[
    ("Default", "Standard"),
    ("Smart", "Intelligent"),
    ("OnlyForced", "Nur erzwungene"),
    ("Always", "Immer"),
    ("None", "Keine"),
];

impl CfgChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::AudioLang => "Bevorzugte Tonsprache",
            Self::SubLang => "Bevorzugte Untertitelsprache",
            Self::SubMode => "Untertitelmodus",
        }
    }
    fn options(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::AudioLang | Self::SubLang => LANGS,
            Self::SubMode => SUB_MODES,
        }
    }
    fn raw(self, c: &UserConfiguration) -> String {
        match self {
            Self::AudioLang => c.audio_language_preference.clone().unwrap_or_default(),
            Self::SubLang => c.subtitle_language_preference.clone().unwrap_or_default(),
            Self::SubMode => c
                .subtitle_mode
                .clone()
                .unwrap_or_else(|| "Default".to_owned()),
        }
    }
    /// Anzeigetext des aktuellen Werts (unbekannte Sprachen als Kürzel).
    pub fn show(self, c: &UserConfiguration) -> String {
        let raw = self.raw(c);
        self.options()
            .iter()
            .find(|(v, _)| *v == raw)
            .map_or_else(|| raw.to_uppercase(), |(_, l)| (*l).to_owned())
    }
    /// Schritt in Richtung `dir`.
    pub fn step(self, c: &mut UserConfiguration, dir: i32) {
        let opts = self.options();
        let raw = self.raw(c);
        let cur = opts.iter().position(|(v, _)| *v == raw).unwrap_or(0);
        let next = opts[cycle(cur, opts.len(), dir)].0;
        match self {
            Self::AudioLang => {
                c.audio_language_preference = (!next.is_empty()).then(|| next.to_owned());
            }
            Self::SubLang => {
                c.subtitle_language_preference = (!next.is_empty()).then(|| next.to_owned());
            }
            Self::SubMode => c.subtitle_mode = Some(next.to_owned()),
        }
    }
}

// ---------------------------------------------------------------- Richtlinie (Admin)

pub struct PolicyDef {
    pub label: &'static str,
    pub get: fn(&UserPolicy) -> bool,
    pub set: fn(&mut UserPolicy, bool),
}

/// Die wichtigsten Rechte. Index 0 und 1 (Administrator, gesperrt) schützen das eigene Konto.
pub const POLICIES: &[PolicyDef] = &[
    PolicyDef {
        label: "Administrator",
        get: |p| p.is_administrator,
        set: |p, v| p.is_administrator = v,
    },
    PolicyDef {
        label: "Konto deaktiviert",
        get: |p| p.is_disabled,
        set: |p, v| p.is_disabled = v,
    },
    PolicyDef {
        label: "In der Anmeldeliste verbergen",
        get: |p| p.is_hidden,
        set: |p, v| p.is_hidden = v,
    },
    PolicyDef {
        label: "Medien wiedergeben",
        get: |p| p.enable_media_playback,
        set: |p, v| p.enable_media_playback = v,
    },
    PolicyDef {
        label: "Videos umwandeln",
        get: |p| p.enable_video_playback_transcoding,
        set: |p, v| p.enable_video_playback_transcoding = v,
    },
    PolicyDef {
        label: "Live-TV ansehen",
        get: |p| p.enable_live_tv_access,
        set: |p, v| p.enable_live_tv_access = v,
    },
    PolicyDef {
        label: "Herunterladen erlauben",
        get: |p| p.enable_content_downloading,
        set: |p, v| p.enable_content_downloading = v,
    },
    PolicyDef {
        label: "Inhalte löschen erlauben",
        get: |p| p.enable_content_deletion,
        set: |p, v| p.enable_content_deletion = v,
    },
    PolicyDef {
        label: "Zugriff von außen",
        get: |p| p.enable_remote_access,
        set: |p, v| p.enable_remote_access = v,
    },
    PolicyDef {
        label: "Sammlungen verwalten",
        get: |p| p.enable_collection_management,
        set: |p, v| p.enable_collection_management = v,
    },
    PolicyDef {
        label: "Untertitel verwalten",
        get: |p| p.enable_subtitle_management,
        set: |p, v| p.enable_subtitle_management = v,
    },
    PolicyDef {
        label: "Alle Bibliotheken sichtbar",
        get: |p| p.enable_all_folders,
        set: |p, v| p.enable_all_folders = v,
    },
    PolicyDef {
        label: "Alle Geräte erlaubt",
        get: |p| p.enable_all_devices,
        set: |p, v| p.enable_all_devices = v,
    },
];

/// Ist der Schalter für das eigene Konto gesperrt (nicht selbst entmachten oder aussperren)?
pub fn policy_protected(idx: usize, is_self: bool) -> bool {
    is_self && idx <= 1
}

// ---------------------------------------------------------------- Formate

/// `2026-01-05T12:30:00.000Z` → `05.01.2026 12:30` (UTC, ohne Zeitzonen-Umrechnung).
pub fn fmt_date(iso: &str) -> String {
    let b = iso.as_bytes();
    if b.len() < 16 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') {
        return iso.to_owned();
    }
    format!(
        "{}.{}.{} {}",
        &iso[8..10],
        &iso[5..7],
        &iso[0..4],
        &iso[11..16]
    )
}

pub fn fmt_bytes(n: i64) -> String {
    let n = n.max(0) as f64;
    if n >= 1_048_576.0 {
        format!("{:.1} MB", n / 1_048_576.0).replace('.', ",")
    } else if n >= 1024.0 {
        format!("{:.0} KB", n / 1024.0)
    } else {
        format!("{n:.0} B")
    }
}

/// Ticks (100 ns) → `h:mm:ss` bzw. `m:ss`.
pub fn fmt_ticks(ticks: i64) -> String {
    let s = (ticks.max(0) / 10_000_000) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Erste Zeile und Zweitzeile einer Sitzung.
pub fn session_lines(s: &SessionInfo) -> (String, String, bool) {
    let who = s
        .user_name
        .clone()
        .unwrap_or_else(|| "Unbekannt".to_owned());
    let client = s.client.clone().unwrap_or_default();
    let title = if client.is_empty() {
        who
    } else {
        format!("{who} · {client}")
    };
    let device = s.device_name.clone().unwrap_or_default();
    match &s.now_playing_item {
        Some(item) => {
            let pos = s
                .play_state
                .as_ref()
                .and_then(|p| p.position_ticks)
                .map(fmt_ticks)
                .unwrap_or_default();
            let paused = s.play_state.as_ref().is_some_and(|p| p.is_paused);
            (
                title,
                format!(
                    "{device} · {}: {} {pos}",
                    if paused { "Pausiert" } else { "Spielt" },
                    item.name
                ),
                true,
            )
        }
        None => (title, format!("{device} · Inaktiv"), false),
    }
}

/// Status einer geplanten Aufgabe als Text.
pub fn task_state(t: &TaskInfo) -> String {
    match t.state.as_str() {
        "Running" => match t.current_progress_percentage {
            Some(p) => format!("Läuft ({p:.0} %)"),
            None => "Läuft".to_owned(),
        },
        "Cancelling" => "Wird abgebrochen".to_owned(),
        _ => "Bereit".to_owned(),
    }
}

/// Ergebnis der letzten Ausführung.
pub fn task_last(t: &TaskInfo) -> String {
    let Some(r) = &t.last_execution_result else {
        return "Noch nie gelaufen".to_owned();
    };
    let st = match r.status.as_deref() {
        Some("Completed") => "Fertig",
        Some("Failed") => "Fehlgeschlagen",
        Some("Cancelled") => "Abgebrochen",
        Some("Aborted") => "Unterbrochen",
        _ => "Unbekannt",
    };
    match &r.end_time_utc {
        Some(d) => format!("Zuletzt: {st} · {}", fmt_date(d)),
        None => format!("Zuletzt: {st}"),
    }
}

/// Adresse der Server-Verwaltung im Browser.
pub fn dashboard_url(base: &str) -> String {
    format!("{}/web/#/dashboard", base.trim_end_matches('/'))
}

/// Prüft die Passwortfelder.
pub fn password_check(cur: &str, new: &str, rep: &str) -> Result<(), &'static str> {
    if new.is_empty() {
        return Err("Bitte ein neues Passwort eingeben.");
    }
    if new != rep {
        return Err("Die beiden neuen Passwörter stimmen nicht überein.");
    }
    if new == cur {
        return Err("Das neue Passwort ist gleich dem alten.");
    }
    Ok(())
}

/// Bereinigter Quick-Connect-Code (nur Ziffern/Buchstaben).
pub fn clean_code(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Öffnet eine Adresse im Standardbrowser. Gibt `false` zurück, wenn der Start scheitert. Die Adresse wird nie
/// protokolliert. Im Entwicklungsmodus (`dev-automation`) startet nichts, außer `LFX_OPEN_BROWSER=1` ist gesetzt.
pub fn open_in_browser(url: &str) -> bool {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    #[cfg(feature = "dev-automation")]
    if std::env::var_os("LFX_OPEN_BROWSER").is_none() {
        return true;
    }
    #[cfg(windows)]
    let result = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    result.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cycles_wrap() {
        assert_eq!(cycle(0, 4, -1), 3);
        assert_eq!(cycle(3, 4, 1), 0);
        assert_eq!(cycle(1, 4, 1), 2);
        assert_eq!(cycle(0, 0, 1), 0);
    }

    #[test]
    fn local_defaults_and_steps() {
        let d = local_def(KEY_SUB_SIZE).unwrap();
        assert_eq!(local_label(d, None), "Normal");
        let v = local_step(d, None, 1);
        assert_eq!(v, json!("l"));
        assert_eq!(local_label(d, Some(&v)), "Groß");
        let t = local_def(KEY_UPD_AUTO).unwrap();
        assert!(local_bool(t, None));
        assert_eq!(local_step(t, None, 1), json!(false));
        // unbekannter gespeicherter Wert → Standard
        assert_eq!(local_index(d, Some(&json!("zzz"))), 1);
    }

    #[test]
    fn config_choice_steps() {
        let mut c = UserConfiguration::default();
        assert_eq!(CfgChoice::AudioLang.show(&c), "Automatisch");
        CfgChoice::AudioLang.step(&mut c, 1);
        assert_eq!(c.audio_language_preference.as_deref(), Some("ger"));
        CfgChoice::AudioLang.step(&mut c, -1);
        assert_eq!(c.audio_language_preference, None);
        assert_eq!(CfgChoice::SubMode.show(&c), "Standard");
        CfgChoice::SubMode.step(&mut c, -1);
        assert_eq!(c.subtitle_mode.as_deref(), Some("None"));
        c.subtitle_language_preference = Some("pol".into());
        assert_eq!(CfgChoice::SubLang.show(&c), "POL");
    }

    #[test]
    fn config_toggles_roundtrip() {
        let mut c = UserConfiguration::default();
        for t in [
            CfgToggle::HidePlayedInLatest,
            CfgToggle::DisplayMissingEpisodes,
            CfgToggle::PlayDefaultAudio,
            CfgToggle::RememberAudio,
            CfgToggle::RememberSubs,
            CfgToggle::NextEpisode,
        ] {
            t.set(&mut c, true);
            assert!(t.get(&c));
            t.set(&mut c, false);
            assert!(!t.get(&c));
        }
    }

    #[test]
    fn policies_work_and_protect_self() {
        let mut p = UserPolicy::default();
        for d in POLICIES {
            (d.set)(&mut p, true);
            assert!((d.get)(&p), "{}", d.label);
        }
        assert!(policy_protected(0, true));
        assert!(policy_protected(1, true));
        assert!(!policy_protected(2, true));
        assert!(!policy_protected(0, false));
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_date("2026-01-05T12:30:00.000Z"), "05.01.2026 12:30");
        assert_eq!(fmt_date("kaputt"), "kaputt");
        assert_eq!(fmt_bytes(2_621_440), "2,5 MB");
        assert_eq!(fmt_bytes(2048), "2 KB");
        assert_eq!(fmt_bytes(12), "12 B");
        assert_eq!(fmt_ticks(600 * 10_000_000), "10:00");
        assert_eq!(fmt_ticks(3723 * 10_000_000), "1:02:03");
    }

    #[test]
    fn password_rules() {
        assert!(password_check("a", "", "").is_err());
        assert!(password_check("a", "b", "c").is_err());
        assert!(password_check("a", "a", "a").is_err());
        assert!(password_check("a", "b", "b").is_ok());
    }

    #[test]
    fn urls_and_codes() {
        assert_eq!(
            dashboard_url("http://h:8096/"),
            "http://h:8096/web/#/dashboard"
        );
        assert_eq!(clean_code(" 12 34-56 "), "123456");
        assert!(!open_in_browser("file:///etc/passwd"));
    }

    #[test]
    fn sections() {
        assert!(!SetSec::list(false).contains(&SetSec::Admin));
        assert!(SetSec::list(true).contains(&SetSec::Admin));
        assert_eq!(SetSec::list(false).last(), Some(&SetSec::SignOut));
        assert!(!SetSec::About.has_content());
        assert_eq!(AdmSec::ALL.len(), 9);
        assert!(RowKind::Toggle.focusable() && !RowKind::Info.focusable());
    }

    #[test]
    fn previews() {
        assert_eq!(preview_size("xl"), 30.0);
        assert_eq!(preview_bg("box"), 2);
        assert_eq!(preview_rgb("yellow").0, 0xff);
    }
}
