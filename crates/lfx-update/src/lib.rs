//! Auto-Update für LENZFLIX Desktop über GitHub-Releases (`lorenzhdr/LenzFlix-Desktop`).
//!
//! Ablauf: [`Updater::check`] lädt `latest.json` (siehe [`manifest`]) und vergleicht per SemVer,
//! [`Updater::download`] lädt mit Fortschritt und prüft Größe, SHA-256 und Minisign-Signatur gegen
//! [`verify::UPDATE_PUBLIC_KEY`], [`install::start_install`] startet danach den stillen Installer
//! (Windows NSIS) bzw. das Austausch-Skript (macOS); die App beendet sich anschließend selbst.
//! [`settings::UpdateSettings`] hält Kanal, Intervall, „später erinnern“ und Abschalten.
//!
//! Schlüssel: Das Schlüsselpaar erzeugt der Nutzer (`minisign -G`). Der private Schlüssel kommt nur
//! als GitHub-Secret in die Release-Pipeline, nie ins Repo. Bis dahin steht ein Platzhalter im Code
//! und der Updater verweigert Downloads.

pub mod client;
pub mod error;
pub mod install;
pub mod manifest;
pub mod settings;
pub mod verify;
pub mod version;

pub use client::{UpdateConfig, UpdateInfo, Updater};
pub use error::UpdateError;
pub use manifest::{Asset, Manifest, current_platform, platform_for_target};
pub use settings::{Channel, UpdateSettings, unix_now};
pub use version::is_newer;
