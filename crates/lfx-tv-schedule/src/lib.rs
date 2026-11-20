//! Deterministischer TV-Planer: bitgleicher Port von `src/js/tv/schedule/*.js` des Web-Themes.
//!
//! Der Planer ist rein: keine Uhr, kein IO, kein Zufall außer dem portierten Hash. Alle Zeiten sind
//! absolute Millisekunden (UTC); die Planzone MEZ/MESZ wird arithmetisch berechnet. Abgesichert wird
//! der Port über Golden-Vektoren (`tools/golden`), die aus dem Web-Code erzeugt werden.
//!
//! Zeichenketten werden wie in JavaScript als UTF-16-Code-Units verglichen und gehasht
//! (siehe [`hash`]); Zahlen folgen den JS-Regeln (`Math.round`, `Number(...)`, `Date.parse`).
#![forbid(unsafe_code)]

pub mod catalog;
pub mod config;
pub mod fill;
pub mod hash;
pub mod jsval;
pub mod plan;
pub mod pool;
pub mod rating;
pub mod scheduler;
pub mod series;
pub mod themeday;
pub mod time;

mod premiere;

pub use catalog::{Catalog, Episode, Item, Movie, Series, normalize_catalog};
pub use fill::{EntryType, PlanEntry};
pub use jsval::JsVal;
pub use scheduler::{Channel, NowOn, PlanError, Scheduler};
