//! Live-TV, Benutzer-Einstellungen und Verwaltung (Administrator).
//!
//! Die Verwaltungsmethoden brauchen ein Administratorkonto; sonst antwortet der Server mit
//! [`crate::ApiError::Forbidden`]. Pfade und Parameter nach der OpenAPI-Spec (Jellyfin 12.4).

use crate::branding::{Branding, parse_branding};
use crate::client::{Client, Payload};
use crate::dto::*;
use crate::dto_ext::*;
use crate::endpoints::{NO_BODY, enc, opt, q};
use crate::error::{ApiError, Result};
use crate::playback::PlaybackInfoRequest;
use reqwest::Method;
use serde_json::{Value, json};

/// Zeitraum und Auswahl für den Sendeplan ([`Client::live_programs`]). Zeiten als ISO-8601 (UTC, z. B.
/// `2026-01-01T20:00:00Z`).
#[derive(Debug, Clone, Default)]
pub struct ProgramsQuery {
    /// Nur diese Sender.
    pub channel_ids: Vec<String>,
    /// Sendungen, die nach diesem Zeitpunkt noch laufen (Ende ≥).
    pub min_end_date: Option<String>,
    /// Sendungen, die vor diesem Zeitpunkt beginnen (Beginn ≤).
    pub max_start_date: Option<String>,
    /// Nur Sendungen, die gerade laufen.
    pub is_airing: Option<bool>,
    /// Höchstzahl.
    pub limit: Option<u32>,
    /// Startindex.
    pub start_index: Option<u32>,
}

/// Magere Base64-Kodierung (Standardalphabet, mit Auffüllung) für den Profilbild-Upload.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Erkennt PNG/JPEG/WebP/GIF am Anfang der Datei.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
}

impl Client {
    // ------------------------------------------------------------ Live-TV

    /// Sender (`/LiveTv/Channels`), optional mit der laufenden Sendung.
    pub async fn live_channels(
        &self,
        add_current_program: bool,
        limit: u32,
        start_index: u32,
    ) -> Result<QueryResult<BaseItem>> {
        let extra = vec![
            (
                "addCurrentProgram".to_owned(),
                add_current_program.to_string(),
            ),
            ("limit".to_owned(), limit.to_string()),
            ("startIndex".to_owned(), start_index.to_string()),
            ("enableTotalRecordCount".to_owned(), "true".to_owned()),
            ("sortBy".to_owned(), "SortName".to_owned()),
        ];
        self.get_json("/LiveTv/Channels", &self.with_user(&extra)?)
            .await
    }

    /// Sendeplan (Programme/Guide) für einen Zeitraum (`/LiveTv/Programs`).
    pub async fn live_programs(&self, query: &ProgramsQuery) -> Result<QueryResult<BaseItem>> {
        let mut extra = vec![("enableTotalRecordCount".to_owned(), "false".to_owned())];
        if !query.channel_ids.is_empty() {
            extra.push(("channelIds".to_owned(), query.channel_ids.join(",")));
        }
        opt(&mut extra, "minEndDate", query.min_end_date.clone());
        opt(&mut extra, "maxStartDate", query.max_start_date.clone());
        opt(
            &mut extra,
            "isAiring",
            query.is_airing.map(|b| b.to_string()),
        );
        opt(&mut extra, "limit", query.limit.map(|v| v.to_string()));
        opt(
            &mut extra,
            "startIndex",
            query.start_index.map(|v| v.to_string()),
        );
        extra.push(("sortBy".to_owned(), "StartDate".to_owned()));
        self.get_json("/LiveTv/Programs", &self.with_user(&extra)?)
            .await
    }

    /// Aufnahmen (`/LiveTv/Recordings`).
    pub async fn recordings(&self, limit: u32, start_index: u32) -> Result<QueryResult<BaseItem>> {
        let extra = vec![
            ("limit".to_owned(), limit.to_string()),
            ("startIndex".to_owned(), start_index.to_string()),
            ("enableTotalRecordCount".to_owned(), "true".to_owned()),
        ];
        self.get_json("/LiveTv/Recordings", &self.with_user(&extra)?)
            .await
    }

    /// Löscht eine Aufnahme.
    pub async fn delete_recording(&self, recording_id: &str) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            &format!("/LiveTv/Recordings/{}", enc(recording_id)),
            &[],
            NO_BODY,
        )
        .await
    }

    /// Geplante Aufnahmen (`/LiveTv/Timers`).
    pub async fn timers(&self) -> Result<QueryResult<TimerInfo>> {
        self.get_json("/LiveTv/Timers", &[]).await
    }

    /// Vorgabewerte für eine Aufnahme dieser Sendung (Vorlauf, Nachlauf, Kanal, Zeiten).
    pub async fn timer_defaults(&self, program_id: &str) -> Result<Value> {
        self.get_json(
            "/LiveTv/Timers/Defaults",
            &q(&[("programId", program_id.to_owned())]),
        )
        .await
    }

    /// Legt einen Timer an; `timer` ist ein Objekt wie die Antwort von [`Client::timer_defaults`].
    pub async fn create_timer(&self, timer: &Value) -> Result<()> {
        self.call_empty(Method::POST, "/LiveTv/Timers", &[], Some(timer))
            .await
    }

    /// Nimmt eine Sendung auf: holt die Vorgabewerte und legt den Timer an.
    pub async fn record_program(&self, program_id: &str) -> Result<()> {
        let defaults = self.timer_defaults(program_id).await?;
        self.create_timer(&defaults).await
    }

    /// Löscht einen Timer.
    pub async fn cancel_timer(&self, timer_id: &str) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            &format!("/LiveTv/Timers/{}", enc(timer_id)),
            &[],
            NO_BODY,
        )
        .await
    }

    /// Öffnet den Live-Stream eines Senders (oder einer Aufnahme): PlaybackInfo des Senders. Die Quelle trägt
    /// `live_stream_id`, `transcoding_url` oder den Direktpfad; beim Beenden [`Client::close_live_stream`] rufen.
    pub async fn open_live_stream(
        &self,
        channel_id: &str,
        request: &PlaybackInfoRequest,
    ) -> Result<PlaybackInfoResponse> {
        self.playback_info(channel_id, request).await
    }

    /// Schließt einen Live-Stream (`POST /LiveStreams/Close`).
    pub async fn close_live_stream(&self, live_stream_id: &str) -> Result<()> {
        self.call_empty(
            Method::POST,
            "/LiveStreams/Close",
            &q(&[("liveStreamId", live_stream_id.to_owned())]),
            NO_BODY,
        )
        .await
    }

    // ------------------------------------------------------------ Benutzer-Einstellungen

    /// Konfiguration des angemeldeten Benutzers (Sprachen, Untertitelmodus, ...).
    pub async fn user_configuration(&self) -> Result<UserConfiguration> {
        Ok(self.me().await?.configuration.unwrap_or_default())
    }

    /// Speichert die Konfiguration (ersetzt das ganze Objekt; vorher mit [`Client::user_configuration`] lesen
    /// und nur ändern, was nötig ist).
    pub async fn update_user_configuration(&self, config: &UserConfiguration) -> Result<()> {
        self.call_empty(
            Method::POST,
            "/Users/Configuration",
            &q(&[("userId", self.user_id()?)]),
            Some(config),
        )
        .await
    }

    /// Ändert das eigene Passwort.
    pub async fn change_password(&self, change: &PasswordChange) -> Result<()> {
        self.call_empty(
            Method::POST,
            "/Users/Password",
            &q(&[("userId", self.user_id()?)]),
            Some(change),
        )
        .await
    }

    /// Lädt ein Profilbild hoch (PNG, JPEG, WebP oder GIF). Der Server erwartet den Inhalt Base64-kodiert.
    pub async fn upload_profile_image(&self, image: &[u8]) -> Result<()> {
        let content_type = sniff_image(image)
            .ok_or_else(|| ApiError::Decode("kein unterstütztes Bildformat".into()))?
            .to_owned();
        self.send_payload(
            Method::POST,
            "/UserImage",
            &q(&[("userId", self.user_id()?)]),
            Some(Payload::Raw {
                content_type,
                bytes: base64(image).into_bytes(),
            }),
        )
        .await
        .map(|_| ())
    }

    /// Entfernt das Profilbild.
    pub async fn delete_profile_image(&self) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            "/UserImage",
            &q(&[("userId", self.user_id()?)]),
            NO_BODY,
        )
        .await
    }

    // ------------------------------------------------------------ Branding

    /// Eigenes CSS des Servers (`/Branding/Css`, ohne Anmeldung).
    pub async fn branding_css(&self) -> Result<String> {
        self.get_text("/Branding/Css", &[]).await
    }

    /// Branding-CSS ausgewertet: Charts-/Statistik-URL und Senderbelegung des Web-Themes.
    pub async fn branding(&self) -> Result<Branding> {
        Ok(parse_branding(&self.branding_css().await?))
    }

    /// Branding-Optionen (`/Branding/Configuration`): Anmeldehinweis und CSS.
    pub async fn branding_options(&self) -> Result<BrandingOptions> {
        self.get_json("/Branding/Configuration", &[]).await
    }

    // ------------------------------------------------------------ Verwaltung: System

    /// Systeminformation (`/System/Info`).
    pub async fn system_info(&self) -> Result<SystemInfo> {
        self.get_json("/System/Info", &[]).await
    }

    /// Startet den Server neu.
    pub async fn restart_server(&self) -> Result<()> {
        self.call_empty(Method::POST, "/System/Restart", &[], NO_BODY)
            .await
    }

    /// Fährt den Server herunter.
    pub async fn shutdown_server(&self) -> Result<()> {
        self.call_empty(Method::POST, "/System/Shutdown", &[], NO_BODY)
            .await
    }

    /// Aktivitätsprotokoll (neueste zuerst).
    pub async fn activity_log(
        &self,
        limit: u32,
        start_index: u32,
        min_date: Option<&str>,
    ) -> Result<QueryResult<ActivityLogEntry>> {
        let mut query = vec![
            ("limit".to_owned(), limit.to_string()),
            ("startIndex".to_owned(), start_index.to_string()),
        ];
        opt(&mut query, "minDate", min_date.map(str::to_owned));
        self.get_json("/System/ActivityLog/Entries", &query).await
    }

    /// Liste der Logdateien.
    pub async fn logs(&self) -> Result<Vec<LogFile>> {
        self.get_json("/System/Logs", &[]).await
    }

    /// Inhalt einer Logdatei als Text. Der Name stammt aus [`Client::logs`] und wird prozentkodiert gesendet.
    pub async fn log_file(&self, name: &str) -> Result<String> {
        self.get_text("/System/Logs/Log", &q(&[("name", name.to_owned())]))
            .await
    }

    // ------------------------------------------------------------ Verwaltung: Sitzungen, Geräte

    /// Aktive Sitzungen; `active_within_seconds` begrenzt auf zuletzt aktive.
    pub async fn sessions(&self, active_within_seconds: Option<u32>) -> Result<Vec<SessionInfo>> {
        let mut query = Vec::new();
        opt(
            &mut query,
            "activeWithinSeconds",
            active_within_seconds.map(|v| v.to_string()),
        );
        self.get_json("/Sessions", &query).await
    }

    /// Zeigt auf dem Gerät einer Sitzung eine Nachricht.
    pub async fn session_message(
        &self,
        session_id: &str,
        header: &str,
        text: &str,
        timeout_ms: u32,
    ) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!("/Sessions/{}/Message", enc(session_id)),
            &[],
            Some(&json!({ "Header": header, "Text": text, "TimeoutMs": timeout_ms })),
        )
        .await
    }

    /// Steuert die Wiedergabe einer Sitzung (Pause, Stopp, Springen ...).
    pub async fn session_playstate(
        &self,
        session_id: &str,
        command: PlaystateCommand,
        seek_position_ticks: Option<i64>,
    ) -> Result<()> {
        let mut query = Vec::new();
        opt(
            &mut query,
            "seekPositionTicks",
            seek_position_ticks.map(|v| v.to_string()),
        );
        self.call_empty(
            Method::POST,
            &format!("/Sessions/{}/Playing/{}", enc(session_id), command.as_str()),
            &query,
            NO_BODY,
        )
        .await
    }

    /// Sendet einen allgemeinen Befehl an eine Sitzung (z. B. `GoHome`, `ToggleMute`, `VolumeUp`).
    pub async fn session_command(&self, session_id: &str, command: &str) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!("/Sessions/{}/Command/{}", enc(session_id), enc(command)),
            &[],
            NO_BODY,
        )
        .await
    }

    /// Registrierte Geräte.
    pub async fn devices(&self) -> Result<QueryResult<DeviceInfo>> {
        self.get_json("/Devices", &[]).await
    }

    /// Entfernt ein Gerät (meldet es ab).
    pub async fn delete_device(&self, device_id: &str) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            "/Devices",
            &q(&[("id", device_id.to_owned())]),
            NO_BODY,
        )
        .await
    }

    // ------------------------------------------------------------ Verwaltung: Aufgaben, Bibliotheken

    /// Geplante Aufgaben.
    pub async fn scheduled_tasks(&self, include_hidden: bool) -> Result<Vec<TaskInfo>> {
        let query = if include_hidden {
            Vec::new()
        } else {
            q(&[("isHidden", "false".to_owned())])
        };
        self.get_json("/ScheduledTasks", &query).await
    }

    /// Startet eine Aufgabe.
    pub async fn start_task(&self, task_id: &str) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!("/ScheduledTasks/Running/{}", enc(task_id)),
            &[],
            NO_BODY,
        )
        .await
    }

    /// Bricht eine laufende Aufgabe ab.
    pub async fn stop_task(&self, task_id: &str) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            &format!("/ScheduledTasks/Running/{}", enc(task_id)),
            &[],
            NO_BODY,
        )
        .await
    }

    /// Aktualisiert alle Bibliotheken (`POST /Library/Refresh`).
    pub async fn refresh_library(&self) -> Result<()> {
        self.call_empty(Method::POST, "/Library/Refresh", &[], NO_BODY)
            .await
    }

    /// Bibliotheken (virtuelle Ordner).
    pub async fn virtual_folders(&self) -> Result<Vec<VirtualFolder>> {
        self.get_json("/Library/VirtualFolders", &[]).await
    }

    /// Legt eine Bibliothek an (`collection_type` z. B. `movies`, `tvshows`, `music`).
    pub async fn add_virtual_folder(
        &self,
        name: &str,
        collection_type: &str,
        paths: &[&str],
        refresh: bool,
    ) -> Result<()> {
        let mut query = q(&[
            ("name", name.to_owned()),
            ("collectionType", collection_type.to_owned()),
            ("refreshLibrary", refresh.to_string()),
        ]);
        if !paths.is_empty() {
            query.push(("paths".to_owned(), paths.join(",")));
        }
        self.call_empty(Method::POST, "/Library/VirtualFolders", &query, NO_BODY)
            .await
    }

    /// Entfernt eine Bibliothek (nur aus Jellyfin, nicht von der Platte).
    pub async fn remove_virtual_folder(&self, name: &str, refresh: bool) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            "/Library/VirtualFolders",
            &q(&[
                ("name", name.to_owned()),
                ("refreshLibrary", refresh.to_string()),
            ]),
            NO_BODY,
        )
        .await
    }

    // ------------------------------------------------------------ Verwaltung: Benutzer

    /// Alle Benutzer.
    pub async fn users(&self) -> Result<Vec<UserDto>> {
        self.get_json("/Users", &[]).await
    }

    /// Ein Benutzer samt Richtlinie.
    pub async fn user(&self, user_id: &str) -> Result<UserDto> {
        self.get_json(&format!("/Users/{}", enc(user_id)), &[])
            .await
    }

    /// Legt einen Benutzer an.
    pub async fn create_user(&self, name: &str, password: &str) -> Result<UserDto> {
        self.call_json(
            Method::POST,
            "/Users/New",
            &[],
            Some(&json!({ "Name": name, "Password": password })),
        )
        .await
    }

    /// Speichert die Richtlinie eines Benutzers (vorher mit [`Client::user`] lesen, dann ändern).
    pub async fn update_user_policy(&self, user_id: &str, policy: &UserPolicy) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!("/Users/{}/Policy", enc(user_id)),
            &[],
            Some(policy),
        )
        .await
    }

    /// Löscht einen Benutzer.
    pub async fn delete_user(&self, user_id: &str) -> Result<()> {
        self.call_empty(
            Method::DELETE,
            &format!("/Users/{}", enc(user_id)),
            &[],
            NO_BODY,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn sniffs_image_formats() {
        assert_eq!(
            sniff_image(&[0x89, b'P', b'N', b'G', 0, 0]),
            Some("image/png")
        );
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"RIFFxxxxWEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<html>"), None);
    }
}
