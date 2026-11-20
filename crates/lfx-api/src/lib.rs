//! Jellyfin-API-Client für LENZFLIX Desktop.
//!
//! - [`Client`]: Anfrage-Pipeline mit Auth-Header, Retry (nur GET), geschwärzten Logs.
//! - Tokens stecken immer in [`Secret`] (Debug/Display = `***`), URLs im Log nur über [`redact_url`].
//! - Bild- und Stream-URLs enthalten nie das Token; Laden mit Header ([`Client::fetch_bytes`]).

pub mod branding;
pub mod charts;
mod client;
pub mod discovery;
pub mod dto;
mod dto_ext;
mod endpoints;
mod error;
mod images;
mod library_ext;
mod livetv_user_admin;
mod playback;
mod query;
mod redact;
mod secret;
mod w4_account; // W4-account

pub use branding::{Branding, parse_branding};
pub use charts::{
    ChartEntry, Charts, ChartsClient, MeStats, StatsBinge, StatsBucket, StatsCompare, StatsDevice,
    StatsHeat, StatsMethod, StatsMethods, StatsRange, StatsSeriesDay, StatsTimeline, StatsTop,
    StatsTopDay, StatsTotals,
};
pub use client::{
    CLIENT_NAME, Client, ClientInfo, ClientOptions, RetryPolicy, Session, normalize_base,
};
pub use discovery::{DiscoveredServer, discover, discover_at, parse_discovery_response};
pub use dto::*;
pub use dto_ext::*;
pub use error::{ApiError, Result};
pub use images::{ImageOptions, ImageType};
pub use library_ext::{AudioStreamOptions, SearchHintOptions, UserDataUpdate};
pub use livetv_user_admin::ProgramsQuery;
pub use playback::{
    DeviceProfile, DirectPlayProfile, PlayMethod, PlaybackInfoRequest, PlaybackReport,
    PlaybackStop, SubtitleProfile,
};
pub use query::{ItemFilter, ItemsQuery, SortBy, SortOrder};
pub use redact::redact_url;
pub use secret::Secret;
