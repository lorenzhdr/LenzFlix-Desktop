//! Weitere DTOs: Suche, Filter, Zähler, Wiedergabelisten, Live-TV, Verwaltung, Branding.
//! Feldnamen nach der OpenAPI-Spec (Jellyfin 12.4); unbekannte Felder werden ignoriert.

use crate::dto::{BaseItem, MediaSource, NameId};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- Suche, Filter, Zähler

/// Treffer der Suchvorschläge (`/Search/Hints`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SearchHint {
    /// Element-Id.
    pub item_id: String,
    /// Name.
    pub name: String,
    /// Getroffener Suchbegriff.
    pub matched_term: Option<String>,
    /// Art (`Movie`, `Series`, `Person`, `Genre`, `MusicArtist`, ...).
    #[serde(rename = "Type")]
    pub kind: String,
    /// Produktionsjahr.
    pub production_year: Option<i32>,
    /// Medientyp.
    pub media_type: Option<String>,
    /// Tag des Hauptbilds.
    pub primary_image_tag: Option<String>,
    /// Serienname (Folgen).
    pub series: Option<String>,
    /// Album (Titel).
    pub album: Option<String>,
    /// Albumkünstler (Titel/Alben).
    pub album_artist: Option<String>,
    /// Interpreten.
    pub artists: Vec<String>,
    /// Ordner?
    pub is_folder: Option<bool>,
    /// Laufzeit in Ticks.
    pub run_time_ticks: Option<i64>,
    /// Sendername (Live-TV).
    pub channel_name: Option<String>,
    /// Seitenverhältnis des Posters.
    pub primary_image_aspect_ratio: Option<f64>,
}

/// Antwort von `/Search/Hints`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SearchHintResult {
    /// Treffer.
    pub search_hints: Vec<SearchHint>,
    /// Gesamtzahl.
    pub total_record_count: i64,
}

/// Mögliche Filterwerte (`/Items/Filters2`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QueryFilters {
    /// Genres.
    pub genres: Vec<NameId>,
    /// Schlagwörter.
    pub tags: Vec<String>,
    /// Tonsprachen.
    pub audio_languages: Vec<String>,
    /// Untertitelsprachen.
    pub subtitle_languages: Vec<String>,
}

/// Anzahl Elemente je Art (`/Items/Counts`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ItemCounts {
    /// Filme.
    pub movie_count: i64,
    /// Serien.
    pub series_count: i64,
    /// Folgen.
    pub episode_count: i64,
    /// Künstler.
    pub artist_count: i64,
    /// Sendungen.
    pub program_count: i64,
    /// Trailer.
    pub trailer_count: i64,
    /// Titel.
    pub song_count: i64,
    /// Alben.
    pub album_count: i64,
    /// Musikvideos.
    pub music_video_count: i64,
    /// Sammlungen.
    pub box_set_count: i64,
    /// Bücher.
    pub book_count: i64,
    /// Alle Elemente.
    pub item_count: i64,
}

// ---------------------------------------------------------------- Wiedergabelisten

/// Ergebnis von `POST /Playlists`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaylistCreationResult {
    /// Id der neuen Liste.
    pub id: String,
}

/// Wiedergabeliste (Einstellungen und Einträge).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaylistDto {
    /// Für alle lesbar?
    pub open_access: bool,
    /// Ids der Elemente in Reihenfolge.
    pub item_ids: Vec<String>,
}

// ---------------------------------------------------------------- Live-TV

/// Aufnahme-Timer (Auszug; `Id` und Zeiten genügen zum Anzeigen und Löschen).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TimerInfo {
    /// Timer-Id.
    pub id: String,
    /// Sender-Id.
    pub channel_id: Option<String>,
    /// Sendername.
    pub channel_name: Option<String>,
    /// Sendungs-Id.
    pub program_id: Option<String>,
    /// Titel.
    pub name: Option<String>,
    /// Beschreibung.
    pub overview: Option<String>,
    /// Beginn (ISO-8601).
    pub start_date: Option<String>,
    /// Ende (ISO-8601).
    pub end_date: Option<String>,
    /// Zustand (`New`, `InProgress`, `Completed`, `Cancelled`, ...).
    pub status: Option<String>,
    /// Zugehöriger Serien-Timer.
    pub series_timer_id: Option<String>,
    /// Vorlauf in Sekunden.
    pub pre_padding_seconds: i64,
    /// Nachlauf in Sekunden.
    pub post_padding_seconds: i64,
}

/// Antwort auf `/LiveStreams/Open`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LiveStreamResponse {
    /// Geöffnete Quelle.
    pub media_source: Option<MediaSource>,
}

// ---------------------------------------------------------------- Verwaltung

/// Systeminformation (`/System/Info`, nur Administratoren).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SystemInfo {
    /// Servername.
    pub server_name: String,
    /// Version.
    pub version: String,
    /// Produktname.
    pub product_name: String,
    /// Server-Id.
    pub id: String,
    /// Betriebssystem.
    pub operating_system_display_name: Option<String>,
    /// Architektur.
    pub system_architecture: Option<String>,
    /// Neustart nötig?
    pub has_pending_restart: bool,
    /// Wird heruntergefahren?
    pub is_shutting_down: bool,
    /// Kann sich selbst neu starten?
    pub can_self_restart: bool,
    /// Update verfügbar?
    pub has_update_available: bool,
}

/// Wiedergabezustand einer Sitzung.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlayerState {
    /// Position in Ticks.
    pub position_ticks: Option<i64>,
    /// Pausiert?
    pub is_paused: bool,
    /// Stumm?
    pub is_muted: bool,
    /// Lautstärke 0–100.
    pub volume_level: Option<i32>,
    /// Wiedergabemethode.
    pub play_method: Option<String>,
}

/// Aktive Sitzung eines Geräts (`/Sessions`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SessionInfo {
    /// Sitzungs-Id.
    pub id: String,
    /// Benutzer-Id.
    pub user_id: String,
    /// Benutzername.
    pub user_name: Option<String>,
    /// Client-Name.
    pub client: Option<String>,
    /// Gerätename.
    pub device_name: Option<String>,
    /// Geräte-Id.
    pub device_id: Option<String>,
    /// Version des Clients.
    pub application_version: Option<String>,
    /// Letzte Aktivität (ISO-8601).
    pub last_activity_date: Option<String>,
    /// Läuft gerade etwas?
    pub now_playing_item: Option<BaseItem>,
    /// Zustand der Wiedergabe.
    pub play_state: Option<PlayerState>,
    /// Aktiv?
    pub is_active: bool,
    /// Fernsteuerbar?
    pub supports_remote_control: bool,
}

/// Ergebnis der letzten Ausführung einer Aufgabe.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TaskResult {
    /// Beginn (ISO-8601).
    pub start_time_utc: Option<String>,
    /// Ende (ISO-8601).
    pub end_time_utc: Option<String>,
    /// `Completed`, `Failed`, `Cancelled`, `Aborted`.
    pub status: Option<String>,
    /// Fehlermeldung.
    pub error_message: Option<String>,
}

/// Geplante Aufgabe.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TaskInfo {
    /// Aufgaben-Id.
    pub id: String,
    /// Name.
    pub name: String,
    /// `Idle`, `Running`, `Cancelling`.
    pub state: String,
    /// Fortschritt in Prozent (nur während der Ausführung).
    pub current_progress_percentage: Option<f64>,
    /// Beschreibung.
    pub description: Option<String>,
    /// Kategorie.
    pub category: Option<String>,
    /// Versteckt?
    pub is_hidden: bool,
    /// Letzte Ausführung.
    pub last_execution_result: Option<TaskResult>,
}

/// Bibliothek (virtueller Ordner) in der Verwaltung.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct VirtualFolder {
    /// Name.
    pub name: String,
    /// Pfade auf dem Server (nur Verwaltung, nie loggen).
    pub locations: Vec<String>,
    /// Sammlungstyp.
    pub collection_type: Option<String>,
    /// Element-Id.
    pub item_id: Option<String>,
    /// Fortschritt der Aktualisierung in Prozent.
    pub refresh_progress: Option<f64>,
    /// Zustand der Aktualisierung.
    pub refresh_status: Option<String>,
}

/// Eintrag des Aktivitätsprotokolls.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ActivityLogEntry {
    /// Eintrags-Id.
    pub id: i64,
    /// Titel.
    pub name: String,
    /// Beschreibung.
    pub overview: Option<String>,
    /// Kurzbeschreibung.
    pub short_overview: Option<String>,
    /// Art.
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// Zeitpunkt (ISO-8601).
    pub date: Option<String>,
    /// Benutzer-Id.
    pub user_id: Option<String>,
    /// Schweregrad (`Information`, `Warning`, `Error`, ...).
    pub severity: Option<String>,
}

/// Logdatei des Servers.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LogFile {
    /// Dateiname.
    pub name: String,
    /// Größe in Bytes.
    pub size: i64,
    /// Zuletzt geändert (ISO-8601).
    pub date_modified: Option<String>,
}

/// Registriertes Gerät.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DeviceInfo {
    /// Geräte-Id.
    pub id: String,
    /// Name.
    pub name: Option<String>,
    /// Eigener Name.
    pub custom_name: Option<String>,
    /// Programm.
    pub app_name: Option<String>,
    /// Programmversion.
    pub app_version: Option<String>,
    /// Zuletzt angemeldeter Benutzer.
    pub last_user_name: Option<String>,
    /// Letzte Aktivität (ISO-8601).
    pub date_last_activity: Option<String>,
}

/// Branding-Einstellungen (`/Branding/Configuration`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct BrandingOptions {
    /// Hinweistext auf der Anmeldeseite.
    pub login_disclaimer: Option<String>,
    /// Eigenes CSS des Servers.
    pub custom_css: Option<String>,
}

// ---------------------------------------------------------------- Eingaben

/// Eingabe für `POST /Users/Password`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PasswordChange {
    /// Aktuelles Passwort.
    pub current_pw: String,
    /// Neues Passwort (leer = Passwort entfernen).
    pub new_pw: String,
}

/// Bewegung in der Wiedergabe einer fremden Sitzung (`/Sessions/{id}/Playing/{command}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaystateCommand {
    /// Anhalten.
    Stop,
    /// Pausieren.
    Pause,
    /// Fortsetzen.
    Unpause,
    /// Umschalten.
    PlayPause,
    /// Nächstes Element.
    NextTrack,
    /// Vorheriges Element.
    PreviousTrack,
    /// Springen (`seek_position_ticks` angeben).
    Seek,
}

impl PlaystateCommand {
    /// Pfadsegment der API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "Stop",
            Self::Pause => "Pause",
            Self::Unpause => "Unpause",
            Self::PlayPause => "PlayPause",
            Self::NextTrack => "NextTrack",
            Self::PreviousTrack => "PreviousTrack",
            Self::Seek => "Seek",
        }
    }
}
