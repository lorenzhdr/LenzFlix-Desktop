//! Endpunkte: Server, Anmeldung, Quick Connect, Bibliothek, Benutzerdaten, Segmente.

use crate::client::{Client, Session};
use crate::dto::*;
use crate::error::{ApiError, Result};
use crate::query::ItemsQuery;
use reqwest::Method;
use serde::Serialize;

/// Kein Body.
pub(crate) const NO_BODY: Option<&()> = None;

pub(crate) fn q(pairs: &[(&str, String)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

pub(crate) fn opt(list: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        list.push((key.to_owned(), v));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct AuthByName<'a> {
    username: &'a str,
    pw: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct AuthQuickConnect<'a> {
    secret: &'a str,
}

impl Client {
    // ------------------------------------------------------------ Server

    /// Öffentliche Serverinfo, ohne Anmeldung (auch zum Prüfen einer Adresse).
    pub async fn public_info(&self) -> Result<PublicSystemInfo> {
        self.get_json("/System/Info/Public", &[]).await
    }

    // ------------------------------------------------------------ Anmeldung

    fn adopt(&self, result: &AuthenticationResult) {
        self.set_session(Some(Session {
            token: result.access_token.clone(),
            user_id: result.user.id.clone(),
        }));
    }

    /// Anmeldung mit Benutzername und Passwort. Setzt bei Erfolg die Sitzung des Clients.
    pub async fn login(&self, username: &str, password: &str) -> Result<AuthenticationResult> {
        // Ein altes Token darf nicht mitgesendet werden.
        self.set_session(None);
        let body = AuthByName {
            username,
            pw: password,
        };
        let result: AuthenticationResult = self
            .call_json(Method::POST, "/Users/AuthenticateByName", &[], Some(&body))
            .await?;
        self.adopt(&result);
        Ok(result)
    }

    /// Meldet die Sitzung ab (`/Sessions/Logout`) und löscht sie lokal – auch wenn der Server-Aufruf scheitert.
    pub async fn logout(&self) -> Result<()> {
        let res = self
            .call_empty(Method::POST, "/Sessions/Logout", &[], NO_BODY)
            .await;
        self.set_session(None);
        res
    }

    /// Ist Quick Connect auf dem Server aktiv?
    pub async fn quick_connect_enabled(&self) -> Result<bool> {
        self.get_json("/QuickConnect/Enabled", &[]).await
    }

    /// Startet Quick Connect: liefert Code (anzeigen) und Secret (zum Abfragen).
    pub async fn quick_connect_initiate(&self) -> Result<QuickConnectResult> {
        self.call_json(Method::POST, "/QuickConnect/Initiate", &[], NO_BODY)
            .await
            .map_err(|e| match e {
                ApiError::Unauthorized | ApiError::Forbidden => ApiError::QuickConnectDisabled,
                other => other,
            })
    }

    /// Fragt den Zustand ab (Polling): `authenticated == true`, sobald der Nutzer den Code bestätigt hat.
    pub async fn quick_connect_poll(&self, secret: &crate::Secret) -> Result<QuickConnectResult> {
        self.get_json(
            "/QuickConnect/Connect",
            &q(&[("secret", secret.expose().to_owned())]),
        )
        .await
    }

    /// Schließt Quick Connect ab, sobald `authenticated` wahr ist. Setzt die Sitzung des Clients.
    pub async fn quick_connect_login(
        &self,
        secret: &crate::Secret,
    ) -> Result<AuthenticationResult> {
        self.set_session(None);
        let body = AuthQuickConnect {
            secret: secret.expose(),
        };
        let result: AuthenticationResult = self
            .call_json(
                Method::POST,
                "/Users/AuthenticateWithQuickConnect",
                &[],
                Some(&body),
            )
            .await?;
        self.adopt(&result);
        Ok(result)
    }

    /// Bestätigt einen Quick-Connect-Code mit der aktuellen (angemeldeten) Sitzung, z. B. für ein anderes Gerät.
    pub async fn quick_connect_authorize(&self, code: &str) -> Result<bool> {
        let user = self.user_id()?;
        self.call_json(
            Method::POST,
            "/QuickConnect/Authorize",
            &q(&[("code", code.to_owned()), ("userId", user)]),
            NO_BODY,
        )
        .await
    }

    /// Angemeldeter Benutzer (`/Users/Me`).
    pub async fn me(&self) -> Result<UserDto> {
        self.get_json("/Users/Me", &[]).await
    }

    // ------------------------------------------------------------ Bibliothek

    pub(crate) fn with_user(&self, extra: &[(String, String)]) -> Result<Vec<(String, String)>> {
        let mut out = vec![("userId".to_owned(), self.user_id()?)];
        out.extend_from_slice(extra);
        Ok(out)
    }

    /// Ansichten (Bibliotheken) des Benutzers.
    pub async fn views(&self) -> Result<QueryResult<BaseItem>> {
        self.get_json("/UserViews", &self.with_user(&[])?).await
    }

    /// Allgemeine Abfrage (`/Items`).
    pub async fn items(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
        self.get_json("/Items", &self.with_user(query.build())?)
            .await
    }

    /// Ein Element per Id (`Fields` z. B. `Overview,Genres,People,MediaSources,Chapters,Trickplay`).
    pub async fn item(&self, id: &str, fields: &[&str]) -> Result<BaseItem> {
        let mut extra = Vec::new();
        if !fields.is_empty() {
            extra.push(("fields".to_owned(), fields.join(",")));
        }
        self.get_json(&format!("/Items/{}", enc(id)), &self.with_user(&extra)?)
            .await
    }

    /// Zuletzt hinzugefügt (`/Items/Latest` liefert ein Array).
    pub async fn latest(
        &self,
        parent_id: Option<&str>,
        kinds: &[ItemKind],
        limit: u32,
        fields: &[&str],
    ) -> Result<Vec<BaseItem>> {
        let mut extra = vec![("limit".to_owned(), limit.to_string())];
        opt(&mut extra, "parentId", parent_id.map(str::to_owned));
        if !kinds.is_empty() {
            extra.push((
                "includeItemTypes".to_owned(),
                kinds
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        opt(
            &mut extra,
            "fields",
            (!fields.is_empty()).then(|| fields.join(",")),
        );
        self.get_json("/Items/Latest", &self.with_user(&extra)?)
            .await
    }

    /// Weiterschauen (`/UserItems/Resume`).
    pub async fn resume(
        &self,
        limit: u32,
        media_types: &[&str],
        fields: &[&str],
    ) -> Result<QueryResult<BaseItem>> {
        let mut extra = vec![("limit".to_owned(), limit.to_string())];
        opt(
            &mut extra,
            "mediaTypes",
            (!media_types.is_empty()).then(|| media_types.join(",")),
        );
        opt(
            &mut extra,
            "fields",
            (!fields.is_empty()).then(|| fields.join(",")),
        );
        self.get_json("/UserItems/Resume", &self.with_user(&extra)?)
            .await
    }

    /// Als Nächstes (`/Shows/NextUp`), optional für eine Serie.
    pub async fn next_up(
        &self,
        series_id: Option<&str>,
        limit: u32,
        fields: &[&str],
    ) -> Result<QueryResult<BaseItem>> {
        let mut extra = vec![("limit".to_owned(), limit.to_string())];
        opt(&mut extra, "seriesId", series_id.map(str::to_owned));
        opt(
            &mut extra,
            "fields",
            (!fields.is_empty()).then(|| fields.join(",")),
        );
        self.get_json("/Shows/NextUp", &self.with_user(&extra)?)
            .await
    }

    /// Staffeln einer Serie.
    pub async fn seasons(&self, series_id: &str, fields: &[&str]) -> Result<QueryResult<BaseItem>> {
        let mut extra = Vec::new();
        opt(
            &mut extra,
            "fields",
            (!fields.is_empty()).then(|| fields.join(",")),
        );
        self.get_json(
            &format!("/Shows/{}/Seasons", enc(series_id)),
            &self.with_user(&extra)?,
        )
        .await
    }

    /// Folgen einer Serie, optional nur einer Staffel.
    pub async fn episodes(
        &self,
        series_id: &str,
        season_id: Option<&str>,
        fields: &[&str],
    ) -> Result<QueryResult<BaseItem>> {
        let mut extra = Vec::new();
        opt(&mut extra, "seasonId", season_id.map(str::to_owned));
        opt(
            &mut extra,
            "fields",
            (!fields.is_empty()).then(|| fields.join(",")),
        );
        self.get_json(
            &format!("/Shows/{}/Episodes", enc(series_id)),
            &self.with_user(&extra)?,
        )
        .await
    }

    /// Ähnliche Titel.
    pub async fn similar(
        &self,
        id: &str,
        limit: u32,
        fields: &[&str],
    ) -> Result<QueryResult<BaseItem>> {
        let mut extra = vec![("limit".to_owned(), limit.to_string())];
        opt(
            &mut extra,
            "fields",
            (!fields.is_empty()).then(|| fields.join(",")),
        );
        self.get_json(
            &format!("/Items/{}/Similar", enc(id)),
            &self.with_user(&extra)?,
        )
        .await
    }

    /// Genres, optional je Bibliothek/Elementart.
    pub async fn genres(
        &self,
        parent_id: Option<&str>,
        kinds: &[ItemKind],
    ) -> Result<QueryResult<BaseItem>> {
        let mut extra = vec![("sortBy".to_owned(), "SortName".to_owned())];
        opt(&mut extra, "parentId", parent_id.map(str::to_owned));
        if !kinds.is_empty() {
            extra.push((
                "includeItemTypes".to_owned(),
                kinds
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        self.get_json("/Genres", &self.with_user(&extra)?).await
    }

    // ------------------------------------------------------------ Benutzerdaten

    /// Favorit setzen (`true`) oder entfernen (`false`).
    pub async fn set_favorite(&self, id: &str, favorite: bool) -> Result<UserItemData> {
        let method = if favorite {
            Method::POST
        } else {
            Method::DELETE
        };
        self.call_json(
            method,
            &format!("/UserFavoriteItems/{}", enc(id)),
            &self.with_user(&[])?,
            NO_BODY,
        )
        .await
    }

    /// Als gesehen markieren (`true`) oder zurücknehmen (`false`).
    pub async fn set_played(&self, id: &str, played: bool) -> Result<UserItemData> {
        let method = if played { Method::POST } else { Method::DELETE };
        self.call_json(
            method,
            &format!("/UserPlayedItems/{}", enc(id)),
            &self.with_user(&[])?,
            NO_BODY,
        )
        .await
    }

    /// Bewertung setzen: `Some(true)` gefällt, `Some(false)` gefällt nicht, `None` löscht.
    pub async fn set_rating(&self, id: &str, likes: Option<bool>) -> Result<UserItemData> {
        let path = format!("/UserItems/{}/Rating", enc(id));
        match likes {
            Some(v) => {
                self.call_json(
                    Method::POST,
                    &path,
                    &self.with_user(&[("likes".to_owned(), v.to_string())])?,
                    NO_BODY,
                )
                .await
            }
            None => {
                self.call_json(Method::DELETE, &path, &self.with_user(&[])?, NO_BODY)
                    .await
            }
        }
    }

    // ------------------------------------------------------------ Segmente, Trickplay

    /// Medien-Segmente (Intro, Abspann, ...) eines Elements.
    pub async fn media_segments(&self, item_id: &str) -> Result<QueryResult<MediaSegment>> {
        self.get_json(&format!("/MediaSegments/{}", enc(item_id)), &[])
            .await
    }

    /// Trickplay-Infos eines Elements: Quell-Id → Breite → Info. Leer, wenn keine vorhanden.
    pub async fn trickplay_info(
        &self,
        item_id: &str,
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, TrickplayInfo>>>
    {
        Ok(self
            .item(item_id, &["Trickplay"])
            .await?
            .trickplay
            .unwrap_or_default())
    }
}

/// Prozentkodiert ein Pfadsegment (Ids sind normalerweise Hex, aber nie ungeprüft einsetzen).
pub(crate) fn enc(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::enc;

    #[test]
    fn encodes_path_segments() {
        assert_eq!(enc("abc-123"), "abc-123");
        assert_eq!(enc("../x"), "..%2Fx");
    }
}
