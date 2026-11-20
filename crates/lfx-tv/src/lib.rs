//! UI-freie Logik des LENZFLIX-TV-Modus (Seite `lfx=tv` des Web-Themes).
//!
//! Aufbau:
//! * [`controller`]: Zustandsmaschine ([`TvController`]), sans-IO mit Ausgaben ([`TvOutput`]),
//!   injizierter Uhr ([`clock::Clock`]) und Kennungen für Player-Antworten.
//! * [`model`], [`epg`], [`info_card`], [`channel_list`], [`logos`]: Anzeige-Modelle auf dem
//!   bitgleichen Planer (`lfx-tv-schedule`) über [`plan::PlanSource`].
//! * [`digits`], [`neighbors`], [`watch`], [`filler`], [`zapstats`]: Zifferneingabe, Vorwärm-
//!   Kandidaten, Gesehen-Regel, Füller/Trailer, Messanzeige.
//! * [`stream`], [`fallback`], [`tracks`], [`request`]: Einordnung der PlaybackInfo, Rückfallkette
//!   mit Fehlercodes, Spurwahl und Datentypen für den Player (`TvPlayRequest`, `report_progress=false`).
//! * [`catalog`], [`config`], [`api_source`]: Katalog laden und zwischenspeichern, Senderbelegung 5 bis 16,
//!   Adapter für `lfx-api`.
//!
//! Zeit nur injiziert, keine Lokalzeit, kein Zufall: gleiche Eingaben ergeben gleiche Ausgaben.
#![forbid(unsafe_code)]

pub mod api_source;
pub mod catalog;
pub mod channel_list;
pub mod clock;
pub mod config;
pub mod controller;
pub mod digits;
pub mod epg;
pub mod error;
pub mod fallback;
pub mod filler;
pub mod format;
pub mod info_card;
pub mod logos;
pub mod model;
pub mod neighbors;
pub mod plan;
pub mod request;
pub mod stream;
pub mod tracks;
pub mod watch;
pub mod zapstats;

#[doc(hidden)]
pub mod testing;

pub use clock::{Clock, ManualClock, SystemClock};
pub use controller::{Card, Phase, Reason, Snapshot, TvController, TvOptions, TvOutput};
pub use error::TvError;
pub use plan::{ItemInfo, ItemType, PlanSource};
pub use request::{PlayResult, Token, TvPlayRequest};
