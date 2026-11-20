//! UI-freie Logik der LENZFLIX-Startseite und der Bibliotheksseiten.
//!
//! Portiert aus dem Web-Theme (Semantik, nicht Struktur): Zeilen-Quellen, Billboard-Pool,
//! Datensparmodus, Bildgrößen, Bibliotheks-/Meine-Liste-Seiten, „Neu und beliebt“, Cache-Regeln.
//!
//! Grundsätze:
//! - **Rein:** keine Uhr, kein Zufall, kein Speicher, kein Netz im Kern. Zeit (`now_ms`), lokales Datum,
//!   Markenname, Design-Id und Zufallsquelle kommen von außen. Netz nur über [`ApiCall::execute`].
//! - **Abfragen sind Daten:** jede Zeile beschreibt ihre Anfrage als [`ApiCall`] (Pfad + Parameter),
//!   damit „exakt die Web-Parameter“ ohne Server prüfbar ist.
//! - **Deterministisch:** Tagesmischung und Zufall über denselben FNV-/mulberry32-Kern wie das Web
//!   (UTF-16-Code-Units), Golden-Tests gegen die JS-Funktionen (`tests/golden/`).

pub mod billboard;
pub mod cache;
pub mod call;
pub mod collate;
pub mod color; // W2-home
pub mod datasaver;
pub mod dayseed;
pub mod images;
pub mod library;
pub mod newpopular;
pub mod prefetch;
pub mod rows;
pub mod trend; // W2-home
mod util;

pub use call::{ApiCall, Endpoint};
pub use util::{decode_query_component, encode_uri_component, parse_ms};
