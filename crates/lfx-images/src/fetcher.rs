//! Abruf der kodierten Bild-Bytes über einen austauschbaren Fetcher.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use lfx_api::{Client, ImageOptions};

use crate::types::{ImageError, ImageRequest};

/// Boxed Future ohne Zusatzabhängigkeit.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Holt die kodierten Bytes (JPEG/PNG/WebP) zu einer Anfrage.
pub trait ImageFetcher: Send + Sync + 'static {
    /// Lädt das Bild. Fehlermeldungen dürfen keine Tokens oder Server-URLs enthalten.
    fn fetch<'a>(&'a self, request: &'a ImageRequest) -> BoxFuture<'a, Result<Bytes, ImageError>>;
}

/// Fetcher über den Jellyfin-Client: Auth per Header, die URL trägt kein Token.
#[derive(Clone)]
pub struct ApiFetcher {
    client: Client,
}

impl ApiFetcher {
    /// Verpackt einen angemeldeten Client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl ImageFetcher for ApiFetcher {
    fn fetch<'a>(&'a self, request: &'a ImageRequest) -> BoxFuture<'a, Result<Bytes, ImageError>> {
        Box::pin(async move {
            let url = self
                .client
                .image_url(
                    &request.item_id,
                    request.kind,
                    request.tag.as_deref(),
                    ImageOptions {
                        max_width: request.max_width,
                        max_height: request.max_height,
                        quality: request.quality,
                    },
                )
                .map_err(|e| ImageError::Fetch(e.to_string()))?;
            self.client
                .fetch_bytes(&url)
                .await
                .map_err(|e| ImageError::Fetch(e.to_string()))
        })
    }
}
