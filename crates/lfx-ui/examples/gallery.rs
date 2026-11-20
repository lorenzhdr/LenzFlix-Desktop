//! Komponenten-Galerie: alle Bausteine in den vier Designs.
//!
//! Start: `cargo run -p lfx-ui --example gallery`.
//! Tasten: 1 LENZFLIX, 2 LENZ+, 3 LENZPRIME, 4 LENZ//84; Pfeil links/rechts wechselt die Szene
//! (Knöpfe, Karten, Zeilen, Leisten und Eingaben, Rückmeldungen). Alle Inhalte sind erfunden.

use lfx_ui::{Gallery, demo};
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let ui = Gallery::new()?;
    demo::fill_gallery(&ui);
    ui.run()
}
