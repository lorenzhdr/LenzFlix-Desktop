//! Ton- und Untertitelspuren (Port von `player/tracks.js`): Wahl der Spuren nach Nutzerwunsch.
//! Die Streams stammen aus der PlaybackInfo; hier nur reine Logik.

/// Art eines Streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamType {
    Video,
    Audio,
    Subtitle,
    #[default]
    Other,
}

/// Ein Stream der Medienquelle (nur die für den TV-Modus nötigen Felder).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamInfo {
    pub index: i32,
    pub kind: StreamType,
    pub codec: String,
    pub language: String,
    pub display_title: Option<String>,
    pub title: Option<String>,
    pub is_default: bool,
    pub is_forced: bool,
    pub profile: Option<String>,
    pub pixel_format: Option<String>,
    pub bit_depth: Option<i32>,
    pub level: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub video_range: Option<String>,
    pub video_range_type: Option<String>,
    pub color_transfer: Option<String>,
    pub is_interlaced: bool,
}

/// Eine wählbare Spur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub index: i32,
    pub label: String,
    pub lang: String,
    pub is_default: bool,
    pub codec: String,
    /// Nur Untertitel.
    pub forced: bool,
}

/// Spuren der Quelle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tracks {
    pub audio: Vec<Track>,
    pub subs: Vec<Track>,
    /// Aktive Tonspur.
    pub audio_index: Option<i32>,
    /// Aktiver Untertitel, `-1` = aus.
    pub subtitle_index: i32,
}

impl Tracks {
    /// Leere Spuren (kein Player): kein Ton, Untertitel aus.
    pub fn empty() -> Self {
        Self {
            audio: Vec::new(),
            subs: Vec::new(),
            audio_index: None,
            subtitle_index: -1,
        }
    }
}

/// Nutzer-Wünsche aus dem Konto.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserPrefs {
    pub audio_lang: String,
    pub sub_lang: String,
    /// `Always`, `OnlyForced`, sonst aus.
    pub sub_mode: String,
}

const TEXT_SUBS: [&str; 8] = [
    "subrip", "srt", "ass", "ssa", "webvtt", "vtt", "mov_text", "text",
];

fn norm_lang(l: &str) -> String {
    let s = lfx_tv_schedule::jsval::js_trim(l).to_lowercase();
    let m = match s.as_str() {
        "de" | "deu" | "ger" => "ger",
        "en" | "eng" => "eng",
        "fr" | "fre" | "fra" => "fra",
        "es" | "spa" => "spa",
        "it" | "ita" => "ita",
        "nl" | "dut" | "nld" => "nld",
        "zh" | "chi" | "zho" => "zho",
        "cs" | "cze" | "ces" => "ces",
        "ja" | "jpn" => "jpn",
        "pt" | "por" => "por",
        "ru" | "rus" => "rus",
        "tr" | "tur" => "tur",
        "pl" | "pol" => "pol",
        _ => return s,
    };
    m.to_string()
}

fn label_of(s: &StreamInfo, n: usize) -> String {
    [
        s.display_title.as_deref(),
        s.title.as_deref(),
        Some(s.language.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|x| !x.is_empty())
    .map_or_else(|| format!("Spur {n}"), str::to_string)
}

/// Ton- und Untertitelspuren (nur Text-Untertitel; Bild-Untertitel stehen nicht in der Liste).
pub fn build_tracks(streams: &[StreamInfo]) -> (Vec<Track>, Vec<Track>) {
    let mut audio = Vec::new();
    let mut subs = Vec::new();
    for s in streams {
        match s.kind {
            StreamType::Audio => audio.push(Track {
                index: s.index,
                label: label_of(s, audio.len() + 1),
                lang: s.language.clone(),
                is_default: s.is_default,
                codec: s.codec.to_lowercase(),
                forced: false,
            }),
            StreamType::Subtitle if TEXT_SUBS.contains(&s.codec.to_lowercase().as_str()) => {
                subs.push(Track {
                    index: s.index,
                    label: label_of(s, subs.len() + 1),
                    lang: s.language.clone(),
                    is_default: s.is_default,
                    codec: s.codec.to_lowercase(),
                    forced: s.is_forced,
                });
            }
            _ => {}
        }
    }
    (audio, subs)
}

/// Index der Tonspur, die der Server als Standard liefert.
pub fn server_default_audio(default_index: Option<i32>, audio: &[Track]) -> Option<i32> {
    if default_index.is_some() {
        return default_index;
    }
    audio
        .iter()
        .find(|a| a.is_default)
        .or_else(|| audio.first())
        .map(|a| a.index)
}

/// Gewünschte Tonspur: Sprache des Nutzers, sonst Server-Standard.
pub fn pick_audio(audio: &[Track], default_index: Option<i32>, user: &UserPrefs) -> Option<i32> {
    let want = norm_lang(&user.audio_lang);
    if !want.is_empty() {
        let m: Vec<&Track> = audio
            .iter()
            .filter(|a| norm_lang(&a.lang) == want)
            .collect();
        if let Some(t) = m.iter().find(|a| a.is_default).or_else(|| m.first()) {
            return Some(t.index);
        }
    }
    default_index
}

/// Standard-Untertitel nach Nutzerwahl; `-1` = aus.
pub fn pick_subtitle(subs: &[Track], user: &UserPrefs) -> i32 {
    let want = norm_lang(&user.sub_lang);
    match user.sub_mode.as_str() {
        "Always" => {
            let m: Vec<&Track> = subs
                .iter()
                .filter(|s| !want.is_empty() && norm_lang(&s.lang) == want)
                .collect();
            m.iter()
                .find(|s| !s.forced)
                .or_else(|| m.first())
                .map_or(-1, |t| t.index)
        }
        "OnlyForced" => {
            let f: Vec<&Track> = subs.iter().filter(|s| s.forced).collect();
            let m = f
                .iter()
                .find(|s| !want.is_empty() && norm_lang(&s.lang) == want);
            let pick = m.or(if want.is_empty() { f.first() } else { None });
            pick.map_or(-1, |t| t.index)
        }
        _ => -1,
    }
}

/// `aac` und `mp3` brauchen keine Tonumwandlung.
pub fn is_plain_audio_codec(codec: &str) -> bool {
    matches!(codec.to_lowercase().as_str(), "aac" | "mp3")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(
        index: i32,
        kind: StreamType,
        codec: &str,
        lang: &str,
        def: bool,
        forced: bool,
    ) -> StreamInfo {
        StreamInfo {
            index,
            kind,
            codec: codec.into(),
            language: lang.into(),
            is_default: def,
            is_forced: forced,
            ..StreamInfo::default()
        }
    }

    fn sample() -> Vec<StreamInfo> {
        vec![
            st(0, StreamType::Video, "h264", "", false, false),
            st(1, StreamType::Audio, "ac3", "eng", true, false),
            st(2, StreamType::Audio, "aac", "ger", false, false),
            st(3, StreamType::Subtitle, "subrip", "ger", false, true),
            st(4, StreamType::Subtitle, "srt", "de", false, false),
            st(5, StreamType::Subtitle, "pgssub", "ger", false, false),
        ]
    }

    #[test]
    fn nur_text_untertitel_in_der_liste() {
        let (a, s) = build_tracks(&sample());
        assert_eq!(a.len(), 2);
        assert_eq!(s.iter().map(|t| t.index).collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(a[0].label, "eng");
        assert_eq!(a[1].codec, "aac");
    }

    #[test]
    fn tonspur_nach_sprache_sonst_standard() {
        let (a, _) = build_tracks(&sample());
        let de = UserPrefs {
            audio_lang: "de".into(),
            ..UserPrefs::default()
        };
        assert_eq!(pick_audio(&a, Some(1), &de), Some(2));
        let fr = UserPrefs {
            audio_lang: "fr".into(),
            ..UserPrefs::default()
        };
        assert_eq!(pick_audio(&a, Some(1), &fr), Some(1));
        assert_eq!(pick_audio(&a, Some(1), &UserPrefs::default()), Some(1));
        assert_eq!(server_default_audio(None, &a), Some(1));
        assert_eq!(server_default_audio(Some(2), &a), Some(2));
        assert_eq!(server_default_audio(None, &[]), None);
    }

    #[test]
    fn untertitel_modi() {
        let (_, s) = build_tracks(&sample());
        let mut u = UserPrefs {
            sub_lang: "ger".into(),
            sub_mode: "Always".into(),
            ..UserPrefs::default()
        };
        assert_eq!(pick_subtitle(&s, &u), 4); // nicht erzwungen bevorzugt
        u.sub_mode = "OnlyForced".into();
        assert_eq!(pick_subtitle(&s, &u), 3);
        u.sub_lang = String::new();
        assert_eq!(pick_subtitle(&s, &u), 3);
        u.sub_lang = "fr".into();
        assert_eq!(pick_subtitle(&s, &u), -1);
        u.sub_mode = "Default".into();
        assert_eq!(pick_subtitle(&s, &u), -1);
    }

    #[test]
    fn schlichte_tonformate() {
        assert!(is_plain_audio_codec("AAC"));
        assert!(!is_plain_audio_codec("eac3"));
    }
}
