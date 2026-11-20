//! Wiedergabe: DeviceProfile, PlaybackInfo, Fortschrittsmeldungen, Stream-/Bild-URLs.
//!
//! mpv spielt (fast) alles direkt. Das Profil `direct_play_everything` meldet deshalb alle Container und
//! Codecs als direkt abspielbar und alle Untertitelformate als `External`, damit der Server nie Untertitel
//! einbrennt (das wäre eine Video-Umwandlung). Ton- und Untertitelspuren wählt mpv selbst.

use crate::client::Client;
use crate::dto::PlaybackInfoResponse;
use crate::endpoints::enc;
use crate::error::Result;
use reqwest::Method;
use serde::Serialize;

/// Direktwiedergabe-Profil.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DirectPlayProfile {
    /// Container (leer = alle).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// `Video`, `Audio` oder `Photo`.
    #[serde(rename = "Type")]
    pub kind: String,
}

/// Untertitelprofil.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SubtitleProfile {
    /// Format (srt, ass, pgssub, ...).
    pub format: String,
    /// Auslieferung: `External` = der Client lädt die Datei, der Server brennt nichts ein.
    pub method: String,
}

/// Geräteprofil für PlaybackInfo (nur die genutzten Felder).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeviceProfile {
    /// Profilname.
    pub name: String,
    /// Höchste Streaming-Bitrate in Bit/s.
    pub max_streaming_bitrate: i64,
    /// Höchste statische Bitrate in Bit/s.
    pub max_static_bitrate: i64,
    /// Direkt abspielbar.
    pub direct_play_profiles: Vec<DirectPlayProfile>,
    /// Umwandlungsprofile (leer: der Server darf nichts umwandeln).
    pub transcoding_profiles: Vec<serde_json::Value>,
    /// Codec-Einschränkungen (leer: keine).
    pub codec_profiles: Vec<serde_json::Value>,
    /// Untertitelprofile.
    pub subtitle_profiles: Vec<SubtitleProfile>,
}

/// Untertitelformate, die als `External` gemeldet werden.
const SUBTITLE_FORMATS: &[&str] = &[
    "srt",
    "subrip",
    "ass",
    "ssa",
    "vtt",
    "webvtt",
    "sub",
    "subviewer",
    "smi",
    "ttml",
    "pgs",
    "pgssub",
    "dvdsub",
    "dvbsub",
    "idx",
    "microdvd",
    "mpl2",
    "pjs",
    "sami",
    "text",
    "realtext",
    "stl",
    "jacosub",
    "lrc",
];

impl DeviceProfile {
    /// Profil für mpv: alles direkt abspielbar, keine Umwandlung, Untertitel immer extern.
    pub fn direct_play_everything() -> Self {
        Self {
            name: "LENZFLIX mpv".to_owned(),
            max_streaming_bitrate: 1_000_000_000,
            max_static_bitrate: 1_000_000_000,
            direct_play_profiles: ["Video", "Audio"]
                .iter()
                .map(|kind| DirectPlayProfile {
                    container: None,
                    kind: (*kind).to_owned(),
                })
                .collect(),
            transcoding_profiles: Vec::new(),
            codec_profiles: Vec::new(),
            subtitle_profiles: SUBTITLE_FORMATS
                .iter()
                .map(|f| SubtitleProfile {
                    format: (*f).to_owned(),
                    method: "External".to_owned(),
                })
                .collect(),
        }
    }
}

/// Anfrage-Body für PlaybackInfo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfoRequest {
    /// Benutzer-Id.
    pub user_id: String,
    /// Geräteprofil.
    pub device_profile: DeviceProfile,
    /// Startposition in Ticks (bei Direktwiedergabe nur Information).
    pub start_time_ticks: i64,
    /// `-1` = keine Untertitel vom Server einbrennen (nie ändern, solange umgewandelt werden könnte).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_stream_index: Option<i32>,
    /// Gewünschte Tonspur (nur zusammen mit `media_source_id` wirksam).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_stream_index: Option<i32>,
    /// Gewünschte Quelle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_source_id: Option<String>,
    /// Direktwiedergabe erlaubt?
    pub enable_direct_play: bool,
    /// Direktes Streamen erlaubt?
    pub enable_direct_stream: bool,
    /// Umwandlung erlaubt?
    pub enable_transcoding: bool,
    /// Videostrom kopieren erlaubt?
    pub allow_video_stream_copy: bool,
    /// Tonstrom kopieren erlaubt?
    pub allow_audio_stream_copy: bool,
    /// Live-Streams automatisch öffnen?
    pub auto_open_live_stream: bool,
}

impl PlaybackInfoRequest {
    /// Anfrage für mpv: Direktwiedergabe, keine Umwandlung, keine eingebrannten Untertitel.
    pub fn direct_play(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            device_profile: DeviceProfile::direct_play_everything(),
            start_time_ticks: 0,
            subtitle_stream_index: Some(-1),
            audio_stream_index: None,
            media_source_id: None,
            enable_direct_play: true,
            enable_direct_stream: true,
            enable_transcoding: false,
            allow_video_stream_copy: true,
            allow_audio_stream_copy: true,
            auto_open_live_stream: false,
        }
    }
}

/// Wiedergabeart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PlayMethod {
    /// Datei direkt.
    DirectPlay,
    /// Container umgepackt.
    DirectStream,
    /// Umgewandelt.
    Transcode,
}

/// Meldung „Wiedergabe gestartet“ bzw. „Fortschritt“ (gleiche Felder).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackReport {
    /// Element.
    pub item_id: String,
    /// Medienquelle.
    pub media_source_id: String,
    /// Sitzungs-Id aus PlaybackInfo.
    pub play_session_id: String,
    /// Position in Ticks.
    pub position_ticks: i64,
    /// Pausiert?
    pub is_paused: bool,
    /// Stumm?
    pub is_muted: bool,
    /// Lautstärke 0–100.
    pub volume_level: u8,
    /// Spulen möglich?
    pub can_seek: bool,
    /// Wiedergabeart.
    pub play_method: PlayMethod,
    /// Gewählte Tonspur.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_stream_index: Option<i32>,
    /// Gewählte Untertitelspur (`-1`/`None` = keine).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_stream_index: Option<i32>,
}

impl PlaybackReport {
    /// Standardwerte für Direktwiedergabe.
    pub fn direct(
        item_id: impl Into<String>,
        media_source_id: impl Into<String>,
        play_session_id: impl Into<String>,
        position_ticks: i64,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            media_source_id: media_source_id.into(),
            play_session_id: play_session_id.into(),
            position_ticks,
            is_paused: false,
            is_muted: false,
            volume_level: 100,
            can_seek: true,
            play_method: PlayMethod::DirectPlay,
            audio_stream_index: None,
            subtitle_stream_index: None,
        }
    }
}

/// Meldung „Wiedergabe beendet“.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackStop {
    /// Element.
    pub item_id: String,
    /// Medienquelle.
    pub media_source_id: String,
    /// Sitzungs-Id aus PlaybackInfo.
    pub play_session_id: String,
    /// Endposition in Ticks.
    pub position_ticks: i64,
    /// Wiedergabe fehlgeschlagen?
    pub failed: bool,
}

impl Client {
    /// PlaybackInfo (POST mit Geräteprofil).
    pub async fn playback_info(
        &self,
        item_id: &str,
        request: &PlaybackInfoRequest,
    ) -> Result<PlaybackInfoResponse> {
        let query = vec![("userId".to_owned(), self.user_id()?)];
        self.call_json(
            Method::POST,
            &format!("/Items/{}/PlaybackInfo", enc(item_id)),
            &query,
            Some(request),
        )
        .await
    }

    /// Meldet den Start der Wiedergabe (`/Sessions/Playing`).
    pub async fn report_playing(&self, report: &PlaybackReport) -> Result<()> {
        self.call_empty(Method::POST, "/Sessions/Playing", &[], Some(report))
            .await
    }

    /// Meldet den Fortschritt (`/Sessions/Playing/Progress`).
    pub async fn report_progress(&self, report: &PlaybackReport) -> Result<()> {
        self.call_empty(
            Method::POST,
            "/Sessions/Playing/Progress",
            &[],
            Some(report),
        )
        .await
    }

    /// Meldet das Ende (`/Sessions/Playing/Stopped`).
    pub async fn report_stopped(&self, stop: &PlaybackStop) -> Result<()> {
        self.call_empty(Method::POST, "/Sessions/Playing/Stopped", &[], Some(stop))
            .await
    }

    /// Direkt-Stream-URL **ohne Token**. mpv bekommt den Header über `http-header-fields`
    /// (siehe [`Client::auth_header_value`]).
    pub fn stream_url(&self, item_id: &str, media_source_id: &str) -> Result<String> {
        let query = vec![
            ("static".to_owned(), "true".to_owned()),
            ("MediaSourceId".to_owned(), media_source_id.to_owned()),
        ];
        Ok(self
            .url(&format!("/Videos/{}/stream", enc(item_id)), &query)?
            .into())
    }

    /// Absolute Adresse einer serverrelativen Angabe (z. B. `DeliveryUrl` externer Untertitel), ohne Token-Zusatz.
    pub fn absolute_url(&self, server_relative: &str) -> Result<String> {
        Ok(self
            .url(
                server_relative.split('?').next().unwrap_or(""),
                &parse_query(server_relative),
            )?
            .into())
    }

    /// URL eines Trickplay-Kachelbilds (`index` beginnt bei 0), ohne Token.
    pub fn trickplay_tile_url(
        &self,
        item_id: &str,
        width: u32,
        index: u32,
        media_source_id: Option<&str>,
    ) -> Result<String> {
        let mut query = Vec::new();
        if let Some(id) = media_source_id {
            query.push(("MediaSourceId".to_owned(), id.to_owned()));
        }
        Ok(self
            .url(
                &format!("/Videos/{}/Trickplay/{width}/{index}.jpg", enc(item_id)),
                &query,
            )?
            .into())
    }
}

fn parse_query(path_and_query: &str) -> Vec<(String, String)> {
    let Some((_, query)) = path_and_query.split_once('?') else {
        return Vec::new();
    };
    reqwest::Url::parse(&format!("http://x.invalid/?{query}"))
        .map(|u| {
            u.query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}
