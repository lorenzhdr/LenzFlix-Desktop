//! W4-account: Scan einer einzelnen Bibliothek.

use crate::client::Client;
use crate::endpoints::{NO_BODY, enc, q};
use crate::error::Result;
use reqwest::Method;

impl Client {
    /// Scannt eine Bibliothek (oder ein einzelnes Element) neu (`POST /Items/{id}/Refresh`). Die Id ist
    /// `VirtualFolder::item_id`. Braucht ein Administratorkonto.
    pub async fn refresh_item(&self, item_id: &str) -> Result<()> {
        self.call_empty(
            Method::POST,
            &format!("/Items/{}/Refresh", enc(item_id)),
            &q(&[
                ("Recursive", "true".to_owned()),
                ("ImageRefreshMode", "Default".to_owned()),
                ("MetadataRefreshMode", "Default".to_owned()),
                ("ReplaceAllImages", "false".to_owned()),
                ("ReplaceAllMetadata", "false".to_owned()),
            ]),
            NO_BODY,
        )
        .await
    }
}
