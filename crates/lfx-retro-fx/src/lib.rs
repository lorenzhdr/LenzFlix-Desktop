//! UI-freie Teile der LENZ//84-Effekte (Port von `features/retro/fx/*` des Web-Themes).
//!
//! - [`synth`]: Klang-Synthesizer (Oszillatoren, Rauschen, Hüllkurven, Filter, Mischer), rein
//!   rechnend und offline in Puffer renderbar; [`output`] (Feature `audio`) spielt ihn über cpal.
//! - [`events`]: Ereignisnamen der Seiten (`emitFx`) und Vorlieben (Sound standardmäßig aus).
//! - [`boot`], [`glitch`], [`cheat`], [`terminal`], [`rain`]: Zustandsmaschinen und Zeitpläne als
//!   reine Daten und Zeitfunktionen für die spätere Slint-Darstellung. Zufall nur über [`rng`]
//!   (deterministisch per Seed).

pub mod boot;
pub mod cheat;
pub mod events;
pub mod glitch;
#[cfg(feature = "audio")]
pub mod output;
pub mod rain;
pub mod rng;
pub mod synth;
pub mod terminal;
