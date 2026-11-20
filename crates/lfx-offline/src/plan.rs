//! Planung eines Downloads: Elemente auflösen (Film, Folge, Staffel, Serie), Quelle und Größe wählen,
//! Download-Adresse bauen, Zubehör (Bilder, Untertitel) bestimmen. Reine Funktionen auf Server-JSON,
//! damit sie ohne Netz testbar sind; nur [`resolve`] spricht mit dem Server.

use crate::error::{OfflineError, Result};
use crate::layout::{safe_extension, safe_segment};
use crate::types::{Quality, TranscodeProfile};
use lfx_api::Client;
use serde_json::Value;

/// Felder, die für die Offline-Anzeige gespeichert werden.
pub(crate) const FIELDS: &str =
    "MediaSources,MediaStreams,Overview,Genres,People,Chapters,Studios,Taglines,ProviderIds";

/// Ein aufgelöstes Ziel: das Element samt Serien-/Staffel-JSON (für Folgen).
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub item: Value,
    pub series: Option<Value>,
    pub season: Option<Value>,
}

/// Elementarten mit einer abspielbaren Datei.
pub(crate) fn is_playable(kind: &str) -> bool {
    matches!(kind, "Movie" | "Episode" | "Video" | "MusicVideo" | "Audio")
}

pub(crate) fn jstr<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub(crate) fn jint(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

/// Entfernt Serverpfade (`Path`) aus dem Element-JSON (privat, für die Anzeige unnötig).
pub(crate) fn scrub(mut item: Value) -> Value {
    if let Some(obj) = item.as_object_mut() {
        obj.remove("Path");
        if let Some(sources) = obj.get_mut("MediaSources").and_then(Value::as_array_mut) {
            for s in sources {
                if let Some(o) = s.as_object_mut() {
                    o.remove("Path");
                }
            }
        }
    }
    item
}

async fn fetch_item(client: &Client, id: &str) -> Result<Value> {
    safe_segment(id)?;
    let query = vec![
        ("userId".to_owned(), client.user_id()?),
        ("fields".to_owned(), FIELDS.to_owned()),
    ];
    Ok(scrub(
        client
            .get_json::<Value>(&format!("/Items/{id}"), &query)
            .await?,
    ))
}

async fn fetch_episodes(
    client: &Client,
    series_id: &str,
    season_id: Option<&str>,
) -> Result<Vec<Value>> {
    safe_segment(series_id)?;
    let mut query = vec![
        ("userId".to_owned(), client.user_id()?),
        ("fields".to_owned(), FIELDS.to_owned()),
    ];
    if let Some(season) = season_id {
        safe_segment(season)?;
        query.push(("seasonId".to_owned(), season.to_owned()));
    }
    let list: Value = client
        .get_json(&format!("/Shows/{series_id}/Episodes"), &query)
        .await?;
    Ok(list
        .get("Items")
        .and_then(Value::as_array)
        .map(|a| a.iter().cloned().map(scrub).collect())
        .unwrap_or_default())
}

/// Löst ein Element in Ziele auf. Serien und Staffeln werden zu ihren Folgen; `only_unplayed` lässt
/// bereits gesehene Folgen weg.
pub(crate) async fn resolve(
    client: &Client,
    item_id: &str,
    only_unplayed: bool,
) -> Result<Vec<Target>> {
    let item = fetch_item(client, item_id).await?;
    let kind = jstr(&item, "Type").unwrap_or("").to_owned();
    match kind.as_str() {
        "Series" => {
            let episodes = fetch_episodes(client, item_id, None).await?;
            Ok(episode_targets(client, episodes, Some(item), None, only_unplayed).await)
        }
        "Season" => {
            let series_id = jstr(&item, "SeriesId")
                .ok_or_else(|| OfflineError::Unsupported("Staffel ohne Serie".into()))?
                .to_owned();
            let episodes = fetch_episodes(client, &series_id, Some(item_id)).await?;
            let series = fetch_item(client, &series_id).await.ok();
            Ok(episode_targets(client, episodes, series, Some(item), only_unplayed).await)
        }
        k if is_playable(k) => {
            let (series, season) = if k == "Episode" {
                parents(client, &item).await
            } else {
                (None, None)
            };
            Ok(vec![Target {
                item,
                series,
                season,
            }])
        }
        other => Err(OfflineError::Unsupported(other.to_owned())),
    }
}

async fn parents(client: &Client, episode: &Value) -> (Option<Value>, Option<Value>) {
    let series = match jstr(episode, "SeriesId") {
        Some(id) => fetch_item(client, id).await.ok(),
        None => None,
    };
    let season = match jstr(episode, "SeasonId") {
        Some(id) => fetch_item(client, id).await.ok(),
        None => None,
    };
    (series, season)
}

async fn episode_targets(
    client: &Client,
    episodes: Vec<Value>,
    series: Option<Value>,
    season: Option<Value>,
    only_unplayed: bool,
) -> Vec<Target> {
    let mut out = Vec::new();
    let mut season_cache: Vec<(String, Option<Value>)> = Vec::new();
    for ep in episodes {
        let played = ep
            .get("UserData")
            .and_then(|u| u.get("Played"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if only_unplayed && played {
            continue;
        }
        if !is_playable(jstr(&ep, "Type").unwrap_or("")) {
            continue;
        }
        // Staffel je Folge: gegebene Staffel, sonst einmal je Staffel-Id nachladen.
        let season_json = match (&season, jstr(&ep, "SeasonId")) {
            (Some(s), _) => Some(s.clone()),
            (None, Some(sid)) => match season_cache.iter().find(|(k, _)| k == sid) {
                Some((_, v)) => v.clone(),
                None => {
                    let fetched = fetch_item(client, sid).await.ok();
                    season_cache.push((sid.to_owned(), fetched.clone()));
                    fetched
                }
            },
            (None, None) => None,
        };
        out.push(Target {
            item: ep,
            series: series.clone(),
            season: season_json,
        });
    }
    out
}

/// Gewählte Medienquelle eines Elements (Standard: die erste).
pub(crate) fn pick_source(item: &Value) -> Option<&Value> {
    item.get("MediaSources")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
}

/// Schätzung der Dateigröße für die Platzprüfung.
pub(crate) fn estimate_size(item: &Value, quality: &Quality) -> u64 {
    let source_size = pick_source(item)
        .and_then(|s| jint(s, "Size"))
        .map(|v| v.max(0) as u64);
    match quality {
        Quality::Original => source_size.unwrap_or(0),
        Quality::Transcoded(p) => {
            let secs = jint(item, "RunTimeTicks").unwrap_or(0).max(0) as u64 / 10_000_000;
            let est = if secs == 0 {
                1_073_741_824
            } else {
                secs.saturating_mul(p.max_bitrate) / 8 * 21 / 20
            };
            // Die Umwandlung wird nie größer als das Original angenommen.
            source_size.map_or(est, |s| est.min(s.max(1)))
        }
    }
}

/// Entfernt Zugangsangaben (`api_key`, `ApiKey`) aus einer serverrelativen Adresse.
pub(crate) fn strip_secret_query(path_and_query: &str) -> String {
    let Some((path, query)) = path_and_query.split_once('?') else {
        return path_and_query.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| {
            let key = p.split('=').next().unwrap_or("").to_ascii_lowercase();
            key != "api_key" && key != "apikey"
        })
        .collect();
    if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

/// Adresse der Umwandlung (`/Videos/{id}/stream.mp4`). Bewusst ohne `StartTimeTicks`,
/// `SubtitleStreamIndex`/`SubtitleMethod` (der Server soll nichts einbrennen) und ohne Token;
/// die Anmeldung läuft über den Header.
pub(crate) fn transcode_url(
    client: &Client,
    item_id: &str,
    source_id: &str,
    profile: &TranscodeProfile,
    play_session_id: &str,
) -> Result<String> {
    let video_bitrate = profile.max_bitrate.saturating_sub(128_000).max(100_000);
    let mut query: Vec<(String, String)> = vec![
        ("MediaSourceId".into(), source_id.to_owned()),
        ("DeviceId".into(), client.info().device_id.clone()),
        ("PlaySessionId".into(), play_session_id.to_owned()),
        ("VideoCodec".into(), "h264".into()),
        ("AudioCodec".into(), "aac".into()),
        (
            "MaxStreamingBitrate".into(),
            profile.max_bitrate.to_string(),
        ),
        ("VideoBitRate".into(), video_bitrate.to_string()),
        ("AudioBitRate".into(), "128000".into()),
        ("TranscodingMaxAudioChannels".into(), "2".into()),
    ];
    if let Some(w) = profile.max_width {
        query.push(("maxWidth".into(), w.to_string()));
    }
    if let Some(h) = profile.max_height {
        query.push(("maxHeight".into(), h.to_string()));
    }
    Ok(client
        .url(&format!("/Videos/{item_id}/stream.mp4"), &query)?
        .into())
}

/// Ein zu ladender Untertitel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubtitlePlan {
    pub index: i32,
    pub url_path: String,
    pub extension: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub is_default: bool,
}

/// Bestimmt die Untertitel, die als Dateien mitgeladen werden: externe Textuntertitel immer; bei
/// umgewandelten Downloads zusätzlich eingebettete Textuntertitel (stecken sonst nicht in der MP4).
pub(crate) fn subtitle_plans(
    item_id: &str,
    source: &Value,
    quality: &Quality,
) -> Vec<SubtitlePlan> {
    let source_id = jstr(source, "Id").unwrap_or("");
    let Some(streams) = source.get("MediaStreams").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in streams {
        if jstr(s, "Type") != Some("Subtitle") {
            continue;
        }
        let external = s
            .get("IsExternal")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = s
            .get("IsTextSubtitleStream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let wanted = if external {
            true
        } else {
            matches!(quality, Quality::Transcoded(_)) && text
        };
        let Some(index) = jint(s, "Index").and_then(|i| i32::try_from(i).ok()) else {
            continue;
        };
        if !wanted {
            continue;
        }
        let codec = jstr(s, "Codec").map(str::to_owned);
        let extension = safe_extension(codec.as_deref().unwrap_or(""), "srt");
        let url_path = match jstr(s, "DeliveryUrl") {
            Some(d) if external => strip_secret_query(d),
            _ => format!("/Videos/{item_id}/{source_id}/Subtitles/{index}/0/Stream.{extension}"),
        };
        out.push(SubtitlePlan {
            index,
            url_path,
            extension,
            language: jstr(s, "Language").map(str::to_owned),
            title: jstr(s, "DisplayTitle")
                .or_else(|| jstr(s, "Title"))
                .map(str::to_owned),
            codec,
            is_default: s.get("IsDefault").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    out
}

/// Ein zu ladendes Bild: (Art, Element-Id, Bildart der API, Tag).
pub(crate) struct ImagePlan {
    pub kind: &'static str,
    pub owner_id: String,
    pub api_kind: lfx_api::ImageType,
    pub tag: Option<String>,
}

/// Bestimmt Poster, Hintergrund und Logo; Folgen greifen auf Serienbilder zurück.
pub(crate) fn image_plans(item: &Value) -> Vec<ImagePlan> {
    let id = jstr(item, "Id").unwrap_or("").to_owned();
    let tags = item.get("ImageTags");
    let tag_of = |k: &str| {
        tags.and_then(|t| t.get(k))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let mut out = Vec::new();
    // Poster
    if let Some(tag) = tag_of("Primary") {
        out.push(ImagePlan {
            kind: "primary",
            owner_id: id.clone(),
            api_kind: lfx_api::ImageType::Primary,
            tag: Some(tag),
        });
    } else if let (Some(series), Some(tag)) =
        (jstr(item, "SeriesId"), jstr(item, "SeriesPrimaryImageTag"))
    {
        out.push(ImagePlan {
            kind: "primary",
            owner_id: series.to_owned(),
            api_kind: lfx_api::ImageType::Primary,
            tag: Some(tag.to_owned()),
        });
    }
    // Hintergrund
    let first = |v: Option<&Value>| {
        v.and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    if let Some(tag) = first(item.get("BackdropImageTags")) {
        out.push(ImagePlan {
            kind: "backdrop",
            owner_id: id.clone(),
            api_kind: lfx_api::ImageType::Backdrop,
            tag: Some(tag),
        });
    } else if let (Some(owner), Some(tag)) = (
        jstr(item, "ParentBackdropItemId"),
        first(item.get("ParentBackdropImageTags")),
    ) {
        out.push(ImagePlan {
            kind: "backdrop",
            owner_id: owner.to_owned(),
            api_kind: lfx_api::ImageType::Backdrop,
            tag: Some(tag),
        });
    }
    // Logo
    if let Some(tag) = tag_of("Logo") {
        out.push(ImagePlan {
            kind: "logo",
            owner_id: id,
            api_kind: lfx_api::ImageType::Logo,
            tag: Some(tag),
        });
    } else if let (Some(owner), Some(tag)) = (
        jstr(item, "ParentLogoItemId"),
        jstr(item, "ParentLogoImageTag"),
    ) {
        out.push(ImagePlan {
            kind: "logo",
            owner_id: owner.to_owned(),
            api_kind: lfx_api::ImageType::Logo,
            tag: Some(tag.to_owned()),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_keys() {
        assert_eq!(
            strip_secret_query("/a/b?x=1&api_key=geheim&ApiKey=g2&y=2"),
            "/a/b?x=1&y=2"
        );
        assert_eq!(strip_secret_query("/a?api_key=geheim"), "/a");
        assert_eq!(strip_secret_query("/a"), "/a");
    }

    #[test]
    fn scrub_removes_paths() {
        let v = scrub(json!({"Path": "/srv/x", "MediaSources": [{"Path": "/srv/y", "Id": "m"}]}));
        assert!(v.get("Path").is_none());
        assert!(v["MediaSources"][0].get("Path").is_none());
        assert_eq!(v["MediaSources"][0]["Id"], "m");
    }

    #[test]
    fn subtitle_plan_original_and_transcoded() {
        let source = json!({"Id": "m1", "MediaStreams": [
            {"Index": 3, "Type": "Subtitle", "Codec": "srt", "IsTextSubtitleStream": true, "IsExternal": false},
            {"Index": 5, "Type": "Subtitle", "Codec": "srt", "Language": "fra", "IsExternal": true,
             "DeliveryUrl": "/Videos/i/m1/Subtitles/5/0/Stream.srt?api_key=geheim"},
            {"Index": 6, "Type": "Subtitle", "Codec": "pgssub", "IsExternal": false}
        ]});
        let orig = subtitle_plans("i", &source, &Quality::Original);
        assert_eq!(orig.len(), 1);
        assert_eq!(orig[0].url_path, "/Videos/i/m1/Subtitles/5/0/Stream.srt");
        let tc = subtitle_plans(
            "i",
            &source,
            &Quality::Transcoded(TranscodeProfile::small()),
        );
        assert_eq!(tc.iter().map(|p| p.index).collect::<Vec<_>>(), [3, 5]);
        assert!(tc[0].url_path.ends_with("/Subtitles/3/0/Stream.srt"));
    }

    #[test]
    fn size_estimate() {
        let item = json!({"RunTimeTicks": 3600i64 * 10_000_000, "MediaSources": [{"Size": 5_000_000_000i64}]});
        assert_eq!(estimate_size(&item, &Quality::Original), 5_000_000_000);
        let tc = estimate_size(&item, &Quality::Transcoded(TranscodeProfile::medium()));
        assert!(tc > 1_300_000_000 && tc < 1_450_000_000, "{tc}");
        let small = json!({"RunTimeTicks": 3600i64 * 10_000_000, "MediaSources": [{"Size": 100}]});
        assert_eq!(
            estimate_size(&small, &Quality::Transcoded(TranscodeProfile::medium())),
            100
        );
    }

    #[test]
    fn image_fallbacks_for_episodes() {
        let ep = json!({"Id": "e1", "SeriesId": "s1", "SeriesPrimaryImageTag": "t1",
            "ParentBackdropItemId": "s1", "ParentBackdropImageTags": ["b1"],
            "ParentLogoItemId": "s1", "ParentLogoImageTag": "l1"});
        let plans = image_plans(&ep);
        let kinds: Vec<_> = plans
            .iter()
            .map(|p| (p.kind, p.owner_id.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [("primary", "s1"), ("backdrop", "s1"), ("logo", "s1")]
        );
    }
}
