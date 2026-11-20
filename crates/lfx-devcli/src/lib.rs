//! Entwickler-Werkzeug `lfx-dev` für LENZFLIX Desktop.
//!
//! - [`commands`]: `login` (Quick Connect), `whoami`, `servers`, `logout`, `discover`.
//! - [`g4`]: Live-Prüflauf (Gate G4) mit Wiederherstellung der Benutzerdaten.
//! - [`branding`]: Auswertung des Branding-CSS (`--lfx-charts-url`, `--lfx-stats-url`, `--lfx-tv-channels`).

pub mod branding;
pub mod commands;
pub mod g4;

pub use commands::Ctx;
