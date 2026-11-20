//! Handgeschriebene FFI für libmpv (LGPL, nur dynamisch per `libloading` geladen).
//!
//! * [`ffi`]: C-Typen und Konstanten (client.h, render.h, render_gl.h, render_d3d11.h).
//! * [`api`]: Laden der Bibliothek und Funktionszeiger-Tabelle ([`Api`]).
//! * [`safe`]: minimale sichere Hülle ([`Mpv`], [`Event`], [`RenderContext`]).
//!
//! Die Bibliothek wird nie mitgebaut oder eingecheckt; Suchpfad siehe [`api::candidate_paths`].

pub mod api;
pub mod ffi;
pub mod safe;

pub use api::{Api, LoadError};
pub use safe::{EndReason, Event, Mpv, MpvError, PropValue, RenderContext};
