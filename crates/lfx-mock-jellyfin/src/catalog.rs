//! Synthetischer Katalog: ausschließlich erfundene Titel und Personen.

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Ticks je Sekunde.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// Ein Katalogeintrag (Film, Serie, Staffel, Folge, Ansicht).
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub parent_id: String,
    pub series_id: Option<String>,
    pub season_id: Option<String>,
    pub series_name: Option<String>,
    pub season_name: Option<String>,
    pub genres: Vec<&'static str>,
    pub year: i32,
    pub rating: f64,
    pub runtime_min: i64,
    pub index: Option<i32>,
    pub parent_index: Option<i32>,
    pub created: String,
    pub overview: String,
}

/// Veränderlicher Zustand je Benutzer (Favoriten, gesehen, Positionen, Bewertung).
#[derive(Debug, Default)]
pub struct UserState {
    pub favorites: HashSet<String>,
    pub played: HashSet<String>,
    pub positions: HashMap<String, i64>,
    pub likes: HashMap<String, bool>,
}

const MOVIES: &[(&str, &str, i32, f64, i64)] = &[
    ("Die Nebelkuste", "Drama", 2011, 7.8, 118),
    ("Sternenstaub-Archiv", "Science-Fiction", 2019, 8.1, 132),
    ("Der letzte Leuchtturm", "Abenteuer", 2008, 6.9, 104),
    ("Mondfalter", "Drama", 2016, 7.4, 97),
    ("Kupferstadt", "Krimi", 2013, 7.1, 110),
    ("Das Echo von Orion", "Science-Fiction", 2022, 8.4, 141),
    ("Salzwiesen", "Komödie", 2005, 6.2, 92),
    ("Papierdrachen", "Komödie", 2018, 7.0, 101),
    ("Nachtzug nach Nirgendwo", "Krimi", 2010, 7.6, 115),
    ("Glaswerk", "Drama", 2021, 7.9, 124),
    ("Sommer der Heuschrecken", "Abenteuer", 2003, 6.5, 99),
    ("Tiefsee-Protokoll", "Science-Fiction", 2024, 8.0, 128),
];

fn hid(prefix: u128, n: u128) -> String {
    format!("{:032x}", (prefix << 16) + n)
}

/// Id des n-ten Films (0-basiert).
pub fn movie_id(n: usize) -> String {
    hid(0x10, n as u128 + 1)
}
/// Id der ersten Serie.
pub fn series_id(n: usize) -> String {
    hid(0x20, n as u128 + 1)
}
/// Id der Ansicht „Filme“.
pub const VIEW_MOVIES: &str = "00000000000000000000000000500001";
/// Id der Ansicht „Serien“.
pub const VIEW_SERIES: &str = "00000000000000000000000000500002";
/// Id des synthetischen Benutzers.
pub const USER_ID: &str = "00000000000000000000000000a00001";
/// Id des synthetischen Servers.
pub const SERVER_ID: &str = "00000000000000000000000000b00001";

/// Baut den festen Katalog.
pub fn build() -> Vec<Item> {
    let mut out = Vec::new();
    let blank = |id: String, name: &str, kind: &'static str, parent: &str| Item {
        id,
        name: name.to_owned(),
        kind,
        parent_id: parent.to_owned(),
        series_id: None,
        season_id: None,
        series_name: None,
        season_name: None,
        genres: Vec::new(),
        year: 2000,
        rating: 0.0,
        runtime_min: 0,
        index: None,
        parent_index: None,
        created: String::new(),
        overview: String::new(),
    };
    out.push(blank(
        VIEW_MOVIES.to_owned(),
        "Filme",
        "CollectionFolder",
        "root",
    ));
    out.push(blank(
        VIEW_SERIES.to_owned(),
        "Serien",
        "CollectionFolder",
        "root",
    ));
    for (i, (name, genre, year, rating, minutes)) in MOVIES.iter().enumerate() {
        let mut it = blank(movie_id(i), name, "Movie", VIEW_MOVIES);
        it.genres = vec![genre];
        it.year = *year;
        it.rating = *rating;
        it.runtime_min = *minutes;
        it.created = format!("2026-03-{:02}T10:00:00.0000000Z", i + 1);
        it.overview = format!("Erfundene Handlung zu „{name}“ für Testzwecke.");
        out.push(it);
    }
    let series: &[(&str, &str, i32, &[usize])] = &[
        ("Die Kartografen von Alderheim", "Abenteuer", 2020, &[3, 3]),
        ("Hafenlichter", "Drama", 2017, &[4]),
    ];
    for (si, (sname, genre, year, seasons)) in series.iter().enumerate() {
        let sid = series_id(si);
        let mut s = blank(sid.clone(), sname, "Series", VIEW_SERIES);
        s.genres = vec![genre];
        s.year = *year;
        s.rating = 8.2 - si as f64;
        s.created = format!("2026-04-{:02}T10:00:00.0000000Z", si + 1);
        s.overview = format!("Erfundene Serie „{sname}“ für Testzwecke.");
        out.push(s);
        for (sn, episodes) in seasons.iter().enumerate() {
            let season_id = hid(0x30 + si as u128, sn as u128 + 1);
            let season_name = format!("Staffel {}", sn + 1);
            let mut season = blank(season_id.clone(), &season_name, "Season", &sid);
            season.series_id = Some(sid.clone());
            season.series_name = Some((*sname).to_owned());
            season.index = Some(sn as i32 + 1);
            season.year = *year;
            out.push(season);
            for e in 0..*episodes {
                let mut ep = blank(
                    hid(0x40 + si as u128, (sn * 10 + e) as u128 + 1),
                    &format!("Folge {}", e + 1),
                    "Episode",
                    &season_id,
                );
                ep.series_id = Some(sid.clone());
                ep.series_name = Some((*sname).to_owned());
                ep.season_id = Some(season_id.clone());
                ep.season_name = Some(season_name.clone());
                ep.index = Some(e as i32 + 1);
                ep.parent_index = Some(sn as i32 + 1);
                ep.genres = vec![genre];
                ep.year = *year;
                ep.runtime_min = 45;
                ep.created = format!("2026-04-{:02}T12:00:00.0000000Z", sn * 5 + e + 1);
                ep.overview = format!("Erfundene Folge {} der {}.", e + 1, season_name);
                out.push(ep);
            }
        }
    }
    out
}

fn is_playable(kind: &str) -> bool {
    matches!(kind, "Movie" | "Episode")
}

/// Ob ein Element Kind (direkt oder über Serie/Staffel) der Eltern-Id ist.
pub fn is_child(item: &Item, parent: &str) -> bool {
    item.parent_id == parent
}

/// Rendert einen Eintrag als Jellyfin-JSON.
pub fn render(catalog: &[Item], it: &Item, st: &UserState, fields: &str) -> Value {
    let runtime = it.runtime_min * 60 * TICKS_PER_SECOND;
    let pos = st.positions.get(&it.id).copied().unwrap_or(0);
    let played = st.played.contains(&it.id);
    let mut user_data = json!({
        "PlaybackPositionTicks": pos,
        "PlayCount": if played { 1 } else { 0 },
        "IsFavorite": st.favorites.contains(&it.id),
        "Played": played,
        "Key": it.id,
        "ItemId": it.id,
    });
    if let Some(l) = st.likes.get(&it.id) {
        user_data["Likes"] = json!(l);
    }
    if it.kind == "Series" || it.kind == "Season" {
        let unplayed = catalog
            .iter()
            .filter(|e| {
                e.kind == "Episode"
                    && (e.series_id.as_deref() == Some(&it.id)
                        || e.season_id.as_deref() == Some(&it.id))
                    && !st.played.contains(&e.id)
            })
            .count();
        user_data["UnplayedItemCount"] = json!(unplayed);
    }
    if runtime > 0 && pos > 0 {
        user_data["PlayedPercentage"] = json!(pos as f64 / runtime as f64 * 100.0);
    }
    let mut v = json!({
        "Id": it.id,
        "Name": it.name,
        "ServerId": SERVER_ID,
        "Type": it.kind,
        "ParentId": it.parent_id,
        "IsFolder": matches!(it.kind, "Series" | "Season" | "CollectionFolder"),
        "ProductionYear": it.year,
        "Overview": it.overview,
        "Genres": it.genres,
        "CommunityRating": it.rating,
        "OfficialRating": "FSK 12",
        "DateCreated": it.created,
        "PrimaryImageAspectRatio": 0.6667,
        "ImageTags": { "Primary": format!("tag{}", &it.id[26..]) },
        "UserData": user_data,
    });
    if is_playable(it.kind) {
        v["RunTimeTicks"] = json!(runtime);
        v["MediaType"] = json!("Video");
    }
    if matches!(it.kind, "Movie" | "Series") {
        v["BackdropImageTags"] = json!([format!("bd{}", &it.id[26..])]);
        v["ImageTags"]["Logo"] = json!(format!("lg{}", &it.id[26..]));
    }
    if it.kind == "CollectionFolder" {
        v["CollectionType"] = json!(if it.id == VIEW_MOVIES {
            "movies"
        } else {
            "tvshows"
        });
    }
    if let Some(x) = &it.series_id {
        v["SeriesId"] = json!(x);
    }
    if let Some(x) = &it.series_name {
        v["SeriesName"] = json!(x);
    }
    if let Some(x) = &it.season_id {
        v["SeasonId"] = json!(x);
    }
    if let Some(x) = &it.season_name {
        v["SeasonName"] = json!(x);
    }
    if let Some(x) = it.index {
        v["IndexNumber"] = json!(x);
    }
    if let Some(x) = it.parent_index {
        v["ParentIndexNumber"] = json!(x);
    }
    let want = |f: &str| fields.split(',').any(|p| p.eq_ignore_ascii_case(f));
    if want("People") {
        v["People"] = json!([
            { "Name": "Mira Kessler", "Id": hid(0x60, 1), "Role": "Hauptrolle", "Type": "Actor" },
            { "Name": "Jonas Albrecht", "Id": hid(0x60, 2), "Type": "Director" },
        ]);
    }
    if is_playable(it.kind) && (want("MediaSources") || want("MediaStreams")) {
        v["MediaSources"] = json!([media_source(it)]);
    }
    if is_playable(it.kind) && want("Chapters") {
        v["Chapters"] = json!([{ "Name": "Anfang", "StartPositionTicks": 0 }, { "Name": "Mitte", "StartPositionTicks": runtime / 2 }]);
    }
    if is_playable(it.kind) && want("Trickplay") {
        v["Trickplay"] = json!({ source_id(it): { "320": {
            "Width": 320, "Height": 180, "TileWidth": 10, "TileHeight": 10,
            "ThumbnailCount": it.runtime_min * 6, "Interval": 10000, "Bandwidth": 120000
        } } });
    }
    crate::w2_home::adjust_item(&mut v, it); // W2-home
    v
}

fn source_id(it: &Item) -> String {
    format!("ms{}", &it.id[16..])
}

/// Medienquelle (eine Datei mit Video, zwei Tonspuren und zwei Untertiteln).
pub fn media_source(it: &Item) -> Value {
    json!({
        "Id": source_id(it),
        "Protocol": "File",
        "Container": "mkv",
        "Name": it.name,
        "Size": 1_500_000_000i64,
        "RunTimeTicks": it.runtime_min * 60 * TICKS_PER_SECOND,
        "Bitrate": 8_000_000,
        "SupportsDirectPlay": true,
        "SupportsDirectStream": true,
        "SupportsTranscoding": false,
        "DefaultAudioStreamIndex": 1,
        "DefaultSubtitleStreamIndex": -1,
        "MediaStreams": [
            { "Index": 0, "Type": "Video", "Codec": "h264", "Width": 1920, "Height": 1080, "BitDepth": 8, "VideoRange": "SDR", "VideoRangeType": "SDR", "IsDefault": true, "AverageFrameRate": 24.0 },
            { "Index": 1, "Type": "Audio", "Codec": "ac3", "Language": "ger", "DisplayTitle": "Deutsch (AC3 5.1)", "Channels": 6, "IsDefault": true },
            { "Index": 2, "Type": "Audio", "Codec": "aac", "Language": "eng", "DisplayTitle": "Englisch (AAC Stereo)", "Channels": 2 },
            { "Index": 3, "Type": "Subtitle", "Codec": "srt", "Language": "ger", "DisplayTitle": "Deutsch (SRT)", "IsTextSubtitleStream": true, "IsExternal": false },
            { "Index": 4, "Type": "Subtitle", "Codec": "ass", "Language": "eng", "DisplayTitle": "Englisch (ASS)", "IsTextSubtitleStream": true, "IsExternal": false },
        ],
    })
}
