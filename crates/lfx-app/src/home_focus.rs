//! Fokuszonen der Startseite (W2-home): Kopfleiste (9 Ziele), Billboard-Knöpfe und Zeilen mit unterschiedlicher
//! Höhe (die Top-10-Zeile ist höher). Rein rechnerisch aus den Maßen, die `HomePage` als `HomeLayout` meldet.

use lfx_nav::{FocusGraph, Rect, Zone};
use lfx_ui::HomeLayout;

use crate::focus::{Z_HERO, Z_NAV, row_zone};

/// Ziele der Kopfleiste: 0..=4 Links (Start, Serien, Filme, Neu und beliebt, Meine Liste), 5 Suche, 6 TV,
/// 7 Datensparmodus, 8 Profil.
pub const NAV_COUNT: usize = 9;
/// Beschriftungen der Links (Breite für die Fokusflächen).
const LINKS: [&str; 5] = ["Start", "Serien", "Filme", "Neu und beliebt", "Meine Liste"];

/// Eine Zeile für den Graphen.
#[derive(Debug, Clone, Copy)]
pub struct RowShape {
    pub len: usize,
    pub top10: bool,
}

/// Anzahl Top-10-Zeilen vor jeder Zeile.
pub fn tall_before(rows: &[RowShape]) -> Vec<i32> {
    let mut n = 0;
    rows.iter()
        .map(|r| {
            let before = n;
            n += i32::from(r.top10);
            before
        })
        .collect()
}

/// Obere Kante der Zeile `r` in Seitenkoordinaten (ohne Scroll).
pub fn row_top(l: &HomeLayout, tall_before: i32, r: usize) -> f32 {
    l.rows_top + r as f32 * l.row_h + tall_before as f32 * l.top_extra
}

/// Baut den Fokusgraphen.
pub fn home_graph(l: &HomeLayout, has_hero: bool, rows: &[RowShape]) -> FocusGraph {
    let mut g = FocusGraph::new();
    // Kopfleiste: Links ab dem Logo, rechts vier Symbole.
    let mut x = l.gutter + 96.0;
    let mut items: Vec<Rect> = Vec::with_capacity(NAV_COUNT);
    for text in LINKS {
        let w = text.chars().count() as f32 * 8.6 + 32.0;
        items.push(Rect::new(x, 20.0, w, 40.0));
        x += w + 4.0;
    }
    let mut rx = l.width - l.gutter - (3.0 * 44.0 + 48.0);
    for _ in 0..3 {
        items.push(Rect::new(rx, 20.0, 40.0, 40.0));
        rx += 44.0;
    }
    items.push(Rect::new(rx, 24.0, 40.0, 32.0));
    g.register_zone(
        Zone::horizontal(Z_NAV, Rect::new(0.0, 0.0, l.width, l.header_h), NAV_COUNT)
            .with_items(items)
            .remembering(true),
    );
    if has_hero {
        let play = Rect::new(l.play_x, l.play_y, l.play_w, l.btn_h);
        let info = Rect::new(l.info_x, l.play_y, l.info_w, l.btn_h);
        g.register_zone(
            Zone::horizontal(
                Z_HERO,
                Rect::new(l.play_x, l.play_y, l.info_x + l.info_w - l.play_x, l.btn_h),
                2,
            )
            .with_items(vec![play, info]),
        );
    }
    let tb = tall_before(rows);
    for (r, shape) in rows.iter().enumerate() {
        let y = row_top(l, tb[r], r) + l.head_h;
        let h = if shape.top10 { l.top_card_h } else { l.card_h };
        let cards: Vec<Rect> = (0..shape.len)
            .map(|i| Rect::new(l.gutter + i as f32 * l.pitch, y, l.card_w, h))
            .collect();
        let width = (shape.len as f32 * l.pitch).max(1.0);
        g.register_zone(
            Zone::horizontal(row_zone(r), Rect::new(l.gutter, y, width, h), shape.len)
                .with_items(cards)
                .remembering(true),
        );
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> HomeLayout {
        HomeLayout {
            width: 1472.0,
            header_h: 80.0,
            gutter: 44.0,
            bb_x: 44.0,
            bb_y: 86.0,
            bb_w: 1384.0,
            bb_h: 634.0,
            play_x: 72.0,
            play_y: 600.0,
            play_w: 178.0,
            info_x: 262.0,
            info_w: 184.0,
            btn_h: 48.0,
            rows_top: 744.0,
            row_h: 228.0,
            top_extra: 40.0,
            pitch: 283.0,
            card_w: 275.0,
            card_h: 155.0,
            top_card_h: 192.0,
            head_h: 33.0,
        }
    }

    #[test]
    fn top10_rows_shift_following_rows() {
        let rows = [
            RowShape {
                len: 5,
                top10: false,
            },
            RowShape {
                len: 10,
                top10: true,
            },
            RowShape {
                len: 8,
                top10: false,
            },
        ];
        assert_eq!(tall_before(&rows), vec![0, 0, 1]);
        let l = layout();
        let tb = tall_before(&rows);
        assert_eq!(row_top(&l, tb[2], 2), 744.0 + 2.0 * 228.0 + 40.0);
        let g = home_graph(&l, true, &rows);
        assert!(g.focused().is_none());
    }

    #[test]
    fn nav_has_nine_targets() {
        let g = home_graph(&layout(), false, &[]);
        assert!(g.zone(Z_NAV).is_some());
    }
}
