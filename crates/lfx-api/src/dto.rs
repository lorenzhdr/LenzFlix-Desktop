//! DTOs: nur die vom Client benötigten Felder (Feldnamen nach der offiziellen OpenAPI-Spec).
//! Unbekannte Felder werden ignoriert, fehlende Felder bekommen Standardwerte.

use crate::secret::Secret;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 1 Tick = 100 ns; 10 Mio. Ticks = 1 s.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// Öffentliche Serverinfo (`/System/Info/Public`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PublicSystemInfo {
    /// Eindeutige Server-Id.
    pub id: String,
    /// Anzeigename des Servers.
    pub server_name: String,
    /// Serverversion.
    pub version: String,
    /// Produktname.
    pub product_name: String,
    /// Vom Server gemeldete lokale Adresse.
    pub local_address: String,
    /// Einrichtungsassistent abgeschlossen?
    pub startup_wizard_completed: bool,
}

/// Benutzerrichtlinie (Auszug).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserPolicy {
    /// Administrator?
    pub is_administrator: bool,
    /// Medienwiedergabe erlaubt?
    pub enable_media_playback: bool,
    /// Videoumwandlung erlaubt?
    pub enable_video_playback_transcoding: bool,
}

/// Benutzerkonfiguration (Auszug).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserConfiguration {
    /// Bevorzugte Tonsprache.
    pub audio_language_preference: Option<String>,
    /// Bevorzugte Untertitelsprache.
    pub subtitle_language_preference: Option<String>,
    /// Standard-Tonspur automatisch wählen?
    pub play_default_audio_track: bool,
    /// Untertitelmodus (Default, Always, OnlyForced, None, Smart).
    pub subtitle_mode: Option<String>,
}

/// Benutzer.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserDto {
    /// Benutzer-Id.
    pub id: String,
    /// Benutzername.
    pub name: String,
    /// Server-Id.
    pub server_id: Option<String>,
    /// Tag des Profilbilds.
    pub primary_image_tag: Option<String>,
    /// Hat ein Passwort?
    pub has_password: bool,
    /// Richtlinie.
    pub policy: Option<UserPolicy>,
    /// Konfiguration.
    pub configuration: Option<UserConfiguration>,
}

/// Antwort einer Anmeldung.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AuthenticationResult {
    /// Angemeldeter Benutzer.
    pub user: UserDto,
    /// Zugriffstoken (geschwärzt in Debug).
    pub access_token: Secret,
    /// Server-Id.
    pub server_id: String,
}

/// Quick-Connect-Zustand.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QuickConnectResult {
    /// Vom Nutzer in einer angemeldeten Sitzung bestätigt?
    pub authenticated: bool,
    /// Geheimer Abfrageschlüssel (nur für diesen Client).
    pub secret: Secret,
    /// Sechsstelliger Code, den der Nutzer eingibt.
    pub code: String,
    /// Geräte-Id des anfragenden Clients.
    pub device_id: Option<String>,
    /// Gerätename des anfragenden Clients.
    pub device_name: Option<String>,
}

/// Seitenweise Ergebnisliste.
#[derive(Debug, Clone, Deserialize)]
#[serde(
    rename_all = "PascalCase",
    default,
    bound(deserialize = "T: Deserialize<'de>")
)]
pub struct QueryResult<T> {
    /// Elemente dieser Seite.
    pub items: Vec<T>,
    /// Gesamtzahl (nur bei `EnableTotalRecordCount`).
    pub total_record_count: i64,
    /// Startindex dieser Seite.
    pub start_index: i64,
}

impl<T> Default for QueryResult<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total_record_count: 0,
            start_index: 0,
        }
    }
}

/// Art eines Elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ItemKind {
    /// Film.
    Movie,
    /// Serie.
    Series,
    /// Staffel.
    Season,
    /// Folge.
    Episode,
    /// Sammlung.
    BoxSet,
    /// Ordner.
    Folder,
    /// Bibliothek (Sammlungsordner).
    CollectionFolder,
    /// Benutzeransicht.
    UserView,
    /// Album.
    MusicAlbum,
    /// Künstler.
    MusicArtist,
    /// Titel.
    Audio,
    /// Buch.
    Book,
    /// Foto.
    Photo,
    /// Wiedergabeliste.
    Playlist,
    /// Live-TV-Kanal.
    TvChannel,
    /// Sendung.
    Program,
    /// Sonstiges Video.
    Video,
    /// Unbekannt oder nicht abgebildet.
    #[default]
    #[serde(other)]
    Unknown,
}

impl ItemKind {
    /// Name, wie ihn die API in Query-Parametern erwartet.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "Movie",
            Self::Series => "Series",
            Self::Season => "Season",
            Self::Episode => "Episode",
            Self::BoxSet => "BoxSet",
            Self::Folder => "Folder",
            Self::CollectionFolder => "CollectionFolder",
            Self::UserView => "UserView",
            Self::MusicAlbum => "MusicAlbum",
            Self::MusicArtist => "MusicArtist",
            Self::Audio => "Audio",
            Self::Book => "Book",
            Self::Photo => "Photo",
            Self::Playlist => "Playlist",
            Self::TvChannel => "TvChannel",
            Self::Program => "Program",
            Self::Video => "Video",
            Self::Unknown => "Unknown",
        }
    }
}

/// Benutzerdaten zu einem Element.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserItemData {
    /// Gemerkte Position in Ticks.
    pub playback_position_ticks: i64,
    /// Anzahl Wiedergaben.
    pub play_count: i64,
    /// Favorit?
    pub is_favorite: bool,
    /// Gesehen?
    pub played: bool,
    /// Gesehen-Anteil in Prozent (Serien/Staffeln/Teilfortschritt).
    pub played_percentage: Option<f64>,
    /// Ungesehene Folgen (Serien/Staffeln).
    pub unplayed_item_count: Option<i64>,
    /// Gefällt/gefällt nicht.
    pub likes: Option<bool>,
    /// Bewertung.
    pub rating: Option<f64>,
    /// Zuletzt gespielt (ISO-8601).
    pub last_played_date: Option<String>,
}

/// Name/Id-Paar (Genres, Studios).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NameId {
    /// Name.
    pub name: String,
    /// Id.
    pub id: String,
}

/// Person (Besetzung/Stab).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Person {
    /// Name.
    pub name: String,
    /// Id.
    pub id: String,
    /// Rolle (bei Schauspielern).
    pub role: Option<String>,
    /// Art (Actor, Director, ...).
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    /// Tag des Bildes.
    pub primary_image_tag: Option<String>,
}

/// Kapitel.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Chapter {
    /// Name.
    pub name: Option<String>,
    /// Beginn in Ticks.
    pub start_position_ticks: i64,
    /// Tag des Kapitelbilds.
    pub image_tag: Option<String>,
}

/// Trickplay-Vorschaubilder (Kachelraster) einer Auflösung.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TrickplayInfo {
    /// Breite einer Vorschau in Pixeln.
    pub width: u32,
    /// Höhe einer Vorschau in Pixeln.
    pub height: u32,
    /// Vorschauen je Kachelbild, horizontal.
    pub tile_width: u32,
    /// Vorschauen je Kachelbild, vertikal.
    pub tile_height: u32,
    /// Gesamtzahl der Vorschauen.
    pub thumbnail_count: u32,
    /// Abstand zwischen zwei Vorschauen in Millisekunden.
    pub interval: u32,
    /// Bitrate (Informationsfeld).
    pub bandwidth: u32,
}

/// Stream (Video/Ton/Untertitel) einer Medienquelle.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaStream {
    /// Index in der Quelle.
    pub index: i32,
    /// Art: `Video`, `Audio`, `Subtitle`, ...
    #[serde(rename = "Type")]
    pub kind: String,
    /// Codec.
    pub codec: Option<String>,
    /// Sprache (ISO 639-2).
    pub language: Option<String>,
    /// Anzeigetitel (bereits lokalisiert).
    pub display_title: Option<String>,
    /// Titel.
    pub title: Option<String>,
    /// Standardspur?
    pub is_default: bool,
    /// Erzwungen?
    pub is_forced: bool,
    /// Externe Datei?
    pub is_external: bool,
    /// Textuntertitel?
    pub is_text_subtitle_stream: bool,
    /// Breite (Video).
    pub width: Option<i32>,
    /// Höhe (Video).
    pub height: Option<i32>,
    /// Bitrate.
    pub bit_rate: Option<i64>,
    /// Bittiefe (Video).
    pub bit_depth: Option<i32>,
    /// Kanäle (Ton).
    pub channels: Option<i32>,
    /// Videobereich (SDR/HDR).
    pub video_range: Option<String>,
    /// Videobereich genauer (HDR10, DOVI, ...).
    pub video_range_type: Option<String>,
    /// Profil.
    pub profile: Option<String>,
    /// Bildrate.
    pub average_frame_rate: Option<f64>,
    /// Serverrelative Adresse externer Untertitel.
    pub delivery_url: Option<String>,
}

/// Medienquelle (Datei) eines Elements.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaSource {
    /// Quell-Id.
    pub id: String,
    /// Name.
    pub name: Option<String>,
    /// Container (mkv, mp4, ...).
    pub container: Option<String>,
    /// Größe in Bytes.
    pub size: Option<i64>,
    /// Laufzeit in Ticks.
    pub run_time_ticks: Option<i64>,
    /// Bitrate.
    pub bitrate: Option<i64>,
    /// Direkte Wiedergabe möglich?
    pub supports_direct_play: bool,
    /// Direktes Streamen möglich?
    pub supports_direct_stream: bool,
    /// Umwandlung möglich?
    pub supports_transcoding: bool,
    /// Umwandlungs-URL (serverrelativ, enthält u. U. Token: nie loggen).
    pub transcoding_url: Option<String>,
    /// Streams.
    pub media_streams: Vec<MediaStream>,
    /// Standard-Tonspur.
    pub default_audio_stream_index: Option<i32>,
    /// Standard-Untertitelspur.
    pub default_subtitle_stream_index: Option<i32>,
    /// Hat Medien-Segmente (Intro/Abspann)?
    pub has_segments: bool,
}

/// Element der Bibliothek (Film, Serie, Folge, ...).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct BaseItem {
    /// Id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Originaltitel.
    pub original_title: Option<String>,
    /// Art.
    #[serde(rename = "Type")]
    pub kind: ItemKind,
    /// Sammlungstyp (movies, tvshows, ...), nur bei Ansichten.
    pub collection_type: Option<String>,
    /// Medientyp (Video, Audio, Photo, ...).
    pub media_type: Option<String>,
    /// Handlung.
    pub overview: Option<String>,
    /// Produktionsjahr.
    pub production_year: Option<i32>,
    /// Premiere (ISO-8601).
    pub premiere_date: Option<String>,
    /// Hinzugefügt am (ISO-8601).
    pub date_created: Option<String>,
    /// Altersfreigabe.
    pub official_rating: Option<String>,
    /// Community-Bewertung.
    pub community_rating: Option<f64>,
    /// Kritikerbewertung.
    pub critic_rating: Option<f64>,
    /// Laufzeit in Ticks.
    pub run_time_ticks: Option<i64>,
    /// Genres.
    pub genres: Vec<String>,
    /// Studios.
    pub studios: Vec<NameId>,
    /// Besetzung und Stab.
    pub people: Vec<Person>,
    /// Ordner/Container?
    pub is_folder: bool,
    /// Übergeordnetes Element.
    pub parent_id: Option<String>,
    /// Serien-Id (Staffeln/Folgen).
    pub series_id: Option<String>,
    /// Serienname.
    pub series_name: Option<String>,
    /// Staffel-Id.
    pub season_id: Option<String>,
    /// Staffelname.
    pub season_name: Option<String>,
    /// Folgennummer.
    pub index_number: Option<i32>,
    /// Staffelnummer.
    pub parent_index_number: Option<i32>,
    /// Anzahl Kindelemente (Staffeln/Folgen).
    pub child_count: Option<i64>,
    /// Benutzerdaten.
    pub user_data: Option<UserItemData>,
    /// Bild-Tags je Typ (Primary, Logo, Thumb, ...).
    pub image_tags: HashMap<String, String>,
    /// Tags der Hintergrundbilder.
    pub backdrop_image_tags: Vec<String>,
    /// Tag des Serien-Posters (Folgen).
    pub series_primary_image_tag: Option<String>,
    /// Eltern-Logo.
    pub parent_logo_item_id: Option<String>,
    /// Eltern-Logo-Tag.
    pub parent_logo_image_tag: Option<String>,
    /// Eltern-Hintergrund.
    pub parent_backdrop_item_id: Option<String>,
    /// Eltern-Hintergrund-Tags.
    pub parent_backdrop_image_tags: Vec<String>,
    /// Eltern-Thumb.
    pub parent_thumb_item_id: Option<String>,
    /// Eltern-Thumb-Tag.
    pub parent_thumb_image_tag: Option<String>,
    /// Seitenverhältnis des Posters.
    pub primary_image_aspect_ratio: Option<f64>,
    /// Medienquellen (nur mit `Fields=MediaSources`).
    pub media_sources: Vec<MediaSource>,
    /// Streams der Standardquelle (nur mit `Fields=MediaStreams`).
    pub media_streams: Vec<MediaStream>,
    /// Kapitel (nur mit `Fields=Chapters`).
    pub chapters: Vec<Chapter>,
    /// Trickplay: Quell-Id → Breite → Info (nur mit `Fields=Trickplay`).
    pub trickplay: Option<HashMap<String, HashMap<String, TrickplayInfo>>>,
    /// Provider-Ids (Imdb, Tmdb, ...).
    pub provider_ids: HashMap<String, String>,
}

impl BaseItem {
    /// Laufzeit in Sekunden.
    pub fn runtime_seconds(&self) -> Option<f64> {
        self.run_time_ticks
            .map(|t| t as f64 / TICKS_PER_SECOND as f64)
    }
}

/// Medien-Segment (Intro, Abspann, ...).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaSegment {
    /// Segment-Id.
    pub id: String,
    /// Element-Id.
    pub item_id: String,
    /// Art: `Intro`, `Outro`, `Recap`, `Preview`, `Commercial`, `Unknown`.
    #[serde(rename = "Type")]
    pub kind: String,
    /// Beginn in Ticks.
    pub start_ticks: i64,
    /// Ende in Ticks.
    pub end_ticks: i64,
}

/// Antwort auf PlaybackInfo.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaybackInfoResponse {
    /// Verfügbare Quellen.
    pub media_sources: Vec<MediaSource>,
    /// Sitzungs-Id für die Fortschrittsmeldungen.
    pub play_session_id: Option<String>,
    /// Fehlercode des Servers (z. B. `NoCompatibleStream`).
    pub error_code: Option<String>,
}
