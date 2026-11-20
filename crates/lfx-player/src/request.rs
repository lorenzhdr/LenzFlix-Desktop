//! Wiedergabe-Auftrag: alles, was der Player zum Öffnen eines Titels braucht.

use std::fmt;

use lfx_api::{PlayMethod, redact_url};

/// Art eines Streams in der Medienquelle (Jellyfin-Sicht).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
}

/// Ein Stream der Medienquelle. Dient dem Abbilden von Jellyfin-Stream-Indizes auf mpv-Spur-Ids
/// (und zurück für Fortschrittsmeldungen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamMap {
    /// Jellyfin-Index in der Quelle.
    pub index: i32,
    pub kind: StreamKind,
    /// Externe Datei (wird per `sub-add` geladen) statt im Container eingebettet.
    pub external: bool,
}

/// Externer Untertitel, der nach dem Laden per `sub-add` angehängt wird.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSubtitle {
    /// Absolute Adresse (ohne Token; Anmeldung läuft über `http_headers`).
    pub url: String,
    /// Sprache (ISO 639), falls bekannt.
    pub lang: Option<String>,
    /// Anzeigetitel, falls bekannt.
    pub title: Option<String>,
}

/// Auftrag für [`crate::Player::open`].
///
/// Die Adresse enthält nie ein Token; der `Authorization`-Header steht in `http_headers` und wird
/// als mpv-Option `http-header-fields` gesetzt. `Debug` schwärzt beides.
#[derive(Clone)]
pub struct PlayRequest {
    /// Jellyfin-Element.
    pub item_id: String,
    /// Medienquelle.
    pub media_source_id: String,
    /// Sitzungs-Id aus PlaybackInfo (für Fortschrittsmeldungen).
    pub play_session_id: Option<String>,
    /// Direkt-Stream- oder HLS-Adresse.
    pub stream_url: String,
    /// Zusätzliche HTTP-Header im Format `Name: Wert` (enthält das Token, nie loggen).
    pub http_headers: Vec<String>,
    /// Startposition in Sekunden (immer über mpv `start`, nie über die URL).
    pub start_seconds: f64,
    /// Gewünschte Tonspur (Jellyfin-Index).
    pub audio_stream_index: Option<i32>,
    /// Gewünschte Untertitelspur (Jellyfin-Index; `-1` = aus; `None` = mpv-Vorgabe).
    pub subtitle_stream_index: Option<i32>,
    /// Externe Untertitel in Reihenfolge der Jellyfin-Indizes.
    pub external_subtitles: Vec<ExternalSubtitle>,
    /// Streams der Quelle (für die Indexumrechnung, siehe `track_mapping`).
    pub streams: Vec<StreamMap>,
    /// Gilt die Zuordnung Jellyfin-Index zu mpv-Spur? Bei Server-Umwandlung (nur gewählte Spuren
    /// im Strom) `false`.
    pub track_mapping: bool,
    /// Wiedergabeart für Fortschrittsmeldungen.
    pub play_method: PlayMethod,
}

impl PlayRequest {
    /// Minimaler Auftrag (Direktwiedergabe, ohne Spurwahl).
    pub fn new(
        item_id: impl Into<String>,
        media_source_id: impl Into<String>,
        stream_url: impl Into<String>,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            media_source_id: media_source_id.into(),
            play_session_id: None,
            stream_url: stream_url.into(),
            http_headers: Vec::new(),
            start_seconds: 0.0,
            audio_stream_index: None,
            subtitle_stream_index: None,
            external_subtitles: Vec::new(),
            streams: Vec::new(),
            track_mapping: false,
            play_method: PlayMethod::DirectPlay,
        }
    }

    /// mpv-Spur-Id (1-basiert je Art) zu einem Jellyfin-Stream-Index.
    ///
    /// Eingebettete Spuren zählen in Containerreihenfolge; externe Untertitel hängt der Player
    /// danach in der Reihenfolge von [`PlayRequest::external_subtitles`] an.
    pub fn mpv_track_id(&self, kind: StreamKind, jellyfin_index: i32) -> Option<i64> {
        if !self.track_mapping {
            return None;
        }
        let embedded = |k: StreamKind| {
            self.streams
                .iter()
                .filter(move |s| s.kind == k && !s.external)
        };
        let target = self
            .streams
            .iter()
            .find(|s| s.kind == kind && s.index == jellyfin_index)?;
        if target.external {
            let n_embedded = embedded(kind).count();
            let pos = self
                .streams
                .iter()
                .filter(|s| s.kind == kind && s.external)
                .position(|s| s.index == jellyfin_index)?;
            Some((n_embedded + pos + 1) as i64)
        } else {
            let pos = embedded(kind).position(|s| s.index == jellyfin_index)?;
            Some(pos as i64 + 1)
        }
    }

    /// Jellyfin-Index zu einer mpv-Spur-Id (Umkehrung von [`PlayRequest::mpv_track_id`]).
    pub fn jellyfin_index(&self, kind: StreamKind, mpv_id: i64) -> Option<i32> {
        if !self.track_mapping || mpv_id < 1 {
            return None;
        }
        let n_embedded = self
            .streams
            .iter()
            .filter(|s| s.kind == kind && !s.external)
            .count() as i64;
        let (external, pos) = if mpv_id > n_embedded {
            (true, (mpv_id - n_embedded - 1) as usize)
        } else {
            (false, (mpv_id - 1) as usize)
        };
        self.streams
            .iter()
            .filter(|s| s.kind == kind && s.external == external)
            .nth(pos)
            .map(|s| s.index)
    }
}

impl fmt::Debug for PlayRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayRequest")
            .field("item_id", &self.item_id)
            .field("media_source_id", &self.media_source_id)
            .field("stream_url", &redact_url(&self.stream_url))
            .field(
                "http_headers",
                &format_args!("<{} Header>", self.http_headers.len()),
            )
            .field("start_seconds", &self.start_seconds)
            .field("audio_stream_index", &self.audio_stream_index)
            .field("subtitle_stream_index", &self.subtitle_stream_index)
            .field("external_subtitles", &self.external_subtitles.len())
            .field("play_method", &self.play_method)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PlayRequest {
        let mut r = PlayRequest::new("i", "m", "http://h/x");
        r.track_mapping = true;
        let s = |index, kind, external| StreamMap {
            index,
            kind,
            external,
        };
        r.streams = vec![
            s(0, StreamKind::Video, false),
            s(1, StreamKind::Audio, false),
            s(2, StreamKind::Audio, false),
            s(3, StreamKind::Subtitle, false),
            s(4, StreamKind::Subtitle, false),
            s(5, StreamKind::Subtitle, true),
            s(6, StreamKind::Subtitle, true),
        ];
        r
    }

    #[test]
    fn maps_jellyfin_index_to_mpv_ids() {
        let r = sample();
        assert_eq!(r.mpv_track_id(StreamKind::Audio, 1), Some(1));
        assert_eq!(r.mpv_track_id(StreamKind::Audio, 2), Some(2));
        assert_eq!(r.mpv_track_id(StreamKind::Subtitle, 3), Some(1));
        assert_eq!(r.mpv_track_id(StreamKind::Subtitle, 4), Some(2));
        // Externe folgen nach den eingebetteten.
        assert_eq!(r.mpv_track_id(StreamKind::Subtitle, 5), Some(3));
        assert_eq!(r.mpv_track_id(StreamKind::Subtitle, 6), Some(4));
        assert_eq!(r.mpv_track_id(StreamKind::Audio, 9), None);
        assert_eq!(r.mpv_track_id(StreamKind::Audio, 3), None);
    }

    #[test]
    fn maps_back_and_forth() {
        let r = sample();
        for (kind, idx) in [
            (StreamKind::Audio, 1),
            (StreamKind::Audio, 2),
            (StreamKind::Subtitle, 3),
            (StreamKind::Subtitle, 4),
            (StreamKind::Subtitle, 5),
            (StreamKind::Subtitle, 6),
        ] {
            let id = r.mpv_track_id(kind, idx).unwrap();
            assert_eq!(r.jellyfin_index(kind, id), Some(idx));
        }
        assert_eq!(r.jellyfin_index(StreamKind::Subtitle, 7), None);
    }

    #[test]
    fn no_mapping_without_flag() {
        let mut r = sample();
        r.track_mapping = false;
        assert_eq!(r.mpv_track_id(StreamKind::Audio, 1), None);
        assert_eq!(r.jellyfin_index(StreamKind::Audio, 1), None);
    }

    #[test]
    fn debug_hides_headers_and_token() {
        let mut r = PlayRequest::new("i", "m", "http://h/x?ApiKey=geheim123");
        r.http_headers
            .push("Authorization: MediaBrowser Token=\"geheim456\"".into());
        let d = format!("{r:?}");
        assert!(!d.contains("geheim"), "{d}");
        assert!(!d.contains("http://h"), "{d}");
    }
}
