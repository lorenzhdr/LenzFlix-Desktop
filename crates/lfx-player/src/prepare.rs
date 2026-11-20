//! PlaybackInfo-Ablauf: aus einem Element wird ein [`PlayRequest`].
//!
//! Zuerst Direktwiedergabe (Profil `direct_play_everything`, keine Umwandlung, Untertitel
//! `External`). Nur wenn der Server nichts direkt liefern kann, folgt eine zweite Anfrage mit
//! erlaubter Umwandlung und die HLS-Adresse des Servers.
//!
//! Server-Fallen (Web-TV-Modus): nie `StartTimeTicks` in die HLS-Adresse (bricht die Playlist,
//! der Start läuft über mpv), kein Token in der Adresse (Header statt `api_key`/`ApiKey`), und
//! `SubtitleStreamIndex` nie in der Adresse lassen (sonst brennt der Server Untertitel ein).

use lfx_api::{ApiError, Client, MediaSource, MediaStream, PlayMethod, PlaybackInfoRequest};

use crate::request::{ExternalSubtitle, PlayRequest, StreamKind, StreamMap};

/// Wünsche für die Vorbereitung.
#[derive(Debug, Clone)]
pub struct PrepareOptions {
    /// Bestimmte Quelle (Version) wählen; sonst die erste abspielbare.
    pub media_source_id: Option<String>,
    /// Tonspur (Jellyfin-Index); sonst Standard der Quelle.
    pub audio_stream_index: Option<i32>,
    /// Untertitel (Jellyfin-Index, `-1` = aus); sonst Standard der Quelle.
    pub subtitle_stream_index: Option<i32>,
    /// Startposition in Sekunden.
    pub start_seconds: f64,
    /// Server-Umwandlung als Rückfall erlauben?
    pub allow_transcode: bool,
    /// Höchstbitrate in Bit/s (W2-player, Qualitätswahl): erzwingt die Server-Umwandlung mit dieser Grenze;
    /// `None` = Original (Direktwiedergabe, wenn möglich).
    pub max_bitrate: Option<u64>,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            media_source_id: None,
            audio_stream_index: None,
            subtitle_stream_index: None,
            start_seconds: 0.0,
            allow_transcode: true,
            max_bitrate: None,
        }
    }
}

/// Fehler der Vorbereitung.
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error(transparent)]
    Api(#[from] ApiError),
    /// Der Server liefert keine abspielbare Quelle.
    #[error("keine abspielbare Quelle (Servercode: {0})")]
    NoPlayableSource(String),
}

/// Bereitet die Wiedergabe von `item_id` vor.
pub async fn prepare(
    client: &Client,
    item_id: &str,
    opts: &PrepareOptions,
) -> Result<PlayRequest, PrepareError> {
    let user_id = client.user_id()?;
    let mut request = PlaybackInfoRequest::direct_play(user_id.clone());
    request.media_source_id = opts.media_source_id.clone();
    let info = client.playback_info(item_id, &request).await?;

    let direct = if opts.max_bitrate.is_some() {
        None
    } else {
        pick_source(&info.media_sources, opts.media_source_id.as_deref(), true)
    };
    if let Some(src) = direct {
        let url = client.stream_url(item_id, &src.id)?;
        return Ok(build(
            client,
            item_id,
            src,
            info.play_session_id,
            url,
            PlayMethod::DirectPlay,
            opts,
        ));
    }

    if !opts.allow_transcode {
        return Err(PrepareError::NoPlayableSource(
            info.error_code.unwrap_or_else(|| "keine Quelle".into()),
        ));
    }

    // Rückfall: Umwandlung erlauben. Mit Tonwahl braucht der Server die Quell-Id; der
    // Untertitel-Index entfällt dann (mit `-1` plus Quell-Id würde `SubtitleMethod=Encode` entstehen).
    let source_id = opts
        .media_source_id
        .clone()
        .or_else(|| info.media_sources.first().map(|s| s.id.clone()));
    let mut second = PlaybackInfoRequest::direct_play(user_id);
    second.enable_transcoding = true;
    second.enable_direct_play = false;
    second.enable_direct_stream = false;
    if let Some(b) = opts.max_bitrate {
        second.device_profile.max_streaming_bitrate = i64::try_from(b).unwrap_or(i64::MAX);
    }
    second.media_source_id = source_id;
    if let (Some(a), Some(_)) = (opts.audio_stream_index, &second.media_source_id) {
        second.audio_stream_index = Some(a);
        second.subtitle_stream_index = None;
    }
    let info = client.playback_info(item_id, &second).await?;
    let src = info
        .media_sources
        .iter()
        .find(|s| s.transcoding_url.is_some())
        .ok_or_else(|| {
            PrepareError::NoPlayableSource(
                info.error_code
                    .clone()
                    .unwrap_or_else(|| "keine Umwandlung".into()),
            )
        })?;
    let relative = sanitize_transcode_url(src.transcoding_url.as_deref().unwrap_or(""));
    let url = client.absolute_url(&relative)?;
    Ok(build(
        client,
        item_id,
        src,
        info.play_session_id.clone(),
        url,
        PlayMethod::Transcode,
        opts,
    ))
}

/// Wählt die Quelle: gewünschte Id, sonst die erste, die direkt abspielbar ist.
fn pick_source<'a>(
    sources: &'a [MediaSource],
    wanted: Option<&str>,
    need_direct: bool,
) -> Option<&'a MediaSource> {
    let ok = |s: &&MediaSource| !need_direct || s.supports_direct_play || s.supports_direct_stream;
    wanted
        .and_then(|w| sources.iter().find(|s| s.id == w))
        .filter(ok)
        .or_else(|| sources.iter().find(ok))
}

/// Entfernt aus einer serverrelativen Umwandlungsadresse alles, was nicht hineingehört:
/// Token (`api_key`/`ApiKey`), `StartTimeTicks`, `SubtitleStreamIndex`, `SubtitleMethod`.
pub fn sanitize_transcode_url(relative: &str) -> String {
    const DROP: &[&str] = &[
        "api_key",
        "apikey",
        "starttimeticks",
        "subtitlestreamindex",
        "subtitlemethod",
    ];
    let Some((path, query)) = relative.split_once('?') else {
        return relative.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| {
            let key = p.split('=').next().unwrap_or("").to_ascii_lowercase();
            !p.is_empty() && !DROP.contains(&key.as_str())
        })
        .collect();
    if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

fn stream_kind(s: &MediaStream) -> Option<StreamKind> {
    match s.kind.as_str() {
        "Video" => Some(StreamKind::Video),
        "Audio" => Some(StreamKind::Audio),
        "Subtitle" => Some(StreamKind::Subtitle),
        _ => None,
    }
}

fn build(
    client: &Client,
    item_id: &str,
    src: &MediaSource,
    play_session_id: Option<String>,
    stream_url: String,
    method: PlayMethod,
    opts: &PrepareOptions,
) -> PlayRequest {
    let direct = method != PlayMethod::Transcode;
    let mut req = PlayRequest::new(item_id, src.id.clone(), stream_url);
    req.play_session_id = play_session_id;
    req.http_headers = vec![format!("Authorization: {}", client.auth_header_value())];
    req.start_seconds = opts.start_seconds.max(0.0);
    req.play_method = method;
    req.track_mapping = direct;
    if direct {
        req.audio_stream_index = opts.audio_stream_index.or(src.default_audio_stream_index);
        req.subtitle_stream_index = opts
            .subtitle_stream_index
            .or(src.default_subtitle_stream_index);

        let mut external: Vec<&MediaStream> = Vec::new();
        for s in &src.media_streams {
            let Some(kind) = stream_kind(s) else { continue };
            // Externe Untertitel ohne Adresse lassen sich nicht laden: nicht abbilden.
            if kind == StreamKind::Subtitle && s.is_external {
                if s.delivery_url.is_some() {
                    external.push(s);
                } else {
                    continue;
                }
            }
            req.streams.push(StreamMap {
                index: s.index,
                kind,
                external: s.is_external,
            });
        }
        external.sort_by_key(|s| s.index);
        for s in external {
            let Some(url) = s
                .delivery_url
                .as_deref()
                .and_then(|u| client.absolute_url(u).ok())
            else {
                continue;
            };
            req.external_subtitles.push(ExternalSubtitle {
                url,
                lang: s.language.clone(),
                title: s.display_title.clone().or_else(|| s.title.clone()),
            });
        }
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_drops_token_start_and_subtitle_params() {
        let u = "/videos/x/master.m3u8?MediaSourceId=a&api_key=geheim&StartTimeTicks=5&SubtitleStreamIndex=2&SubtitleMethod=Encode&ApiKey=g2&VideoCodec=h264";
        assert_eq!(
            sanitize_transcode_url(u),
            "/videos/x/master.m3u8?MediaSourceId=a&VideoCodec=h264"
        );
        assert_eq!(sanitize_transcode_url("/a/b"), "/a/b");
        assert_eq!(sanitize_transcode_url("/a?api_key=x"), "/a");
    }
}
