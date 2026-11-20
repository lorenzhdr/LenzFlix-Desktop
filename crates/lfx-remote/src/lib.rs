//! Fernsteuerung und SyncPlay für LENZFLIX Desktop (UI-frei).
//!
//! - [`message`]: typisierte Nachrichten des Session-WebSockets (`/socket`).
//! - [`capabilities`]: Fähigkeitsmeldung (`POST /Sessions/Capabilities/Full`).
//! - [`syncplay`]: Sans-IO-Kern (Ausführungszeit, Drift-Korrektur, Ready/Buffering); [`timesync`]: Uhrabgleich;
//!   [`syncplay_api`]: REST der Gruppenverwaltung.
//! - [`session`]: Laufzeit mit Wiederverbindung und Ereignis-Strom; [`player`]: Trait [`PlayerControl`] für die App.
//!
//! Tokens und Server-URLs werden nie geloggt oder in Fehlern ausgegeben.

pub mod capabilities;
pub mod enums;
pub mod message;
pub mod player;
pub mod session;
pub mod syncplay;
pub mod syncplay_api;
pub mod time;
pub mod timesync;
pub mod ws;

pub use capabilities::Capabilities;
pub use enums::*;
pub use message::*;
pub use player::{NullPlayer, PlayerAction, PlayerControl, PlayerSnapshot, RecordingPlayer};
pub use session::{
    Backoff, ConnectionState, DisconnectReason, RemoteConfig, RemoteEvent, RemoteHandle,
    RemoteSession, SyncPlayEvent,
};
pub use syncplay::{DriftConfig, ReadyRequest, Scheduled, SyncCore, SyncReport};
pub use syncplay_api::SyncPlayApi;
pub use time::{Clock, ManualClock, SystemClock};
pub use timesync::{TimeSample, TimeSync, TimeSyncPolicy};
pub use ws::{WsAuth, WsError};
