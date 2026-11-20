//! Datensparmodus (Port von `core/datasaver.js` und `core/imgscale.js`).
//!
//! Zustand: Schalter + abgeschaltete Teile (der Speicher hält die **abgeschalteten** Teile, ein unbekannter
//! Teil zählt als an). Wirkt nur in `flix|plus|prime|retro` (VIDEOTHEK ausgenommen). Die Persistenz
//! (eigener App-Speicher, nie Jellyfin-DisplayPreferences) liegt außerhalb dieses Crates.

use crate::util::js_round;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Teilschalter (`DATASAVER_PARTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    /// Keine automatischen Trailer.
    Trailer,
    /// Hero nur 5 Titel.
    Hero,
    /// Kleinere Bilder.
    Images,
    /// Kein Vorladen der Startseite.
    Prefetch,
    /// Zeilen erst kurz vor Sichtbarkeit.
    Lazy,
    /// TV: kein Vorwärmen.
    #[serde(rename = "tvwarm")]
    TvWarm,
}

impl Part {
    /// Alle Teile in Web-Reihenfolge.
    pub const ALL: [Part; 6] = [
        Part::Trailer,
        Part::Hero,
        Part::Images,
        Part::Prefetch,
        Part::Lazy,
        Part::TvWarm,
    ];

    /// Schlüssel wie im Web (`trailer|hero|images|prefetch|lazy|tvwarm`).
    pub fn key(self) -> &'static str {
        match self {
            Part::Trailer => "trailer",
            Part::Hero => "hero",
            Part::Images => "images",
            Part::Prefetch => "prefetch",
            Part::Lazy => "lazy",
            Part::TvWarm => "tvwarm",
        }
    }

    /// Teil aus Schlüssel (unbekannt = `None`).
    pub fn from_key(key: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Anzeigetext (`DATASAVER_PART_LABELS`).
    pub fn label(self) -> &'static str {
        match self {
            Part::Trailer => "Keine automatischen Trailer",
            Part::Hero => "Hero nur 5 Titel",
            Part::Images => "Kleinere Bilder",
            Part::Prefetch => "Kein Vorladen der Startseite",
            Part::Lazy => "Zeilen erst kurz vor Sichtbarkeit",
            Part::TvWarm => "TV: kein Vorwärmen",
        }
    }
}

/// Designs, in denen der Modus verfügbar ist (`DATASAVER_THEMES`).
pub const DATASAVER_THEMES: [&str; 4] = ["flix", "plus", "prime", "retro"];

/// Ist der Modus im Design (`flix|plus|prime|retro`) verfügbar?
pub fn available_for_design(design_id: &str) -> bool {
    DATASAVER_THEMES.contains(&design_id)
}

/// Gespeicherter Zustand je Gerät und Benutzer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DataSaver {
    /// Schalter (Standard aus).
    pub on: bool,
    /// Abgeschaltete Teile.
    pub disabled_parts: BTreeSet<Part>,
}

impl DataSaver {
    /// Modus wirkt: an UND im Design verfügbar (`isDataSaver()`).
    pub fn is_active(&self, available: bool) -> bool {
        available && self.on
    }

    /// Teil wirkt: aktiv UND Teil nicht abgeschaltet (`isDataSaver(part)`).
    pub fn is_part_active(&self, available: bool, part: Part) -> bool {
        self.is_active(available) && !self.disabled_parts.contains(&part)
    }

    /// Zustand aller Teile (`getDataSaverParts`: `true` = Teil spart).
    pub fn parts_state(&self) -> [(Part, bool); 6] {
        Part::ALL.map(|p| (p, !self.disabled_parts.contains(&p)))
    }

    /// Teil ein-/ausschalten (`setDataSaverPart`).
    pub fn set_part(&mut self, part: Part, on: bool) {
        if on {
            self.disabled_parts.remove(&part);
        } else {
            self.disabled_parts.insert(part);
        }
    }

    /// Alle Teile wieder an (`resetDataSaverParts`).
    pub fn reset_parts(&mut self) {
        self.disabled_parts.clear();
    }

    /// Liest eine Web-Teileliste (JSON-Array der abgeschalteten Teile); Unbekanntes wird verworfen.
    pub fn parse_disabled_parts(json: &str) -> BTreeSet<Part> {
        serde_json::from_str::<Vec<String>>(json)
            .map(|v| v.iter().filter_map(|k| Part::from_key(k)).collect())
            .unwrap_or_default()
    }

    /// Abgeleitete Wirkungen für die Oberfläche.
    pub fn effects(&self, available: bool, scales: &ImageScales) -> Effects {
        let part = |p| self.is_part_active(available, p);
        Effects {
            active: self.is_active(available),
            auto_trailer: !part(Part::Trailer),
            hero_pool_cap: part(Part::Hero).then_some(crate::billboard::SAVER_POOL),
            image_scale: if part(Part::Images) {
                scales.saver()
            } else {
                scales.normal()
            },
            prefetch: !part(Part::Prefetch),
            row_lazy_margin_px: if part(Part::Lazy) { 100 } else { 600 },
            grid_lazy_margin_px: if part(Part::Lazy) { 100 } else { 800 },
            tv_warm: !part(Part::TvWarm),
        }
    }
}

/// Wirkungen des Datensparmodus (Vorgabewerte = Verhalten ohne Modus).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effects {
    /// Modus wirkt.
    pub active: bool,
    /// Billboard darf lokale Trailer automatisch starten.
    pub auto_trailer: bool,
    /// Höchstzahl Billboard-Titel (`Some(5)` im Modus).
    pub hero_pool_cap: Option<usize>,
    /// Skala für Bildbreiten.
    pub image_scale: f64,
    /// Hintergrund-Vorladen der Zeilen erlaubt.
    pub prefetch: bool,
    /// Lazy-Rand der Zeilen in px (100 statt 600).
    pub row_lazy_margin_px: i32,
    /// Lazy-Rand des Rasters in px (100 statt 800).
    pub grid_lazy_margin_px: i32,
    /// TV-Modus darf vorwärmen.
    pub tv_warm: bool,
}

/// Standard-Skala im Datensparmodus (exakt 2/3).
pub const DATASAVER_SCALE: f64 = 2.0 / 3.0;
/// Bereich der normalen Skala.
pub const NORMAL_RANGE: (f64, f64) = (0.5, 2.0);
/// Bereich der Spar-Skala.
pub const SAVER_RANGE: (f64, f64) = (0.25, 1.0);

fn fix(v: f64, (lo, hi): (f64, f64)) -> f64 {
    (js_round(v * 20.0) / 20.0).clamp(lo, hi)
}

/// Bildgrößen-Regler (`lfx.imgscale.<user>` = `{normal, saver}`); `None` = Standard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageScales {
    /// Eigene normale Skala.
    pub normal: Option<f64>,
    /// Eigene Spar-Skala.
    pub saver: Option<f64>,
}

impl ImageScales {
    /// Normale Skala (Standard 1; 0,5–2 in 0,05er-Schritten).
    pub fn normal(&self) -> f64 {
        self.normal
            .filter(|v| v.is_finite())
            .map_or(1.0, |v| fix(v, NORMAL_RANGE))
    }

    /// Spar-Skala (Standard exakt 2/3; 0,25–1 in 0,05er-Schritten).
    pub fn saver(&self) -> f64 {
        self.saver
            .filter(|v| v.is_finite())
            .map_or(DATASAVER_SCALE, |v| fix(v, SAVER_RANGE))
    }

    /// Mindestens ein Wert gesetzt (`isImageScaleCustom`).
    pub fn is_custom(&self) -> bool {
        self.normal.is_some() || self.saver.is_some()
    }

    /// Werte setzen (klemmen, auf 0,05 runden); nicht übergebene bleiben (`setImageScales`).
    pub fn set(&mut self, normal: Option<f64>, saver: Option<f64>) {
        if let Some(v) = normal.filter(|v| v.is_finite()) {
            self.normal = Some(fix(v, NORMAL_RANGE));
        }
        if let Some(v) = saver.filter(|v| v.is_finite()) {
            self.saver = Some(fix(v, SAVER_RANGE));
        }
    }
}

/// `scaled`: Breite/Höhe × Skala, gerundet, mindestens 1; Skala außerhalb (0, 2] gilt als 1.
pub fn scaled(v: u32, scale: f64) -> u32 {
    let f = if scale > 0.0 && scale <= 2.0 {
        scale
    } else {
        1.0
    };
    if v == 0 || f == 1.0 {
        v
    } else {
        js_round(f64::from(v) * f).max(1.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_follow_web_semantics() {
        let mut d = DataSaver {
            on: true,
            ..Default::default()
        };
        assert!(d.is_part_active(true, Part::Trailer));
        assert!(
            !d.is_part_active(false, Part::Trailer),
            "store: wirkungslos"
        );
        d.set_part(Part::Trailer, false);
        assert!(!d.is_part_active(true, Part::Trailer));
        assert!(d.is_active(true));
        d.reset_parts();
        assert!(d.is_part_active(true, Part::Trailer));
    }

    #[test]
    fn disabled_parts_parse_drops_unknown() {
        let s = DataSaver::parse_disabled_parts(r#"["hero","x","lazy"]"#);
        assert_eq!(s, BTreeSet::from([Part::Hero, Part::Lazy]));
        assert!(DataSaver::parse_disabled_parts("kaputt").is_empty());
    }

    #[test]
    fn scales_and_effects() {
        let s = ImageScales::default();
        assert_eq!(s.normal(), 1.0);
        assert_eq!(s.saver(), 2.0 / 3.0);
        let d = DataSaver {
            on: true,
            ..Default::default()
        };
        let e = d.effects(true, &s);
        assert!(!e.auto_trailer && !e.prefetch && !e.tv_warm);
        assert_eq!(e.hero_pool_cap, Some(5));
        assert_eq!((e.row_lazy_margin_px, e.grid_lazy_margin_px), (100, 100));
        let off = DataSaver::default().effects(true, &s);
        assert!(off.auto_trailer && off.prefetch);
        assert_eq!(
            (off.row_lazy_margin_px, off.grid_lazy_margin_px),
            (600, 800)
        );
        assert_eq!(off.image_scale, 1.0);
    }

    #[test]
    fn scale_clamps_and_rounds() {
        let mut s = ImageScales::default();
        s.set(Some(3.0), Some(0.1));
        assert_eq!((s.normal(), s.saver()), (2.0, 0.25));
        s.set(Some(1.234), None);
        assert_eq!(s.normal(), 1.25);
        assert_eq!(scaled(600, DATASAVER_SCALE), 400);
        assert_eq!(scaled(1, 0.25), 1);
        assert_eq!(scaled(300, 5.0), 300);
    }

    #[test]
    fn design_availability() {
        assert!(available_for_design("retro"));
        assert!(!available_for_design("store"));
    }
}
