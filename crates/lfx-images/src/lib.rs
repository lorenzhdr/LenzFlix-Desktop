//! Bild-Pipeline für LENZFLIX Desktop, ohne UI-Abhängigkeit.
//!
//! - [`ImageLoader`]: asynchron, Speicher-LRU mit Byte-Budget, Plattencache ([`DiskCache`]),
//!   Deduplizierung, Priorität (sichtbar vor Vorladen), Abbruch per Handle-`drop`, begrenztes Vorladen.
//! - [`ImageFetcher`]: injizierter Abruf; [`ApiFetcher`] nutzt `lfx_api::Client` (Auth per Header).
//! - [`decode_image`]: JPEG/PNG/WebP → RGBA8, hochwertig verkleinert (nie vergrößert).
//! - Modul `ambient`: dominante Farbe/Ambient-Farbe (Port von `core/color.js`).
//! - Modul `placeholder`: Blurhash-Platzhalter.

pub mod ambient;
mod decode;
mod disk;
mod fetcher;
mod loader;
pub mod placeholder;
mod types;

pub use ambient::{
    AmbientClamp, DEFAULT_AMBIENT, ambient_from_image, ambient_from_pixels, clamp_ambient,
    dominant_color,
};
pub use decode::{decode_image, fit_within, premultiply};
pub use disk::{DiskCache, DiskEntry, DiskIndex, FileIndex, file_name_for};
pub use fetcher::{ApiFetcher, BoxFuture, ImageFetcher};
pub use loader::{ImageHandle, ImageLoader, LoadResult, LoaderConfig, LoaderStats};
pub use placeholder::{blurhash_for, decode_blurhash, placeholder_size};
pub use types::{DecodedImage, ImageError, ImageRequest, Priority};
