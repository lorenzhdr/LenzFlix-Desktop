//! Bild-URLs ohne Token. Geladen wird mit [`crate::Client::fetch_bytes`] (Auth über Header).

use crate::client::Client;
use crate::endpoints::enc;
use crate::error::Result;

/// Bildart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    /// Poster/Hauptbild.
    Primary,
    /// Hintergrund.
    Backdrop,
    /// Querformat-Vorschau.
    Thumb,
    /// Titel-Logo.
    Logo,
    /// Banner.
    Banner,
    /// Disc-Bild.
    Disc,
}

impl ImageType {
    /// Pfadsegment der API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Backdrop => "Backdrop",
            Self::Thumb => "Thumb",
            Self::Logo => "Logo",
            Self::Banner => "Banner",
            Self::Disc => "Disc",
        }
    }
}

/// Optionen für Bild-URLs.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageOptions {
    /// Höchstbreite in Pixeln.
    pub max_width: Option<u32>,
    /// Höchsthöhe in Pixeln.
    pub max_height: Option<u32>,
    /// JPEG-Qualität 0–100.
    pub quality: Option<u8>,
}

impl Client {
    /// Bild-URL (ohne Token). `tag` aus `ImageTags`/`BackdropImageTags` macht die URL cache-stabil.
    pub fn image_url(
        &self,
        item_id: &str,
        kind: ImageType,
        tag: Option<&str>,
        options: ImageOptions,
    ) -> Result<String> {
        let mut query = Vec::new();
        if let Some(v) = options.max_width {
            query.push(("maxWidth".to_owned(), v.to_string()));
        }
        if let Some(v) = options.max_height {
            query.push(("maxHeight".to_owned(), v.to_string()));
        }
        if let Some(v) = options.quality {
            query.push(("quality".to_owned(), v.to_string()));
        }
        if let Some(tag) = tag {
            query.push(("tag".to_owned(), tag.to_owned()));
        }
        Ok(self
            .url(
                &format!("/Items/{}/Images/{}", enc(item_id), kind.as_str()),
                &query,
            )?
            .into())
    }
}
