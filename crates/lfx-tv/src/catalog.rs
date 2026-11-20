//! Katalog laden und zwischenspeichern (Port von `features/tvdata/data.js`).
//!
//! Der Roh-Katalog (Filme, Serien, Folgen, Sammlungen mit schlanken Feldern) wird seitenweise über
//! das Trait [`CatalogSource`] geholt (Server-Zugriff, Adapter für `lfx-api` in
//! [`crate::api_source`]) und je Benutzer und Sendetag über [`CatalogCache`] gehalten
//! (In-Memory-Form: [`MemoryCatalogCache`]; die App bindet später die Datenbank an). Es landen keine
//! Schlüssel im Cache. Anders als im Web (gerätelokale Zeit) bestimmt die Planzone (MEZ/MESZ) den
//! Sendetag des Cache-Schlüssels, damit der Schlüssel deterministisch bleibt.

use indexmap::IndexSet;
use lfx_tv_schedule::JsVal;
use lfx_tv_schedule::time::{broadcast_day_of, civil_from_days};
use serde_json::{Map, Value};

use crate::error::TvError;

/// Seitengröße für Filme, Serien, Sammlungen.
pub const PAGE_SIZE: usize = 1000;
/// Seitengröße für Folgen.
pub const EPISODE_PAGE_SIZE: usize = 2000;

const BASE_FIELDS: &str =
    "Genres,OfficialRating,ProductionYear,PremiereDate,DateCreated,RunTimeTicks";

/// Teilkatalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogKind {
    Movies,
    Series,
    Episodes,
    Boxsets,
}

impl CatalogKind {
    /// Alle vier in Abrufreihenfolge.
    pub const ALL: [CatalogKind; 4] = [
        CatalogKind::Movies,
        CatalogKind::Series,
        CatalogKind::Episodes,
        CatalogKind::Boxsets,
    ];

    /// Jellyfin-Typ (`IncludeItemTypes`).
    pub fn item_type(self) -> &'static str {
        match self {
            CatalogKind::Movies => "Movie",
            CatalogKind::Series => "Series",
            CatalogKind::Episodes => "Episode",
            CatalogKind::Boxsets => "BoxSet",
        }
    }

    /// Angeforderte Felder.
    pub fn fields(self) -> &'static str {
        match self {
            CatalogKind::Movies | CatalogKind::Series => BASE_FIELDS,
            CatalogKind::Episodes => "DateCreated,PremiereDate,RunTimeTicks,OfficialRating",
            CatalogKind::Boxsets => "ChildCount",
        }
    }

    /// Seitengröße.
    pub fn page_size(self) -> usize {
        if self == CatalogKind::Episodes {
            EPISODE_PAGE_SIZE
        } else {
            PAGE_SIZE
        }
    }

    /// Bild-Tags mitliefern (nur Filme und Serien).
    pub fn with_image_tags(self) -> bool {
        matches!(self, CatalogKind::Movies | CatalogKind::Series)
    }

    /// Felder, die in Katalog und Cache wandern (Reihenfolge ist Vertrag: sie bestimmt den
    /// Gleichstand bei der Deduplizierung im Planer).
    pub fn keep(self) -> &'static [&'static str] {
        match self {
            CatalogKind::Movies => &[
                "Id",
                "Name",
                "RunTimeTicks",
                "Genres",
                "OfficialRating",
                "ProductionYear",
                "PremiereDate",
                "DateCreated",
                "LocalTrailerCount",
                "ImageTags",
                "BackdropImageTags",
            ],
            CatalogKind::Series => &[
                "Id",
                "Name",
                "Genres",
                "OfficialRating",
                "ProductionYear",
                "DateCreated",
                "ImageTags",
                "BackdropImageTags",
            ],
            CatalogKind::Episodes => &[
                "Id",
                "Name",
                "SeriesId",
                "SeriesName",
                "ParentIndexNumber",
                "IndexNumber",
                "RunTimeTicks",
                "PremiereDate",
                "DateCreated",
                "OfficialRating",
            ],
            CatalogKind::Boxsets => &["Id", "Name", "ChildCount"],
        }
    }
}

/// Anfrage einer Seite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub kind: CatalogKind,
    pub start_index: usize,
    pub limit: usize,
}

/// Eine Seite roher Server-Einträge (PascalCase-JSON wie vom Server).
#[derive(Debug, Clone, Default)]
pub struct RawPage {
    pub items: Vec<Value>,
    /// `TotalRecordCount`, falls geliefert.
    pub total: Option<i64>,
}

/// Quelle des Katalogs (Server). Fehler dürfen keine Schlüssel/Adressen enthalten.
#[allow(async_fn_in_trait)]
pub trait CatalogSource {
    /// Holt eine Seite.
    async fn fetch_page(&self, req: &PageRequest) -> Result<RawPage, TvError>;
}

/// Roh-Katalog (`RawCatalog` des Vertrags).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawCatalog {
    pub movies: Vec<Value>,
    pub series: Vec<Value>,
    pub episodes: Vec<Value>,
    pub boxsets: Vec<Value>,
    /// Zeitpunkt des Ladens (ms).
    pub loaded_at: i64,
}

impl RawCatalog {
    /// Als JSON (Cache-Form).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("movies".into(), Value::Array(self.movies.clone()));
        m.insert("series".into(), Value::Array(self.series.clone()));
        m.insert("episodes".into(), Value::Array(self.episodes.clone()));
        m.insert("boxsets".into(), Value::Array(self.boxsets.clone()));
        m.insert("loadedAt".into(), Value::from(self.loaded_at));
        Value::Object(m)
    }

    /// Aus JSON; `None`, wenn eine der vier Listen fehlt (`validRaw`).
    pub fn from_json(v: &Value) -> Option<Self> {
        let list = |k: &str| v.get(k).and_then(Value::as_array).cloned();
        Some(Self {
            movies: list("movies")?,
            series: list("series")?,
            episodes: list("episodes")?,
            boxsets: list("boxsets")?,
            loaded_at: v.get("loadedAt").and_then(Value::as_i64).unwrap_or(0),
        })
    }

    /// Für `lfx_tv_schedule::normalize_catalog`.
    pub fn to_jsval(&self) -> JsVal {
        JsVal::from_json(&self.to_json())
    }

    fn slot(&mut self, kind: CatalogKind) -> &mut Vec<Value> {
        match kind {
            CatalogKind::Movies => &mut self.movies,
            CatalogKind::Series => &mut self.series,
            CatalogKind::Episodes => &mut self.episodes,
            CatalogKind::Boxsets => &mut self.boxsets,
        }
    }
}

/// Zwischenspeicher je Benutzer und Sendetag. Die App kann die Datenbank dahinter binden.
pub trait CatalogCache {
    /// Eintrag zum Schlüssel.
    fn get(&self, key: &str) -> Option<RawCatalog>;
    /// Speichert und entfernt ältere Einträge desselben Benutzers.
    fn put(&mut self, user_id: &str, key: &str, value: &RawCatalog);
    /// Leert alles (Geheimmenü: alle Benutzer/Sendetage).
    fn clear(&mut self);
}

/// Cache im Speicher (Einfügereihenfolge deterministisch).
#[derive(Debug, Default)]
pub struct MemoryCatalogCache {
    entries: Vec<(String, RawCatalog)>,
}

impl MemoryCatalogCache {
    /// Leerer Cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gespeicherte Schlüssel.
    pub fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.0.as_str()).collect()
    }
}

impl CatalogCache for MemoryCatalogCache {
    fn get(&self, key: &str) -> Option<RawCatalog> {
        self.entries
            .iter()
            .find(|e| e.0 == key)
            .map(|e| e.1.clone())
    }

    fn put(&mut self, user_id: &str, key: &str, value: &RawCatalog) {
        let prefix = format!("{}:", if user_id.is_empty() { "anon" } else { user_id });
        self.entries
            .retain(|e| !(e.0.starts_with(&prefix) && e.0 != key));
        if let Some(e) = self.entries.iter_mut().find(|e| e.0 == key) {
            e.1 = value.clone();
        } else {
            self.entries.push((key.to_string(), value.clone()));
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Sendetag als `YYYY-MM-DD` (Planzone, Beginn 05:00).
pub fn broadcast_day_key(now: i64) -> String {
    let (y, m, d) = civil_from_days(broadcast_day_of(now));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Cache-Schlüssel `<Benutzer>:<Sendetag>`.
pub fn cache_key(user_id: &str, now: i64) -> String {
    format!(
        "{}:{}",
        if user_id.is_empty() { "anon" } else { user_id },
        broadcast_day_key(now)
    )
}

/// Schlanke Kopie eines Server-Eintrags: nur die Vertragsfelder in `keep`-Reihenfolge, ohne
/// fehlende oder `null`-Werte; Bild-Tags nur Thumb/Primary bzw. das erste Hintergrund-Tag.
pub fn compact(item: &Value, kind: CatalogKind) -> Value {
    let mut out = Map::new();
    for k in kind.keep() {
        match item.get(*k) {
            None | Some(Value::Null) => {}
            Some(v) => {
                out.insert((*k).to_string(), v.clone());
            }
        }
    }
    if let Some(Value::Object(tags)) = out.get("ImageTags").cloned() {
        let mut t = Map::new();
        for key in ["Thumb", "Primary"] {
            if let Some(v) = tags
                .get(key)
                .filter(|v| !v.is_null() && v.as_str() != Some(""))
            {
                t.insert(key.to_string(), v.clone());
            }
        }
        out.insert("ImageTags".into(), Value::Object(t));
    }
    if let Some(Value::Array(b)) = out.get("BackdropImageTags").cloned() {
        out.insert(
            "BackdropImageTags".into(),
            Value::Array(b.into_iter().take(1).collect()),
        );
    }
    Value::Object(out)
}

/// Fortschritt (0..1) und Beschriftung.
pub type Progress<'a> = &'a mut dyn FnMut(f64, &str);

/// Holt einen Teilkatalog seitenweise bis `TotalRecordCount` (doppelte und id-lose Einträge
/// entfallen). `part` meldet den Teilfortschritt 0..1.
pub async fn fetch_kind<S: CatalogSource>(
    src: &S,
    kind: CatalogKind,
    part: &mut dyn FnMut(f64),
) -> Result<Vec<Value>, TvError> {
    let size = kind.page_size();
    let mut out = Vec::new();
    let mut seen: IndexSet<String> = IndexSet::new();
    let mut total: Option<i64> = None;
    let mut start = 0usize;
    loop {
        let page = src
            .fetch_page(&PageRequest {
                kind,
                start_index: start,
                limit: size,
            })
            .await?;
        let n = page.items.len();
        for it in &page.items {
            let Some(id) = it
                .get("Id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue;
            }
            out.push(compact(it, kind));
        }
        if page.total.is_some() {
            total = page.total;
        }
        let done = n < size || total.is_some_and(|t| (start + size) as i64 >= t);
        part(if done {
            1.0
        } else {
            total
                .filter(|t| *t > 0)
                .map_or(0.5, |t| ((start + size) as f64 / t as f64).min(0.99))
        });
        if done {
            break;
        }
        start += size;
    }
    Ok(out)
}

/// Lädt alle vier Teilkataloge (nacheinander; ein Host kann [`fetch_kind`] auch parallel laufen
/// lassen). `on_progress(Anteil 0..1, Beschriftung)`.
pub async fn fetch_catalog<S: CatalogSource>(
    src: &S,
    now: i64,
    on_progress: Option<Progress<'_>>,
) -> Result<RawCatalog, TvError> {
    let mut raw = RawCatalog {
        loaded_at: now,
        ..RawCatalog::default()
    };
    let mut cb = on_progress;
    let mut done_parts = 0.0;
    for kind in CatalogKind::ALL {
        let base = done_parts;
        let items = fetch_kind(src, kind, &mut |f| {
            if let Some(cb) = cb.as_mut() {
                cb((base + f) / 4.0, "Sendersuchlauf …");
            }
        })
        .await?;
        *raw.slot(kind) = items;
        done_parts += 1.0;
    }
    Ok(raw)
}

/// Katalog für den aktuellen Sendetag: Cache, sonst Server (dann zurückschreiben). Fehler der
/// Quelle werden zurückgegeben, nicht gecacht.
pub async fn load_catalog<S: CatalogSource>(
    src: &S,
    cache: &mut dyn CatalogCache,
    user_id: &str,
    now: i64,
    mut on_progress: Option<Progress<'_>>,
) -> Result<RawCatalog, TvError> {
    let key = cache_key(user_id, now);
    if let Some(hit) = cache.get(&key) {
        if let Some(cb) = on_progress.as_mut() {
            cb(1.0, "Sendersuchlauf …");
        }
        return Ok(hit);
    }
    let raw = fetch_catalog(src, now, on_progress).await?;
    cache.put(user_id, &key, &raw);
    Ok(raw)
}

/// Leert den Cache (Geheimmenü).
pub fn clear_catalog_cache(cache: &mut dyn CatalogCache) {
    cache.clear();
}

/// Bild-Tags je Id aus dem Roh-Katalog (Filme und Serien), für Bildadressen mit `tag`.
pub fn image_tag_map(raw: &RawCatalog) -> std::collections::HashMap<String, ImageTags> {
    let mut m = std::collections::HashMap::new();
    for it in raw.movies.iter().chain(raw.series.iter()) {
        let Some(id) = it.get("Id").and_then(Value::as_str) else {
            continue;
        };
        let tag = |k: &str| {
            it.get("ImageTags")
                .and_then(|t| t.get(k))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let backdrop = it
            .get("BackdropImageTags")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        let t = ImageTags {
            primary: tag("Primary"),
            thumb: tag("Thumb"),
            backdrop,
        };
        if t.primary.is_some() || t.thumb.is_some() || t.backdrop.is_some() {
            m.insert(id.to_string(), t);
        }
    }
    m
}

/// Bild-Tags eines Titels (Cache-Kennung der Bildadresse).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageTags {
    pub primary: Option<String>,
    pub thumb: Option<String>,
    pub backdrop: Option<String>,
}
