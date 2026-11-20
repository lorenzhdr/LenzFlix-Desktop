//! Trend-Badges aus den Charts (Port von `shared/trend.js`).
//!
//! Aus den Chart-Listen (Filme, Serien) entsteht ein Rang je Titel. Platz 1 bis 3 zeigt „AM BELIEBTESTEN“,
//! Platz 4 bis 10 „IM TREND“. Ohne Charts: Film oder Serie mit Bewertung ab 8 zeigt „AM BELIEBTESTEN“.

use lfx_api::{BaseItem, ItemKind};
use std::collections::HashMap;

/// Rang je normalisierter Id (ohne Bindestriche, klein), 1-basiert je Liste.
#[derive(Debug, Clone, Default)]
pub struct TrendInfo {
    /// Charts waren erreichbar.
    pub available: bool,
    rank: HashMap<String, usize>,
}

fn norm(id: &str) -> String {
    id.replace('-', "").to_ascii_lowercase()
}

impl TrendInfo {
    /// Aus den Id-Listen von Filmen und Serien (Reihenfolge = Rang). Der erste Eintrag einer Id gewinnt.
    pub fn from_lists(movies: &[String], series: &[String]) -> Self {
        let mut rank = HashMap::new();
        for list in [movies, series] {
            for (i, id) in list.iter().enumerate() {
                let key = norm(id);
                if !key.is_empty() {
                    rank.entry(key).or_insert(i + 1);
                }
            }
        }
        Self {
            available: true,
            rank,
        }
    }

    /// Rang eines Titels (Episoden zählen über ihre Serie).
    pub fn rank_of(&self, item: &BaseItem) -> Option<usize> {
        if !self.available {
            return None;
        }
        let id = if item.kind == ItemKind::Episode {
            item.series_id.as_deref()?
        } else {
            item.id.as_str()
        };
        self.rank.get(&norm(id)).copied()
    }
}

/// Badge-Text oder `None` (`trendBadge`).
pub fn trend_badge(item: &BaseItem, trend: &TrendInfo) -> Option<&'static str> {
    if !trend.available {
        let rated = item.community_rating.is_some_and(|r| r >= 8.0);
        return (matches!(item.kind, ItemKind::Movie | ItemKind::Series) && rated)
            .then_some("AM BELIEBTESTEN");
    }
    match trend.rank_of(item)? {
        1..=3 => Some("AM BELIEBTESTEN"),
        4..=10 => Some("IM TREND"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(id: &str) -> BaseItem {
        BaseItem {
            id: id.into(),
            kind: ItemKind::Movie,
            ..BaseItem::default()
        }
    }

    #[test]
    fn ranks_map_to_badges() {
        let ids: Vec<String> = (0..6).map(|i| format!("ab{i}")).collect();
        let t = TrendInfo::from_lists(&ids, &[]);
        assert_eq!(trend_badge(&movie("ab0"), &t), Some("AM BELIEBTESTEN"));
        assert_eq!(trend_badge(&movie("ab4"), &t), Some("IM TREND"));
        assert_eq!(trend_badge(&movie("zz"), &t), None);
    }

    #[test]
    fn without_charts_rating_decides() {
        let mut m = movie("x");
        m.community_rating = Some(8.4);
        assert_eq!(
            trend_badge(&m, &TrendInfo::default()),
            Some("AM BELIEBTESTEN")
        );
        m.community_rating = Some(7.0);
        assert_eq!(trend_badge(&m, &TrendInfo::default()), None);
    }
}
