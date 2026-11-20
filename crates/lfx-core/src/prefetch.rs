//! Regeln für Hintergrund-Vorladen und Lazy-Laden der Zeilen (Port von `rows/prefetch.js`, `core/net.js`,
//! Teile von `rows.js`). Reine Entscheidungen ohne Zeitgeber.

use crate::datasaver::Effects;

/// Spätestens dann startet das Vorladen, auch wenn „oben“ nicht fertig meldet.
pub const MAX_WAIT_MS: u64 = 15_000;
/// Wartezeit auf die Bilder der oberen Zeilen.
pub const IMG_WAIT_MS: u64 = 4_000;
/// Abfrageintervall.
pub const POLL_MS: u64 = 250;
/// Wie viele obere Zeilen als „oben“ gelten.
pub const TOP_ROWS: usize = 3;
/// Wie viele Karten je oberer Zeile beobachtet werden.
pub const TOP_CARDS: usize = 8;
/// Nach Info-Fenster/Hintergrund-TV erst nach so viel Ruhe weiter.
pub const RESUME_MS: u64 = 2_000;
/// Cache-Pfad: so viele erste Zeilen ohne Lazy/Async-Bilder.
pub const EAGER_ROWS: usize = 3;
/// Cache-Pfad: so viele Karten je Zeile sofort ladend.
pub const EAGER_CARDS: usize = 8;
/// Raster: so viele Karten der ersten (gecachten) Seite sofort ladend.
pub const GRID_EAGER_CARDS: usize = 12;

/// Netzzustand (`navigator.connection`); Desktop liefert, was die Plattform weiß.
#[derive(Debug, Clone, Default)]
pub struct NetInfo {
    /// Datensparwunsch des Systems.
    pub save_data: bool,
    /// Mobilfunk (getaktete Verbindung).
    pub cellular: bool,
    /// Effektiver Typ (`slow-2g|2g|3g|4g`), falls bekannt.
    pub effective_type: Option<String>,
}

/// `netOk`: false bei Datenspar-Wunsch, Mobilfunk oder langsamem Netz; ohne Angabe (`None`) true.
pub fn net_ok(info: Option<&NetInfo>) -> bool {
    let Some(c) = info else {
        return true;
    };
    !(c.save_data
        || c.cellular
        || matches!(c.effective_type.as_deref(), Some("slow-2g" | "2g" | "3g")))
}

/// Eingaben der Pausenregel des Vorladens.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrefetchInputs {
    /// Fenster/Tab nicht sichtbar.
    pub hidden: bool,
    /// Info-Fenster offen.
    pub modal_open: bool,
    /// TV-Modus läuft im Hintergrund-Bild-in-Bild.
    pub tv_background_playing: bool,
}

/// Vorladen erlaubt: Netzregel UND Datensparmodus (Teil `prefetch`) nicht aktiv.
pub fn allow_prefetch(net: Option<&NetInfo>, fx: &Effects) -> bool {
    net_ok(net) && fx.prefetch
}

/// Vorladen pausiert (`paused()`).
pub fn prefetch_paused(i: &PrefetchInputs, fx: &Effects) -> bool {
    i.hidden || !fx.prefetch || i.modal_open || i.tv_background_playing
}

/// Hero fertig: kein Skelett, Bilder geladen, kein wartender Trailer.
pub fn hero_settled(skeleton: bool, trailer_waiting: bool, all_images_loaded: bool) -> bool {
    !skeleton && !trailer_waiting && all_images_loaded
}

/// Start des Vorladens: Hero fertig UND obere Zeilen bereit, oder `MAX_WAIT_MS` verstrichen.
pub fn prefetch_go(elapsed_ms: u64, hero_ok: bool, top_ready: bool) -> bool {
    elapsed_ms >= MAX_WAIT_MS || (hero_ok && top_ready)
}

/// Obere Zeilen bereit: Bilder geladen oder `IMG_WAIT_MS` nach dem Erscheinen verstrichen.
pub fn top_ready(waited_ms: u64, all_images_loaded: bool) -> bool {
    waited_ms >= IMG_WAIT_MS || all_images_loaded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasaver::{DataSaver, ImageScales};

    #[test]
    fn net_rules() {
        assert!(net_ok(None));
        assert!(!net_ok(Some(&NetInfo {
            cellular: true,
            ..Default::default()
        })));
        assert!(!net_ok(Some(&NetInfo {
            effective_type: Some("3g".into()),
            ..Default::default()
        })));
        assert!(net_ok(Some(&NetInfo {
            effective_type: Some("4g".into()),
            ..Default::default()
        })));
    }

    #[test]
    fn datasaver_blocks_prefetch() {
        let on = DataSaver {
            on: true,
            ..Default::default()
        }
        .effects(true, &ImageScales::default());
        let off = DataSaver::default().effects(true, &ImageScales::default());
        assert!(!allow_prefetch(None, &on) && allow_prefetch(None, &off));
        assert!(prefetch_paused(&PrefetchInputs::default(), &on));
        assert!(!prefetch_paused(&PrefetchInputs::default(), &off));
        assert!(prefetch_paused(
            &PrefetchInputs {
                modal_open: true,
                ..Default::default()
            },
            &off
        ));
    }

    #[test]
    fn go_rules() {
        assert!(prefetch_go(15_000, false, false));
        assert!(prefetch_go(1, true, true));
        assert!(!prefetch_go(1, true, false));
        assert!(top_ready(4_000, false) && top_ready(0, true) && !top_ready(100, false));
        assert!(hero_settled(false, false, true) && !hero_settled(false, true, true));
    }
}
