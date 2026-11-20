//! Fokuszonen der Seiten (rein rechnerisch, aus den Maßen, die die Slint-Seiten melden).
//!
//! Die Zonen werden in Seitenkoordinaten (Ursprung unter der Titelleiste, unabhängig vom Scrollen) angelegt;
//! die Slint-Seiten zeigen den Fokus über `focus-row`/`focus-index` bzw. `focus-slot` an.

use lfx_nav::{FocusGraph, Rect, Target, Zone, ZoneId};

/// Navigationsleiste der Startseite.
pub const Z_NAV: ZoneId = ZoneId(0);
/// Knöpfe des Billboards.
pub const Z_HERO: ZoneId = ZoneId(1);
/// Erste Zeile; Zeile `r` hat die Id `Z_ROW0 + r`.
pub const Z_ROW0: u32 = 10;

/// Maße der Startseite in Pixeln (aus Slints `HomeGeometry`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HomeGeo {
    pub width: f32,
    pub nav_h: f32,
    pub hero_h: f32,
    pub row_h: f32,
    pub gutter: f32,
    pub pitch: f32,
    pub card_w: f32,
    pub card_h: f32,
    pub play_x: f32,
    pub play_y: f32,
    pub play_w: f32,
    pub info_x: f32,
    pub info_w: f32,
    pub btn_h: f32,
}

impl From<lfx_ui::HomeGeometry> for HomeGeo {
    fn from(g: lfx_ui::HomeGeometry) -> Self {
        Self {
            width: g.width,
            nav_h: g.nav_h,
            hero_h: g.hero_h,
            row_h: g.row_h,
            gutter: g.gutter,
            pitch: g.pitch,
            card_w: g.card_w,
            card_h: g.card_h,
            play_x: g.play_x,
            play_y: g.play_y,
            play_w: g.play_w,
            info_x: g.info_x,
            info_w: g.info_w,
            btn_h: g.btn_h,
        }
    }
}

/// Zone einer Zeile.
pub fn row_zone(r: usize) -> ZoneId {
    ZoneId(Z_ROW0 + r as u32)
}

/// Zeilennummer einer Zone, falls es eine Zeile ist.
pub fn zone_row(z: ZoneId) -> Option<usize> {
    z.0.checked_sub(Z_ROW0).map(|r| r as usize)
}

/// Baut den Fokusgraphen der Startseite: Navigationsleiste, Billboard-Knöpfe (nur mit Titel) und Zeilen.
pub fn home_graph(
    geo: &HomeGeo,
    nav_count: usize,
    has_hero: bool,
    row_lens: &[usize],
) -> FocusGraph {
    let mut g = FocusGraph::new();
    let nav_items: Vec<Rect> = (0..nav_count)
        .map(|i| Rect::new(geo.gutter + i as f32 * 130.0, 6.0, 120.0, geo.nav_h - 12.0))
        .collect();
    g.register_zone(
        Zone::horizontal(Z_NAV, Rect::new(0.0, 0.0, geo.width, geo.nav_h), nav_count)
            .with_items(nav_items)
            .remembering(true),
    );
    if has_hero {
        let play = Rect::new(geo.play_x, geo.play_y, geo.play_w, geo.btn_h);
        let info = Rect::new(geo.info_x, geo.play_y, geo.info_w, geo.btn_h);
        g.register_zone(
            Zone::horizontal(
                Z_HERO,
                Rect::new(
                    geo.play_x,
                    geo.play_y,
                    geo.info_x + geo.info_w - geo.play_x,
                    geo.btn_h,
                ),
                2,
            )
            .with_items(vec![play, info]),
        );
    }
    for (r, &len) in row_lens.iter().enumerate() {
        let y = geo.nav_h + geo.hero_h + r as f32 * geo.row_h;
        let items: Vec<Rect> = (0..len)
            .map(|i| {
                Rect::new(
                    geo.gutter + i as f32 * geo.pitch,
                    y + 36.0,
                    geo.card_w,
                    geo.card_h,
                )
            })
            .collect();
        let width = len as f32 * geo.pitch;
        g.register_zone(
            Zone::horizontal(
                row_zone(r),
                Rect::new(geo.gutter, y + 36.0, width.max(1.0), geo.card_h),
                len,
            )
            .with_items(items)
            .remembering(true),
        );
    }
    g
}

/// Anzeige-Werte der Startseite (`focus-row`, `focus-index`) für einen Fokus.
pub fn home_props(t: Option<Target>) -> (i32, i32) {
    match t {
        None => (-3, 0),
        Some(t) if t.zone == Z_NAV => (-2, t.index as i32),
        Some(t) if t.zone == Z_HERO => (-1, t.index as i32),
        Some(t) => (zone_row(t.zone).map_or(-3, |r| r as i32), t.index as i32),
    }
}

/// Eine einfache senkrechte Liste (Formulare). Fokus-Slot = Index.
pub fn column_graph(count: usize) -> FocusGraph {
    let mut g = FocusGraph::new();
    g.register_zone(Zone::vertical(
        ZoneId(0),
        Rect::new(0.0, 0.0, 400.0, count as f32 * 60.0),
        count,
    ));
    g
}

/// „Wer schaut?“: Profile in einer Reihe (Zone 0), darunter zwei Knöpfe (Zone 1). Slots: Profile `0..n`, Knöpfe `n..n+2`.
pub fn profiles_graph(n: usize) -> FocusGraph {
    let mut g = FocusGraph::new();
    let avatars: Vec<Rect> = (0..n)
        .map(|i| Rect::new(i as f32 * 150.0, 0.0, 120.0, 150.0))
        .collect();
    if n > 0 {
        g.register_zone(
            Zone::horizontal(ZoneId(0), Rect::new(0.0, 0.0, n as f32 * 150.0, 150.0), n)
                .with_items(avatars),
        );
    }
    let mid = (n as f32 * 150.0 / 2.0 - 190.0).max(0.0);
    g.register_zone(
        Zone::horizontal(ZoneId(1), Rect::new(mid, 220.0, 380.0, 50.0), 2).with_items(vec![
            Rect::new(mid, 220.0, 180.0, 50.0),
            Rect::new(mid + 200.0, 220.0, 180.0, 50.0),
        ]),
    );
    g
}

/// Einstellungen: Design-Raster 2×2 (Zone 0, Slots 0..3), darunter drei Knöpfe (Zone 1, Slots 4..6).
pub fn settings_graph() -> FocusGraph {
    let mut g = FocusGraph::new();
    g.register_zone(Zone::grid(
        ZoneId(0),
        Rect::new(0.0, 0.0, 400.0, 120.0),
        4,
        2,
    ));
    g.register_zone(Zone::vertical(
        ZoneId(1),
        Rect::new(0.0, 160.0, 400.0, 200.0),
        3,
    ));
    g
}

/// Slot-Nummer (Anzeige) für einen Fokus in Formular-Seiten, wobei Zone 1 bei `base` beginnt.
pub fn slot_of(t: Option<Target>, zone1_base: usize) -> i32 {
    match t {
        None => -1,
        Some(t) if t.zone.0 == 0 => t.index as i32,
        Some(t) => (zone1_base + t.index) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lfx_nav::{NavAction, NavOutcome};

    fn geo() -> HomeGeo {
        HomeGeo {
            width: 1280.0,
            nav_h: 52.0,
            hero_h: 400.0,
            row_h: 220.0,
            gutter: 48.0,
            pitch: 220.0,
            card_w: 210.0,
            card_h: 118.0,
            play_x: 48.0,
            play_y: 380.0,
            play_w: 170.0,
            info_x: 232.0,
            info_w: 190.0,
            btn_h: 44.0,
        }
    }

    #[test]
    fn walk_down_and_right() {
        let mut g = home_graph(&geo(), 8, true, &[10, 10, 3]);
        assert!(g.set_focus(Target {
            zone: Z_HERO,
            index: 0
        }));
        let r = g.apply(NavAction::Down);
        assert!(matches!(r, NavOutcome::Moved { to, .. } if to.zone == row_zone(0)));
        g.apply(NavAction::Right);
        g.apply(NavAction::Right);
        assert_eq!(g.focused().unwrap().index, 2);
        g.apply(NavAction::Down);
        assert_eq!(g.focused().unwrap().zone, row_zone(1));
        assert_eq!(g.focused().unwrap().index, 2);
        g.apply(NavAction::Up);
        g.apply(NavAction::Left);
        g.apply(NavAction::Left);
        g.apply(NavAction::Up);
        assert_eq!(g.focused().unwrap().zone, Z_HERO);
        g.apply(NavAction::Up);
        assert_eq!(g.focused().unwrap().zone, Z_NAV);
    }

    #[test]
    fn short_row_clamps_to_nearest_card() {
        let mut g = home_graph(&geo(), 8, true, &[10, 2]);
        g.set_focus(Target {
            zone: row_zone(0),
            index: 7,
        });
        g.apply(NavAction::Down);
        let f = g.focused().unwrap();
        assert_eq!(f.zone, row_zone(1));
        assert_eq!(f.index, 1);
    }

    #[test]
    fn no_hero_zone_without_hero() {
        let g = home_graph(&geo(), 8, false, &[4]);
        assert!(g.zone(Z_HERO).is_none());
        assert!(g.zone(row_zone(0)).is_some());
    }

    #[test]
    fn props_mapping() {
        assert_eq!(home_props(None), (-3, 0));
        assert_eq!(
            home_props(Some(Target {
                zone: Z_NAV,
                index: 3
            })),
            (-2, 3)
        );
        assert_eq!(
            home_props(Some(Target {
                zone: Z_HERO,
                index: 1
            })),
            (-1, 1)
        );
        assert_eq!(
            home_props(Some(Target {
                zone: row_zone(2),
                index: 5
            })),
            (2, 5)
        );
    }

    #[test]
    fn profiles_and_settings_slots() {
        let mut g = profiles_graph(3);
        g.focus_zone(ZoneId(0));
        g.apply(NavAction::Down);
        assert_eq!(
            slot_of(g.focused(), 3),
            3 + g.focused().unwrap().index as i32
        );
        let mut s = settings_graph();
        s.focus_zone(ZoneId(0));
        s.apply(NavAction::Right);
        assert_eq!(slot_of(s.focused(), 4), 1);
        s.apply(NavAction::Down);
        s.apply(NavAction::Down);
        assert!(slot_of(s.focused(), 4) >= 4);
    }

    #[test]
    fn column_moves() {
        let mut g = column_graph(5);
        g.focus_zone(ZoneId(0));
        g.apply(NavAction::Down);
        g.apply(NavAction::Down);
        assert_eq!(g.focused().unwrap().index, 2);
        g.apply(NavAction::Up);
        assert_eq!(g.focused().unwrap().index, 1);
    }
}
