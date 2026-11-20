//! Datentypen des Offline-Modus: Qualität, Zustände, Datensatz eines Downloads, Ereignisse.

use serde::{Deserialize, Serialize};

/// Umwandlungsziel für einen transkodierten Download (MP4-Datei).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodeProfile {
    /// Höchste Gesamtbitrate in Bit/s.
    pub max_bitrate: u64,
    /// Höchste Breite in Pixeln.
    pub max_width: Option<u32>,
    /// Höchste Höhe in Pixeln.
    pub max_height: Option<u32>,
}

impl TranscodeProfile {
    /// Voreinstellung „Mittel“ (720p, 3 MBit/s).
    pub fn medium() -> Self {
        Self {
            max_bitrate: 3_000_000,
            max_width: Some(1280),
            max_height: Some(720),
        }
    }

    /// Voreinstellung „Klein“ (480p, 1,2 MBit/s) für Handy und wenig Platz.
    pub fn small() -> Self {
        Self {
            max_bitrate: 1_200_000,
            max_width: Some(854),
            max_height: Some(480),
        }
    }
}

/// Qualität eines Downloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    /// Originaldatei (`/Items/{id}/Download`, braucht das Recht `EnableContentDownloading`).
    Original,
    /// Vom Server umgewandelte MP4-Datei. Nicht fortsetzbar (der Server kennt weder Länge noch `Range`).
    Transcoded(TranscodeProfile),
}

/// Zustand eines Downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    /// Wartet auf einen freien Platz.
    Queued,
    /// Läuft.
    Downloading,
    /// Vom Nutzer angehalten.
    Paused,
    /// Fertig und abspielbar.
    Completed,
    /// Fehlgeschlagen (Fehlertext im Datensatz); mit `resume` erneut versuchen.
    Failed,
}

impl DownloadState {
    /// Kurztext für die Datenbankspalte `state`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Eine zum Download gehörende Datei (Pfad relativ zum Elementordner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredFile {
    /// Art: `primary`, `backdrop`, `logo`, `subtitle`, `item`, `series`, `season`.
    pub kind: String,
    /// Relativer Pfad.
    pub rel: String,
    /// Sprache (Untertitel).
    pub language: Option<String>,
    /// Anzeigetitel (Untertitel).
    pub title: Option<String>,
    /// Codec/Format (Untertitel).
    pub codec: Option<String>,
    /// Jellyfin-Stream-Index (Untertitel).
    pub index: Option<i32>,
    /// Standardspur?
    pub is_default: bool,
}

/// Lokal geänderte Benutzerdaten eines Offline-Titels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalUserData {
    /// Position in Ticks.
    pub position_ticks: i64,
    /// Gesehen?
    pub played: bool,
    /// Wiedergaben (lokal gezählt, nur zur Anzeige).
    pub play_count: i64,
    /// Zeitpunkt der letzten lokalen Wiedergabe (Unix-Millisekunden).
    pub last_played_ms: i64,
}

/// Datensatz eines Downloads (liegt als JSON in der Tabelle `downloads`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadRecord {
    /// Kennung (über alle Konten eindeutig).
    pub id: String,
    /// Server.
    pub server_id: String,
    /// Benutzer.
    pub user_id: String,
    /// Element auf dem Server.
    pub item_id: String,
    /// Elementart (`Movie`, `Episode`, …).
    pub kind: String,
    /// Titel zur Anzeige.
    pub name: String,
    /// Serien-Id (Folgen).
    pub series_id: Option<String>,
    /// Serienname (Folgen).
    pub series_name: Option<String>,
    /// Staffel-Id (Folgen).
    pub season_id: Option<String>,
    /// Staffelnummer.
    pub season_number: Option<i32>,
    /// Folgennummer.
    pub episode_number: Option<i32>,
    /// Produktionsjahr.
    pub year: Option<i32>,
    /// Laufzeit in Ticks.
    pub runtime_ticks: Option<i64>,
    /// Qualität.
    pub quality: Quality,
    /// Zustand.
    pub state: DownloadState,
    /// Medienquelle auf dem Server.
    pub media_source_id: String,
    /// Dateiendung der Mediendatei (ohne Punkt).
    pub extension: String,
    /// Größe laut Server (Content-Length/Content-Range); bei Umwandlung unbekannt.
    pub bytes_total: Option<u64>,
    /// Schätzung für die Platzprüfung.
    pub size_estimate: u64,
    /// Zuletzt gemeldeter Fortschritt (maßgeblich ist die Länge der `.part`-Datei).
    pub bytes_done: u64,
    /// SHA-256 der fertigen Mediendatei (hex), lokal berechnet.
    pub sha256: Option<String>,
    /// Erwartete SHA-256, falls bekannt (wird beim Abschluss geprüft).
    pub expected_sha256: Option<String>,
    /// Gespeicherte Dateien (relativ zum Elementordner), ohne Mediendatei.
    pub files: Vec<StoredFile>,
    /// Zubehör, das nicht geladen werden konnte (nur Kurztexte).
    pub accessory_errors: Vec<String>,
    /// Letzter Fehlertext (nur bei `Failed`).
    pub error: Option<String>,
    /// Wiedergabesitzung einer Umwandlung (für das Beenden des Serverjobs).
    pub play_session_id: Option<String>,
    /// Lokale Benutzerdaten (Fortschritt offline).
    pub local: Option<LocalUserData>,
    /// Sammelkennung (z. B. Serien-Id bei Sammel-Downloads).
    pub group: Option<String>,
    /// Angelegt (Unix-Millisekunden).
    pub created_ms: i64,
}

/// Ereignis des Managers.
#[derive(Debug, Clone, PartialEq)]
pub enum OfflineEvent {
    /// Neuer Download angelegt.
    Added {
        /// Download.
        id: String,
    },
    /// Zustand geändert.
    State {
        /// Download.
        id: String,
        /// Neuer Zustand.
        state: DownloadState,
    },
    /// Fortschritt der Mediendatei.
    Progress {
        /// Download.
        id: String,
        /// Bisher geladene Bytes.
        done: u64,
        /// Gesamtgröße, falls bekannt.
        total: Option<u64>,
    },
    /// Download samt Dateien entfernt.
    Removed {
        /// Download.
        id: String,
    },
}
