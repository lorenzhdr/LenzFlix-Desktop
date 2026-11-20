//! Query-Builder für `/Items` (Parameternamen nach der OpenAPI-Spec).

use crate::dto::ItemKind;

/// Filter (`Filters=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemFilter {
    /// Gesehen.
    IsPlayed,
    /// Ungesehen.
    IsUnplayed,
    /// Favorit.
    IsFavorite,
    /// Nicht als Favorit.
    IsNotFavorite,
    /// Angefangen (Weiterschauen).
    IsResumable,
    /// Gefällt.
    Likes,
    /// Gefällt nicht.
    Dislikes,
}

impl ItemFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::IsPlayed => "IsPlayed",
            Self::IsUnplayed => "IsUnplayed",
            Self::IsFavorite => "IsFavorite",
            Self::IsNotFavorite => "IsNotFavorite",
            Self::IsResumable => "IsResumable",
            Self::Likes => "Likes",
            Self::Dislikes => "Dislikes",
        }
    }
}

/// Sortierschlüssel (`SortBy=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    /// Name (Sortiername).
    SortName,
    /// Hinzugefügt am.
    DateCreated,
    /// Zuletzt gespielt.
    DatePlayed,
    /// Premierendatum.
    PremiereDate,
    /// Produktionsjahr.
    ProductionYear,
    /// Community-Bewertung.
    CommunityRating,
    /// Laufzeit.
    Runtime,
    /// Anzahl Wiedergaben.
    PlayCount,
    /// Zufall.
    Random,
}

impl SortBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::SortName => "SortName",
            Self::DateCreated => "DateCreated",
            Self::DatePlayed => "DatePlayed",
            Self::PremiereDate => "PremiereDate",
            Self::ProductionYear => "ProductionYear",
            Self::CommunityRating => "CommunityRating",
            Self::Runtime => "Runtime",
            Self::PlayCount => "PlayCount",
            Self::Random => "Random",
        }
    }
}

/// Sortierrichtung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// Aufsteigend.
    Ascending,
    /// Absteigend.
    Descending,
}

/// Baut die Parameterliste für `/Items`. Alle Setter nehmen `self` und geben `Self` zurück.
#[derive(Debug, Clone, Default)]
pub struct ItemsQuery {
    params: Vec<(String, String)>,
}

fn join<I: IntoIterator<Item = S>, S: AsRef<str>>(parts: I) -> String {
    parts
        .into_iter()
        .map(|s| s.as_ref().to_owned())
        .collect::<Vec<_>>()
        .join(",")
}

impl ItemsQuery {
    /// Leere Abfrage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Beliebigen Parameter setzen (ersetzt einen vorhandenen gleichen Namens).
    pub fn raw(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.retain(|(k, _)| k != key);
        self.params.push((key.to_owned(), value.into()));
        self
    }

    /// `IncludeItemTypes`.
    pub fn include_types(self, kinds: &[ItemKind]) -> Self {
        self.raw("includeItemTypes", join(kinds.iter().map(|k| k.as_str())))
    }

    /// `ExcludeItemTypes`.
    pub fn exclude_types(self, kinds: &[ItemKind]) -> Self {
        self.raw("excludeItemTypes", join(kinds.iter().map(|k| k.as_str())))
    }

    /// `MediaTypes` (Video, Audio, Photo, Book).
    pub fn media_types(self, types: &[&str]) -> Self {
        self.raw("mediaTypes", join(types))
    }

    /// `Filters`.
    pub fn filters(self, filters: &[ItemFilter]) -> Self {
        self.raw("filters", join(filters.iter().map(|f| f.as_str())))
    }

    /// `SortBy`.
    pub fn sort_by(self, keys: &[SortBy]) -> Self {
        self.raw("sortBy", join(keys.iter().map(|k| k.as_str())))
    }

    /// `SortOrder`.
    pub fn order(self, order: SortOrder) -> Self {
        self.raw(
            "sortOrder",
            match order {
                SortOrder::Ascending => "Ascending",
                SortOrder::Descending => "Descending",
            },
        )
    }

    /// `Fields` (z. B. `Overview`, `Genres`, `People`, `MediaSources`, `Trickplay`).
    pub fn fields(self, fields: &[&str]) -> Self {
        self.raw("fields", join(fields))
    }

    /// `Limit`.
    pub fn limit(self, limit: u32) -> Self {
        self.raw("limit", limit.to_string())
    }

    /// `StartIndex`.
    pub fn start_index(self, index: u32) -> Self {
        self.raw("startIndex", index.to_string())
    }

    /// `ParentId`.
    pub fn parent_id(self, id: &str) -> Self {
        self.raw("parentId", id)
    }

    /// `SearchTerm`.
    pub fn search_term(self, term: &str) -> Self {
        self.raw("searchTerm", term)
    }

    /// `Genres` (Namen, exakt).
    pub fn genres(self, genres: &[&str]) -> Self {
        // Jellyfin trennt Genres mit `|`.
        self.raw("genres", genres.join("|"))
    }

    /// `Recursive`.
    pub fn recursive(self, on: bool) -> Self {
        self.raw("recursive", on.to_string())
    }

    /// `IsFavorite`.
    pub fn is_favorite(self, on: bool) -> Self {
        self.raw("isFavorite", on.to_string())
    }

    /// `Ids`.
    pub fn ids(self, ids: &[&str]) -> Self {
        self.raw("ids", join(ids))
    }

    /// `MinCommunityRating`.
    pub fn min_community_rating(self, rating: f32) -> Self {
        self.raw("minCommunityRating", rating.to_string())
    }

    /// `Years`.
    pub fn years(self, years: &[i32]) -> Self {
        self.raw("years", join(years.iter().map(|y| y.to_string())))
    }

    /// `EnableUserData`.
    pub fn enable_user_data(self, on: bool) -> Self {
        self.raw("enableUserData", on.to_string())
    }

    /// `EnableImages`.
    pub fn enable_images(self, on: bool) -> Self {
        self.raw("enableImages", on.to_string())
    }

    /// `ImageTypeLimit`.
    pub fn image_type_limit(self, limit: u32) -> Self {
        self.raw("imageTypeLimit", limit.to_string())
    }

    /// `EnableImageTypes` (z. B. `Primary`, `Backdrop`, `Thumb`, `Logo`).
    pub fn enable_image_types(self, types: &[&str]) -> Self {
        self.raw("enableImageTypes", join(types))
    }

    /// `EnableTotalRecordCount`.
    pub fn total_count(self, on: bool) -> Self {
        self.raw("enableTotalRecordCount", on.to_string())
    }

    /// Fertige Parameterliste.
    pub fn build(&self) -> &[(String, String)] {
        &self.params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_params() {
        let q = ItemsQuery::new()
            .include_types(&[ItemKind::Movie, ItemKind::Series])
            .filters(&[ItemFilter::IsUnplayed])
            .sort_by(&[SortBy::CommunityRating])
            .order(SortOrder::Descending)
            .limit(20)
            .genres(&["Drama", "Komödie"])
            .recursive(true)
            .limit(30);
        let p: std::collections::HashMap<_, _> = q.build().iter().cloned().collect();
        assert_eq!(p["includeItemTypes"], "Movie,Series");
        assert_eq!(p["filters"], "IsUnplayed");
        assert_eq!(p["sortBy"], "CommunityRating");
        assert_eq!(p["sortOrder"], "Descending");
        assert_eq!(p["limit"], "30"); // später gesetzter Wert ersetzt den früheren
        assert_eq!(p["genres"], "Drama|Komödie");
        assert_eq!(p["recursive"], "true");
    }
}
