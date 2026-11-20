//! Gemeinsame Typen: Anfrage, dekodiertes Bild, Fehler.

use std::fmt;
use std::sync::Arc;

use lfx_api::ImageType;

/// Fehler der Bild-Pipeline. Meldungen enthalten nie Tokens oder Server-URLs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// Der Abruf (Netzwerk/Server) ist fehlgeschlagen.
    #[error("Bild konnte nicht geladen werden: {0}")]
    Fetch(String),
    /// Die Bytes sind kein lesbares Bild.
    #[error("Bild konnte nicht dekodiert werden: {0}")]
    Decode(String),
    /// Die Anfrage wurde abgebrochen (Handle verworfen, Lader beendet, Vorladen aus).
    #[error("Bildanfrage abgebrochen")]
    Cancelled,
    /// Ungültige Parameter (z. B. Blurhash).
    #[error("ungültige Bildangabe: {0}")]
    Invalid(String),
}

/// Wichtigkeit einer Anfrage. Sichtbares wird vor Vorladen abgearbeitet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Priority {
    /// Hintergrund-Vorladen (Datensparmodus-Teil `prefetch`).
    Prefetch,
    /// Wird gerade angezeigt.
    #[default]
    Visible,
}

/// Was geladen werden soll: Jellyfin-Bild eines Elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    /// Element-ID (Pfad `/Items/<id>/Images/<Typ>`).
    pub item_id: String,
    /// Bildart.
    pub kind: ImageType,
    /// Bild-Tag aus `ImageTags`/`BackdropImageTags` (macht den Schlüssel inhaltsstabil).
    pub tag: Option<String>,
    /// Höchstbreite in Pixeln (Server-Parameter und Zielgröße).
    pub max_width: Option<u32>,
    /// Höchsthöhe in Pixeln.
    pub max_height: Option<u32>,
    /// JPEG-Qualität 0–100 für die Server-Antwort.
    pub quality: Option<u8>,
    /// Ergebnis mit vormultipliziertem Alpha liefern (sonst gerade).
    pub premultiplied: bool,
}

impl ImageRequest {
    /// Anfrage ohne Größenlimit.
    pub fn new(item_id: impl Into<String>, kind: ImageType) -> Self {
        Self {
            item_id: item_id.into(),
            kind,
            tag: None,
            max_width: None,
            max_height: None,
            quality: None,
            premultiplied: false,
        }
    }

    /// Setzt den Tag.
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Setzt die Größenbegrenzung.
    #[must_use]
    pub fn max_size(mut self, width: u32, height: u32) -> Self {
        self.max_width = Some(width);
        self.max_height = Some(height);
        self
    }

    /// Setzt die Server-Qualität.
    #[must_use]
    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = Some(quality);
        self
    }

    /// Vormultipliziertes Alpha anfordern.
    #[must_use]
    pub fn premultiplied(mut self, on: bool) -> Self {
        self.premultiplied = on;
        self
    }

    /// Schlüssel der kodierten Bytes (Plattencache, Abruf): ohne `premultiplied`.
    pub fn disk_key(&self) -> String {
        format!(
            "{}|{}|{}|{}x{}|q{}",
            self.item_id,
            self.kind.as_str(),
            self.tag.as_deref().unwrap_or(""),
            self.max_width.unwrap_or(0),
            self.max_height.unwrap_or(0),
            self.quality.unwrap_or(0),
        )
    }

    /// Schlüssel des dekodierten Ergebnisses (Speichercache, Deduplizierung).
    pub fn cache_key(&self) -> String {
        format!("{}|p{}", self.disk_key(), u8::from(self.premultiplied))
    }
}

/// Dekodiertes Bild als RGBA8, Zeile für Zeile, ohne Zeilenpolsterung.
#[derive(Clone)]
pub struct DecodedImage {
    /// Breite in Pixeln.
    pub width: u32,
    /// Höhe in Pixeln.
    pub height: u32,
    /// `width * height * 4` Bytes RGBA.
    pub pixels: Arc<[u8]>,
    /// `true`, wenn die Farbkanäle mit Alpha vormultipliziert sind.
    pub premultiplied: bool,
}

impl DecodedImage {
    /// Speicherbedarf der Pixel.
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }
}

impl fmt::Debug for DecodedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.pixels.len())
            .field("premultiplied", &self.premultiplied)
            .finish()
    }
}

impl PartialEq for DecodedImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.premultiplied == other.premultiplied
            && self.pixels == other.pixels
    }
}
