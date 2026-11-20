//! Billboard (Hero): Pool, Rotation, Trailer-Wahl, Metazeile (Port von `features/billboard.js`,
//! `billboard/format.js`, `shared/trailer.js`).

use crate::call::{ApiCall, Endpoint};
use crate::images::{ImagePick, ImageRef, Slot, own_image, pick_logo, pixel_width};
use crate::rows::logic::ITEM_FIELDS;
use crate::util::{DAY_MS, js_round};
use lfx_api::{BaseItem, ImageType, ItemKind, ItemsQuery, SortBy, SortOrder};
use serde_json::Value;

/// Wechsel der Folien in Millisekunden.
pub const ROTATE_MS: u64 = 10_000;
/// Das Titelbild steht mindestens so lange, bevor der Trailer eingeblendet wird.
pub const TRAILER_MIN_IMAGE_MS: u64 = 3_000;
/// So lange hält die Rotation für einen ladenden Trailer an.
pub const TRAILER_WAIT_MAX_MS: u64 = 25_000;
/// Der Trailer lädt spätestens dann vor, auch ohne „Zeilen fertig“.
pub const ROWS_WAIT_MAX_MS: u64 = 8_000;
/// Überblendung des Bildes.
pub const IMG_FADE_MS: u64 = 1_000;
/// Ausblenden des Textes beim Wechsel.
pub const TEXT_FADE_MS: u64 = 400;
/// Pool-Größe im Datensparmodus (`hero`).
pub const SAVER_POOL: usize = 5;
/// „Neu“-Fenster in Tagen.
pub const NEW_DAYS: f64 = 14.0;

/// Billboard-Eintrag mit den Trailer-Feldern, die [`BaseItem`] (noch) nicht trägt.
#[derive(Debug, Clone, Default)]
pub struct HeroItem {
    /// Das Element.
    pub item: BaseItem,
    /// `LocalTrailerCount`.
    pub local_trailer_count: i64,
    /// URLs aus `RemoteTrailers[].Url`.
    pub remote_trailer_urls: Vec<String>,
}

impl HeroItem {
    /// Aus rohem JSON (wie von [`ApiCall::execute_values`]); nicht lesbar = `None`.
    pub fn from_json(v: &Value) -> Option<Self> {
        let item: BaseItem = serde_json::from_value(v.clone()).ok()?;
        let local_trailer_count = v
            .get("LocalTrailerCount")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let remote_trailer_urls = v
            .get("RemoteTrailers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.get("Url").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            item,
            local_trailer_count,
            remote_trailer_urls,
        })
    }
}

/// Was der Pool von seinen Einträgen wissen muss.
pub trait PoolEntry {
    /// Id.
    fn id(&self) -> &str;
    /// Eigenes Logo und Backdrop vorhanden (`hasLogoAndBackdrop`).
    fn has_logo_and_backdrop(&self) -> bool;
    /// Lokale Trailer.
    fn local_trailer_count(&self) -> i64;
}

fn base_has_logo_and_backdrop(it: &BaseItem) -> bool {
    !it.id.is_empty()
        && it.image_tags.get("Logo").is_some_and(|t| !t.is_empty())
        && !it.backdrop_image_tags.is_empty()
}

impl PoolEntry for HeroItem {
    fn id(&self) -> &str {
        &self.item.id
    }
    fn has_logo_and_backdrop(&self) -> bool {
        base_has_logo_and_backdrop(&self.item)
    }
    fn local_trailer_count(&self) -> i64 {
        self.local_trailer_count
    }
}

impl PoolEntry for BaseItem {
    fn id(&self) -> &str {
        &self.id
    }
    fn has_logo_and_backdrop(&self) -> bool {
        base_has_logo_and_backdrop(self)
    }
    /// Das DTO trägt `LocalTrailerCount` noch nicht: immer 0 (für Trailer-Wahl [`HeroItem`] nutzen).
    fn local_trailer_count(&self) -> i64 {
        0
    }
}

/// Optionen für [`build_pool`].
#[derive(Debug, Clone, Default)]
pub struct PoolOpts {
    /// Anzahl neuer Titel (Standard 4).
    pub new_count: usize,
    /// Anzahl Zufallstitel (Standard 8).
    pub random_count: usize,
    /// Dieser Titel steht nie an erster Stelle (zuletzt gezeigter), außer es gibt keine Alternative.
    pub avoid_first_id: Option<String>,
}

impl PoolOpts {
    /// Standardwerte der Startseite (4 neue, 8 zufällige).
    pub fn home(avoid_first_id: Option<String>) -> Self {
        Self {
            new_count: 4,
            random_count: 8,
            avoid_first_id,
        }
    }

    /// Werte der Bibliotheksseiten (3 neue, 7 zufällige).
    pub fn library() -> Self {
        Self {
            new_count: 3,
            random_count: 7,
            avoid_first_id: None,
        }
    }
}

fn shuffled<T: Clone>(list: &[T], rng: &mut dyn FnMut() -> f64) -> Vec<T> {
    let mut a = list.to_vec();
    for i in (1..a.len()).rev() {
        let j = (rng() * (i as f64 + 1.0)).floor() as usize;
        a.swap(i, j);
    }
    a
}

/// Billboard-Pool: Zufall zuerst, dann abwechselnd zufällig/neu (Z N Z N … Z Z).
/// Neue: die `new_count` neuesten (Logo + Backdrop) in zufälliger Reihenfolge; Zufällige: `random_count`
/// ohne Überschneidung. Folie 1: bevorzugt Zufallstitel mit lokalem Trailer, nie `avoid_first_id`.
/// `rng` liefert Werte in [0,1) (im Web `Math.random`).
pub fn build_pool<T: PoolEntry + Clone>(
    latest: &[T],
    random: &[T],
    opts: &PoolOpts,
    rng: &mut dyn FnMut() -> f64,
) -> Vec<T> {
    fn uniq<T: PoolEntry + Clone>(list: &[T], skip: &[String]) -> Vec<T> {
        let mut seen: Vec<String> = skip.to_vec();
        let mut out = Vec::new();
        for it in list {
            if it.has_logo_and_backdrop() && !seen.iter().any(|s| s == it.id()) {
                seen.push(it.id().to_owned());
                out.push(it.clone());
            }
        }
        out
    }
    let newest: Vec<T> = uniq(latest, &[]).into_iter().take(opts.new_count).collect();
    let a = shuffled(&newest, rng);
    let a_ids: Vec<String> = a.iter().map(|i| i.id().to_owned()).collect();
    let mut b: Vec<T> = uniq(random, &a_ids)
        .into_iter()
        .take(opts.random_count)
        .collect();
    let first = {
        let fresh: Vec<usize> = (0..b.len())
            .filter(|&i| opts.avoid_first_id.as_deref() != Some(b[i].id()))
            .collect();
        fresh
            .iter()
            .copied()
            .find(|&i| b[i].local_trailer_count() > 0)
            .or_else(|| fresh.first().copied())
    };
    if let Some(i) = first {
        let it = b.remove(i);
        b.insert(0, it);
    }
    let mut out = Vec::new();
    for i in 0..a.len().max(b.len()) {
        if let Some(x) = b.get(i) {
            out.push(x.clone());
        }
        if let Some(x) = a.get(i) {
            out.push(x.clone());
        }
    }
    out
}

/// Datensparmodus: Pool auf [`SAVER_POOL`] kürzen (die ersten fünf = 3 zufällige + 2 neue); `keep_id`
/// (gezeigter Titel) ersetzt den letzten Platz, falls er sonst herausfiele.
pub fn saver_pool<T: PoolEntry + Clone>(pool: &[T], keep_id: Option<&str>) -> Vec<T> {
    if pool.len() <= SAVER_POOL {
        return pool.to_vec();
    }
    let mut out: Vec<T> = pool[..SAVER_POOL].to_vec();
    if let Some(k) = keep_id
        && !out.iter().any(|i| i.id() == k)
        && let Some(cur) = pool.iter().find(|i| i.id() == k)
    {
        out[SAVER_POOL - 1] = cur.clone();
    }
    out
}

/// Entfernt Titel aus dem Pool (`onChanged`): neuer Pool, neuer Index, „aktueller Titel weg“.
/// Leerer Pool = Billboard ausblenden.
pub fn remove_from_pool<T: PoolEntry + Clone>(
    pool: &[T],
    index: usize,
    removed: &[String],
) -> (Vec<T>, usize, bool) {
    let cur_id = pool.get(index).map(|i| i.id().to_owned());
    let kept: Vec<T> = pool
        .iter()
        .filter(|i| !removed.iter().any(|r| r == i.id()))
        .cloned()
        .collect();
    let cur_gone = cur_id.as_ref().is_some_and(|c| removed.contains(c));
    let new_index = if kept.is_empty() {
        0
    } else if cur_gone {
        index.min(kept.len() - 1)
    } else {
        cur_id
            .and_then(|c| kept.iter().position(|i| i.id() == c))
            .unwrap_or(0)
    };
    (kept, new_index, cur_gone)
}

// ---------------------------------------------------------------- Abfragen

/// `FIELDS` der Startseiten-Billboard-Abfrage.
pub const HOME_FIELDS: [&str; 7] = [
    "Genres",
    "OfficialRating",
    "ProductionYear",
    "RunTimeTicks",
    "ChildCount",
    "DateCreated",
    "LocalTrailerCount",
];

fn pool_query(types: &[ItemKind], fields: &[&str], image_types: &[&str], limit: u32) -> ItemsQuery {
    ItemsQuery::new()
        .recursive(true)
        .include_types(types)
        .raw("imageTypes", "Logo,Backdrop")
        .fields(fields)
        .enable_user_data(true)
        .enable_image_types(image_types)
        .image_type_limit(1)
        .limit(limit)
}

/// Pool-Abfragen der Startseite: (neueste, zufällige), je 12 Titel.
pub fn home_pool_calls() -> (ApiCall, ApiCall) {
    let base = || {
        pool_query(
            &[ItemKind::Movie, ItemKind::Series],
            &HOME_FIELDS,
            &["Logo", "Backdrop", "Primary"],
            12,
        )
    };
    (
        ApiCall::new(
            Endpoint::Items,
            base()
                .sort_by(&[SortBy::DateCreated])
                .order(SortOrder::Descending),
        ),
        ApiCall::new(Endpoint::Items, base().sort_by(&[SortBy::Random])),
    )
}

/// Pool-Abfragen der Bibliotheksseiten: (30 neueste, 40 zufällige) mit `RemoteTrailers` und `LocalTrailerCount`.
/// Bei Serien zählt hier wie im Web `DateCreated` (nicht `DateLastContentAdded`).
pub fn library_pool_calls(kind: ItemKind, genre_id: Option<&str>) -> (ApiCall, ApiCall) {
    let mut fields: Vec<&str> = ITEM_FIELDS.to_vec();
    fields.extend(["RemoteTrailers", "LocalTrailerCount"]);
    let base = |limit| {
        let q = ItemsQuery::new()
            .fields(&fields)
            .enable_user_data(true)
            .image_type_limit(1)
            .enable_image_types(&["Primary", "Backdrop", "Thumb", "Logo"])
            .include_types(&[kind])
            .recursive(true)
            .limit(limit);
        match genre_id {
            Some(g) => q.raw("genreIds", g),
            None => q,
        }
    };
    (
        ApiCall::new(
            Endpoint::Items,
            base(30)
                .sort_by(&[SortBy::DateCreated])
                .order(SortOrder::Descending),
        ),
        ApiCall::new(Endpoint::Items, base(40).sort_by(&[SortBy::Random])),
    )
}

/// Rückfall der Bibliotheks-Pool-Abfrage ohne passende Logo+Backdrop-Paare: bis 8 eindeutige Titel mit Backdrop.
pub fn library_pool_fallback(latest: &[HeroItem], random: &[HeroItem]) -> Vec<HeroItem> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for it in latest.iter().chain(random) {
        if it.item.id.is_empty()
            || it.item.backdrop_image_tags.is_empty()
            || seen.contains(&it.item.id.as_str())
        {
            continue;
        }
        seen.push(&it.item.id);
        out.push(it.clone());
        if out.len() == 8 {
            break;
        }
    }
    out
}

/// Cache-Schlüssel des Startseiten-Pools (`home:<user>:billboard`).
pub fn pool_cache_key(user_id: &str) -> String {
    crate::cache::home_key(user_id, &["billboard"])
}

/// Ein leerer Pool wird nicht gecacht.
pub fn should_cache_pool(pool_len: usize) -> bool {
    pool_len > 0
}

// ---------------------------------------------------------------- Rotation

/// Eingaben der Pausenregel (`paused()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PauseInputs {
    /// Zeiger über dem Billboard.
    pub hovering: bool,
    /// Tastaturfokus im Billboard (WCAG 2.2.2); Mausklick-Fokus zählt nicht.
    pub keyboard_focus: bool,
    /// Trailer läuft sichtbar.
    pub trailer_active: bool,
    /// Trailer lädt noch (höchstens [`TRAILER_WAIT_MAX_MS`]).
    pub trailer_pending: bool,
    /// Fenster nicht sichtbar.
    pub window_hidden: bool,
    /// Info-Fenster offen.
    pub modal_open: bool,
    /// Titelmenü offen.
    pub menu_open: bool,
    /// Download-Auswahl offen.
    pub chooser_open: bool,
}

/// Rotation pausiert?
pub fn rotation_paused(p: &PauseInputs) -> bool {
    p.hovering
        || p.keyboard_focus
        || p.trailer_active
        || p.trailer_pending
        || p.window_hidden
        || p.modal_open
        || p.menu_open
        || p.chooser_open
}

/// Rotation überhaupt aktiv (`scheduleRotate`): mindestens 2 Folien, „Bewegung reduzieren“ aus.
pub fn rotation_enabled(pool_len: usize, reduced_motion: bool) -> bool {
    pool_len >= 2 && !reduced_motion
}

/// Nächster Index (zyklisch).
pub fn next_index(index: usize, pool_len: usize) -> usize {
    if pool_len == 0 {
        0
    } else {
        (index + 1) % pool_len
    }
}

/// Eingaben der Trailer-Sperre (`noAuto`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TrailerPolicy {
    /// Datensparmodus, Teil `trailer`, wirkt.
    pub datasaver_trailer: bool,
    /// „Bewegung reduzieren“.
    pub reduced_motion: bool,
    /// TV-Modus läuft im Hintergrund-Bild-in-Bild.
    pub tv_background_playing: bool,
}

impl TrailerPolicy {
    /// Automatische Trailer gesperrt?
    pub fn no_auto(&self) -> bool {
        self.datasaver_trailer || self.reduced_motion || self.tv_background_playing
    }
}

/// Trailer, der für einen Titel abgespielt wird.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrailerPlan {
    /// Lokaler Trailer (Id wird über `LocalTrailers` aufgelöst, Strom über [`local_trailer_stream`]).
    Local,
    /// Direkt abspielbarer Remote-Trailer (nur Knopfdruck).
    RemoteDirect(String),
    /// Keiner.
    None,
}

/// Direkt abspielbarer Remote-Trailer: nur `https`-Videodatei-URLs (`.mp4|.m4v|.webm`), keine Plattform-Links.
pub fn direct_remote_trailer_url(urls: &[String]) -> Option<&str> {
    urls.iter().map(String::as_str).find(|u| {
        let lower = u.to_ascii_lowercase();
        if !lower.starts_with("https://") {
            return false;
        }
        [".mp4", ".m4v", ".webm"].iter().any(|ext| {
            lower.match_indices(ext).any(|(i, _)| {
                matches!(
                    lower.as_bytes().get(i + ext.len()),
                    None | Some(b'?') | Some(b'#')
                )
            })
        })
    })
}

/// Wahl des Trailers. Automatisch (`manual = false`): nur lokale, nie Remote, und nur ohne Sperre.
/// Per Knopf (`manual = true`): lokal zuerst, sonst direkter Remote-Trailer (diese Reihenfolge ist eine
/// Ableitung, das Web hat im Billboard keinen Remote-Knopf).
pub fn choose_trailer(item: &HeroItem, manual: bool, policy: &TrailerPolicy) -> TrailerPlan {
    if item.local_trailer_count > 0 && (manual || !policy.no_auto()) {
        return TrailerPlan::Local;
    }
    if manual && let Some(u) = direct_remote_trailer_url(&item.remote_trailer_urls) {
        return TrailerPlan::RemoteDirect(u.to_owned());
    }
    TrailerPlan::None
}

/// Abfrage der lokalen Trailer eines Titels (erstes Element mit Id zählt).
pub fn local_trailers_call(item_id: &str) -> ApiCall {
    ApiCall::new(
        Endpoint::LocalTrailers(item_id.to_owned()),
        ItemsQuery::new(),
    )
}

/// Strom eines lokalen Trailers: Pfad und Parameter **ohne Token** (der Player schickt den Auth-Header;
/// `static=true` wie im Web).
pub fn local_trailer_stream(trailer_id: &str) -> (String, Vec<(String, String)>) {
    (
        format!(
            "/Videos/{}/stream.mp4",
            crate::util::encode_uri_component(trailer_id)
        ),
        vec![("static".to_owned(), "true".to_owned())],
    )
}

/// Wartezeit, bis der Trailer sichtbar werden darf (Bild stand schon `TRAILER_MIN_IMAGE_MS`?).
pub fn trailer_reveal_wait_ms(image_shown_at_ms: i64, now_ms: i64) -> u64 {
    (image_shown_at_ms + TRAILER_MIN_IMAGE_MS as i64 - now_ms).max(0) as u64
}

/// Wartet die Rotation noch auf einen ladenden Trailer? (`trailerPending`)
pub fn trailer_pending(
    has_local_trailer: bool,
    failed: bool,
    active: bool,
    policy: &TrailerPolicy,
    slide_age_ms: u64,
) -> bool {
    has_local_trailer
        && !failed
        && !active
        && !policy.no_auto()
        && slide_age_ms < TRAILER_WAIT_MAX_MS
}

// ---------------------------------------------------------------- Bilder

/// Bilder einer Folie: Hintergrund (Breitbild), Poster (Handy), Logo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeroImages {
    /// Backdrop (1600 px).
    pub backdrop: Option<ImageRef>,
    /// Primary-Poster am Handy (720 px).
    pub poster: Option<ImageRef>,
    /// Logo (720 px) oder `None` → Titel als Text.
    pub logo: Option<ImageRef>,
}

/// Bildwahl einer Billboard-Folie (`buildImage`/`buildContent`).
pub fn hero_images(item: &BaseItem, dpr: f64, scale: f64) -> HeroImages {
    let r = |p: Option<ImagePick>, slot: Slot| {
        p.map(|pick| ImageRef {
            pick,
            max_width: pixel_width(slot, false, dpr, scale),
            quality: slot.quality(),
        })
    };
    HeroImages {
        backdrop: r(
            own_image(item, ImageType::Backdrop),
            Slot::BillboardBackdrop,
        ),
        poster: r(own_image(item, ImageType::Primary), Slot::BillboardPoster),
        logo: r(pick_logo(item), Slot::BillboardLogo),
    }
}

// ---------------------------------------------------------------- Format

/// „1 Std. 46 Min.“ aus Ticks (auf Minuten gerundet); 0 oder negativ = leer.
pub fn format_runtime(ticks: Option<i64>) -> String {
    let total = js_round(ticks.unwrap_or(0) as f64 / 600_000_000.0);
    if total <= 0.0 {
        return String::new();
    }
    let total = total as i64;
    let (hrs, min) = (total / 60, total % 60);
    if hrs == 0 {
        format!("{min} Min.")
    } else if min != 0 {
        format!("{hrs} Std. {min} Min.")
    } else {
        format!("{hrs} Std.")
    }
}

/// „FSK-12“ → „12“ (auch „DE 12“, „fsk12“); Unbekanntes bleibt unverändert, leer = leer.
pub fn format_rating(rating: Option<&str>) -> String {
    let s = rating.unwrap_or("").trim();
    if s.is_empty() {
        return String::new();
    }
    let lower = s.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("fsk")
        .or_else(|| lower.strip_prefix("de"))
        .map(|r| r.trim_start_matches(|c: char| c.is_whitespace() || c == '-'));
    match rest {
        Some(d) if (1..=2).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()) => {
            d.to_owned()
        }
        _ => s.to_owned(),
    }
}

/// „1 Staffel“ / „3 Staffeln“; sonst leer.
pub fn format_seasons(n: Option<i64>) -> String {
    match n.unwrap_or(0) {
        c if c <= 0 => String::new(),
        1 => "1 Staffel".into(),
        c => format!("{c} Staffeln"),
    }
}

/// Teil der Metazeile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaPart {
    /// Text.
    pub text: String,
    /// Altersfreigabe (eigene Darstellung).
    pub rating: bool,
}

/// Metazeile: Art, erstes Genre, Jahr, Staffeln bzw. Dauer, Altersfreigabe.
pub fn meta_parts(item: &BaseItem) -> Vec<MetaPart> {
    let series = item.kind == ItemKind::Series;
    let plain = |t: &str| MetaPart {
        text: t.to_owned(),
        rating: false,
    };
    let mut parts = vec![plain(if series { "Serie" } else { "Film" })];
    if let Some(g) = item.genres.first().filter(|g| !g.is_empty()) {
        parts.push(plain(g));
    }
    if let Some(y) = item.production_year.filter(|y| *y != 0) {
        parts.push(plain(&y.to_string()));
    }
    let extra = if series {
        format_seasons(item.child_count)
    } else {
        format_runtime(item.run_time_ticks)
    };
    if !extra.is_empty() {
        parts.push(plain(&extra));
    }
    let rating = format_rating(item.official_rating.as_deref());
    if !rating.is_empty() {
        parts.push(MetaPart {
            text: rating,
            rating: true,
        });
    }
    parts
}

/// Handy-Genrezeile: bis zu 3 Genres mit „ • “.
pub fn genre_line(item: &BaseItem) -> String {
    item.genres
        .iter()
        .filter(|g| !g.is_empty())
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(" • ")
}

/// Badge des Billboards: `DateCreated` in den letzten 14 Tagen (mit Untergrenze 0, anders als die Zeilen).
pub fn badge_for(item: &BaseItem, now_ms: f64) -> Option<&'static str> {
    let t = crate::util::parse_opt(item.date_created.as_deref())?;
    (now_ms - t >= 0.0 && now_ms - t < NEW_DAYS * DAY_MS).then_some("Neu hinzugefügt")
}

/// Fortschritt vorhanden → „Fortsetzen“ statt „Abspielen“ (`hasProgress`).
pub fn has_progress(item: &BaseItem) -> bool {
    item.user_data
        .as_ref()
        .is_some_and(|u| u.playback_position_ticks > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dayseed::seeded_random;

    fn hero(id: &str, trailers: i64) -> HeroItem {
        let mut item = BaseItem {
            id: id.into(),
            ..Default::default()
        };
        item.image_tags.insert("Logo".into(), "l".into());
        item.backdrop_image_tags = vec!["b".into()];
        HeroItem {
            item,
            local_trailer_count: trailers,
            remote_trailer_urls: vec![],
        }
    }

    fn ids(p: &[HeroItem]) -> Vec<String> {
        p.iter().map(|i| i.item.id.clone()).collect()
    }

    #[test]
    fn pool_interleaves_random_first() {
        let latest: Vec<_> = (0..6).map(|i| hero(&format!("n{i}"), 0)).collect();
        let random: Vec<_> = (0..10).map(|i| hero(&format!("r{i}"), 0)).collect();
        let mut rng = seeded_random("t");
        let pool = build_pool(&latest, &random, &PoolOpts::home(None), &mut || rng.next());
        let id = ids(&pool);
        assert_eq!(id.len(), 4 + 8);
        assert!(id[0].starts_with('r') && id[1].starts_with('n'));
        assert!(
            id[8..].iter().all(|i| i.starts_with('r')),
            "Rest nur Zufall"
        );
    }

    #[test]
    fn pool_first_prefers_trailer_and_avoids_last() {
        let random = vec![hero("a", 0), hero("b", 1), hero("c", 0)];
        let mut rng = || 0.5;
        let p = build_pool::<HeroItem>(&[], &random, &PoolOpts::home(None), &mut rng);
        assert_eq!(p[0].item.id, "b");
        let p = build_pool::<HeroItem>(&[], &random, &PoolOpts::home(Some("b".into())), &mut rng);
        assert_eq!(p[0].item.id, "a");
        let only = vec![hero("b", 1)];
        let p = build_pool::<HeroItem>(&[], &only, &PoolOpts::home(Some("b".into())), &mut rng);
        assert_eq!(p.len(), 1, "keine Alternative: bleibt");
    }

    #[test]
    fn pool_skips_items_without_logo_or_backdrop() {
        let mut bad = hero("x", 0);
        bad.item.image_tags.clear();
        let p = build_pool::<HeroItem>(&[bad], &[], &PoolOpts::home(None), &mut || 0.0);
        assert!(p.is_empty());
    }

    #[test]
    fn saver_pool_keeps_current() {
        let pool: Vec<_> = (0..9).map(|i| hero(&format!("i{i}"), 0)).collect();
        assert_eq!(saver_pool(&pool, None).len(), 5);
        let kept = saver_pool(&pool, Some("i7"));
        assert_eq!(kept[4].item.id, "i7");
        assert_eq!(saver_pool(&pool[..3], None).len(), 3);
    }

    #[test]
    fn remove_adjusts_index() {
        let pool: Vec<_> = (0..4).map(|i| hero(&format!("i{i}"), 0)).collect();
        let (p, idx, gone) = remove_from_pool(&pool, 2, &["i0".into()]);
        assert_eq!((p.len(), idx, gone), (3, 1, false));
        let (_, idx, gone) = remove_from_pool(&pool, 3, &["i3".into()]);
        assert_eq!((idx, gone), (2, true));
        let (p, _, _) = remove_from_pool(&pool, 0, &ids(&pool));
        assert!(p.is_empty());
    }

    #[test]
    fn formats_in_german() {
        assert_eq!(format_runtime(Some(106 * 600_000_000)), "1 Std. 46 Min.");
        assert_eq!(format_runtime(Some(120 * 600_000_000)), "2 Std.");
        assert_eq!(format_runtime(Some(45 * 600_000_000)), "45 Min.");
        assert_eq!(format_runtime(Some(0)), "");
        assert_eq!(format_runtime(None), "");
        assert_eq!(format_rating(Some("FSK-12")), "12");
        assert_eq!(format_rating(Some("de 16")), "16");
        assert_eq!(format_rating(Some("FSK 6")), "6");
        assert_eq!(format_rating(Some("PG-13")), "PG-13");
        assert_eq!(format_rating(Some("FSK-123")), "FSK-123");
        assert_eq!(format_rating(Some("  ")), "");
        assert_eq!(format_seasons(Some(1)), "1 Staffel");
        assert_eq!(format_seasons(Some(3)), "3 Staffeln");
        assert_eq!(format_seasons(Some(0)), "");
    }

    #[test]
    fn meta_line() {
        let mut m = BaseItem {
            kind: ItemKind::Movie,
            genres: vec!["Drama".into(), "Krimi".into()],
            production_year: Some(2011),
            run_time_ticks: Some(118 * 600_000_000),
            official_rating: Some("FSK 12".into()),
            ..Default::default()
        };
        let t: Vec<String> = meta_parts(&m).iter().map(|p| p.text.clone()).collect();
        assert_eq!(t, ["Film", "Drama", "2011", "1 Std. 58 Min.", "12"]);
        assert!(meta_parts(&m).last().unwrap().rating);
        m.kind = ItemKind::Series;
        m.child_count = Some(2);
        assert_eq!(meta_parts(&m)[3].text, "2 Staffeln");
        m.genres = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        assert_eq!(genre_line(&m), "A • B • C");
    }

    #[test]
    fn badge_needs_past_date() {
        let now = 1_795_217_400_000.0;
        let mut m = BaseItem {
            date_created: Some("2026-11-15T00:00:00Z".into()),
            ..Default::default()
        };
        assert_eq!(badge_for(&m, now), Some("Neu hinzugefügt"));
        m.date_created = Some("2026-12-15T00:00:00Z".into());
        assert_eq!(
            badge_for(&m, now),
            None,
            "Zukunft: kein Badge (anders als bei Zeilen)"
        );
    }

    #[test]
    fn trailer_rules() {
        let mut h = hero("a", 1);
        h.remote_trailer_urls = vec![
            "https://video.example/watch?v=1".into(),
            "http://x.example/a.mp4".into(),
            "https://cdn.example/clip.MP4?x=1".into(),
        ];
        let ok = TrailerPolicy::default();
        assert_eq!(choose_trailer(&h, false, &ok), TrailerPlan::Local);
        let saver = TrailerPolicy {
            datasaver_trailer: true,
            ..Default::default()
        };
        assert_eq!(choose_trailer(&h, false, &saver), TrailerPlan::None);
        assert_eq!(
            choose_trailer(&h, true, &saver),
            TrailerPlan::Local,
            "Knopf bleibt erlaubt"
        );
        h.local_trailer_count = 0;
        assert_eq!(
            choose_trailer(&h, false, &ok),
            TrailerPlan::None,
            "Remote nie automatisch"
        );
        assert_eq!(
            choose_trailer(&h, true, &ok),
            TrailerPlan::RemoteDirect("https://cdn.example/clip.MP4?x=1".into())
        );
        assert!(direct_remote_trailer_url(&["https://a.example/x.mp4s".into()]).is_none());
        let (p, q) = local_trailer_stream("t1");
        assert_eq!(
            (p.as_str(), q[0].0.as_str()),
            ("/Videos/t1/stream.mp4", "static")
        );
    }

    #[test]
    fn pause_and_wait_rules() {
        assert!(!rotation_paused(&PauseInputs::default()));
        assert!(rotation_paused(&PauseInputs {
            modal_open: true,
            ..Default::default()
        }));
        assert!(
            rotation_enabled(2, false) && !rotation_enabled(1, false) && !rotation_enabled(5, true)
        );
        assert_eq!(next_index(3, 4), 0);
        assert_eq!(trailer_reveal_wait_ms(1_000, 2_000), 2_000);
        assert_eq!(trailer_reveal_wait_ms(1_000, 9_000), 0);
        let p = TrailerPolicy::default();
        assert!(trailer_pending(true, false, false, &p, 1_000));
        assert!(!trailer_pending(true, false, false, &p, 26_000));
    }

    #[test]
    fn pool_calls_match_web() {
        let (latest, random) = home_pool_calls();
        assert_eq!(
            (latest.param("SortBy"), latest.param("SortOrder")),
            (Some("DateCreated"), Some("Descending"))
        );
        assert_eq!(random.param("SortBy"), Some("Random"));
        assert_eq!(latest.param("Limit"), Some("12"));
        assert_eq!(latest.param("ImageTypes"), Some("Logo,Backdrop"));
        assert_eq!(
            latest.param("EnableImageTypes"),
            Some("Logo,Backdrop,Primary")
        );
        assert!(
            latest
                .param("Fields")
                .unwrap()
                .ends_with("LocalTrailerCount")
        );
        let (l, r) = library_pool_calls(ItemKind::Series, Some("g1"));
        assert_eq!(
            (l.param("Limit"), r.param("Limit")),
            (Some("30"), Some("40"))
        );
        assert_eq!(l.param("GenreIds"), Some("g1"));
        assert!(
            l.param("Fields")
                .unwrap()
                .ends_with("RemoteTrailers,LocalTrailerCount")
        );
        assert_eq!(l.param("IncludeItemTypes"), Some("Series"));
    }
}
