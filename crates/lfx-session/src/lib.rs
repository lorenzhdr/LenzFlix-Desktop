//! Sitzungsverwaltung für LENZFLIX Desktop.
//!
//! [`SessionManager`] verwaltet Server (Liste in der lokalen Konfiguration), Benutzer („Wer schaut?“),
//! Anmeldung per Passwort und Quick Connect, den Wechsel zwischen Benutzern ohne neues Login (Token liegt
//! im OS-Schlüsselbund), die aktive Sitzung ([`ActiveSession`]) und die Erreichbarkeit des Servers
//! (Heartbeat mit Backoff). Änderungen kommen über den Ereignisbus ([`SessionEvent`]).
//!
//! Tokens verlassen diese Crate nur im [`lfx_api::Client`]; sie werden nie geloggt.

mod error;
mod heartbeat;
mod manager;
mod quickconnect;
mod types;

pub use error::{Result, SessionError};
pub use manager::SessionManager;
pub use quickconnect::QuickConnectHandle;
pub use types::{
    ActiveSession, QuickConnectState, SessionEvent, SessionId, SessionOptions, UserChoice,
};
