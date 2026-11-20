//! Zeilen der Startseite und der Bibliotheksseiten: Definitionen, Aufbau und Laden
//! (Port von `rows/sources.js`, `rows/logic.js`, `library/sources.js`).

pub mod home;
pub mod logic;

use crate::call::ApiCall;
use lfx_api::{ApiError, BaseItem, Client, ItemKind, ItemsQuery};
use std::collections::HashSet;

/// Kartenart der Zeile (`kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Querformat (mit Poster-Rückfall am Handy).
    Landscape,
    /// Weiterschauen (Fortschrittsbalken).
    Resume,
    /// Top 10 (Rangzahl).
    Top10,
}

/// Wie eine Zeile ihre Titel bekommt.
#[derive(Debug, Clone)]
pub enum RowSource {
    /// Eine Abfrage; `min_items` (Genre-/Jahrzehnt-Zeilen: 8) leert die Zeile bei zu wenigen Treffern,
    /// `type_filter` behält nur eine Art (Bibliotheksseiten).
    Query {
        /// Abfrage.
        call: ApiCall,
        /// Mindestzahl Treffer, sonst leer (0 = keine Grenze).
        min_items: usize,
        /// Nur diese Art behalten.
        type_filter: Option<ItemKind>,
    },
    /// Schon aufgelöste Titel (Charts).
    Fixed(Vec<BaseItem>),
    /// Neueste Filme/Serien als Top 10 (Rückfall ohne Charts): Abfrage, dann [`logic::select_top10`].
    TopNew {
        /// Abfrage (neueste nach `DateCreated`).
        call: ApiCall,
    },
    /// „Weil du … angesehen hast“: zuletzt Gesehenes, dann `Similar` dazu.
    Because {
        /// Abfrage nach dem zuletzt Gesehenen (`Limit 1`).
        played: ApiCall,
        /// Anzahl Ähnlicher.
        similar_limit: u32,
        /// Nur diese Art behalten.
        type_filter: Option<ItemKind>,
    },
    /// Weiterschauen innerhalb eines Genres (Bibliotheksseite): Art-Filter, dann clientseitiger Genre-Filter.
    ResumeGenre {
        /// Weiterschauen-Abfrage.
        call: ApiCall,
        /// Erwartete Art (`Movie` bzw. `Episode`).
        type_filter: ItemKind,
        /// Genre-Id.
        genre_id: String,
        /// Genre-Name.
        genre_name: String,
    },
}

/// Zeilen-Definition (`{id, title, kind, load}`).
#[derive(Debug, Clone)]
pub struct RowDef {
    /// Eindeutige Id (Bibliotheksseiten: Präfix `lib:<kind>:<genre|all>:`).
    pub id: String,
    /// Titel; `None` setzt erst das Laden („Weil du …“).
    pub title: Option<String>,
    /// Kartenart.
    pub kind: RowKind,
    /// Quelle.
    pub source: RowSource,
    /// Ergebnis nicht 10 min cachen (Ersatzzeile bei fehlenden Charts, damit sie sich erholt).
    pub no_cache: bool,
}

/// Ergebnis einer Zeile.
#[derive(Debug, Clone)]
pub struct RowResult {
    /// Titel (nur Elemente mit Id).
    pub items: Vec<BaseItem>,
    /// Zur Laufzeit ermittelter Titel (überschreibt [`RowDef::title`]).
    pub title: Option<String>,
}

impl RowResult {
    fn items(items: Vec<BaseItem>) -> Self {
        Self { items, title: None }
    }
}

/// Zeilen ohne Titel entfallen (`loadRow`: leer = Zeile entfernen).
pub fn is_empty(result: &RowResult) -> bool {
    result.items.is_empty()
}

/// `Items`-Abfrage mit Basis-Parametern, Art und `Recursive` (die `items(params)`-Hilfe der Web-Quellen).
pub(crate) fn items_call(
    types: &[ItemKind],
    extra: impl FnOnce(ItemsQuery) -> ItemsQuery,
) -> ApiCall {
    ApiCall::new(
        crate::Endpoint::Items,
        extra(logic::base_query().include_types(types).recursive(true)),
    )
}

fn normalize_id(id: &str) -> String {
    id.replace('-', "").to_lowercase()
}

/// Liegt das Item (direkt) im Genre? `None` = unbekannt (keine Genre-Angaben am Item).
/// Hinweis: `GenreItems` (Ids) kennt das DTO nicht, es gilt der Name.
pub fn item_in_genre(item: &BaseItem, genre_name: &str) -> Option<bool> {
    if item.genres.is_empty() {
        None
    } else {
        Some(item.genres.iter().any(|g| g == genre_name))
    }
}

/// Zeile laden (ohne Cache): führt die Abfragen aus und wendet die Filterregeln der Web-Quellen an.
pub async fn load_row(client: &Client, def: &RowDef) -> Result<RowResult, ApiError> {
    let mut result = match &def.source {
        RowSource::Query {
            call,
            min_items,
            type_filter,
        } => {
            let mut items = call.execute(client).await?;
            if let Some(t) = type_filter {
                items.retain(|i| i.kind == *t);
            }
            if items.len() < *min_items {
                items.clear();
            }
            RowResult::items(items)
        }
        RowSource::Fixed(items) => RowResult::items(items.clone()),
        RowSource::TopNew { call } => {
            RowResult::items(logic::select_top10(&call.execute(client).await?))
        }
        RowSource::Because {
            played,
            similar_limit,
            type_filter,
        } => because(client, played, *similar_limit, *type_filter).await?,
        RowSource::ResumeGenre {
            call,
            type_filter,
            genre_id,
            genre_name,
        } => {
            let mut list = call.execute(client).await?;
            list.retain(|i| i.kind == *type_filter);
            RowResult::items(filter_by_genre(client, list, genre_id, genre_name).await)
        }
    };
    result.items.retain(|i| !i.id.is_empty());
    Ok(result)
}

async fn because(
    client: &Client,
    played: &ApiCall,
    limit: u32,
    type_filter: Option<ItemKind>,
) -> Result<RowResult, ApiError> {
    let list = played.execute(client).await?;
    let Some(last) = list.first() else {
        return Ok(RowResult::items(Vec::new()));
    };
    let (base_id, name) = if last.kind == ItemKind::Episode {
        (last.series_id.clone(), last.series_name.clone())
    } else {
        (Some(last.id.clone()), Some(last.name.clone()))
    };
    let (Some(base_id), Some(name)) = (
        base_id.filter(|s| !s.is_empty()),
        name.filter(|s| !s.is_empty()),
    ) else {
        return Ok(RowResult::items(Vec::new()));
    };
    let call = ApiCall::new(
        crate::Endpoint::Similar(base_id.clone()),
        logic::base_query().limit(limit),
    );
    let mut similar = call.execute(client).await?;
    similar.retain(|i| i.id != base_id && type_filter.is_none_or(|t| i.kind == t));
    let title = (!similar.is_empty()).then(|| format!("Weil du {name} angesehen hast"));
    Ok(RowResult {
        items: similar,
        title,
    })
}

/// Weiterschauen im Genre: Filme clientseitig; Folgen ohne eigene Genres über die Serie (eine Zusatzabfrage).
/// Die Reihenfolge der Eingabe bleibt erhalten; ein Fehler der Zusatzabfrage lässt die unklaren Titel weg.
async fn filter_by_genre(
    client: &Client,
    list: Vec<BaseItem>,
    genre_id: &str,
    genre_name: &str,
) -> Vec<BaseItem> {
    let (mut keep, unknown) = split_by_genre(&list, genre_name);
    if !unknown.is_empty() {
        let base = |i: &BaseItem| {
            if i.kind == ItemKind::Episode {
                i.series_id.clone().unwrap_or_default()
            } else {
                i.id.clone()
            }
        };
        let mut ids: Vec<String> = Vec::new();
        for i in &unknown {
            let b = base(i);
            if !b.is_empty() && !ids.contains(&b) {
                ids.push(b);
            }
        }
        if !ids.is_empty() {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let call = ApiCall::new(
                crate::Endpoint::Items,
                ItemsQuery::new()
                    .ids(&refs)
                    .raw("genreIds", genre_id)
                    .enable_images(false)
                    .enable_user_data(false),
            );
            if let Ok(hits) = call.execute(client).await {
                let hit: HashSet<String> = hits.iter().map(|h| normalize_id(&h.id)).collect();
                keep.extend(
                    unknown
                        .into_iter()
                        .filter(|i| hit.contains(&normalize_id(&base(i)))),
                );
            }
        }
    }
    let pos = |i: &BaseItem| list.iter().position(|x| x.id == i.id).unwrap_or(usize::MAX);
    keep.sort_by_key(pos);
    keep
}

/// Teilt in „sicher im Genre“ und „unbekannt“ (kein Genre am Item); fremde Genres fallen weg.
pub fn split_by_genre(list: &[BaseItem], genre_name: &str) -> (Vec<BaseItem>, Vec<BaseItem>) {
    let mut keep = Vec::new();
    let mut unknown = Vec::new();
    for it in list {
        match item_in_genre(it, genre_name) {
            Some(true) => keep.push(it.clone()),
            None => unknown.push(it.clone()),
            Some(false) => {}
        }
    }
    (keep, unknown)
}

/// Doppelte Titel über mehrere Zeilen entfernen: ein Titel bleibt nur in der **ersten** Zeile, die ihn
/// enthält; Zeilen, die dadurch leer werden, bleiben leer. **Desktop-Erweiterung** – das Web dedupliziert
/// nur innerhalb einer Zeile und im Billboard-Pool. Weiterschauen/Top 10 sind ausgenommen (`keep_kinds`).
pub fn dedupe_across_rows(rows: &mut [(RowKind, Vec<BaseItem>)], exempt: &[RowKind]) {
    let mut seen: HashSet<String> = HashSet::new();
    for (kind, items) in rows.iter_mut() {
        if exempt.contains(kind) {
            continue;
        }
        items.retain(|i| seen.insert(i.id.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(id: &str, genres: &[&str]) -> BaseItem {
        BaseItem {
            id: id.into(),
            genres: genres.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn genre_split() {
        let l = [it("a", &["Drama"]), it("b", &["Krimi"]), it("c", &[])];
        let (k, u) = split_by_genre(&l, "Drama");
        assert_eq!((k.len(), u.len()), (1, 1));
        assert_eq!((k[0].id.as_str(), u[0].id.as_str()), ("a", "c"));
    }

    #[test]
    fn dedupe_keeps_first_and_exempts() {
        let mut rows = vec![
            (RowKind::Resume, vec![it("a", &[])]),
            (RowKind::Landscape, vec![it("a", &[]), it("b", &[])]),
            (RowKind::Landscape, vec![it("b", &[]), it("c", &[])]),
        ];
        dedupe_across_rows(&mut rows, &[RowKind::Resume, RowKind::Top10]);
        assert_eq!(rows[0].1.len(), 1);
        assert_eq!(
            rows[1].1.len(),
            2,
            "a stammt nur aus der ausgenommenen Zeile"
        );
        assert_eq!(
            rows[2].1.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["c"]
        );
    }
}
