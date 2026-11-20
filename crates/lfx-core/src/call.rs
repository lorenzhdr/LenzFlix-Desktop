//! Abfragen als Daten: Pfad + Parameterliste, ausführbar über den öffentlichen `lfx-api`-Client.
//!
//! `lfx-api` bietet für `Latest`, `Resume`, `NextUp`, `Similar` nur eine Auswahl an Parametern;
//! das Web schickt zusätzlich `EnableImageTypes`, `ImageTypeLimit`, `GroupItems`, `EnableResumable` usw.
//! Deshalb bauen die Zeilen ihre Parameter selbst (mit [`ItemsQuery`] als Listenbauer) und führen sie
//! über `Client::url` + `Client::fetch_bytes` aus (Auth per Header, nie Token in der URL).

use crate::util::encode_uri_component as seg;
use lfx_api::{ApiError, BaseItem, Client, ItemsQuery};
use serde_json::Value;

/// Ziel einer Abfrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `/Items`.
    Items,
    /// `/Items/Latest` (liefert ein Array).
    Latest,
    /// `/UserItems/Resume`.
    Resume,
    /// `/Shows/NextUp`.
    NextUp,
    /// `/Items/{id}/Similar`.
    Similar(String),
    /// `/Genres`.
    Genres,
    /// `/UserViews`.
    Views,
    /// `/Users/{userId}/Items/{id}/LocalTrailers` (Array).
    LocalTrailers(String),
}

/// Eine Abfrage (ohne `userId`, den ergänzt [`ApiCall::execute`] wie `core/api.js`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApiCall {
    /// Ziel.
    pub endpoint: Endpoint,
    /// Parameter in Reihenfolge (Namen wie `ItemsQuery` sie schreibt, camelCase).
    pub params: Vec<(String, String)>,
}

impl ApiCall {
    /// Abfrage aus Endpunkt und Listenbauer.
    pub fn new(endpoint: Endpoint, query: ItemsQuery) -> Self {
        Self {
            endpoint,
            params: query.build().to_vec(),
        }
    }

    /// Wert eines Parameters (Name ohne Beachtung der Groß-/Kleinschreibung, wie der Server).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Pfad (mit Benutzer-Id, wo nötig).
    pub fn path(&self, user_id: &str) -> String {
        match &self.endpoint {
            Endpoint::Items => "/Items".into(),
            Endpoint::Latest => "/Items/Latest".into(),
            Endpoint::Resume => "/UserItems/Resume".into(),
            Endpoint::NextUp => "/Shows/NextUp".into(),
            Endpoint::Similar(id) => format!("/Items/{}/Similar", seg(id)),
            Endpoint::Genres => "/Genres".into(),
            Endpoint::Views => "/UserViews".into(),
            Endpoint::LocalTrailers(id) => {
                format!("/Users/{}/Items/{}/LocalTrailers", seg(user_id), seg(id))
            }
        }
    }

    async fn fetch(&self, client: &Client) -> Result<Value, ApiError> {
        let user = client.user_id()?;
        let mut query = Vec::with_capacity(self.params.len() + 1);
        if !matches!(self.endpoint, Endpoint::LocalTrailers(_)) {
            query.push(("userId".to_owned(), user.clone()));
        }
        query.extend(self.params.iter().cloned());
        let url = client.url(&self.path(&user), &query)?;
        let bytes = client.fetch_bytes(url.as_str()).await?;
        serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// Führt die Abfrage aus und liefert die rohen Elemente (Array oder `Items`).
    pub async fn execute_values(&self, client: &Client) -> Result<Vec<Value>, ApiError> {
        Ok(items_of(self.fetch(client).await?))
    }

    /// Eine Seite mit Gesamtzahl (`TotalRecordCount`, nur mit `EnableTotalRecordCount`).
    pub async fn execute_page(
        &self,
        client: &Client,
    ) -> Result<(Vec<BaseItem>, Option<usize>), ApiError> {
        let value = self.fetch(client).await?;
        let total = value
            .get("TotalRecordCount")
            .and_then(Value::as_u64)
            .map(|t| t as usize);
        let items = items_of(value)
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        Ok((items, total))
    }

    /// Wie [`Self::execute_values`], aber als [`BaseItem`]; nicht lesbare Elemente werden übersprungen.
    pub async fn execute(&self, client: &Client) -> Result<Vec<BaseItem>, ApiError> {
        Ok(self
            .execute_values(client)
            .await?
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect())
    }
}

/// `asItems` aus `rows/sources.js`: Array oder `{ Items: [...] }`, sonst leer.
pub fn items_of(value: Value) -> Vec<Value> {
    match value {
        Value::Array(a) => a,
        Value::Object(mut o) => match o.remove("Items") {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn as_items_like_web() {
        assert_eq!(items_of(json!([1, 2])).len(), 2);
        assert_eq!(items_of(json!({"Items": [1]})).len(), 1);
        assert!(items_of(json!({"Items": 3})).is_empty());
        assert!(items_of(json!(null)).is_empty());
    }

    #[test]
    fn params_are_case_insensitive() {
        let c = ApiCall::new(Endpoint::Items, ItemsQuery::new().limit(5));
        assert_eq!(c.param("Limit"), Some("5"));
        assert_eq!(c.param("limit"), Some("5"));
        assert_eq!(c.param("nope"), None);
    }

    #[test]
    fn paths() {
        let c = ApiCall::new(Endpoint::Similar("ab c".into()), ItemsQuery::new());
        assert_eq!(c.path("u"), "/Items/ab%20c/Similar");
        let t = ApiCall::new(Endpoint::LocalTrailers("i".into()), ItemsQuery::new());
        assert_eq!(t.path("u1"), "/Users/u1/Items/i/LocalTrailers");
    }
}
