//! Intro-/Abspann-Überspringen und „Nächste Folge“: reine Logik plus die nötigen API-Aufrufe.

use lfx_api::{ApiError, BaseItem, Client, TICKS_PER_SECOND};

/// Art eines Medien-Segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Intro,
    Outro,
    Recap,
    Preview,
    Commercial,
}

/// Segment in Sekunden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start: f64,
    pub end: f64,
}

/// Was beim „Überspringen“ passiert.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SkipAction {
    /// Zu dieser Position springen.
    SeekTo(f64),
    /// Das Segment reicht bis zum Ende: gleich die nächste Folge (bzw. Ende) wählen.
    NextItem,
}

fn kind_of(s: &str) -> Option<SegmentKind> {
    Some(match s {
        "Intro" => SegmentKind::Intro,
        "Outro" => SegmentKind::Outro,
        "Recap" => SegmentKind::Recap,
        "Preview" => SegmentKind::Preview,
        "Commercial" => SegmentKind::Commercial,
        _ => return None,
    })
}

/// Lädt die Segmente eines Elements (nach Beginn sortiert; unbekannte Arten und leere Segmente
/// entfallen).
pub async fn fetch_segments(client: &Client, item_id: &str) -> Result<Vec<Segment>, ApiError> {
    let res = client.media_segments(item_id).await?;
    let t = TICKS_PER_SECOND as f64;
    let mut v: Vec<Segment> = res
        .items
        .iter()
        .filter_map(|s| {
            let seg = Segment {
                kind: kind_of(&s.kind)?,
                start: s.start_ticks as f64 / t,
                end: s.end_ticks as f64 / t,
            };
            (seg.end > seg.start).then_some(seg)
        })
        .collect();
    v.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(v)
}

/// Segment, das bei `position` übersprungen werden kann, samt Aktion.
pub fn skippable_at(
    segments: &[Segment],
    position: f64,
    duration: Option<f64>,
) -> Option<(Segment, SkipAction)> {
    let seg = *segments
        .iter()
        .find(|s| position >= s.start && position < s.end)?;
    let reaches_end = duration.is_none_or(|d| seg.end >= d - 2.0);
    let action = if seg.kind == SegmentKind::Outro && reaches_end {
        SkipAction::NextItem
    } else {
        SkipAction::SeekTo(seg.end)
    };
    Some((seg, action))
}

/// Soll die „Nächste Folge“-Karte erscheinen? Mit Abspann-Segment ab dessen Beginn, sonst in
/// den letzten `fallback_secs` Sekunden.
pub fn should_show_up_next(
    segments: &[Segment],
    position: f64,
    duration: Option<f64>,
    fallback_secs: f64,
) -> bool {
    if let Some(outro) = segments.iter().rev().find(|s| s.kind == SegmentKind::Outro) {
        return position >= outro.start;
    }
    duration.is_some_and(|d| d > 0.0 && d - position <= fallback_secs)
}

fn order_key(e: &BaseItem) -> (i32, i32) {
    (
        e.parent_index_number.unwrap_or(i32::MAX),
        e.index_number.unwrap_or(i32::MAX),
    )
}

/// Folge nach `current_id` in Staffel-/Folgenreihenfolge. Specials (Staffel 0) zählen nur, wenn
/// die laufende Folge selbst eins ist.
pub fn pick_next_episode<'a>(episodes: &'a [BaseItem], current_id: &str) -> Option<&'a BaseItem> {
    let current = episodes.iter().find(|e| e.id == current_id)?;
    let special = current.parent_index_number == Some(0);
    let mut list: Vec<&BaseItem> = episodes
        .iter()
        .filter(|e| special || e.parent_index_number != Some(0))
        .collect();
    list.sort_by_key(|e| order_key(e));
    let pos = list.iter().position(|e| e.id == current_id)?;
    list.get(pos + 1).copied()
}

/// Nächste Folge der Serie zu `current` (Folge). `None` bei Filmen oder wenn es keine mehr gibt.
pub async fn next_episode(
    client: &Client,
    current: &BaseItem,
) -> Result<Option<BaseItem>, ApiError> {
    let Some(series) = current.series_id.as_deref() else {
        return Ok(None);
    };
    let all = client.episodes(series, None, &[]).await?;
    Ok(pick_next_episode(&all.items, &current.id).cloned())
}

/// Serverseitig „als Nächstes“ für eine Serie (Weiterschauen-Logik des Servers).
pub async fn next_up_for_series(
    client: &Client,
    series_id: &str,
) -> Result<Option<BaseItem>, ApiError> {
    Ok(client
        .next_up(Some(series_id), 1, &[])
        .await?
        .items
        .into_iter()
        .next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(kind: SegmentKind, start: f64, end: f64) -> Segment {
        Segment { kind, start, end }
    }

    fn ep(id: &str, season: i32, n: i32) -> BaseItem {
        BaseItem {
            id: id.into(),
            parent_index_number: Some(season),
            index_number: Some(n),
            ..Default::default()
        }
    }

    #[test]
    fn skip_intro_seeks_to_end() {
        let s = [seg(SegmentKind::Intro, 30.0, 90.0)];
        assert_eq!(skippable_at(&s, 29.0, Some(1400.0)), None);
        let (_, a) = skippable_at(&s, 30.0, Some(1400.0)).unwrap();
        assert_eq!(a, SkipAction::SeekTo(90.0));
        assert_eq!(skippable_at(&s, 90.0, Some(1400.0)), None);
    }

    #[test]
    fn outro_to_the_end_means_next_item() {
        let s = [seg(SegmentKind::Outro, 1300.0, 1400.0)];
        let (_, a) = skippable_at(&s, 1310.0, Some(1400.0)).unwrap();
        assert_eq!(a, SkipAction::NextItem);
        let s = [seg(SegmentKind::Outro, 1300.0, 1350.0)];
        let (_, a) = skippable_at(&s, 1310.0, Some(1400.0)).unwrap();
        assert_eq!(a, SkipAction::SeekTo(1350.0));
    }

    #[test]
    fn up_next_uses_outro_or_fallback() {
        let s = [seg(SegmentKind::Outro, 1300.0, 1400.0)];
        assert!(!should_show_up_next(&s, 1299.0, Some(1400.0), 30.0));
        assert!(should_show_up_next(&s, 1300.0, Some(1400.0), 30.0));
        assert!(!should_show_up_next(&[], 1300.0, Some(1400.0), 30.0));
        assert!(should_show_up_next(&[], 1371.0, Some(1400.0), 30.0));
        assert!(!should_show_up_next(&[], 10.0, None, 30.0));
    }

    #[test]
    fn next_episode_crosses_seasons_and_skips_specials() {
        let eps = [
            ep("s0e1", 0, 1),
            ep("s2e1", 2, 1),
            ep("s1e2", 1, 2),
            ep("s1e1", 1, 1),
        ];
        assert_eq!(pick_next_episode(&eps, "s1e1").unwrap().id, "s1e2");
        assert_eq!(pick_next_episode(&eps, "s1e2").unwrap().id, "s2e1");
        assert!(pick_next_episode(&eps, "s2e1").is_none());
        assert!(pick_next_episode(&eps, "gibts-nicht").is_none());
        // Aus einem Special heraus zählt die Specials-Reihenfolge.
        assert_eq!(pick_next_episode(&eps, "s0e1").unwrap().id, "s1e1");
    }
}
