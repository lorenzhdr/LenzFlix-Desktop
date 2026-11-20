//! W2-detail: reine Logik für Hovercard, Detail-Modal, Detailseite und Kontextmenü (ohne Oberfläche, testbar).
//!
//! Ports aus der Web-Vorlage: `features/hovercard/meta.js` (`computePopupRect`, `formatDuration`, `qualityBadge`,
//! `remainingText`, `episodeLabel`), `features/modal/format.js` (`ratingText`, `peopleOf`, `pickInitialSeason`)
//! und die Aktionen von `core/itemmenu.js` (hier als eigenes Menü, weil die Vorlage Jellyfins Menü per Proxy öffnet).

use lfx_api::{BaseItem, ItemKind, MediaSource, MediaStream, Person};
use lfx_core::billboard::{format_runtime, format_seasons};
use lfx_nav::{Rect, Zone, ZoneId};

/// Rechteck in logischen Pixeln.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Zielrechteck der Hovercard: Breite, linke obere Ecke und Höhe des 16:9-Bildteils.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub image_h: f32,
}

/// Hovercard-Maße (Web: `SCALE`, `MIN_WIDTH`).
pub const HC_SCALE: f32 = 1.5;
pub const HC_MIN_WIDTH: f32 = 300.0;
pub const HC_PAD: f32 = 8.0;
/// Ungefähre Höhe des Infoteils unter dem Bild (für die senkrechte Klemmung am Fensterrand).
pub const HC_INFO_H: f32 = 150.0;
/// Hover-Verzögerung, Schließ-Gnadenfrist und Ausblenddauer (Web: `DEFAULT_DELAY`, `CLOSE_GRACE`, `CLOSE_ANIM`).
pub const HC_DELAY_MS: u64 = 500;
pub const HC_CLOSE_GRACE_MS: u64 = 150;
pub const HC_CLOSE_ANIM_MS: u64 = 200;

/// Popup-Rechteck aus dem Kartenrechteck (`computePopupRect`): Breite = max(Karte × Skala, Mindestbreite), das
/// Bild sitzt vertikal mittig auf der Karte; horizontal zentriert, am Rand bündig mit der Karte, zuletzt hart geklemmt.
pub fn popup_rect(card: RectF, viewport_w: f32, viewport_h: f32) -> PopupRect {
    let width = (card.w * HC_SCALE)
        .max(HC_MIN_WIDTH)
        .min((viewport_w - 2.0 * HC_PAD).max(0.0));
    let image_h = width * 9.0 / 16.0;
    let mut left = card.x + card.w / 2.0 - width / 2.0;
    if left < HC_PAD {
        left = card.x;
    } else if left + width > viewport_w - HC_PAD {
        left = card.x + card.w - width;
    }
    left = left.min(viewport_w - HC_PAD - width).max(HC_PAD);
    let mut top = card.y + card.h / 2.0 - image_h / 2.0;
    if viewport_h > 0.0 {
        top = top
            .min(viewport_h - HC_PAD - image_h - HC_INFO_H)
            .max(HC_PAD);
    }
    PopupRect {
        left,
        top,
        width,
        image_h,
    }
}

const TICKS_PER_MIN: f64 = 600_000_000.0;

/// „1 Std. 46 Min.“ / „2 Std.“ / „46 Min.“; 0 = leer.
pub fn format_minutes(total_min: f64) -> String {
    let min = total_min.round() as i64;
    if min <= 0 {
        return String::new();
    }
    let (h, m) = (min / 60, min % 60);
    match (h, m) {
        (0, m) => format!("{m} Min."),
        (h, 0) => format!("{h} Std."),
        (h, m) => format!("{h} Std. {m} Min."),
    }
}

/// Laufzeit bzw. Umfang: Film „1 Std. 46 Min.“, Serie „N Staffeln“ (Miniserie: „N Folgen“), Staffel „N Folgen“,
/// Sammlung „N Titel“ (`formatDuration`).
pub fn duration_text(item: &BaseItem) -> String {
    let count = item.child_count.unwrap_or(0);
    let episodes = item.recursive_item_count.unwrap_or(0);
    let folgen = |n: i64| format!("{n} {}", if n == 1 { "Folge" } else { "Folgen" });
    match item.kind {
        ItemKind::Series => {
            if count == 1 && episodes > 0 {
                folgen(episodes)
            } else if count > 0 {
                format_seasons(Some(count))
            } else if episodes > 0 {
                folgen(episodes)
            } else {
                String::new()
            }
        }
        ItemKind::Season => {
            if count > 0 {
                folgen(count)
            } else {
                String::new()
            }
        }
        ItemKind::BoxSet => {
            if count > 0 {
                format!("{count} Titel")
            } else {
                String::new()
            }
        }
        _ => format_runtime(item.run_time_ticks),
    }
}

/// Breite des Videos aus dem Element oder dem ersten Videostream.
fn video_width(item: &BaseItem) -> i32 {
    if let Some(w) = item.width.filter(|w| *w > 0) {
        return w;
    }
    item.media_sources
        .iter()
        .flat_map(|s| s.media_streams.iter())
        .chain(item.media_streams.iter())
        .find(|s| s.kind == "Video" && s.width.unwrap_or(0) > 0)
        .and_then(|s| s.width)
        .unwrap_or(0)
}

/// „4K“ / „HD“ oder leer. Die Hovercard nimmt 3800 als Grenze für 4K, das Modal 3000.
pub fn quality_label(item: &BaseItem, four_k_from: i32) -> &'static str {
    let w = video_width(item);
    if w >= four_k_from {
        "4K"
    } else if w >= 1200 {
        "HD"
    } else {
        ""
    }
}

/// Lesbarer Text zur Altersfreigabe („FSK-12“ → „Freigegeben ab 12 Jahren“).
pub fn rating_text(rating: Option<&str>) -> String {
    let r = rating.unwrap_or("").trim();
    if r.is_empty() {
        return String::new();
    }
    let digits: String = r
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .take(2)
        .collect();
    if digits.is_empty() {
        let l = r.to_lowercase();
        return if ["0", "ohne", "alle", "g", "fsk-0", "fsk 0"].contains(&l.as_str()) {
            "Ohne Altersbeschränkung".into()
        } else {
            r.to_owned()
        };
    }
    match digits.parse::<u32>() {
        Ok(0) => "Ohne Altersbeschränkung".into(),
        Ok(n) => format!("Freigegeben ab {n} Jahren"),
        Err(_) => r.to_owned(),
    }
}

/// Fortschritt 0..100 aus den Benutzerdaten.
pub fn progress_percent(item: &BaseItem) -> f64 {
    let Some(ud) = &item.user_data else {
        return 0.0;
    };
    let rt = item.run_time_ticks.unwrap_or(0);
    if rt > 0 && ud.playback_position_ticks > 0 {
        return (ud.playback_position_ticks as f64 / rt as f64 * 100.0).clamp(0.0, 100.0);
    }
    match ud.played_percentage {
        Some(p) if p > 0.0 && p < 100.0 => p,
        _ => 0.0,
    }
}

/// „noch 38 Min.“ (aufgerundet) oder leer.
pub fn remaining_text(item: &BaseItem) -> String {
    let (Some(rt), Some(ud)) = (item.run_time_ticks.filter(|r| *r > 0), &item.user_data) else {
        return String::new();
    };
    if ud.playback_position_ticks <= 0 {
        return String::new();
    }
    let left = ((rt - ud.playback_position_ticks) as f64 / TICKS_PER_MIN)
        .ceil()
        .max(1.0);
    format!("noch {}", format_minutes(left))
}

/// Episodenzeile der Hovercard: F3 „Titel“.
pub fn episode_label(item: &BaseItem) -> String {
    let name = if item.name.is_empty() {
        String::new()
    } else {
        format!("„{}“", item.name)
    };
    let n = item
        .index_number
        .map(|n| format!("F{n}"))
        .unwrap_or_default();
    [n, name]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Personen einer Art („Actor“, „Director“, „Writer“), ohne Doppelte.
pub fn people_of<'a>(item: &'a BaseItem, kind: &str) -> Vec<&'a Person> {
    let mut seen = std::collections::HashSet::new();
    item.people
        .iter()
        .filter(|p| p.kind.as_deref() == Some(kind) && !p.name.is_empty())
        .filter(|p| seen.insert(p.name.clone()))
        .collect()
}

/// Staffel, die nach dem Öffnen vorgewählt wird (`pickInitialSeason`): die bevorzugte, sonst die erste reguläre.
pub fn initial_season(seasons: &[BaseItem], preferred: Option<&str>) -> Option<usize> {
    if seasons.is_empty() {
        return None;
    }
    if let Some(p) = preferred
        && let Some(i) = seasons.iter().position(|s| s.id == p)
    {
        return Some(i);
    }
    Some(
        seasons
            .iter()
            .position(|s| s.index_number.unwrap_or(0) > 0)
            .unwrap_or(0),
    )
}

/// Ob der Titel abspielbar ist (Knopf „Abspielen“).
pub fn is_playable(item: &BaseItem) -> bool {
    matches!(
        item.kind,
        ItemKind::Movie
            | ItemKind::Episode
            | ItemKind::Series
            | ItemKind::Season
            | ItemKind::Video
            | ItemKind::MusicVideo
            | ItemKind::Trailer
    )
}

/// Ob „Gesehen“ angeboten wird (nicht bei Sammlungen und Personen; Staffeln haben den Knopf in der Folgenliste).
pub fn can_mark_played(item: &BaseItem) -> bool {
    !matches!(
        item.kind,
        ItemKind::BoxSet | ItemKind::Person | ItemKind::Season | ItemKind::Unknown
    )
}

// ---------------------------------------------------------------- Spurwahl

/// Audio-, Untertitel- und Videostreams einer Quelle.
pub fn streams_of<'a>(src: &'a MediaSource, kind: &str) -> Vec<&'a MediaStream> {
    src.media_streams
        .iter()
        .filter(|s| s.kind == kind)
        .collect()
}

/// Anzeigetext eines Streams („Deutsch (AC3 5.1)“).
pub fn stream_label(s: &MediaStream) -> String {
    for t in [&s.display_title, &s.title] {
        if let Some(t) = t.as_deref().filter(|t| !t.is_empty()) {
            return t.to_owned();
        }
    }
    let lang = s.language.as_deref().unwrap_or("");
    let codec = s.codec.as_deref().unwrap_or("").to_uppercase();
    match (lang.is_empty(), codec.is_empty()) {
        (false, false) => format!("{lang} ({codec})"),
        (false, true) => lang.to_owned(),
        (true, false) => codec,
        _ => format!("Spur {}", s.index),
    }
}

/// Nächster Wert in einer geschlossenen Liste (nach dem letzten wieder der erste); unbekannter Wert → erster.
pub fn cycle_value(options: &[i32], current: i32) -> i32 {
    match options.iter().position(|o| *o == current) {
        Some(i) => options[(i + 1) % options.len()],
        None => options.first().copied().unwrap_or(current),
    }
}

// ---------------------------------------------------------------- Kontextmenü

/// Aktion eines Menüeintrags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Play,
    FromStart,
    ToggleFav,
    TogglePlayed,
    Download,
    Details,
    GoSeries,
    GoSeason,
}

/// Ein Eintrag des Kontextmenüs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub action: MenuAction,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}

fn entry(action: MenuAction, label: &str) -> MenuItem {
    MenuItem {
        action,
        label: label.to_owned(),
        enabled: true,
        checked: false,
    }
}

/// Einträge des Kontextmenüs eines Titels (nach `core/itemmenu.js`, ohne „Teilen“; „Herunterladen“ nur mit Recht).
pub fn menu_for(item: &BaseItem, can_download: bool) -> Vec<MenuItem> {
    let ud = item.user_data.clone().unwrap_or_default();
    let mut out = Vec::new();
    if is_playable(item) {
        out.push(entry(
            MenuAction::Play,
            if ud.playback_position_ticks > 0 {
                "Fortsetzen"
            } else {
                "Abspielen"
            },
        ));
        if ud.playback_position_ticks > 0
            && matches!(item.kind, ItemKind::Movie | ItemKind::Episode)
        {
            out.push(entry(MenuAction::FromStart, "Von Anfang an abspielen"));
        }
    }
    let mut fav = entry(
        MenuAction::ToggleFav,
        if ud.is_favorite {
            "Aus Meine Liste entfernen"
        } else {
            "Zu Meine Liste hinzufügen"
        },
    );
    fav.checked = ud.is_favorite;
    out.push(fav);
    if can_mark_played(item) {
        let mut p = entry(
            MenuAction::TogglePlayed,
            if ud.played {
                "Als ungesehen markieren"
            } else {
                "Als gesehen markieren"
            },
        );
        p.checked = ud.played;
        out.push(p);
    }
    if can_download
        && matches!(
            item.kind,
            ItemKind::Movie | ItemKind::Episode | ItemKind::Video
        )
    {
        out.push(entry(MenuAction::Download, "Herunterladen"));
    }
    if item.kind == ItemKind::Episode {
        if item.series_id.is_some() {
            out.push(entry(MenuAction::GoSeries, "Zur Serie"));
        }
        if item.season_id.is_some() {
            out.push(entry(MenuAction::GoSeason, "Zur Staffel"));
        }
    }
    out.push(entry(MenuAction::Details, "Details"));
    out
}

// ---------------------------------------------------------------- Fokuszonen

/// Zone je Abschnitt des Detailinhalts (Abschnitt `s` hat die Id `Z_DETAIL0 + s`).
pub const Z_DETAIL0: u32 = 100;
/// Abschnitte (müssen zu `DetailState.focus-sec` in `detail-state.slint` passen).
pub const SEC_ACT: u32 = 0;
pub const SEC_TRACKS: u32 = 1;
pub const SEC_SEASON: u32 = 2;
pub const SEC_EPS: u32 = 3;
pub const SEC_SIM: u32 = 4;
pub const SEC_PEOPLE: u32 = 5;
pub const SEC_LINK: u32 = 6;
pub const SECTION_COUNT: u32 = 7;

pub fn detail_zone(sec: u32) -> ZoneId {
    ZoneId(Z_DETAIL0 + sec)
}

/// Abschnitt einer Zone, falls es eine Detail-Zone ist.
pub fn zone_section(z: ZoneId) -> Option<u32> {
    z.0.checked_sub(Z_DETAIL0).filter(|s| *s < SECTION_COUNT)
}

/// Größen der Abschnitte für den Fokusgraphen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailCounts {
    pub actions: usize,
    pub tracks: usize,
    pub season: usize,
    pub episodes: usize,
    pub similar: usize,
    pub people: usize,
    pub link: usize,
    pub sim_cols: usize,
    pub people_cols: usize,
}

/// Zonen des Detailinhalts, senkrecht gestapelt (die Fokusbewegung ist geometrisch nach oben und unten).
pub fn detail_zones(c: &DetailCounts) -> Vec<Zone> {
    let rect = |sec: u32| Rect::new(0.0, sec as f32 * 400.0, 1000.0, 300.0);
    let mut z = vec![
        Zone::horizontal(detail_zone(SEC_ACT), rect(SEC_ACT), c.actions),
        Zone::horizontal(detail_zone(SEC_TRACKS), rect(SEC_TRACKS), c.tracks),
        Zone::horizontal(detail_zone(SEC_SEASON), rect(SEC_SEASON), c.season),
        Zone::vertical(detail_zone(SEC_EPS), rect(SEC_EPS), c.episodes).remembering(true),
        Zone::grid(
            detail_zone(SEC_SIM),
            rect(SEC_SIM),
            c.similar,
            c.sim_cols.max(1),
        ),
        Zone::grid(
            detail_zone(SEC_PEOPLE),
            rect(SEC_PEOPLE),
            c.people,
            c.people_cols.max(1),
        ),
        Zone::horizontal(detail_zone(SEC_LINK), rect(SEC_LINK), c.link),
    ];
    for zone in &mut z {
        zone.page_size = 5;
    }
    z
}

#[cfg(test)]
mod tests {
    use super::*;
    use lfx_api::UserItemData;

    fn item(kind: ItemKind) -> BaseItem {
        BaseItem {
            id: "x".into(),
            name: "Testfilm".into(),
            kind,
            ..BaseItem::default()
        }
    }

    #[test]
    fn popup_centers_on_card() {
        let r = popup_rect(
            RectF {
                x: 500.0,
                y: 400.0,
                w: 240.0,
                h: 135.0,
            },
            1920.0,
            1080.0,
        );
        // Breite = max(240 × 1,5; 300) = 360, mittig auf der Karte
        assert_eq!(r.width, 360.0);
        assert_eq!(r.left, 500.0 + 120.0 - 180.0);
        assert!((r.image_h - 202.5).abs() < 0.01);
        // Bild vertikal an der Kartenmitte zentriert
        assert!((r.top - (400.0 + 67.5 - 101.25)).abs() < 0.01);
    }

    #[test]
    fn popup_is_flush_with_card_at_edges() {
        let left = popup_rect(
            RectF {
                x: 20.0,
                y: 300.0,
                w: 240.0,
                h: 135.0,
            },
            1920.0,
            1080.0,
        );
        assert_eq!(left.left, 20.0);
        let right = popup_rect(
            RectF {
                x: 1700.0,
                y: 300.0,
                w: 200.0,
                h: 112.0,
            },
            1920.0,
            1080.0,
        );
        // rechte Kante bündig mit der Karte (1900), Breite 300
        assert_eq!(right.left + right.width, 1900.0);
    }

    #[test]
    fn popup_is_clamped_vertically_and_to_narrow_viewports() {
        let r = popup_rect(
            RectF {
                x: 10.0,
                y: 1000.0,
                w: 240.0,
                h: 135.0,
            },
            1920.0,
            1080.0,
        );
        assert!(r.top + r.image_h + HC_INFO_H <= 1080.0 - HC_PAD + 0.01);
        let narrow = popup_rect(
            RectF {
                x: 0.0,
                y: 100.0,
                w: 200.0,
                h: 112.0,
            },
            250.0,
            600.0,
        );
        assert_eq!(narrow.width, 234.0);
        assert!(narrow.left >= HC_PAD);
    }

    #[test]
    fn minutes_and_duration() {
        assert_eq!(format_minutes(106.0), "1 Std. 46 Min.");
        assert_eq!(format_minutes(120.0), "2 Std.");
        assert_eq!(format_minutes(46.0), "46 Min.");
        assert_eq!(format_minutes(0.0), "");
        let mut s = item(ItemKind::Series);
        s.child_count = Some(3);
        assert_eq!(duration_text(&s), "3 Staffeln");
        s.child_count = Some(1);
        s.recursive_item_count = Some(8);
        assert_eq!(duration_text(&s), "8 Folgen");
        let mut m = item(ItemKind::Movie);
        m.run_time_ticks = Some(106 * 600_000_000);
        assert_eq!(duration_text(&m), "1 Std. 46 Min.");
        let mut b = item(ItemKind::BoxSet);
        b.child_count = Some(4);
        assert_eq!(duration_text(&b), "4 Titel");
    }

    #[test]
    fn quality_thresholds_differ_between_hovercard_and_modal() {
        let mut m = item(ItemKind::Movie);
        m.width = Some(3200);
        assert_eq!(quality_label(&m, 3800), "HD");
        assert_eq!(quality_label(&m, 3000), "4K");
        m.width = Some(640);
        assert_eq!(quality_label(&m, 3000), "");
    }

    #[test]
    fn rating_texts() {
        assert_eq!(rating_text(Some("FSK-12")), "Freigegeben ab 12 Jahren");
        assert_eq!(rating_text(Some("FSK 0")), "Ohne Altersbeschränkung");
        assert_eq!(rating_text(Some("PG-13")), "Freigegeben ab 13 Jahren");
        assert_eq!(rating_text(Some("ohne")), "Ohne Altersbeschränkung");
        assert_eq!(rating_text(None), "");
    }

    #[test]
    fn progress_and_remaining() {
        let mut m = item(ItemKind::Movie);
        m.run_time_ticks = Some(100 * 600_000_000);
        m.user_data = Some(UserItemData {
            playback_position_ticks: 62 * 600_000_000,
            ..UserItemData::default()
        });
        assert!((progress_percent(&m) - 62.0).abs() < 0.01);
        assert_eq!(remaining_text(&m), "noch 38 Min.");
        m.user_data = None;
        assert_eq!(remaining_text(&m), "");
    }

    #[test]
    fn episode_label_has_number_and_name() {
        let mut e = item(ItemKind::Episode);
        e.name = "Die Brücke".into();
        e.index_number = Some(3);
        assert_eq!(episode_label(&e), "F3 „Die Brücke“");
    }

    #[test]
    fn season_choice() {
        let mk = |id: &str, n: i32| BaseItem {
            id: id.into(),
            index_number: Some(n),
            kind: ItemKind::Season,
            ..BaseItem::default()
        };
        let seasons = [mk("sp", 0), mk("s1", 1), mk("s2", 2)];
        assert_eq!(initial_season(&seasons, Some("s2")), Some(2));
        assert_eq!(initial_season(&seasons, Some("fremd")), Some(1));
        assert_eq!(initial_season(&seasons, None), Some(1));
        assert_eq!(initial_season(&[], None), None);
    }

    #[test]
    fn track_cycle_wraps() {
        assert_eq!(cycle_value(&[-1, 3, 4], -1), 3);
        assert_eq!(cycle_value(&[-1, 3, 4], 4), -1);
        assert_eq!(cycle_value(&[-1, 3, 4], 99), -1);
    }

    #[test]
    fn menu_depends_on_type_and_state() {
        let mut m = item(ItemKind::Movie);
        let plain = menu_for(&m, false);
        let actions: Vec<_> = plain.iter().map(|e| e.action).collect();
        assert_eq!(
            actions,
            vec![
                MenuAction::Play,
                MenuAction::ToggleFav,
                MenuAction::TogglePlayed,
                MenuAction::Details
            ]
        );
        m.user_data = Some(UserItemData {
            playback_position_ticks: 5,
            is_favorite: true,
            ..UserItemData::default()
        });
        let with_pos = menu_for(&m, true);
        assert!(with_pos.iter().any(|e| e.action == MenuAction::FromStart));
        assert!(with_pos.iter().any(|e| e.action == MenuAction::Download));
        assert_eq!(with_pos[0].label, "Fortsetzen");
        assert!(
            with_pos
                .iter()
                .any(|e| e.action == MenuAction::ToggleFav && e.checked)
        );
        let mut ep = item(ItemKind::Episode);
        ep.series_id = Some("s".into());
        ep.season_id = Some("t".into());
        let ep_menu = menu_for(&ep, false);
        assert!(ep_menu.iter().any(|e| e.action == MenuAction::GoSeries));
        assert!(ep_menu.iter().any(|e| e.action == MenuAction::GoSeason));
        assert!(!ep_menu.iter().any(|e| e.action == MenuAction::Download));
    }

    #[test]
    fn zones_cover_all_sections() {
        let z = detail_zones(&DetailCounts {
            actions: 5,
            similar: 6,
            sim_cols: 3,
            people: 4,
            people_cols: 4,
            ..DetailCounts::default()
        });
        assert_eq!(z.len(), SECTION_COUNT as usize);
        assert_eq!(zone_section(detail_zone(SEC_EPS)), Some(SEC_EPS));
        assert_eq!(zone_section(ZoneId(11)), None);
    }
}
