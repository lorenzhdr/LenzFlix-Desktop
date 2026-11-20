//! Erweiterte Mock-Endpunkte: Suche, Personen, Musik, Fotos, Bücher, Wiedergabelisten, Live-TV,
//! Benutzer-Einstellungen und Verwaltung. Alle Daten sind erfunden. Die Medien dieses Moduls stehen nicht im
//! Hauptkatalog (`catalog.rs`), damit dessen Listen unverändert bleiben; `/Items` fragt sie nur bei passenden
//! Elementarten oder Eltern ab.

use super::{
    DEMO_PASSWORD, DEMO_USER, GUEST_ID, GUEST_USER, MockHandle, S, Shared, csv, jv, limit_of, lower,
};
use crate::catalog::{self, SERVER_ID, TICKS_PER_SECOND, USER_ID};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------- Bild

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Erzeugt ein gültiges PNG (RGBA, Farbverlauf, korrekte Prüfsummen, unkomprimierter Deflate-Strom).
pub fn make_png(width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 4 + 1) as usize * height as usize);
    for y in 0..height {
        raw.push(0); // Filter „keiner“
        for x in 0..width {
            raw.push((40 + x * 160 / width.max(1)) as u8);
            raw.push((50 + y * 140 / height.max(1)) as u8);
            raw.push(120);
            raw.push(255);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Das Platzhalterbild des Mocks (24×36, gültiges PNG).
pub fn placeholder_png() -> &'static [u8] {
    static PNG: OnceLock<Vec<u8>> = OnceLock::new();
    PNG.get_or_init(|| make_png(24, 36))
}

// ---------------------------------------------------------------- Zeit

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// ISO-8601 (UTC) aus Unix-Sekunden.
fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Tage seit 1970-01-01 → Datum (Verfahren von Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.0000000Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Unix-Sekunden aus ISO-8601 (`YYYY-MM-DDTHH:MM:SS…`, UTC); ungültig → `None`.
fn parse_iso(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, c: usize| text.get(a..c)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

// ---------------------------------------------------------------- Ids

/// Id der Ansicht „Musik“.
pub const VIEW_MUSIC: &str = "00000000000000000000000000500003";
/// Id der Ansicht „Fotos“.
pub const VIEW_PHOTOS: &str = "00000000000000000000000000500004";
/// Id der Ansicht „Bücher“.
pub const VIEW_BOOKS: &str = "00000000000000000000000000500005";
/// Id der Ansicht „Live-TV“.
pub const VIEW_LIVETV: &str = "00000000000000000000000000500006";

fn xid(prefix: u128, n: u128) -> String {
    format!("{:032x}", (prefix << 16) + n)
}

/// Id des n-ten Künstlers (0-basiert).
pub fn artist_id(n: usize) -> String {
    xid(0x70, n as u128 + 1)
}
/// Id des n-ten Albums (0-basiert).
pub fn album_id(n: usize) -> String {
    xid(0x71, n as u128 + 1)
}
/// Id des n-ten Titels (0-basiert).
pub fn track_id(n: usize) -> String {
    xid(0x72, n as u128 + 1)
}
/// Id des n-ten Buchs (0-basiert).
pub fn book_id(n: usize) -> String {
    xid(0x75, n as u128 + 1)
}
/// Id des n-ten Senders (0-basiert).
pub fn channel_id(n: usize) -> String {
    xid(0x80, n as u128 + 1)
}
/// Id der n-ten Aufnahme (0-basiert).
pub fn recording_id(n: usize) -> String {
    xid(0xA0, n as u128 + 1)
}

/// Sendung: Sender-Nummer (0-basiert) und Stunde seit 1970.
fn program_id(channel: usize, hour: i64) -> String {
    format!(
        "{:032x}",
        (0x9u128 << 64) + (channel as u128) * 1_000_000 + hour as u128
    )
}

fn decode_program(id: &str) -> Option<(usize, i64)> {
    let n = u128::from_str_radix(id, 16)
        .ok()?
        .checked_sub(0x9u128 << 64)?;
    (n < 100_000_000).then_some(((n / 1_000_000) as usize, (n % 1_000_000) as i64))
}

// ---------------------------------------------------------------- Medien

struct MItem {
    id: String,
    kind: &'static str,
    parent: String,
    name: String,
    json: Value,
}

fn base(id: &str, name: &str, kind: &'static str, parent: &str, folder: bool) -> Value {
    json!({
        "Id": id, "Name": name, "ServerId": SERVER_ID, "Type": kind, "ParentId": parent,
        "IsFolder": folder, "ImageTags": { "Primary": format!("tag{}", &id[26..]) },
        "PrimaryImageAspectRatio": 1.0,
    })
}

const ARTISTS: &[&str] = &["Nora Tiefenbach", "Kollektiv Nordlicht"];
/// (Name, Künstler-Index, Jahr, Titel)
const ALBUMS: &[(&str, usize, i32, &[&str])] = &[
    (
        "Leuchtfeuer",
        0,
        2014,
        &["Erste Welle", "Salzluft", "Nachtfähre", "Ankerplatz"],
    ),
    ("Gezeiten", 0, 2019, &["Ebbe", "Flut", "Nippzeit"]),
    (
        "Papierflieger",
        1,
        2022,
        &["Aufwind", "Landeanflug", "Windstille"],
    ),
];
const BOOKS: &[(&str, &str, &str)] = &[
    ("Der Kompass im Nebel", "epub", "Mira Kessler"),
    ("Handbuch der Leuchttürme", "pdf", "Jonas Albrecht"),
    ("Kurze Geschichten vom Kai", "epub", "Lena Voigt"),
];
const CHANNELS: &[&str] = &["Kanal 1", "Kanal 2", "Kanal 3", "Kanal 4"];
const SHOWS: &[&str] = &[
    "Morgenjournal",
    "Hafenmagazin",
    "Küstenwetter",
    "Nachtkino (erfunden)",
    "Kochen am Kai",
    "Lichtblicke",
    "Reise ohne Karte",
    "Das Quiz am Abend",
];
const PEOPLE: &[(&str, &str)] = &[
    ("Mira Kessler", "Actor"),
    ("Jonas Albrecht", "Director"),
    ("Lena Voigt", "Actor"),
];
const STUDIOS: &[&str] = &["Studio Nordlicht", "Atelier Fjord"];
const MUSIC_GENRES: &[&str] = &["Elektro", "Folk"];

fn media() -> &'static [MItem] {
    static MEDIA: OnceLock<Vec<MItem>> = OnceLock::new();
    MEDIA.get_or_init(build_media)
}

fn build_media() -> Vec<MItem> {
    let mut out = Vec::new();
    let mut push = |json: Value| {
        out.push(MItem {
            id: json["Id"].as_str().unwrap_or("").to_owned(),
            kind: match json["Type"].as_str().unwrap_or("") {
                "MusicArtist" => "MusicArtist",
                "MusicAlbum" => "MusicAlbum",
                "Audio" => "Audio",
                "PhotoAlbum" => "PhotoAlbum",
                "Photo" => "Photo",
                "Book" => "Book",
                _ => "Other",
            },
            parent: json["ParentId"].as_str().unwrap_or("").to_owned(),
            name: json["Name"].as_str().unwrap_or("").to_owned(),
            json,
        });
    };
    for (i, name) in ARTISTS.iter().enumerate() {
        let mut v = base(&artist_id(i), name, "MusicArtist", VIEW_MUSIC, true);
        v["Overview"] = json!(format!("Erfundene Musikgruppe „{name}“."));
        v["AlbumCount"] = json!(ALBUMS.iter().filter(|a| a.1 == i).count());
        v["SongCount"] = json!(
            ALBUMS
                .iter()
                .filter(|a| a.1 == i)
                .map(|a| a.3.len())
                .sum::<usize>()
        );
        push(v);
    }
    let mut track_no = 0;
    for (ai, (name, artist, year, tracks)) in ALBUMS.iter().enumerate() {
        let mut v = base(&album_id(ai), name, "MusicAlbum", VIEW_MUSIC, true);
        v["ProductionYear"] = json!(year);
        v["AlbumArtist"] = json!(ARTISTS[*artist]);
        v["AlbumArtists"] = json!([{ "Name": ARTISTS[*artist], "Id": artist_id(*artist) }]);
        v["Artists"] = json!([ARTISTS[*artist]]);
        v["ArtistItems"] = json!([{ "Name": ARTISTS[*artist], "Id": artist_id(*artist) }]);
        v["ChildCount"] = json!(tracks.len());
        v["Genres"] = json!([MUSIC_GENRES[ai % 2]]);
        push(v);
        for (ti, title) in tracks.iter().enumerate() {
            let mut t = base(&track_id(track_no), title, "Audio", &album_id(ai), false);
            t["MediaType"] = json!("Audio");
            t["Album"] = json!(name);
            t["AlbumId"] = json!(album_id(ai));
            t["AlbumArtist"] = json!(ARTISTS[*artist]);
            t["AlbumArtists"] = json!([{ "Name": ARTISTS[*artist], "Id": artist_id(*artist) }]);
            t["Artists"] = json!([ARTISTS[*artist]]);
            t["ArtistItems"] = json!([{ "Name": ARTISTS[*artist], "Id": artist_id(*artist) }]);
            t["IndexNumber"] = json!(ti + 1);
            t["ParentIndexNumber"] = json!(1);
            t["RunTimeTicks"] = json!((180 + 20 * ti as i64) * TICKS_PER_SECOND);
            t["Container"] = json!("flac");
            t["ProductionYear"] = json!(year);
            t["MediaSources"] = json!([{
                "Id": format!("ms{}", &track_id(track_no)[16..]), "Protocol": "File", "Container": "flac",
                "SupportsDirectPlay": true, "SupportsDirectStream": true, "SupportsTranscoding": true,
                "RunTimeTicks": (180 + 20 * ti as i64) * TICKS_PER_SECOND,
                "MediaStreams": [{ "Index": 0, "Type": "Audio", "Codec": "flac", "Channels": 2,
                                   "SampleRate": 44100, "BitDepth": 16, "IsDefault": true }]
            }]);
            push(t);
            track_no += 1;
        }
    }
    let album = xid(0x73, 1);
    let mut a = base(
        &album,
        "Urlaub am Kai (erfunden)",
        "PhotoAlbum",
        VIEW_PHOTOS,
        true,
    );
    a["ChildCount"] = json!(3);
    push(a);
    for n in 0..5usize {
        let parent = if n < 3 { album.as_str() } else { VIEW_PHOTOS };
        let mut p = base(
            &xid(0x74, n as u128 + 1),
            &format!("Foto {}", n + 1),
            "Photo",
            parent,
            false,
        );
        p["MediaType"] = json!("Photo");
        p["Width"] = json!(4000);
        p["Height"] = json!(3000);
        p["Container"] = json!("jpg");
        p["PrimaryImageAspectRatio"] = json!(1.3333);
        p["CameraMake"] = json!("Testkamera");
        push(p);
    }
    for (i, (name, container, author)) in BOOKS.iter().enumerate() {
        let mut b = base(&book_id(i), name, "Book", VIEW_BOOKS, false);
        b["MediaType"] = json!("Book");
        b["Container"] = json!(container);
        b["AlbumArtist"] = json!(author);
        b["Artists"] = json!([author]);
        b["CanDownload"] = json!(true);
        b["Overview"] = json!(format!("Erfundenes Buch „{name}“."));
        push(b);
    }
    out
}

fn is_extra_id(id: &str) -> bool {
    media().iter().any(|m| m.id == id)
        || [VIEW_MUSIC, VIEW_PHOTOS, VIEW_BOOKS, VIEW_LIVETV].contains(&id)
        || (0..CHANNELS.len()).any(|c| channel_id(c) == id)
        || (0..2).any(|r| recording_id(r) == id)
        || decode_program(id).is_some()
}

/// Elementarten, die `/Items` aus diesem Modul bedient.
const EXTRA_KINDS: &[&str] = &[
    "MusicArtist",
    "MusicAlbum",
    "Audio",
    "PhotoAlbum",
    "Photo",
    "Book",
    "Playlist",
    "Recording",
    "TvChannel",
    "LiveTvChannel",
    "Program",
    "LiveTvProgram",
];

fn views_json() -> Vec<Value> {
    [
        (VIEW_MUSIC, "Musik", "music"),
        (VIEW_PHOTOS, "Fotos", "homevideos"),
        (VIEW_BOOKS, "Bücher", "books"),
        (VIEW_LIVETV, "Live-TV", "livetv"),
    ]
    .iter()
    .map(|(id, name, ct)| {
        let mut v = base(id, name, "CollectionFolder", "root", true);
        v["CollectionType"] = json!(ct);
        v
    })
    .collect()
}

// ---------------------------------------------------------------- Zustand

#[derive(Default)]
pub(crate) struct ExtraState {
    media_views: bool,
    demo_admin: bool,
    password: Option<String>,
    config: Option<Value>,
    avatar: Option<(String, usize)>,
    playlists: Vec<Playlist>,
    next_playlist: u32,
    next_entry: u32,
    timers: Vec<Value>,
    next_timer: u32,
    deleted_recordings: Vec<String>,
    open_streams: Vec<String>,
    next_stream: u32,
    encodings_deleted: Vec<(String, String)>,
    events: Vec<String>,
    running_tasks: Vec<String>,
    folders: Option<Vec<(String, String, Vec<String>)>>,
    users: Vec<(String, String)>,
    policies: HashMap<String, Value>,
    deleted_users: Vec<String>,
    next_user: u32,
    deleted_devices: Vec<String>,
}

struct Playlist {
    id: String,
    name: String,
    entries: Vec<(String, String)>,
}

impl MockHandle {
    /// Schaltet die Ansichten Musik, Fotos, Bücher und Live-TV in `/UserViews` ein (Standard: aus).
    pub fn set_media_views(&self, on: bool) {
        self.shared.lock().extra.media_views = on;
    }
    /// Macht das Demo-Konto zum Administrator (Standard: nein; Verwaltungs-Endpunkte antworten dann 403).
    pub fn set_demo_admin(&self, on: bool) {
        self.shared.lock().extra.demo_admin = on;
    }
    /// Wiedergabelisten: Name und Element-Ids in Reihenfolge.
    pub fn playlists(&self) -> Vec<(String, Vec<String>)> {
        self.shared
            .lock()
            .extra
            .playlists
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.entries.iter().map(|e| e.1.clone()).collect(),
                )
            })
            .collect()
    }
    /// Angelegte Aufnahme-Timer (JSON).
    pub fn timers(&self) -> Vec<Value> {
        self.shared.lock().extra.timers.clone()
    }
    /// Offene Live-Streams (Ids).
    pub fn open_live_streams(&self) -> Vec<String> {
        self.shared.lock().extra.open_streams.clone()
    }
    /// Gemeldete `DELETE /Videos/ActiveEncodings` als (DeviceId, PlaySessionId).
    pub fn encodings_deleted(&self) -> Vec<(String, String)> {
        self.shared.lock().extra.encodings_deleted.clone()
    }
    /// Hochgeladenes Profilbild: Inhaltstyp und Länge der Base64-Daten.
    pub fn avatar(&self) -> Option<(String, usize)> {
        self.shared.lock().extra.avatar.clone()
    }
    /// Verwaltungsereignisse als Text (`Nachricht:…`, `Steuerung:…`, `Aufgabe gestartet:…`, `Neustart`, ...).
    pub fn admin_events(&self) -> Vec<String> {
        self.shared.lock().extra.events.clone()
    }
}

fn forbidden() -> Response {
    StatusCode::FORBIDDEN.into_response()
}

fn is_admin(shared: &Shared) -> bool {
    shared.lock().extra.demo_admin
}

fn user_data_json(shared: &Shared, id: &str) -> Value {
    let d = shared.lock();
    let pos = d.users.positions.get(id).copied().unwrap_or(0);
    json!({
        "PlaybackPositionTicks": pos, "PlayCount": 0, "IsFavorite": d.users.favorites.contains(id),
        "Played": d.users.played.contains(id), "Key": id, "ItemId": id,
    })
}

/// Ergänzt `/Users/Me` um gespeicherte Konfiguration und Administratorflag.
pub(crate) fn overlay_me(shared: &Shared, user: &mut Value, guest: bool) {
    let d = shared.lock();
    if guest {
        return;
    }
    if let Some(cfg) = &d.extra.config {
        user["Configuration"] = cfg.clone();
    }
    if d.extra.demo_admin {
        user["Policy"]["IsAdministrator"] = json!(true);
    }
}

/// Ergänzt `/UserViews` um die Medienansichten, falls eingeschaltet.
pub(crate) fn extend_views(shared: &Shared, response: Response) -> Response {
    if !shared.lock().extra.media_views {
        return response;
    }
    // Die Ansichten des Hauptkatalogs stehen schon in der Antwort; hier nur neu aufbauen.
    let mut items: Vec<Value> = shared
        .catalog
        .iter()
        .filter(|i| i.kind == "CollectionFolder")
        .map(|i| catalog::render(&shared.catalog, i, &shared.lock().users, ""))
        .collect();
    items.extend(views_json());
    let n = items.len();
    drop(response);
    jv(json!({ "Items": items, "TotalRecordCount": n, "StartIndex": 0 }))
}

// ---------------------------------------------------------------- /Items-Erweiterung

fn playlist_json(shared: &Shared, p: &Playlist) -> Value {
    let mut v = base(&p.id, &p.name, "Playlist", "root", true);
    v["ChildCount"] = json!(p.entries.len());
    v["MediaType"] = json!("Audio");
    let _ = shared;
    v
}

fn render_any(shared: &Shared, id: &str) -> Option<Value> {
    let mut v = if let Some(m) = media().iter().find(|m| m.id == id) {
        m.json.clone()
    } else if let Some(c) = (0..CHANNELS.len()).find(|c| channel_id(*c) == id) {
        channel_json(c, false)
    } else if let Some(r) = (0..2).find(|r| recording_id(*r) == id) {
        recording_json(r)
    } else if let Some((c, h)) = decode_program(id) {
        program_json(c, h)
    } else if let Some(v) = views_json()
        .into_iter()
        .find(|v| v["Id"].as_str() == Some(id))
    {
        v
    } else {
        let d = shared.lock();
        let p = d.extra.playlists.iter().find(|p| p.id == id)?;
        let mut v = base(&p.id, &p.name, "Playlist", "root", true);
        v["ChildCount"] = json!(p.entries.len());
        v["MediaType"] = json!("Audio");
        v
    };
    v["UserData"] = user_data_json(shared, id);
    Some(v)
}

/// Bedient `/Items` für die Medien dieses Moduls; `None`, wenn der Hauptkatalog zuständig ist.
pub(crate) fn try_items(shared: &Shared, params: &HashMap<String, String>) -> Option<Response> {
    let include = csv(params, "includeitemtypes");
    let ids = csv(params, "ids");
    let parent = params.get("parentid").cloned();
    let by_kind = include
        .iter()
        .any(|k| EXTRA_KINDS.iter().any(|e| e.eq_ignore_ascii_case(k)));
    let by_parent = parent.as_deref().is_some_and(|p| {
        is_extra_id(p) || shared.lock().extra.playlists.iter().any(|pl| pl.id == p)
    });
    let by_ids = !ids.is_empty()
        && ids
            .iter()
            .all(|i| is_extra_id(i) || shared.lock().extra.playlists.iter().any(|p| p.id == *i));
    if !(by_kind || by_parent || by_ids) {
        return None;
    }
    let recursive = params
        .get("recursive")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let term = params.get("searchterm").map(|t| t.to_lowercase());
    let artists = csv(params, "artistids");
    let mut rows: Vec<Value> = Vec::new();
    // Wiedergabelisten (nur wenn danach gefragt wird).
    if include.iter().any(|k| k.eq_ignore_ascii_case("Playlist")) {
        let d = shared.lock();
        rows.extend(d.extra.playlists.iter().map(|p| playlist_json(shared, p)));
    }
    // Wiedergabelisten-Inhalt (ParentId = Liste).
    if let Some(p) = &parent {
        let d = shared.lock();
        if let Some(pl) = d.extra.playlists.iter().find(|pl| pl.id == *p) {
            let entries = pl.entries.clone();
            drop(d);
            for (entry, item) in entries {
                if let Some(mut v) = render_any(shared, &item).or_else(|| {
                    shared
                        .catalog
                        .iter()
                        .find(|i| i.id == item)
                        .map(|i| catalog::render(&shared.catalog, i, &shared.lock().users, ""))
                }) {
                    v["PlaylistItemId"] = json!(entry);
                    rows.push(v);
                }
            }
        }
    }
    for m in media() {
        let kind_ok = include.is_empty() || include.iter().any(|k| k.eq_ignore_ascii_case(m.kind));
        let place_ok = match &parent {
            Some(p) if recursive => {
                m.parent == *p
                    || media().iter().any(|x| x.id == m.parent && x.parent == *p)
                    || (p == VIEW_MUSIC && matches!(m.kind, "Audio"))
                    || (p == VIEW_PHOTOS && m.kind == "Photo")
            }
            Some(p) => m.parent == *p,
            None => recursive || by_ids,
        };
        let ids_ok = ids.is_empty() || ids.contains(&m.id);
        let term_ok = term
            .as_ref()
            .is_none_or(|t| m.name.to_lowercase().contains(t));
        let artist_ok = artists.is_empty()
            || m.json["ArtistItems"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| artists.iter().any(|id| x["Id"] == *id)))
            || artists.contains(&m.id);
        if kind_ok && place_ok && ids_ok && term_ok && artist_ok {
            let mut v = m.json.clone();
            v["UserData"] = user_data_json(shared, &m.id);
            rows.push(v);
        }
    }
    if parent.is_none() || recursive {
        // Live-TV und Aufnahmen über /Items
        for c in 0..CHANNELS.len() {
            if include.iter().any(|k| {
                k.eq_ignore_ascii_case("TvChannel") || k.eq_ignore_ascii_case("LiveTvChannel")
            }) {
                rows.push(channel_json(c, false));
            }
        }
        if include.iter().any(|k| k.eq_ignore_ascii_case("Recording")) {
            rows.extend((0..2).map(recording_json));
        }
    }
    let sort = csv(params, "sortby");
    let sort_text = sort.join(",");
    if sort_text.contains("IndexNumber") {
        rows.sort_by_key(|v| {
            (
                v["ParentIndexNumber"].as_i64().unwrap_or(0),
                v["IndexNumber"].as_i64().unwrap_or(0),
            )
        });
    } else if sort_text.contains("IsFolder") {
        rows.sort_by_key(|v| {
            (
                !v["IsFolder"].as_bool().unwrap_or(false),
                v["Name"].as_str().unwrap_or("").to_lowercase(),
            )
        });
    } else if sort_text.contains("SortName") {
        rows.sort_by_key(|v| v["Name"].as_str().unwrap_or("").to_lowercase());
    }
    if params
        .get("sortorder")
        .is_some_and(|o| o.eq_ignore_ascii_case("Descending"))
    {
        rows.reverse();
    }
    let total = rows.len();
    let start = params
        .get("startindex")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let page: Vec<Value> = rows
        .into_iter()
        .skip(start)
        .take(limit_of(params))
        .collect();
    Some(jv(
        json!({ "Items": page, "TotalRecordCount": total, "StartIndex": start }),
    ))
}

/// `GET /Items/{id}` für Medien dieses Moduls.
pub(crate) fn try_item(shared: &Shared, id: &str) -> Option<Response> {
    render_any(shared, id).map(jv)
}

/// Prüft, ob ein Bild für diese Id existiert.
pub(crate) fn has_image(shared: &Shared, id: &str) -> bool {
    is_extra_id(id) || shared.lock().extra.playlists.iter().any(|p| p.id == id)
}

/// `POST /Items/{id}/PlaybackInfo` für Live-Sender und Aufnahmen; `None` für alle anderen.
pub(crate) fn try_playback_info(shared: &Shared, id: &str) -> Option<Response> {
    let is_channel = (0..CHANNELS.len()).any(|c| channel_id(c) == id);
    let is_recording = (0..2).any(|r| recording_id(r) == id);
    if !is_channel && !is_recording {
        return None;
    }
    let mut d = shared.lock();
    d.extra.next_stream += 1;
    let live = format!("mock-live-{}", d.extra.next_stream);
    d.extra.open_streams.push(live.clone());
    drop(d);
    let streams = json!([
        { "Index": 0, "Type": "Video", "Codec": "h264", "Width": 1280, "Height": 720, "VideoRange": "SDR", "IsDefault": true },
        { "Index": 1, "Type": "Audio", "Codec": "aac", "Channels": 2, "Language": "ger", "IsDefault": true },
    ]);
    Some(jv(json!({
        "PlaySessionId": format!("play-{live}"),
        "MediaSources": [{
            "Id": format!("src-{id}"), "Protocol": "Http", "Container": "ts",
            "IsInfiniteStream": is_channel, "RequiresOpening": false, "RequiresClosing": true,
            "LiveStreamId": live, "SupportsDirectPlay": false, "SupportsDirectStream": true,
            "SupportsTranscoding": true,
            "TranscodingUrl": format!("/videos/{id}/master.m3u8?MediaSourceId=src-{id}&LiveStreamId={live}"),
            "TranscodingSubProtocol": "hls", "MediaStreams": streams,
        }],
    })))
}

// ---------------------------------------------------------------- Live-TV-Daten

fn program_json(channel: usize, hour: i64) -> Value {
    let start = hour * 3600;
    let title = SHOWS[(channel * 3 + hour.rem_euclid(SHOWS.len() as i64) as usize) % SHOWS.len()];
    let mut v = base(
        &program_id(channel, hour),
        title,
        "Program",
        &channel_id(channel),
        false,
    );
    v["ChannelId"] = json!(channel_id(channel));
    v["ChannelName"] = json!(CHANNELS[channel]);
    v["StartDate"] = json!(iso(start));
    v["EndDate"] = json!(iso(start + 3600));
    v["RunTimeTicks"] = json!(3600 * TICKS_PER_SECOND);
    v["Overview"] = json!(format!(
        "Erfundene Sendung „{title}“ auf {}.",
        CHANNELS[channel]
    ));
    v["Genres"] = json!(["Unterhaltung"]);
    v["IsLive"] = json!(false);
    v["IsSeries"] = json!(false);
    v
}

fn channel_json(channel: usize, with_program: bool) -> Value {
    let mut v = base(
        &channel_id(channel),
        CHANNELS[channel],
        "TvChannel",
        VIEW_LIVETV,
        false,
    );
    v["ChannelNumber"] = json!((channel + 1).to_string());
    v["MediaType"] = json!("Video");
    if with_program {
        v["CurrentProgram"] = program_json(channel, now_secs() / 3600);
    }
    v
}

fn recording_json(n: usize) -> Value {
    let mut v = base(
        &recording_id(n),
        ["Aufgezeichnete Sendung A", "Aufgezeichnete Sendung B"][n % 2],
        "Recording",
        VIEW_LIVETV,
        false,
    );
    v["MediaType"] = json!("Video");
    v["ChannelName"] = json!(CHANNELS[n % CHANNELS.len()]);
    v["RunTimeTicks"] = json!(3600 * TICKS_PER_SECOND);
    v["StartDate"] = json!("2026-01-01T20:00:00.0000000Z");
    v["EndDate"] = json!("2026-01-01T21:00:00.0000000Z");
    v
}

// ---------------------------------------------------------------- Routen

pub(crate) fn routes() -> Router<Arc<Shared>> {
    Router::new()
        // Suche und Listen
        .route("/Search/Hints", get(search_hints))
        .route("/Persons", get(persons))
        .route("/Studios", get(studios))
        .route("/MusicGenres", get(music_genres))
        .route("/Items/Filters2", get(filters2))
        .route("/Items/Counts", get(counts))
        .route("/Items/{id}/SpecialFeatures", get(special_features))
        .route("/Items/{id}/LocalTrailers", get(|| async { jv(json!([])) }))
        .route(
            "/Items/{id}/Intros",
            get(|| async { jv(json!({ "Items": [], "TotalRecordCount": 0, "StartIndex": 0 })) }),
        )
        .route("/Items/{id}/InstantMix", get(instant_mix))
        .route("/Items/{id}/Download", get(crate::w35_offline::download))
        .route("/UserItems/{id}/UserData", post(update_user_data))
        .route("/Items/{id}", delete(delete_item))
        .route("/Videos/ActiveEncodings", delete(delete_encodings))
        // Musik
        .route("/Artists", get(artists))
        .route("/Artists/AlbumArtists", get(artists))
        .route("/Audio/{id}/universal", get(audio_universal))
        // Wiedergabelisten
        .route("/Playlists", post(playlist_create))
        .route("/Playlists/{id}", get(playlist_get).post(playlist_rename))
        .route(
            "/Playlists/{id}/Items",
            get(playlist_items)
                .post(playlist_add)
                .delete(playlist_remove),
        )
        .route(
            "/Playlists/{id}/Items/{entry}/Move/{index}",
            post(playlist_move),
        )
        // Live-TV
        .route("/LiveTv/Channels", get(live_channels))
        .route("/LiveTv/Programs", get(live_programs))
        .route("/LiveTv/Recordings", get(recordings))
        .route("/LiveTv/Recordings/{id}", delete(recording_delete))
        .route("/LiveTv/Timers", get(timers_list).post(timer_create))
        .route("/LiveTv/Timers/Defaults", get(timer_defaults))
        .route("/LiveTv/Timers/{id}", delete(timer_delete))
        .route("/LiveStreams/Close", post(live_close))
        // Benutzer-Einstellungen
        .route("/Users/Configuration", post(user_config))
        .route("/Users/Password", post(user_password))
        .route("/UserImage", post(avatar_upload).delete(avatar_delete))
        .route(
            "/Branding/Configuration",
            get(|| async {
                jv(json!({ "LoginDisclaimer": "Hinweis des Mocks", "CustomCss": "" }))
            }),
        )
        // Verwaltung
        .route("/System/Info", get(system_info))
        .route("/System/Restart", post(restart))
        .route("/System/Shutdown", post(shutdown))
        .route("/System/ActivityLog/Entries", get(activity_log))
        .route("/System/Logs", get(logs))
        .route("/System/Logs/Log", get(log_file))
        .route("/Sessions", get(sessions))
        .route("/Sessions/{id}/Message", post(session_message))
        .route("/Sessions/{id}/Playing/{command}", post(session_playstate))
        .route("/Sessions/{id}/Command/{command}", post(session_command))
        .route("/Devices", get(devices).delete(device_delete))
        .route("/ScheduledTasks", get(tasks))
        .route(
            "/ScheduledTasks/Running/{id}",
            post(task_start).delete(task_stop),
        )
        .route("/Library/Refresh", post(library_refresh))
        .route(
            "/Library/VirtualFolders",
            get(folders).post(folder_add).delete(folder_remove),
        )
        .route("/Users", get(users_list))
        .route("/Users/New", post(user_create))
        .route("/Users/{id}", get(user_get).delete(user_delete))
        .route("/Users/{id}/Policy", post(user_policy))
}

// ---------------------------------------------------------------- Suche, Listen

async fn search_hints(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let term = p
        .get("searchterm")
        .map(|t| t.to_lowercase())
        .unwrap_or_default();
    let flag = |k: &str| p.get(k).is_none_or(|v| !v.eq_ignore_ascii_case("false"));
    let include = csv(&p, "includeitemtypes");
    let mut hits: Vec<Value> = Vec::new();
    let hint = |id: &str, name: &str, kind: &str| {
        json!({ "ItemId": id, "Id": id, "Name": name, "Type": kind, "MatchedTerm": name,
                "PrimaryImageTag": format!("tag{}", &id[26..]) })
    };
    if flag("includemedia") {
        for i in shared
            .catalog
            .iter()
            .filter(|i| matches!(i.kind, "Movie" | "Series" | "Episode"))
        {
            if i.name.to_lowercase().contains(&term) {
                let mut h = hint(&i.id, &i.name, i.kind);
                h["ProductionYear"] = json!(i.year);
                if let Some(s) = &i.series_name {
                    h["Series"] = json!(s);
                }
                hits.push(h);
            }
        }
        for m in media()
            .iter()
            .filter(|m| matches!(m.kind, "MusicAlbum" | "Audio" | "Book"))
        {
            if m.name.to_lowercase().contains(&term) {
                let mut h = hint(&m.id, &m.name, m.kind);
                h["AlbumArtist"] = m.json["AlbumArtist"].clone();
                hits.push(h);
            }
        }
    }
    if flag("includeartists") {
        for m in media().iter().filter(|m| m.kind == "MusicArtist") {
            if m.name.to_lowercase().contains(&term) {
                hits.push(hint(&m.id, &m.name, "MusicArtist"));
            }
        }
    }
    if flag("includepeople") {
        for (n, (name, _)) in PEOPLE.iter().enumerate() {
            if name.to_lowercase().contains(&term) {
                hits.push(hint(&xid(0x60, n as u128 + 1), name, "Person"));
            }
        }
    }
    if flag("includegenres") {
        for (n, g) in ["Drama", "Abenteuer", "Komödie"].iter().enumerate() {
            if g.to_lowercase().contains(&term) {
                hits.push(hint(
                    &format!("{:032x}", 0x7000_0000u32 + n as u32),
                    g,
                    "Genre",
                ));
            }
        }
    }
    if flag("includestudios") {
        for (n, s) in STUDIOS.iter().enumerate() {
            if s.to_lowercase().contains(&term) {
                hits.push(hint(&xid(0x77, n as u128 + 1), s, "Studio"));
            }
        }
    }
    if !include.is_empty() {
        hits.retain(|h| {
            include
                .iter()
                .any(|k| h["Type"].as_str() == Some(k.as_str()))
        });
    }
    let total = hits.len();
    hits.truncate(limit_of(&p));
    jv(json!({ "SearchHints": hits, "TotalRecordCount": total }))
}

fn named_list(names: &[(&str, String)], kind: &str, p: &HashMap<String, String>) -> Response {
    let term = p.get("searchterm").map(|t| t.to_lowercase());
    let prefix = p.get("namestartswith").map(|t| t.to_lowercase());
    let mut rows: Vec<Value> = names
        .iter()
        .filter(|(n, _)| term.as_ref().is_none_or(|t| n.to_lowercase().contains(t)))
        .filter(|(n, _)| {
            prefix
                .as_ref()
                .is_none_or(|t| n.to_lowercase().starts_with(t))
        })
        .map(|(n, id)| json!({ "Id": id, "Name": n, "Type": kind, "ServerId": SERVER_ID }))
        .collect();
    let total = rows.len();
    let start = p
        .get("startindex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    rows = rows.into_iter().skip(start).take(limit_of(p)).collect();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": start }))
}

async fn persons(Query(params): Query<HashMap<String, String>>) -> Response {
    let names: Vec<(&str, String)> = PEOPLE
        .iter()
        .enumerate()
        .map(|(n, (name, _))| (*name, xid(0x60, n as u128 + 1)))
        .collect();
    named_list(&names, "Person", &lower(params))
}

async fn studios(Query(params): Query<HashMap<String, String>>) -> Response {
    let names: Vec<(&str, String)> = STUDIOS
        .iter()
        .enumerate()
        .map(|(n, s)| (*s, xid(0x77, n as u128 + 1)))
        .collect();
    named_list(&names, "Studio", &lower(params))
}

async fn music_genres(Query(params): Query<HashMap<String, String>>) -> Response {
    let names: Vec<(&str, String)> = MUSIC_GENRES
        .iter()
        .enumerate()
        .map(|(n, s)| (*s, xid(0x78, n as u128 + 1)))
        .collect();
    named_list(&names, "MusicGenre", &lower(params))
}

async fn filters2() -> Response {
    jv(json!({
        "Genres": [
            { "Name": "Abenteuer", "Id": xid(0x79, 1) },
            { "Name": "Drama", "Id": xid(0x79, 2) },
            { "Name": "Komödie", "Id": xid(0x79, 3) },
        ],
        "Tags": ["Mock-Tag"],
        "AudioLanguages": ["ger", "eng"],
        "SubtitleLanguages": ["ger", "eng", "fra"],
    }))
}

async fn counts(axum::extract::State(shared): S) -> Response {
    let c = |k: &str| shared.catalog.iter().filter(|i| i.kind == k).count();
    let m = |k: &str| media().iter().filter(|i| i.kind == k).count();
    let total = shared.catalog.len() + media().len();
    jv(json!({
        "MovieCount": c("Movie"), "SeriesCount": c("Series"), "EpisodeCount": c("Episode"),
        "ArtistCount": m("MusicArtist"), "ProgramCount": 0, "TrailerCount": 0,
        "SongCount": m("Audio"), "AlbumCount": m("MusicAlbum"), "MusicVideoCount": 0,
        "BoxSetCount": 0, "BookCount": m("Book"), "ItemCount": total,
    }))
}

async fn special_features(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !shared
        .catalog
        .iter()
        .any(|i| i.id == id && i.kind == "Movie")
    {
        return jv(json!([]));
    }
    jv(json!([{
        "Id": xid(0x7A, 1), "Name": "Making-of (erfunden)", "Type": "Video", "ExtraType": "BehindTheScenes",
        "ServerId": SERVER_ID, "MediaType": "Video", "RunTimeTicks": 600 * TICKS_PER_SECOND,
    }]))
}

async fn instant_mix(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_extra_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let p = lower(params);
    let mut tracks: Vec<Value> = media()
        .iter()
        .filter(|m| m.kind == "Audio")
        .map(|m| {
            let mut v = m.json.clone();
            v["UserData"] = user_data_json(&shared, &m.id);
            v
        })
        .collect();
    // „Zufall“ deterministisch: umgekehrte Reihenfolge, damit Tests sie vom Albumlauf unterscheiden können.
    tracks.reverse();
    let total = tracks.len();
    tracks.truncate(limit_of(&p));
    jv(json!({ "Items": tracks, "TotalRecordCount": total, "StartIndex": 0 }))
}

pub(crate) async fn download(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    let Some(m) = media().iter().find(|m| m.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let _ = &shared;
    let (ctype, body): (&str, &[u8]) = match m.kind {
        "Photo" => ("image/png", placeholder_png()),
        "Book" => ("application/epub+zip", b"FAKE-BOOK-BYTES"),
        _ => ("audio/flac", b"FAKE-AUDIO-BYTES"),
    };
    ([(header::CONTENT_TYPE, ctype)], body.to_vec()).into_response()
}

async fn update_user_data(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !is_extra_id(&id) && !shared.catalog.iter().any(|i| i.id == id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    {
        let mut d = shared.lock();
        if let Some(pos) = body.get("PlaybackPositionTicks").and_then(Value::as_i64) {
            d.users.positions.insert(id.clone(), pos);
        }
        if let Some(played) = body.get("Played").and_then(Value::as_bool) {
            if played {
                d.users.played.insert(id.clone());
            } else {
                d.users.played.remove(&id);
            }
        }
        crate::w35_offline::note_user_data(&mut d, &id, &body);
        if let Some(fav) = body.get("IsFavorite").and_then(Value::as_bool) {
            if fav {
                d.users.favorites.insert(id.clone());
            } else {
                d.users.favorites.remove(&id);
            }
        }
    }
    jv(user_data_json(&shared, &id))
}

async fn delete_item(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    let mut d = shared.lock();
    let before = d.extra.playlists.len();
    d.extra.playlists.retain(|p| p.id != id);
    if d.extra.playlists.len() == before {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn delete_encodings(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let (Some(device), Some(session)) = (p.get("deviceid"), p.get("playsessionid")) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    shared
        .lock()
        .extra
        .encodings_deleted
        .push((device.clone(), session.clone()));
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------- Musik

async fn artists(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let term = p.get("searchterm").map(|t| t.to_lowercase());
    let rows: Vec<Value> = media()
        .iter()
        .filter(|m| m.kind == "MusicArtist")
        .filter(|m| {
            term.as_ref()
                .is_none_or(|t| m.name.to_lowercase().contains(t))
        })
        .map(|m| {
            let mut v = m.json.clone();
            v["UserData"] = user_data_json(&shared, &m.id);
            v
        })
        .collect();
    let total = rows.len();
    jv(
        json!({ "Items": rows.into_iter().take(limit_of(&p)).collect::<Vec<_>>(),
               "TotalRecordCount": total, "StartIndex": 0 }),
    )
}

async fn audio_universal(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let _ = &shared;
    let p = lower(params);
    // Wie ein echter Server: ohne Benutzer und Gerät keine Auslieferung.
    if !media().iter().any(|m| m.id == id && m.kind == "Audio")
        || !p.contains_key("userid")
        || !p.contains_key("deviceid")
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CONTENT_TYPE, "audio/flac")],
        b"FAKE-AUDIO-BYTES".as_slice(),
    )
        .into_response()
}

// ---------------------------------------------------------------- Wiedergabelisten

async fn playlist_create(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let Some(name) = p.get("name").filter(|n| !n.is_empty()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut d = shared.lock();
    d.extra.next_playlist += 1;
    let id = xid(0x76, u128::from(d.extra.next_playlist));
    let mut entries = Vec::new();
    for item in csv(&p, "ids") {
        d.extra.next_entry += 1;
        entries.push((format!("entry{:04}", d.extra.next_entry), item));
    }
    d.extra.playlists.push(Playlist {
        id: id.clone(),
        name: name.clone(),
        entries,
    });
    jv(json!({ "Id": id }))
}

async fn playlist_get(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    let d = shared.lock();
    match d.extra.playlists.iter().find(|p| p.id == id) {
        Some(p) => jv(json!({ "OpenAccess": false, "Shares": [],
                              "ItemIds": p.entries.iter().map(|e| e.1.clone()).collect::<Vec<_>>() })),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn playlist_rename(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut d = shared.lock();
    match d.extra.playlists.iter_mut().find(|p| p.id == id) {
        Some(p) => {
            if let Some(n) = body.get("Name").and_then(Value::as_str) {
                p.name = n.to_owned();
            }
            StatusCode::NO_CONTENT.into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn playlist_items(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let entries = {
        let d = shared.lock();
        match d.extra.playlists.iter().find(|pl| pl.id == id) {
            Some(pl) => pl.entries.clone(),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let mut rows = Vec::new();
    for (entry, item) in entries {
        let found = render_any(&shared, &item).or_else(|| {
            shared
                .catalog
                .iter()
                .find(|i| i.id == item)
                .map(|i| catalog::render(&shared.catalog, i, &shared.lock().users, ""))
        });
        if let Some(mut v) = found {
            v["PlaylistItemId"] = json!(entry);
            rows.push(v);
        }
    }
    let total = rows.len();
    let start = p
        .get("startindex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let rows: Vec<Value> = rows.into_iter().skip(start).take(limit_of(&p)).collect();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": start }))
}

async fn playlist_add(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let mut d = shared.lock();
    let mut next = d.extra.next_entry;
    let Some(pl) = d.extra.playlists.iter_mut().find(|pl| pl.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let at = p
        .get("position")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(pl.entries.len())
        .min(pl.entries.len());
    for (offset, item) in csv(&p, "ids").into_iter().enumerate() {
        next += 1;
        pl.entries
            .insert(at + offset, (format!("entry{next:04}"), item));
    }
    d.extra.next_entry = next;
    StatusCode::NO_CONTENT.into_response()
}

async fn playlist_remove(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let entries = csv(&p, "entryids");
    let mut d = shared.lock();
    match d.extra.playlists.iter_mut().find(|pl| pl.id == id) {
        Some(pl) => {
            pl.entries.retain(|e| !entries.contains(&e.0));
            StatusCode::NO_CONTENT.into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn playlist_move(
    axum::extract::State(shared): S,
    Path((id, entry, index)): Path<(String, String, usize)>,
) -> Response {
    let mut d = shared.lock();
    let Some(pl) = d.extra.playlists.iter_mut().find(|pl| pl.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(from) = pl.entries.iter().position(|e| e.0 == entry) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let item = pl.entries.remove(from);
    let to = index.min(pl.entries.len());
    pl.entries.insert(to, item);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------- Live-TV

async fn live_channels(Query(params): Query<HashMap<String, String>>) -> Response {
    let p = lower(params);
    let with = p
        .get("addcurrentprogram")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let rows: Vec<Value> = (0..CHANNELS.len()).map(|c| channel_json(c, with)).collect();
    let total = rows.len();
    let start = p
        .get("startindex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let rows: Vec<Value> = rows.into_iter().skip(start).take(limit_of(&p)).collect();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": start }))
}

async fn live_programs(Query(params): Query<HashMap<String, String>>) -> Response {
    let p = lower(params);
    let channels = csv(&p, "channelids");
    let min_end = p.get("minenddate").and_then(|v| parse_iso(v));
    let max_start = p.get("maxstartdate").and_then(|v| parse_iso(v));
    let airing = p
        .get("isairing")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let now = now_secs();
    let this_hour = now / 3600;
    let mut rows = Vec::new();
    for c in 0..CHANNELS.len() {
        if !channels.is_empty() && !channels.contains(&channel_id(c)) {
            continue;
        }
        for h in (this_hour - 3)..(this_hour + 12) {
            let (s, e) = (h * 3600, h * 3600 + 3600);
            if min_end.is_some_and(|m| e < m)
                || max_start.is_some_and(|m| s > m)
                || (airing && !(s <= now && now < e))
            {
                continue;
            }
            rows.push(program_json(c, h));
        }
    }
    rows.sort_by_key(|v| v["StartDate"].as_str().unwrap_or("").to_owned());
    let total = rows.len();
    let start = p
        .get("startindex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let rows: Vec<Value> = rows.into_iter().skip(start).take(limit_of(&p)).collect();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": start }))
}

async fn recordings(axum::extract::State(shared): S) -> Response {
    let d = shared.lock();
    let rows: Vec<Value> = (0..2)
        .filter(|r| !d.extra.deleted_recordings.contains(&recording_id(*r)))
        .map(recording_json)
        .collect();
    let total = rows.len();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": 0 }))
}

async fn recording_delete(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !(0..2).any(|r| recording_id(r) == id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    shared.lock().extra.deleted_recordings.push(id);
    StatusCode::NO_CONTENT.into_response()
}

async fn timers_list(axum::extract::State(shared): S) -> Response {
    let d = shared.lock();
    let total = d.extra.timers.len();
    jv(json!({ "Items": d.extra.timers, "TotalRecordCount": total, "StartIndex": 0 }))
}

async fn timer_defaults(Query(params): Query<HashMap<String, String>>) -> Response {
    let p = lower(params);
    let Some((channel, hour)) = p.get("programid").and_then(|id| decode_program(id)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let prog = program_json(channel, hour);
    jv(json!({
        "ChannelId": prog["ChannelId"], "ChannelName": prog["ChannelName"], "ProgramId": prog["Id"],
        "Name": prog["Name"], "Overview": prog["Overview"], "StartDate": prog["StartDate"],
        "EndDate": prog["EndDate"], "PrePaddingSeconds": 60, "PostPaddingSeconds": 120,
        "IsPrePaddingRequired": false, "IsPostPaddingRequired": false, "Priority": 0,
        "KeepUntil": "UntilDeleted",
    }))
}

async fn timer_create(
    axum::extract::State(shared): S,
    axum::Json(mut body): axum::Json<Value>,
) -> Response {
    if body.get("ProgramId").and_then(Value::as_str).is_none()
        || body.get("StartDate").and_then(Value::as_str).is_none()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut d = shared.lock();
    d.extra.next_timer += 1;
    body["Id"] = json!(format!("timer{:04}", d.extra.next_timer));
    body["Status"] = json!("New");
    d.extra.timers.push(body);
    StatusCode::NO_CONTENT.into_response()
}

async fn timer_delete(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    let mut d = shared.lock();
    let before = d.extra.timers.len();
    d.extra
        .timers
        .retain(|t| t["Id"].as_str() != Some(id.as_str()));
    if d.extra.timers.len() == before {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn live_close(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let p = lower(params);
    let Some(id) = p.get("livestreamid") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    shared.lock().extra.open_streams.retain(|s| s != id);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------- Benutzer-Einstellungen

async fn user_config(
    axum::extract::State(shared): S,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !body.is_object() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    shared.lock().extra.config = Some(body);
    StatusCode::NO_CONTENT.into_response()
}

async fn user_password(
    axum::extract::State(shared): S,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let current = body
        .get("CurrentPw")
        .or_else(|| body.get("CurrentPassword"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let new = body.get("NewPw").and_then(Value::as_str).unwrap_or("");
    let mut d = shared.lock();
    let expected = d
        .extra
        .password
        .clone()
        .unwrap_or_else(|| DEMO_PASSWORD.to_owned());
    if current != expected {
        return StatusCode::FORBIDDEN.into_response();
    }
    d.extra.password = Some(new.to_owned());
    StatusCode::NO_CONTENT.into_response()
}

async fn avatar_upload(
    axum::extract::State(shared): S,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctype = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    // Der Server erwartet Base64-Text.
    let b64 = body
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    if !ctype.starts_with("image/") || body.is_empty() || !b64 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    shared.lock().extra.avatar = Some((ctype, body.len()));
    StatusCode::NO_CONTENT.into_response()
}

async fn avatar_delete(axum::extract::State(shared): S) -> Response {
    shared.lock().extra.avatar = None;
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------- Verwaltung

async fn system_info(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    jv(json!({
        "ServerName": "Mock-Videothek", "Version": "12.4.0", "ProductName": "Mock Media Server",
        "Id": SERVER_ID, "OperatingSystemDisplayName": "Testsystem", "SystemArchitecture": "Arm64",
        "HasPendingRestart": false, "IsShuttingDown": false, "CanSelfRestart": true,
        "HasUpdateAvailable": false, "StartupWizardCompleted": true, "LocalAddress": "http://127.0.0.1",
    }))
}

async fn restart(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    shared.lock().extra.events.push("Neustart".to_owned());
    StatusCode::NO_CONTENT.into_response()
}

async fn shutdown(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    shared.lock().extra.events.push("Herunterfahren".to_owned());
    StatusCode::NO_CONTENT.into_response()
}

async fn activity_log(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let p = lower(params);
    let all: Vec<Value> = (1..=12i64)
        .rev()
        .map(|n| {
            json!({ "Id": n, "Name": format!("Ereignis {n}"), "ShortOverview": "Erfundener Eintrag",
                    "Type": if n % 3 == 0 { "SessionStarted" } else { "VideoPlayback" },
                    "Date": iso(1_767_225_600 + n * 3600), "UserId": USER_ID,
                    "Severity": if n % 4 == 0 { "Warning" } else { "Information" } })
        })
        .collect();
    let total = all.len();
    let start = p
        .get("startindex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let rows: Vec<Value> = all.into_iter().skip(start).take(limit_of(&p)).collect();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": start }))
}

async fn logs(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    jv(json!([
        { "Name": "log_20260101.log", "Size": 120, "DateModified": "2026-01-01T12:00:00.0000000Z" },
        { "Name": "log_20260102.log", "Size": 64, "DateModified": "2026-01-02T12:00:00.0000000Z" },
    ]))
}

async fn log_file(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    match params.get("name").map(String::as_str) {
        Some("log_20260101.log") => (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "[INF] Erfundener Logeintrag eins\n[WRN] Erfundener Logeintrag zwei\n",
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn sessions(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let movie = shared
        .catalog
        .iter()
        .find(|i| i.id == catalog::movie_id(0))
        .map(|i| json!({ "Id": i.id, "Name": i.name, "Type": "Movie", "RunTimeTicks": i.runtime_min * 60 * TICKS_PER_SECOND }));
    jv(json!([
        { "Id": "sess-eigen", "UserId": USER_ID, "UserName": DEMO_USER, "Client": "LENZFLIX",
          "DeviceName": "Dieser Rechner", "DeviceId": "dev-eigen", "ApplicationVersion": "0.1.0",
          "LastActivityDate": "2026-01-01T12:00:00.0000000Z", "IsActive": true,
          "SupportsRemoteControl": true, "PlayState": { "IsPaused": false, "IsMuted": false } },
        { "Id": "sess-tv", "UserId": GUEST_ID, "UserName": GUEST_USER, "Client": "Test-Fernseher",
          "DeviceName": "Wohnzimmer", "DeviceId": "dev-tv", "ApplicationVersion": "1.0",
          "LastActivityDate": "2026-01-01T12:00:00.0000000Z", "IsActive": true,
          "SupportsRemoteControl": true, "NowPlayingItem": movie,
          "PlayState": { "PositionTicks": 600 * TICKS_PER_SECOND, "IsPaused": false, "IsMuted": false, "VolumeLevel": 40 } },
    ]))
}

fn known_session(id: &str) -> bool {
    matches!(id, "sess-eigen" | "sess-tv")
}

async fn session_message(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    if !known_session(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let text = body.get("Text").and_then(Value::as_str).unwrap_or("");
    shared
        .lock()
        .extra
        .events
        .push(format!("Nachricht:{id}:{text}"));
    StatusCode::NO_CONTENT.into_response()
}

async fn session_playstate(
    axum::extract::State(shared): S,
    Path((id, command)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    if !known_session(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let seek = lower(params)
        .get("seekpositionticks")
        .map(|v| format!(":{v}"))
        .unwrap_or_default();
    shared
        .lock()
        .extra
        .events
        .push(format!("Steuerung:{id}:{command}{seek}"));
    StatusCode::NO_CONTENT.into_response()
}

async fn session_command(
    axum::extract::State(shared): S,
    Path((id, command)): Path<(String, String)>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    if !known_session(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    shared
        .lock()
        .extra
        .events
        .push(format!("Befehl:{id}:{command}"));
    StatusCode::NO_CONTENT.into_response()
}

async fn devices(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let d = shared.lock();
    let rows: Vec<Value> = [
        ("dev-eigen", "Dieser Rechner", "LENZFLIX", DEMO_USER),
        ("dev-tv", "Wohnzimmer", "Test-Fernseher", GUEST_USER),
    ]
    .iter()
    .filter(|(id, ..)| !d.extra.deleted_devices.iter().any(|x| x == id))
    .map(|(id, name, app, user)| {
        json!({ "Id": id, "Name": name, "AppName": app, "AppVersion": "1.0",
                "LastUserName": user, "DateLastActivity": "2026-01-01T12:00:00.0000000Z" })
    })
    .collect();
    let total = rows.len();
    jv(json!({ "Items": rows, "TotalRecordCount": total, "StartIndex": 0 }))
}

async fn device_delete(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let Some(id) = params.get("id") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    shared.lock().extra.deleted_devices.push(id.clone());
    StatusCode::NO_CONTENT.into_response()
}

const TASKS: &[(&str, &str, &str)] = &[
    ("task-scan", "Medienbibliothek durchsuchen", "Bibliothek"),
    ("task-clean", "Zwischenspeicher bereinigen", "Wartung"),
    ("task-trickplay", "Vorschaubilder erzeugen", "Bibliothek"),
];

async fn tasks(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let d = shared.lock();
    let rows: Vec<Value> = TASKS
        .iter()
        .map(|(id, name, cat)| {
            let running = d.extra.running_tasks.iter().any(|t| t == id);
            let mut v = json!({
                "Id": id, "Name": name, "Category": cat, "IsHidden": false,
                "Description": "Erfundene Aufgabe", "State": if running { "Running" } else { "Idle" },
                "LastExecutionResult": { "Status": "Completed", "StartTimeUtc": "2026-01-01T03:00:00.0000000Z",
                                         "EndTimeUtc": "2026-01-01T03:05:00.0000000Z" },
            });
            if running {
                v["CurrentProgressPercentage"] = json!(42.0);
            }
            v
        })
        .collect();
    jv(Value::Array(rows))
}

async fn task_start(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    if !TASKS.iter().any(|t| t.0 == id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut d = shared.lock();
    if !d.extra.running_tasks.contains(&id) {
        d.extra.running_tasks.push(id.clone());
    }
    d.extra.events.push(format!("Aufgabe gestartet:{id}"));
    StatusCode::NO_CONTENT.into_response()
}

async fn task_stop(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let mut d = shared.lock();
    d.extra.running_tasks.retain(|t| *t != id);
    d.extra.events.push(format!("Aufgabe gestoppt:{id}"));
    StatusCode::NO_CONTENT.into_response()
}

async fn library_refresh(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    shared
        .lock()
        .extra
        .events
        .push("Bibliothek aktualisiert".to_owned());
    StatusCode::NO_CONTENT.into_response()
}

fn folder_list(extra: &mut ExtraState) -> &mut Vec<(String, String, Vec<String>)> {
    extra.folders.get_or_insert_with(|| {
        vec![
            (
                "Filme".to_owned(),
                "movies".to_owned(),
                vec!["/mock/filme".to_owned()],
            ),
            (
                "Serien".to_owned(),
                "tvshows".to_owned(),
                vec!["/mock/serien".to_owned()],
            ),
        ]
    })
}

async fn folders(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let mut d = shared.lock();
    let rows: Vec<Value> = folder_list(&mut d.extra)
        .iter()
        .enumerate()
        .map(|(n, (name, ct, loc))| {
            json!({ "Name": name, "CollectionType": ct, "Locations": loc,
                    "ItemId": xid(0x7B, n as u128 + 1), "RefreshProgress": 100.0, "RefreshStatus": "Idle" })
        })
        .collect();
    jv(Value::Array(rows))
}

async fn folder_add(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let p = lower(params);
    let Some(name) = p.get("name").filter(|n| !n.is_empty()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut d = shared.lock();
    let list = folder_list(&mut d.extra);
    if list.iter().any(|f| f.0 == *name) {
        return StatusCode::CONFLICT.into_response();
    }
    list.push((
        name.clone(),
        p.get("collectiontype").cloned().unwrap_or_default(),
        csv(&p, "paths"),
    ));
    StatusCode::NO_CONTENT.into_response()
}

async fn folder_remove(
    axum::extract::State(shared): S,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let p = lower(params);
    let Some(name) = p.get("name") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut d = shared.lock();
    let list = folder_list(&mut d.extra);
    let before = list.len();
    list.retain(|f| f.0 != *name);
    if list.len() == before {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

fn default_policy(admin: bool) -> Value {
    json!({
        "IsAdministrator": admin, "IsHidden": false, "IsDisabled": false, "EnableMediaPlayback": true,
        "EnableAudioPlaybackTranscoding": false, "EnableVideoPlaybackTranscoding": false,
        "EnablePlaybackRemuxing": true, "EnableLiveTvAccess": true, "EnableLiveTvManagement": false,
        "EnableContentDeletion": false, "EnableContentDownloading": true, "EnableRemoteAccess": true,
        "EnableAllFolders": true, "EnabledFolders": [], "EnableAllDevices": true,
        "AuthenticationProviderId": "Mock.Auth", "LoginAttemptsBeforeLockout": 3,
    })
}

fn user_json(shared: &Shared, id: &str, name: &str) -> Value {
    let d = shared.lock();
    let admin = id == USER_ID && d.extra.demo_admin;
    let policy = d
        .extra
        .policies
        .get(id)
        .cloned()
        .unwrap_or_else(|| default_policy(admin));
    json!({ "Id": id, "Name": name, "ServerId": SERVER_ID, "HasPassword": true,
            "LastLoginDate": "2026-01-01T12:00:00.0000000Z", "Policy": policy,
            "Configuration": { "PlayDefaultAudioTrack": true, "SubtitleMode": "Default" } })
}

fn all_users(shared: &Shared) -> Vec<(String, String)> {
    let d = shared.lock();
    let mut v = vec![
        (USER_ID.to_owned(), DEMO_USER.to_owned()),
        (GUEST_ID.to_owned(), GUEST_USER.to_owned()),
    ];
    v.extend(d.extra.users.clone());
    v.retain(|(id, _)| !d.extra.deleted_users.contains(id));
    v
}

async fn users_list(axum::extract::State(shared): S) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let rows: Vec<Value> = all_users(&shared)
        .iter()
        .map(|(id, name)| user_json(&shared, id, name))
        .collect();
    jv(Value::Array(rows))
}

async fn user_get(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    match all_users(&shared).into_iter().find(|(i, _)| *i == id) {
        Some((i, n)) => jv(user_json(&shared, &i, &n)),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn user_create(
    axum::extract::State(shared): S,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    let Some(name) = body
        .get("Name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if all_users(&shared).iter().any(|(_, n)| n == name) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let id = {
        let mut d = shared.lock();
        d.extra.next_user += 1;
        let id = xid(0xB0, u128::from(d.extra.next_user));
        d.extra.users.push((id.clone(), name.to_owned()));
        id
    };
    jv(user_json(&shared, &id, name))
}

async fn user_policy(
    axum::extract::State(shared): S,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    if !all_users(&shared).iter().any(|(i, _)| *i == id) || !body.is_object() {
        return StatusCode::NOT_FOUND.into_response();
    }
    shared.lock().extra.policies.insert(id, body);
    StatusCode::NO_CONTENT.into_response()
}

async fn user_delete(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !is_admin(&shared) {
        return forbidden();
    }
    if id == USER_ID || !all_users(&shared).iter().any(|(i, _)| *i == id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    shared.lock().extra.deleted_users.push(id);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------- Charts-Worker-Nachbau

/// Antwort von `/cw/me/stats` für einen Zeitraum; `None` bei unbekanntem Zeitraum (dann 400).
pub(crate) fn stats_json(range: &str) -> Option<Value> {
    let unit = match range {
        "7d" | "30d" => "day",
        "year" => "week",
        "all" => "month",
        _ => return None,
    };
    Some(json!({
        "version": 1, "range": range, "computedAt": "2026-01-01T00:00:00Z", "stale": false,
        "approx": false, "ignored": false,
        "totals": { "seconds": 3600, "plays": 2, "episodes": 2, "movies": 1, "series": 1, "activeDays": 2,
                    "firstAt": "2025-12-30 20:00:00", "lastAt": "2025-12-31 21:00:00" },
        "topSeries": [{ "id": catalog::series_id(0), "seconds": 2400, "episodes": 2 }],
        "topMovies": [{ "id": catalog::movie_id(0), "seconds": 1200, "plays": 1 }],
        "timeline": { "unit": unit, "buckets": [{ "key": "2025-12-30", "seconds": 1800 }, { "key": "2025-12-31", "seconds": 1800 }] },
        "heatmap": [{ "dow": 2, "hour": 20, "seconds": 1800 }, { "dow": 3, "hour": 21, "seconds": 1800 }],
        "binge": { "topDay": { "date": "2025-12-30", "seconds": 1800 },
                   "seriesDay": { "date": "2025-12-30", "seriesId": catalog::series_id(0), "episodes": 2 } },
        "devices": [{ "client": "LENZFLIX", "device": "Dieser Rechner", "plays": 2, "seconds": 3600 }],
        "methods": { "directPlay": { "plays": 2, "seconds": 3600 }, "directStream": { "plays": 0, "seconds": 0 },
                     "transcode": { "plays": 0, "seconds": 0 }, "other": { "plays": 0, "seconds": 0 } },
        "compare": { "rank": 2, "of": 4, "avgSeconds": 5000 },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_valid() {
        let png = make_png(5, 3);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // Alle Blöcke prüfen: Länge, Typ, Daten, CRC.
        let mut pos = 8;
        let mut kinds = Vec::new();
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let body = &png[pos + 4..pos + 8 + len];
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            assert_eq!(crc32(body), crc, "CRC von {:?}", &body[..4]);
            kinds.push(String::from_utf8_lossy(&body[..4]).into_owned());
            pos += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
    }

    #[test]
    fn crc_and_adler_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn iso_roundtrip() {
        for secs in [0, 1_767_225_600, 1_900_000_123] {
            assert_eq!(parse_iso(&iso(secs)), Some(secs));
        }
        assert_eq!(iso(0), "1970-01-01T00:00:00.0000000Z");
    }

    #[test]
    fn program_ids_roundtrip() {
        let id = program_id(3, 490_000);
        assert_eq!(id.len(), 32);
        assert_eq!(decode_program(&id), Some((3, 490_000)));
        assert!(decode_program(&track_id(0)).is_none());
    }

    #[test]
    fn media_ids_are_unique_32_hex() {
        let mut seen = std::collections::HashSet::new();
        for m in media() {
            assert_eq!(m.id.len(), 32);
            assert!(m.id.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(seen.insert(m.id.clone()), "doppelt: {}", m.id);
        }
        let _ = (DEMO_USER, GUEST_USER);
    }
}
