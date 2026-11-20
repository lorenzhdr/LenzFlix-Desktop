//! Einordnung einer PlaybackInfo (Port der Entscheidungslogik von `player/stream.js`): direkt,
//! HLS-Remux oder „nicht empfangbar“ mit Fehlercode, dazu Hinweis auf ungeeignete Bildformate.
//!
//! Die Eingabe ist eine Zusammenfassung ohne URLs: die Transcoding-Adresse trägt den Schlüssel und
//! darf nie in Debug-Ausgaben, Ereignisse oder Fehler gelangen. [`PlaybackSummary`] enthält darum nur
//! abgeleitete Wahrheitswerte und Gründe (siehe [`summarize_transcoding_url`]).

use crate::tracks::{StreamInfo, StreamType};

/// Gründe, die auf ein Problem mit dem Video hindeuten (Server meldet `...NotSupported`).
pub const VIDEO_REASONS: [&str; 14] = [
    "VideoCodecNotSupported",
    "VideoProfileNotSupported",
    "VideoLevelNotSupported",
    "VideoResolutionNotSupported",
    "VideoBitDepthNotSupported",
    "VideoFramerateNotSupported",
    "RefFramesNotSupported",
    "AnamorphicVideoNotSupported",
    "InterlacedVideoNotSupported",
    "VideoBitrateNotSupported",
    "VideoRangeTypeNotSupported",
    "VideoCodecTagNotSupported",
    "ContainerBitrateExceedsLimit",
    "SubtitleCodecNotSupported",
];

/// Warum eine Wiedergabe scheitert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    Video,
    Audio,
    Browser,
    Network,
    Gesture,
    Aborted,
}

impl FailReason {
    /// Name wie im Web.
    pub fn as_str(self) -> &'static str {
        match self {
            FailReason::Video => "video",
            FailReason::Audio => "audio",
            FailReason::Browser => "browser",
            FailReason::Network => "network",
            FailReason::Gesture => "gesture",
            FailReason::Aborted => "aborted",
        }
    }
}

/// Kurzcode für die Fehlerkarte zu den Gründen des Servers.
pub fn video_code(reasons: &[String]) -> &'static str {
    const BY: [(&str, &str); 6] = [
        ("VideoCodecNotSupported", "V-CODEC"),
        ("VideoCodecTagNotSupported", "V-CODEC"),
        ("VideoProfileNotSupported", "V-CODEC"),
        ("VideoRangeTypeNotSupported", "V-RANGE"),
        ("VideoBitDepthNotSupported", "V-10BIT"),
        ("SubtitleCodecNotSupported", "V-SUB"),
    ];
    for (r, c) in BY {
        if reasons.iter().any(|x| x == r) {
            return c;
        }
    }
    "V-OTHER"
}

/// Abgeleitete Angaben zu einer Transcoding-Adresse (ohne die Adresse selbst).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UrlSummary {
    /// Die Adresse schreibt eine Untertitelspur hinein (`SubtitleStreamIndex=<Zahl>`).
    pub burns_subtitles: bool,
    /// `SubtitleMethod=Encode` in der Adresse.
    pub method_encode: bool,
    /// Gründe aus dem Parameter `TranscodeReasons`.
    pub reasons: Vec<String>,
}

fn query_of(url: &str) -> &str {
    let after = url.split_once('?').map_or("", |x| x.1);
    after.split('#').next().unwrap_or("")
}

/// Wertet die Query einer Transcoding-Adresse aus. Die Adresse selbst wird nicht aufbewahrt.
pub fn summarize_transcoding_url(url: &str) -> UrlSummary {
    let mut s = UrlSummary::default();
    for pair in query_of(url).split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k.eq_ignore_ascii_case("SubtitleStreamIndex")
            && v.as_bytes().first().is_some_and(u8::is_ascii_digit)
        {
            s.burns_subtitles = true;
        } else if k.eq_ignore_ascii_case("SubtitleMethod") && v.eq_ignore_ascii_case("Encode") {
            s.method_encode = true;
        } else if k.eq_ignore_ascii_case("TranscodeReasons") {
            s.reasons.extend(
                v.split(',')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty()),
            );
        }
    }
    s
}

/// Zusammenfassung einer Medienquelle aus der PlaybackInfo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaybackSummary {
    pub supports_direct_play: bool,
    pub has_transcoding_url: bool,
    pub url: UrlSummary,
    /// `TranscodeReasons` der Quelle (zusätzlich zu denen der Adresse).
    pub source_reasons: Vec<String>,
    /// Die gewählte Tonspur braucht eine Umwandlung (nicht aac/mp3).
    pub audio_needs_transcode: bool,
    /// Browser/Player kann HLS.
    pub can_hls: bool,
}

/// Ergebnis der Einordnung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Direct,
    Hls,
    Fail {
        reason: FailReason,
        code: &'static str,
    },
}

fn fail(reason: FailReason, code: &'static str) -> Verdict {
    Verdict::Fail { reason, code }
}

/// Entscheidet wie `resolveStream`. `remux` = Tonspur weicht vom Standard ab (kein Direct),
/// `no_direct` = Direktwiedergabe ausgeschlossen (Rückfall).
pub fn classify(p: &PlaybackSummary, remux: bool, no_direct: bool) -> Verdict {
    if p.supports_direct_play && !remux && !no_direct {
        return Verdict::Direct;
    }
    // Einbrennen nur, wenn die Adresse eine Untertitelspur hineinschreibt; sonst zählt
    // SubtitleCodecNotSupported nicht (es brennt nichts ein).
    let burns = p.has_transcoding_url && p.url.burns_subtitles;
    let reasons: Vec<&String> = p
        .source_reasons
        .iter()
        .chain(p.url.reasons.iter())
        .filter(|r| burns || r.as_str() != "SubtitleCodecNotSupported")
        .collect();
    if reasons.iter().any(|r| VIDEO_REASONS.contains(&r.as_str())) {
        let owned: Vec<String> = reasons.iter().map(|r| (*r).clone()).collect();
        return fail(FailReason::Video, video_code(&owned));
    }
    if burns && p.url.method_encode {
        return fail(FailReason::Video, "V-SUB");
    }
    if !p.has_transcoding_url {
        return if p.audio_needs_transcode {
            fail(FailReason::Audio, "A-CODEC")
        } else {
            fail(FailReason::Video, "V-NOURL")
        };
    }
    if !p.can_hls {
        return fail(FailReason::Browser, "B-HLS");
    }
    Verdict::Hls
}

fn short(x: &str, n: usize) -> String {
    x.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .take(n)
        .collect()
}

fn video_of(streams: &[StreamInfo]) -> Option<&StreamInfo> {
    streams.iter().find(|s| s.kind == StreamType::Video)
}

fn range_of(v: &StreamInfo) -> String {
    let t = v.video_range_type.as_deref().unwrap_or("");
    let up = t.to_uppercase();
    if up.starts_with("HDR") || up.starts_with("HLG") || up.starts_with("DOVI") {
        return short(t, 16).to_uppercase();
    }
    let ct = v.color_transfer.as_deref().unwrap_or("").to_lowercase();
    if ct.contains("smpte2084") {
        return "HDR10".into();
    }
    if ct.contains("arib-std-b67") {
        return "HLG".into();
    }
    if v.video_range
        .as_deref()
        .is_some_and(|r| r.eq_ignore_ascii_case("HDR"))
    {
        "HDR".into()
    } else {
        String::new()
    }
}

/// Für Decoder bekannt ungeeignetes Bildformat (h264 mit mehr als 8 Bit, 4:2:2, 4:4:4, HDR,
/// Level über 5.2; bei allen Codecs Breite über 4096) als kurzer deutscher Grund.
pub fn video_format_hint(streams: &[StreamInfo]) -> Option<String> {
    let v = video_of(streams)?;
    let prof = v.profile.as_deref().unwrap_or("").to_lowercase();
    let pf = v.pixel_format.as_deref().unwrap_or("").to_lowercase();
    let bd = v.bit_depth.unwrap_or(0);
    let lv = v.level.unwrap_or(0.0);
    let w = v.width.unwrap_or(0);
    if v.codec.eq_ignore_ascii_case("h264") {
        let hi10 = prof.contains("high10") || prof.contains("high 10") || prof.contains("hi10");
        if bd > 8 || hi10 {
            return Some(format!(
                "Bildformat h264 {} Bit",
                if bd > 8 { bd } else { 10 }
            ));
        }
        if prof.contains("4:2:2") || pf.contains("422") {
            return Some("Bildformat h264 4:2:2".into());
        }
        if prof.contains("4:4:4") || pf.contains("444") {
            return Some("Bildformat h264 4:4:4".into());
        }
        if !pf.is_empty() && !(pf.contains("420") || pf.contains("nv12") || pf.contains("nv21")) {
            return Some(format!("Bildformat h264 {}", short(&pf, 12)));
        }
        let hdr = range_of(v);
        if !hdr.is_empty() {
            return Some(format!(
                "Bildformat h264 {}",
                if hdr == "HLG" { "HLG" } else { "HDR" }
            ));
        }
        if lv > 52.0 {
            return Some(format!(
                "Bildformat h264 Level {}",
                crate::format::js_to_fixed(lv / 10.0, 1)
            ));
        }
    }
    if w > 4096 {
        let h = v
            .height
            .filter(|h| *h != 0)
            .map_or_else(|| "?".to_string(), |h| h.to_string());
        return Some(format!("Auflösung {w}×{h}"));
    }
    None
}

/// Dateiinfo für die Messanzeige, z. B. `High 10 L5.1 10 Bit 3840×1600` (fehlende Werte entfallen).
pub fn format_info(streams: &[StreamInfo]) -> String {
    let Some(v) = video_of(streams) else {
        return String::new();
    };
    let lv = v.level.unwrap_or(0.0);
    let pf = v.pixel_format.as_deref().unwrap_or("").to_lowercase();
    let (w, h) = (v.width.unwrap_or(0), v.height.unwrap_or(0));
    let h264 = v.codec.eq_ignore_ascii_case("h264");
    let mut p: Vec<String> = Vec::new();
    if let Some(pr) = v.profile.as_deref().filter(|s| !s.is_empty()) {
        let clean: String = pr
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ' ' | ':' | '.' | '-'))
            .take(16)
            .collect();
        p.push(clean);
    }
    if lv > 0.0 {
        let n = if h264 {
            crate::format::js_to_fixed(lv / 10.0, 1)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        } else {
            lv.to_string()
        };
        p.push(format!("L{n}"));
    }
    if v.bit_depth.unwrap_or(0) > 0 {
        p.push(format!("{} Bit", v.bit_depth.unwrap_or(0)));
    }
    if !pf.is_empty() && !is_plain_pixel_format(&pf) {
        p.push(short(&pf, 12));
    }
    if w > 0 && h > 0 {
        p.push(format!("{w}×{h}"));
    }
    if v.is_interlaced {
        p.push("interlaced".into());
    }
    let r = range_of(v);
    if !r.is_empty() {
        p.push(r);
    }
    p.into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `/^yuvj?420p(10|12)?(le|be)?$/`
fn is_plain_pixel_format(pf: &str) -> bool {
    let Some(rest) = pf.strip_prefix("yuv") else {
        return false;
    };
    let rest = rest.strip_prefix('j').unwrap_or(rest);
    let Some(rest) = rest.strip_prefix("420p") else {
        return false;
    };
    let rest = rest
        .strip_prefix("10")
        .or_else(|| rest.strip_prefix("12"))
        .unwrap_or(rest);
    let rest = rest
        .strip_suffix("le")
        .or_else(|| rest.strip_suffix("be"))
        .unwrap_or(rest);
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(codec: &str) -> StreamInfo {
        StreamInfo {
            kind: StreamType::Video,
            codec: codec.into(),
            ..StreamInfo::default()
        }
    }

    fn summary() -> PlaybackSummary {
        PlaybackSummary {
            has_transcoding_url: true,
            can_hls: true,
            ..PlaybackSummary::default()
        }
    }

    #[test]
    fn direkt_wenn_moeglich() {
        let p = PlaybackSummary {
            supports_direct_play: true,
            ..summary()
        };
        assert_eq!(classify(&p, false, false), Verdict::Direct);
        assert_eq!(classify(&p, true, false), Verdict::Hls);
        assert_eq!(classify(&p, false, true), Verdict::Hls);
    }

    #[test]
    fn videogruende_und_codes() {
        let mut p = summary();
        p.source_reasons = vec!["VideoCodecNotSupported".into()];
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Video, "V-CODEC")
        );
        p.source_reasons = vec!["VideoBitDepthNotSupported".into()];
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Video, "V-10BIT")
        );
        p.source_reasons = vec!["VideoRangeTypeNotSupported".into()];
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Video, "V-RANGE")
        );
        p.source_reasons = vec!["RefFramesNotSupported".into()];
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Video, "V-OTHER")
        );
        p.source_reasons = vec!["AudioCodecNotSupported".into()];
        assert_eq!(classify(&p, false, false), Verdict::Hls);
    }

    #[test]
    fn untertitel_nur_bei_einbrennen_ein_grund() {
        let mut p = summary();
        p.source_reasons = vec!["SubtitleCodecNotSupported".into()];
        assert_eq!(
            classify(&p, false, false),
            Verdict::Hls,
            "ohne Index brennt nichts ein"
        );
        p.url = summarize_transcoding_url(
            "/v/master.m3u8?SubtitleStreamIndex=3&SubtitleMethod=Encode&ApiKey=geheim",
        );
        assert!(p.url.burns_subtitles && p.url.method_encode);
        assert_eq!(classify(&p, false, false), fail(FailReason::Video, "V-SUB"));
    }

    #[test]
    fn ohne_adresse_ton_oder_video() {
        let mut p = PlaybackSummary {
            can_hls: true,
            ..PlaybackSummary::default()
        };
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Video, "V-NOURL")
        );
        p.audio_needs_transcode = true;
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Audio, "A-CODEC")
        );
    }

    #[test]
    fn ohne_hls_browser_fehler() {
        let p = PlaybackSummary {
            can_hls: false,
            ..summary()
        };
        assert_eq!(
            classify(&p, false, false),
            fail(FailReason::Browser, "B-HLS")
        );
    }

    #[test]
    fn adresse_wird_nicht_aufbewahrt() {
        let s = summarize_transcoding_url(
            "/x?ApiKey=geheim&TranscodeReasons=A,B&SubtitleStreamIndex=2",
        );
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("geheim"));
        assert_eq!(s.reasons, vec!["A", "B"]);
        let none = summarize_transcoding_url("/x?SubtitleStreamIndex=-1");
        assert!(!none.burns_subtitles);
    }

    #[test]
    fn bildformat_hinweise() {
        let mut v = video("h264");
        v.bit_depth = Some(10);
        assert_eq!(
            video_format_hint(&[v]).as_deref(),
            Some("Bildformat h264 10 Bit")
        );
        let mut v = video("h264");
        v.pixel_format = Some("yuv422p".into());
        assert_eq!(
            video_format_hint(&[v]).as_deref(),
            Some("Bildformat h264 4:2:2")
        );
        let mut v = video("h264");
        v.pixel_format = Some("yuvj420p".into());
        v.video_range_type = Some("HDR10".into());
        assert_eq!(
            video_format_hint(&[v]).as_deref(),
            Some("Bildformat h264 HDR")
        );
        let mut v = video("h264");
        v.level = Some(62.0);
        assert_eq!(
            video_format_hint(&[v]).as_deref(),
            Some("Bildformat h264 Level 6.2")
        );
        let mut v = video("hevc");
        v.width = Some(7680);
        v.height = Some(4320);
        assert_eq!(
            video_format_hint(&[v]).as_deref(),
            Some("Auflösung 7680×4320")
        );
        let mut v = video("h264");
        v.level = Some(41.0);
        v.pixel_format = Some("yuv420p".into());
        assert_eq!(video_format_hint(&[v]), None);
        assert_eq!(video_format_hint(&[]), None);
    }

    #[test]
    fn dateiinfo() {
        let mut v = video("hevc");
        v.profile = Some("Main 10".into());
        v.level = Some(153.0);
        v.bit_depth = Some(10);
        v.width = Some(3840);
        v.height = Some(1600);
        v.video_range_type = Some("HDR10".into());
        assert_eq!(format_info(&[v]), "Main 10 L153 10 Bit 3840×1600 HDR10");
        let mut v = video("h264");
        v.profile = Some("High".into());
        v.level = Some(31.0);
        v.pixel_format = Some("yuv420p".into());
        v.width = Some(1280);
        v.height = Some(720);
        assert_eq!(format_info(&[v]), "High L3.1 1280×720");
    }
}
