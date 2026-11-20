//! Reine Zeilenlogik (Port von `rows/logic.js`): Neu-Erkennung, Badges, Top 10, Fortschritt.

use crate::util::{DAY_MS, parse_opt};
use lfx_api::{BaseItem, ItemKind, ItemsQuery};
use std::collections::HashSet;

/// Tage, in denen ein Titel „neu“ ist.
pub const NEW_DAYS: f64 = 14.0;
/// Erstausstrahlung, ab der eine Folge noch als „neu erschienen“ gilt.
pub const NEW_EPISODE_AIRED_DAYS: f64 = 365.0;

/// `ITEM_FIELDS` der Startseiten-Abfragen.
pub const ITEM_FIELDS: [&str; 20] = [
    "PrimaryImageAspectRatio",
    "Overview",
    "Genres",
    "DateCreated",
    "ParentId",
    "ImageTags",
    "BackdropImageTags",
    "ParentThumbItemId",
    "ParentThumbImageTag",
    "ParentBackdropItemId",
    "ParentBackdropImageTags",
    "SeriesId",
    "SeriesPrimaryImageTag",
    "ChildCount",
    "RecursiveItemCount",
    "OfficialRating",
    "ProductionYear",
    "RunTimeTicks",
    "ParentLogoItemId",
    "ParentLogoImageTag",
];

/// `BASE_PARAMS`: Felder, Nutzerdaten, ein Bild je Art, Bildarten Primary/Backdrop/Thumb/Logo.
pub fn base_query() -> ItemsQuery {
    ItemsQuery::new()
        .fields(&ITEM_FIELDS)
        .enable_user_data(true)
        .image_type_limit(1)
        .enable_image_types(&["Primary", "Backdrop", "Thumb", "Logo"])
}

/// `isRecent`: jünger als `days` Tage; **ohne** Untergrenze (Zukunft zählt als neu), ungültiges Datum = nein.
pub fn is_recent(date: Option<&str>, now_ms: f64, days: f64) -> bool {
    parse_opt(date).is_some_and(|t| now_ms - t < days * DAY_MS)
}

/// Serien-Ids für „Neue Folgen“: Folgen, die in den letzten 14 Tagen hinzukamen UND in den letzten
/// 365 Tagen erstausgestrahlt wurden. Folgen ohne `PremiereDate` zählen nicht.
pub fn new_series_ids_from(episodes: &[BaseItem], now_ms: f64) -> HashSet<String> {
    episodes
        .iter()
        .filter_map(|e| {
            let sid = e.series_id.as_deref().filter(|s| !s.is_empty())?;
            (is_recent(e.date_created.as_deref(), now_ms, NEW_DAYS)
                && is_recent(e.premiere_date.as_deref(), now_ms, NEW_EPISODE_AIRED_DAYS))
            .then(|| sid.to_owned())
        })
        .collect()
}

/// Abfrage für das Neue-Folgen-Set (10 min gecacht): Folgen nach `DateCreated` absteigend (Jellyfin 12.4
/// ignoriert `MinDateCreated`), Auswertung clientseitig.
pub fn new_series_query() -> crate::ApiCall {
    crate::ApiCall::new(
        crate::Endpoint::Items,
        ItemsQuery::new()
            .include_types(&[ItemKind::Episode])
            .recursive(true)
            .sort_by(&[lfx_api::SortBy::DateCreated])
            .order(lfx_api::SortOrder::Descending)
            .fields(&["SeriesId", "DateCreated", "PremiereDate"])
            .enable_images(false)
            .enable_user_data(false)
            .limit(200),
    )
}

/// Badge-Text einer Karte: Film/Serie „Neu hinzugefügt“, Serie mit neuen Folgen „Neue Folgen“.
pub fn badge_for(
    item: &BaseItem,
    new_series: Option<&HashSet<String>>,
    now_ms: f64,
) -> Option<&'static str> {
    let recent = || is_recent(item.date_created.as_deref(), now_ms, NEW_DAYS);
    match item.kind {
        ItemKind::Movie => recent().then_some("Neu hinzugefügt"),
        ItemKind::Series => {
            if recent() {
                Some("Neu hinzugefügt")
            } else if new_series.is_some_and(|s| s.contains(&item.id)) {
                Some("Neue Folgen")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Die 10 zuletzt hinzugefügten Filme/Serien, neueste zuerst (dedupliziert; Gleichstand = Eingangsreihenfolge,
/// Titel ohne gültiges Datum zuletzt).
pub fn select_top10(items: &[BaseItem]) -> Vec<BaseItem> {
    let mut seen = HashSet::new();
    let mut list: Vec<(usize, f64, &BaseItem)> = Vec::new();
    for it in items {
        if it.id.is_empty()
            || !matches!(it.kind, ItemKind::Movie | ItemKind::Series)
            || !seen.insert(it.id.as_str())
        {
            continue;
        }
        let ts = parse_opt(it.date_created.as_deref()).unwrap_or(f64::NEG_INFINITY);
        list.push((list.len(), ts, it));
    }
    list.sort_by(|a, b| {
        if a.1 == b.1 {
            a.0.cmp(&b.0)
        } else {
            b.1.total_cmp(&a.1)
        }
    });
    list.into_iter().take(10).map(|x| x.2.clone()).collect()
}

/// Abspielfortschritt 0–100 oder 0.
pub fn progress_percent(item: &BaseItem) -> f64 {
    let Some(ud) = &item.user_data else {
        return 0.0;
    };
    let mut p = ud.played_percentage.unwrap_or(f64::NAN);
    if (p.is_nan() || p <= 0.0)
        && ud.playback_position_ticks != 0
        && let Some(rt) = item.run_time_ticks.filter(|r| *r != 0)
    {
        p = ud.playback_position_ticks as f64 / rt as f64 * 100.0;
    }
    if p > 0.0 { p.clamp(0.0, 100.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lfx_api::UserItemData;

    fn item(id: &str, kind: ItemKind, created: &str) -> BaseItem {
        BaseItem {
            id: id.into(),
            kind,
            date_created: Some(created.into()),
            ..Default::default()
        }
    }
    const NOW: f64 = 1_795_217_400_000.0; // 2026-11-20T23:30:00Z

    #[test]
    fn recent_has_no_lower_bound() {
        assert!(is_recent(Some("2026-11-10T00:00:00Z"), NOW, 14.0));
        assert!(!is_recent(Some("2026-10-01T00:00:00Z"), NOW, 14.0));
        assert!(is_recent(Some("2027-01-01T00:00:00Z"), NOW, 14.0));
        assert!(!is_recent(Some("x"), NOW, 14.0));
        assert!(!is_recent(None, NOW, 14.0));
    }

    #[test]
    fn new_series_need_added_and_aired() {
        let mut e = item("e1", ItemKind::Episode, "2026-11-15T00:00:00Z");
        e.series_id = Some("s1".into());
        e.premiere_date = Some("2026-11-01T00:00:00Z".into());
        let mut old = e.clone();
        old.series_id = Some("s2".into());
        old.premiere_date = Some("2020-01-01T00:00:00Z".into());
        let mut nodate = e.clone();
        nodate.series_id = Some("s3".into());
        nodate.premiere_date = None;
        let set = new_series_ids_from(&[e, old, nodate], NOW);
        assert_eq!(set, HashSet::from(["s1".to_owned()]));
    }

    #[test]
    fn badges() {
        let set = HashSet::from(["s".to_owned()]);
        let m = item("m", ItemKind::Movie, "2026-11-19T00:00:00Z");
        assert_eq!(badge_for(&m, None, NOW), Some("Neu hinzugefügt"));
        let old = item("s", ItemKind::Series, "2020-01-01T00:00:00Z");
        assert_eq!(badge_for(&old, Some(&set), NOW), Some("Neue Folgen"));
        assert_eq!(badge_for(&old, None, NOW), None);
        assert_eq!(
            badge_for(
                &item("e", ItemKind::Episode, "2026-11-19T00:00:00Z"),
                None,
                NOW
            ),
            None
        );
    }

    #[test]
    fn top10_sorts_dedupes_limits() {
        let mut v: Vec<BaseItem> = (0..14)
            .map(|i| {
                item(
                    &format!("m{i}"),
                    ItemKind::Movie,
                    &format!("2026-03-{:02}T10:00:00Z", i + 1),
                )
            })
            .collect();
        v.push(item("m3", ItemKind::Movie, "2026-03-30T00:00:00Z")); // doppelt
        v.push(item("x", ItemKind::Episode, "2026-04-01T00:00:00Z")); // falsche Art
        let t = select_top10(&v);
        assert_eq!(t.len(), 10);
        assert_eq!(t[0].id, "m13");
        assert_eq!(t[9].id, "m4");
        let undated = [
            item("a", ItemKind::Movie, "kaputt"),
            item("b", ItemKind::Movie, "2020-01-01T00:00:00Z"),
        ];
        assert_eq!(select_top10(&undated)[0].id, "b");
    }

    #[test]
    fn progress() {
        let mut i = BaseItem {
            run_time_ticks: Some(1000),
            user_data: Some(UserItemData {
                playback_position_ticks: 250,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(progress_percent(&i), 25.0);
        i.user_data.as_mut().unwrap().played_percentage = Some(130.0);
        assert_eq!(progress_percent(&i), 100.0);
        assert_eq!(progress_percent(&BaseItem::default()), 0.0);
    }

    #[test]
    fn base_query_matches_web() {
        let q = crate::ApiCall::new(crate::Endpoint::Items, base_query());
        assert_eq!(q.param("EnableUserData"), Some("true"));
        assert_eq!(q.param("ImageTypeLimit"), Some("1"));
        assert_eq!(
            q.param("EnableImageTypes"),
            Some("Primary,Backdrop,Thumb,Logo")
        );
        assert!(
            q.param("Fields")
                .unwrap()
                .starts_with("PrimaryImageAspectRatio,Overview,Genres")
        );
        assert!(q.param("Fields").unwrap().ends_with("ParentLogoImageTag"));
    }
}
