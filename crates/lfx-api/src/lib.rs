//! Jellyfin-API-Client für LENZFLIX Desktop.
//!
//! - [`Client`]: Anfrage-Pipeline mit Auth-Header, Retry (nur GET), geschwärzten Logs.
//! - Tokens stecken immer in [`Secret`] (Debug/Display = `***`), URLs im Log nur über [`redact_url`].
//! - Bild- und Stream-URLs enthalten nie das Token; Laden mit Header ([`Client::fetch_bytes`]).

mod client;
pub mod discovery;
pub mod dto;
mod endpoints;
mod error;
mod images;
mod playback;
mod query;
mod redact;
mod secret;

pub use client::{
    CLIENT_NAME, Client, ClientInfo, ClientOptions, RetryPolicy, Session, normalize_base,
};
pub use discovery::{DiscoveredServer, discover, discover_at, parse_discovery_response};
pub use dto::*;
pub use error::{ApiError, Result};
pub use images::{ImageOptions, ImageType};
pub use playback::{
    DeviceProfile, DirectPlayProfile, PlayMethod, PlaybackInfoRequest, PlaybackReport,
    PlaybackStop, SubtitleProfile,
};
pub use query::{ItemFilter, ItemsQuery, SortBy, SortOrder};
pub use redact::redact_url;
pub use secret::Secret;
