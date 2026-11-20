//! Tests des Fokusgraphen, u. a. mit einer Startseite wie bei LENZFLIX
//! (Billboard, 10 Zeilen mit je 20 Karten unterschiedlicher Breite).

use lfx_nav::{Direction, FocusGraph, NavAction, NavOutcome, Rect, Target, Zone, ZoneId};

const BILLBOARD: ZoneId = ZoneId(0);
const SCREEN_W: f32 = 1920.0;

fn row_id(i: u32) -> ZoneId {
    ZoneId(i + 1)
}

/// Kartenbreite wechselt je Index (erfundene, aber ungleichmäßige Breiten).
fn card_w(row: u32, i: usize) -> f32 {
    120.0 + ((i as u32 * 37 + row * 11) % 5) as f32 * 40.0
}

fn row_zone(row: u32, remember: bool) -> Zone {
    let y = 640.0 + row as f32 * 240.0;
    let mut x = 48.0;
    let items: Vec<Rect> = (0..20)
        .map(|i| {
            let w = card_w(row, i);
            let r = Rect::new(x, y, w, 200.0);
            x += w + 16.0;
            r
        })
        .collect();
    Zone::horizontal(row_id(row), Rect::new(0.0, y, SCREEN_W, 200.0), 0)
        .with_items(items)
        .remembering(remember)
}

fn home(remember: bool) -> FocusGraph {
    let mut g = FocusGraph::new();
    g.register_zone(Zone::horizontal(
        BILLBOARD,
        Rect::new(0.0, 80.0, SCREEN_W, 500.0),
        2,
    ));
    for r in 0..10 {
        g.register_zone(row_zone(r, remember));
    }
    g
}

fn at(zone: ZoneId, index: usize) -> Target {
    Target { zone, index }
}

#[test]
fn initial_focus_is_top_zone() {
    let mut g = home(false);
    let out = g.apply(NavAction::Down);
    assert_eq!(
        out,
        NavOutcome::Moved {
            from: None,
            to: at(BILLBOARD, 0)
        }
    );
}

#[test]
fn row_change_jumps_to_geometrically_nearest_card() {
    let mut g = home(false);
    assert!(g.set_focus(at(row_id(0), 6)));
    let from_rect = g.zone(row_id(0)).unwrap().item_rect(6);
    let out = g.apply(NavAction::Down);
    let NavOutcome::Moved { to, .. } = out else {
        panic!("erwartet Moved, war {out:?}")
    };
    assert_eq!(to.zone, row_id(1));
    // Es muss die Karte mit dem geringsten Mittelpunktsabstand sein.
    let z = g.zone(row_id(1)).unwrap();
    let cx = from_rect.center().0;
    let best = (0..20)
        .min_by(|a, b| {
            let da = (z.item_rect(*a).center().0 - cx).abs();
            let db = (z.item_rect(*b).center().0 - cx).abs();
            da.total_cmp(&db)
        })
        .unwrap();
    // entry_score gewichtet den Längsabstand leicht mit, der ist hier für alle gleich.
    assert_eq!(to.index, best);
}

#[test]
fn remembered_item_wins_on_return() {
    let mut g = home(true);
    g.set_focus(at(row_id(0), 3));
    g.apply(NavAction::Down); // Zeile 1
    g.set_focus(at(row_id(1), 12));
    g.apply(NavAction::Down); // Zeile 2
    let from = g.focused().unwrap();
    assert_eq!(from.zone, row_id(2));
    let out = g.apply(NavAction::Up);
    assert_eq!(
        out,
        NavOutcome::Moved {
            from: Some(from),
            to: at(row_id(1), 12)
        }
    );
}

#[test]
fn up_from_first_row_enters_billboard() {
    let mut g = home(false);
    g.set_focus(at(row_id(0), 0));
    let out = g.apply(NavAction::Up);
    assert!(matches!(out, NavOutcome::Moved { to, .. } if to.zone == BILLBOARD));
}

#[test]
fn walking_down_through_all_rows_then_edge_is_unhandled() {
    let mut g = home(false);
    g.set_focus(at(BILLBOARD, 0));
    for r in 0..10 {
        let out = g.apply(NavAction::Down);
        assert!(
            matches!(out, NavOutcome::Moved { to, .. } if to.zone == row_id(r)),
            "Zeile {r}"
        );
    }
    assert_eq!(g.apply(NavAction::Down), NavOutcome::Unhandled);
}

#[test]
fn horizontal_moves_and_edges() {
    let mut g = home(false);
    g.set_focus(at(row_id(0), 18));
    assert_eq!(
        g.apply(NavAction::Right),
        NavOutcome::Moved {
            from: Some(at(row_id(0), 18)),
            to: at(row_id(0), 19)
        }
    );
    assert_eq!(g.apply(NavAction::Right), NavOutcome::Unhandled);
    g.set_focus(at(row_id(0), 0));
    assert_eq!(g.apply(NavAction::Left), NavOutcome::Unhandled);
}

#[test]
fn wrap_goes_around() {
    let mut g = FocusGraph::new();
    g.register_zone(
        Zone::horizontal(ZoneId(1), Rect::new(0.0, 0.0, 400.0, 50.0), 4).wrapping(true),
    );
    g.set_focus(at(ZoneId(1), 3));
    assert!(matches!(g.apply(NavAction::Right), NavOutcome::Moved { to, .. } if to.index == 0));
    assert!(matches!(g.apply(NavAction::Left), NavOutcome::Moved { to, .. } if to.index == 3));
}

#[test]
fn grid_navigation_with_partial_last_row() {
    let mut g = FocusGraph::new();
    // 7 Items, 3 Spalten: Zeilen 0..2, letzte Zeile nur Index 6.
    g.register_zone(Zone::grid(
        ZoneId(1),
        Rect::new(0.0, 0.0, 300.0, 300.0),
        7,
        3,
    ));
    g.set_focus(at(ZoneId(1), 4));
    assert!(matches!(g.apply(NavAction::Down), NavOutcome::Moved { to, .. } if to.index == 6));
    assert_eq!(g.apply(NavAction::Down), NavOutcome::Unhandled);
    assert!(matches!(g.apply(NavAction::Up), NavOutcome::Moved { to, .. } if to.index == 3));
    g.set_focus(at(ZoneId(1), 2));
    assert!(matches!(g.apply(NavAction::Right), NavOutcome::Unhandled));
    assert!(matches!(g.apply(NavAction::Left), NavOutcome::Moved { to, .. } if to.index == 1));
}

#[test]
fn page_keys_step_by_page_size() {
    let mut g = FocusGraph::new();
    let mut z = Zone::vertical(ZoneId(1), Rect::new(0.0, 0.0, 100.0, 1000.0), 30);
    z.page_size = 10;
    g.register_zone(z);
    g.set_focus(at(ZoneId(1), 2));
    assert!(matches!(g.apply(NavAction::PageDown), NavOutcome::Moved { to, .. } if to.index == 12));
    assert!(matches!(g.apply(NavAction::PageUp), NavOutcome::Moved { to, .. } if to.index == 2));
    assert_eq!(
        g.apply(NavAction::PageUp),
        NavOutcome::Moved {
            from: Some(at(ZoneId(1), 2)),
            to: at(ZoneId(1), 0)
        }
    );
    assert_eq!(g.apply(NavAction::PageUp), NavOutcome::Unhandled);
}

#[test]
fn explicit_neighbor_beats_geometry() {
    let mut g = FocusGraph::new();
    // Geometrisch läge "b" unten, explizit führt Down aber zu "c".
    g.register_zone(
        Zone::horizontal(ZoneId(1), Rect::new(0.0, 0.0, 100.0, 50.0), 2)
            .neighbor(Direction::Down, ZoneId(3)),
    );
    g.register_zone(Zone::horizontal(
        ZoneId(2),
        Rect::new(0.0, 60.0, 100.0, 50.0),
        2,
    ));
    g.register_zone(Zone::horizontal(
        ZoneId(3),
        Rect::new(0.0, 500.0, 100.0, 50.0),
        2,
    ));
    g.set_focus(at(ZoneId(1), 0));
    assert!(
        matches!(g.apply(NavAction::Down), NavOutcome::Moved { to, .. } if to.zone == ZoneId(3))
    );
}

#[test]
fn missing_explicit_neighbor_falls_back_to_geometry() {
    let mut g = FocusGraph::new();
    g.register_zone(
        Zone::horizontal(ZoneId(1), Rect::new(0.0, 0.0, 100.0, 50.0), 2)
            .neighbor(Direction::Down, ZoneId(99)),
    );
    g.register_zone(Zone::horizontal(
        ZoneId(2),
        Rect::new(0.0, 60.0, 100.0, 50.0),
        2,
    ));
    g.set_focus(at(ZoneId(1), 0));
    assert!(
        matches!(g.apply(NavAction::Down), NavOutcome::Moved { to, .. } if to.zone == ZoneId(2))
    );
}

#[test]
fn aligned_zone_beats_nearer_unaligned_one() {
    let mut g = FocusGraph::new();
    g.register_zone(Zone::horizontal(
        ZoneId(1),
        Rect::new(0.0, 0.0, 100.0, 50.0),
        1,
    ));
    // Näher, aber seitlich versetzt:
    g.register_zone(Zone::horizontal(
        ZoneId(2),
        Rect::new(300.0, 60.0, 100.0, 50.0),
        1,
    ));
    // Weiter weg, aber darunter:
    g.register_zone(Zone::horizontal(
        ZoneId(3),
        Rect::new(0.0, 400.0, 100.0, 50.0),
        1,
    ));
    g.set_focus(at(ZoneId(1), 0));
    assert!(
        matches!(g.apply(NavAction::Down), NavOutcome::Moved { to, .. } if to.zone == ZoneId(3))
    );
}

fn modal_graph() -> FocusGraph {
    let mut g = home(false);
    g.register_zone(Zone::vertical(
        ZoneId(100),
        Rect::new(600.0, 300.0, 400.0, 300.0),
        3,
    ));
    g.register_zone(Zone::horizontal(
        ZoneId(101),
        Rect::new(600.0, 620.0, 400.0, 60.0),
        2,
    ));
    g
}

#[test]
fn modal_trap_keeps_focus_inside_and_restores() {
    let mut g = modal_graph();
    g.set_focus(at(row_id(2), 5));
    g.push_trap(&[ZoneId(100), ZoneId(101)]);
    assert_eq!(g.focused(), Some(at(ZoneId(100), 0)));
    // Hoch am oberen Rand: nirgends hin, der Fokus bleibt im Modal.
    assert_eq!(g.apply(NavAction::Up), NavOutcome::Unhandled);
    assert_eq!(g.focused(), Some(at(ZoneId(100), 0)));
    // Unten in die zweite Modal-Zone.
    g.apply(NavAction::Down);
    g.apply(NavAction::Down);
    g.apply(NavAction::Down);
    assert_eq!(g.focused().unwrap().zone, ZoneId(101));
    // Von dort weiter nach unten: kein Ausbruch in die Startseite.
    assert_eq!(g.apply(NavAction::Down), NavOutcome::Unhandled);
    // Hover und set_focus außerhalb sind abgewiesen.
    assert_eq!(g.hover(at(row_id(0), 0)), NavOutcome::Unhandled);
    assert!(!g.set_focus(at(row_id(0), 0)));
    g.pop_trap();
    assert_eq!(g.focused(), Some(at(row_id(2), 5)));
    assert!(!g.trap_active());
}

#[test]
fn explicit_neighbor_outside_trap_is_ignored() {
    let mut g = FocusGraph::new();
    g.register_zone(
        Zone::horizontal(ZoneId(1), Rect::new(0.0, 0.0, 100.0, 50.0), 2)
            .neighbor(Direction::Down, ZoneId(2)),
    );
    g.register_zone(Zone::horizontal(
        ZoneId(2),
        Rect::new(0.0, 60.0, 100.0, 50.0),
        2,
    ));
    g.push_trap(&[ZoneId(1)]);
    assert_eq!(g.apply(NavAction::Down), NavOutcome::Unhandled);
}

#[test]
fn hover_sets_focus_and_reports_change_once() {
    let mut g = home(false);
    let t = at(row_id(3), 7);
    assert_eq!(g.hover(t), NavOutcome::Moved { from: None, to: t });
    assert_eq!(g.hover(t), NavOutcome::Unhandled);
    // Tastatur geht von der gehoverten Karte aus weiter.
    assert!(
        matches!(g.apply(NavAction::Right), NavOutcome::Moved { to, .. } if to == at(row_id(3), 8))
    );
}

#[test]
fn activate_and_back() {
    let mut g = home(false);
    assert_eq!(g.apply(NavAction::Activate), NavOutcome::Unhandled);
    g.set_focus(at(row_id(1), 4));
    assert_eq!(
        g.apply(NavAction::Activate),
        NavOutcome::Activated(at(row_id(1), 4))
    );
    assert_eq!(g.apply(NavAction::Back), NavOutcome::Back);
    assert_eq!(g.apply(NavAction::PlayPause), NavOutcome::Unhandled);
}

#[test]
fn history_restores_focus_for_back() {
    let mut g = home(false);
    g.set_focus(at(row_id(4), 9));
    g.push_history();
    g.set_focus(at(row_id(0), 0));
    assert!(g.pop_history());
    assert_eq!(g.focused(), Some(at(row_id(4), 9)));
    assert!(!g.pop_history());
}

#[test]
fn removing_and_updating_zones_keeps_focus_valid() {
    let mut g = home(false);
    g.set_focus(at(row_id(0), 15));
    // Zeile schrumpft auf 5 Karten: Fokus wird geklemmt.
    let z = Zone::horizontal(row_id(0), Rect::new(0.0, 640.0, SCREEN_W, 200.0), 5);
    assert!(g.update_zone(z));
    assert_eq!(g.focused(), Some(at(row_id(0), 4)));
    assert!(!g.update_zone(Zone::horizontal(ZoneId(999), Rect::default(), 1)));
    g.remove_zone(row_id(0));
    assert_eq!(g.focused(), None);
    // Leere Zonen werden übersprungen.
    g.register_zone(Zone::horizontal(
        ZoneId(500),
        Rect::new(0.0, 0.0, 10.0, 10.0),
        0,
    ));
    assert!(
        matches!(g.apply(NavAction::Down), NavOutcome::Moved { to, .. } if to.zone == BILLBOARD)
    );
}

#[test]
fn remembered_index_survives_update_and_is_clamped() {
    let mut g = home(true);
    g.set_focus(at(row_id(1), 18));
    g.set_focus(at(BILLBOARD, 0));
    let z =
        Zone::horizontal(row_id(1), Rect::new(0.0, 880.0, SCREEN_W, 200.0), 10).remembering(true);
    g.update_zone(z);
    assert_eq!(g.remembered(row_id(1)), Some(9));
}
