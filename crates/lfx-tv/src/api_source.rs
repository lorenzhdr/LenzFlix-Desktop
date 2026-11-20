//! Adapter: [`CatalogSource`] über den Jellyfin-Client aus `lfx-api`.
//!
//! Der Client liefert typisierte `BaseItem`; hier werden sie in die PascalCase-Rohform des
//! Katalogs zurückgebaut. Bekannte Lücke: `LocalTrailerCount` fehlt im `BaseItem`-DTO, Filme
//! zeigen deshalb noch keinen lokalen Trailer (der Planer fällt auf Karten zurück), bis das DTO
//! das Feld trägt. Fehlertexte kommen ohne Adresse und Schlüssel aus dem Client.

use lfx_api::{ApiError, BaseItem, Client, ItemKind, ItemsQuery};
use serde_json::{Map, Value, json};

use crate::catalog::{CatalogKind, CatalogSource, PageRequest, RawPage};
use crate::error::{TvError, fatal_text};

/// Katalogquelle über `lfx_api::Client`.
pub struct ApiCatalogSource<'a> {
    client: &'a Client,
}

impl<'a> ApiCatalogSource<'a> {
    /// Quelle für einen angemeldeten Client.
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }
}

fn kind_of(k: CatalogKind) -> ItemKind {
    match k {
        CatalogKind::Movies => ItemKind::Movie,
        CatalogKind::Series => ItemKind::Series,
        CatalogKind::Episodes => ItemKind::Episode,
        CatalogKind::Boxsets => ItemKind::BoxSet,
    }
}

fn put_str(m: &mut Map<String, Value>, k: &str, v: &Option<String>) {
    if let Some(s) = v {
        m.insert(k.to_string(), Value::String(s.clone()));
    }
}

/// `BaseItem` in die Rohform des Katalogs (Felder wie vom Server, fehlende entfallen).
pub fn base_item_to_raw(it: &BaseItem) -> Value {
    let mut m = Map::new();
    m.insert("Id".into(), Value::String(it.id.clone()));
    m.insert("Name".into(), Value::String(it.name.clone()));
    if let Some(t) = it.run_time_ticks {
        m.insert("RunTimeTicks".into(), json!(t));
    }
    m.insert("Genres".into(), json!(it.genres));
    put_str(&mut m, "OfficialRating", &it.official_rating);
    if let Some(y) = it.production_year {
        m.insert("ProductionYear".into(), json!(y));
    }
    put_str(&mut m, "PremiereDate", &it.premiere_date);
    put_str(&mut m, "DateCreated", &it.date_created);
    put_str(&mut m, "SeriesId", &it.series_id);
    put_str(&mut m, "SeriesName", &it.series_name);
    if let Some(n) = it.parent_index_number {
        m.insert("ParentIndexNumber".into(), json!(n));
    }
    if let Some(n) = it.index_number {
        m.insert("IndexNumber".into(), json!(n));
    }
    if let Some(n) = it.child_count {
        m.insert("ChildCount".into(), json!(n));
    }
    if !it.image_tags.is_empty() {
        let mut tags: Vec<(&String, &String)> = it.image_tags.iter().collect();
        tags.sort();
        let obj: Map<String, Value> = tags
            .into_iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        m.insert("ImageTags".into(), Value::Object(obj));
    }
    if !it.backdrop_image_tags.is_empty() {
        m.insert("BackdropImageTags".into(), json!(it.backdrop_image_tags));
    }
    Value::Object(m)
}

fn map_err(e: &ApiError) -> TvError {
    TvError::Source(fatal_text(&e.to_string()))
}

impl CatalogSource for ApiCatalogSource<'_> {
    async fn fetch_page(&self, req: &PageRequest) -> Result<RawPage, TvError> {
        let mut q = ItemsQuery::new()
            .recursive(true)
            .include_types(&[kind_of(req.kind)])
            .fields(&req.kind.fields().split(',').collect::<Vec<_>>())
            .enable_user_data(false)
            .total_count(true)
            .start_index(u32::try_from(req.start_index).unwrap_or(u32::MAX))
            .limit(u32::try_from(req.limit).unwrap_or(u32::MAX));
        q = if req.kind.with_image_tags() {
            q.enable_images(true)
                .image_type_limit(1)
                .enable_image_types(&["Primary", "Thumb", "Backdrop"])
        } else {
            q.enable_images(false)
        };
        let res = self.client.items(&q).await.map_err(|e| map_err(&e))?;
        Ok(RawPage {
            total: (res.total_record_count > 0).then_some(res.total_record_count),
            items: res.items.iter().map(base_item_to_raw).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rohform_traegt_vertragsfelder() {
        let it = BaseItem {
            id: "a1".into(),
            name: "Erfundener Film".into(),
            genres: vec!["Komödie".into()],
            run_time_ticks: Some(54_000_000_000),
            production_year: Some(1999),
            ..BaseItem::default()
        };
        let v = base_item_to_raw(&it);
        assert_eq!(v["Id"], "a1");
        assert_eq!(v["RunTimeTicks"], 54_000_000_000i64);
        assert!(v.get("SeriesId").is_none());
        assert!(v.get("ImageTags").is_none());
    }
}
