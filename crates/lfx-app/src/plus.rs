//! W3-plus: Anbindung des Designs LENZ+ (Startseite mit Hero, Sammlungs-Kacheln und Poster-Zeilen, Titelseite).
//!
//! Die Daten laufen über dieselben Pfade wie bei LENZFLIX (`home.rs`); dieses Modul liefert die Besonderheiten:
//! die Reihenfolge der Kopfleisten-Links, den Fokusgraphen mit Kachelzone, das Laden der Sammlungen und die
//! Ereignisse der Umschalter-Pille.

use std::cell::RefCell;
use std::rc::Rc;

use lfx_api::ItemKind;
use lfx_core::images::{ImagePick, own_image, pick_landscape};
use lfx_core::rows::logic::base_query;
use lfx_nav::{FocusGraph, Rect, Target, Zone, ZoneId};
use lfx_ui::{HomeLayout, PlusBrand};
use slint::{Image, Model, ModelRc, SharedString, VecModel};

use crate::app::{App, post};
use crate::cli::DesignId;
use crate::focus::{Z_HERO, Z_NAV, row_zone};
use crate::home_focus::NAV_COUNT;
use crate::route::{LibraryView, Route};
use lfx_core::library::{LibSort, LibraryKind};

/// Zone der Sammlungs-Kacheln.
pub const Z_BRANDS: ZoneId = ZoneId(2);
/// Fokuszeile für die Kacheln (an Slint gemeldet).
pub const FOCUS_BRANDS: i32 = -4;
/// Anzeigereihenfolge der Links in der LENZ+-Kopfleiste: Startseite, Meine Liste, Filme, Serien, Neu & beliebt.
/// Wert = Index in der LENZFLIX-Reihenfolge (Start, Serien, Filme, Neu und beliebt, Meine Liste).
const NAV_ORDER: [i32; 5] = [0, 4, 2, 1, 3];
/// Höchstzahl der Sammlungs-Kacheln (Web: `fallbackLimit`).
const MAX_BRANDS: usize = 8;

/// Sammlungen (Id, Name) und das Modell der Kacheln.
type BrandState = (Vec<(String, String)>, Option<Rc<VecModel<PlusBrand>>>);

thread_local! {
    /// Sammlungen der Startseite (Id, Name) und das Modell der Kacheln; nur im UI-Thread.
    static BRANDS: RefCell<BrandState> =
        const { RefCell::new((Vec::new(), None)) };
}

/// Position in der LENZ+-Kopfleiste -> Index der gemeinsamen Link-Reihenfolge.
pub fn pos_to_idx(pos: i32) -> i32 {
    usize::try_from(pos)
        .ok()
        .and_then(|p| NAV_ORDER.get(p).copied())
        .unwrap_or(pos)
}

/// Index der gemeinsamen Link-Reihenfolge -> Position in der LENZ+-Kopfleiste.
pub fn idx_to_pos(idx: i32) -> i32 {
    NAV_ORDER
        .iter()
        .position(|&i| i == idx)
        .map_or(idx, |p| p as i32)
}

/// Fokusanzeige der LENZ+-Startseite: wie `focus::home_props`, plus die Kachelzone.
pub fn home_props(t: Option<Target>) -> (i32, i32) {
    match t {
        Some(t) if t.zone == Z_BRANDS => (FOCUS_BRANDS, t.index as i32),
        other => crate::focus::home_props(other),
    }
}

/// Fokusgraph der LENZ+-Startseite. Die Hero-Folie ist ein einziges Ziel, die Kacheln eine Zone vor den Zeilen.
pub fn plus_graph(
    l: &HomeLayout,
    has_hero: bool,
    brand_count: usize,
    row_lens: &[usize],
) -> FocusGraph {
    let mut g = FocusGraph::new();
    // Kopfleiste: fünf Links mittig, rechts Suche, TV, Datensparmodus, Profil links (nur für die Fokusflächen).
    let center = l.width / 2.0;
    let mut items: Vec<Rect> = (0..5)
        .map(|i| Rect::new(center - 300.0 + i as f32 * 120.0, 20.0, 110.0, 26.0))
        .collect();
    let right = l.width - 28.0;
    items.push(Rect::new(right - 36.0, 15.0, 36.0, 36.0)); // Suche
    items.push(Rect::new(right - 82.0, 15.0, 36.0, 36.0)); // TV
    items.push(Rect::new(right - 128.0, 15.0, 36.0, 36.0)); // Datensparmodus
    items.push(Rect::new(20.0, 7.0, 200.0, 52.0)); // Profil
    debug_assert_eq!(items.len(), NAV_COUNT);
    g.register_zone(
        Zone::horizontal(Z_NAV, Rect::new(0.0, 0.0, l.width, l.header_h), NAV_COUNT)
            .with_items(items)
            .remembering(true),
    );
    if has_hero {
        g.register_zone(
            Zone::horizontal(Z_HERO, Rect::new(l.bb_x, l.bb_y, l.bb_w, l.bb_h), 1)
                .with_items(vec![Rect::new(l.bb_x, l.bb_y, l.bb_w, l.bb_h)]),
        );
    }
    if brand_count > 0 {
        let tile_h = l.card_w * 9.0 / 16.0;
        let y = l.rows_top - (tile_h + 56.0) + 14.0;
        let cards: Vec<Rect> = (0..brand_count)
            .map(|i| Rect::new(l.gutter + i as f32 * l.pitch, y, l.card_w, tile_h))
            .collect();
        let width = (brand_count as f32 * l.pitch).max(1.0);
        g.register_zone(
            Zone::horizontal(Z_BRANDS, Rect::new(l.gutter, y, width, tile_h), brand_count)
                .with_items(cards)
                .remembering(true),
        );
    }
    for (r, &len) in row_lens.iter().enumerate() {
        let y = l.rows_top + r as f32 * l.row_h + l.head_h;
        let cards: Vec<Rect> = (0..len)
            .map(|i| Rect::new(l.gutter + i as f32 * l.pitch, y, l.card_w, l.card_h))
            .collect();
        let width = (len as f32 * l.pitch).max(1.0);
        g.register_zone(
            Zone::horizontal(row_zone(r), Rect::new(l.gutter, y, width, l.card_h), len)
                .with_items(cards)
                .remembering(true),
        );
    }
    g
}

/// Anzahl der Kacheln (für den Graphen).
pub fn brand_count() -> usize {
    BRANDS.with(|b| b.borrow().0.len())
}

impl App {
    /// Läuft gerade das Design LENZ+?
    pub fn is_plus(&self) -> bool {
        self.st.borrow().design == DesignId::Plus
    }

    /// Lädt die Sammlungen (BoxSets mit Etikett `lenzplus`, sonst die größten) und ihre Bilder.
    pub fn plus_load_brands(self: &Rc<Self>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        let gen_at = self.st.borrow().home.generation;
        self.rt.spawn(async move {
            let q = || {
                base_query()
                    .include_types(&[ItemKind::BoxSet])
                    .recursive(true)
                    .raw("sortBy", "SortName")
                    .raw("sortOrder", "Ascending")
            };
            let mut list = client
                .items(&q().raw("tags", "lenzplus"))
                .await
                .map(|r| r.items)
                .unwrap_or_default();
            if list.is_empty() {
                list = client
                    .items(&q())
                    .await
                    .map(|r| r.items)
                    .unwrap_or_default();
                list.sort_by_key(|i| std::cmp::Reverse(i.child_count.unwrap_or(0)));
                list.truncate(MAX_BRANDS);
            }
            post(move |app| app.plus_brands_loaded(gen_at, list));
        });
    }

    fn plus_brands_loaded(self: &Rc<Self>, gen_at: u64, list: Vec<lfx_api::BaseItem>) {
        if self.st.borrow().home.generation != gen_at {
            return;
        }
        let picks: Vec<Option<ImagePick>> = list
            .iter()
            .map(|i| pick_landscape(i).or_else(|| own_image(i, lfx_api::ImageType::Primary)))
            .collect();
        let tiles: Vec<PlusBrand> = list
            .iter()
            .map(|i| PlusBrand {
                name: SharedString::from(i.name.as_str()),
                image: Image::default(),
                has_image: false,
            })
            .collect();
        let model = Rc::new(VecModel::from(tiles));
        BRANDS.with(|b| {
            *b.borrow_mut() = (
                list.iter()
                    .map(|i| (i.id.clone(), i.name.clone()))
                    .collect(),
                Some(model.clone()),
            );
        });
        self.win().set_plus_brands(ModelRc::from(model.clone()));
        self.rebuild_home_graph();
        self.show_focus();
        let (_, dpr) = self.shell.window_info();
        let scale = self.image_scale();
        for (idx, pick) in picks.into_iter().enumerate() {
            let Some(pick) = pick else { continue };
            let r = lfx_core::images::ImageRef {
                max_width: (640.0 * dpr.max(1.0) * scale).round().max(64.0) as u32,
                quality: 80,
                pick,
            };
            self.fetch_image(&r, lfx_images::Priority::Visible, move |_, img| {
                let model = BRANDS.with(|b| b.borrow().1.clone());
                if let Some(model) = model
                    && let Some(mut t) = model.row_data(idx)
                {
                    t.image = img;
                    t.has_image = true;
                    model.set_row_data(idx, t);
                }
            });
        }
    }

    /// Aktivierung auf der LENZ+-Startseite: Hero-Folie und Kacheln. `true` = erledigt.
    pub fn plus_activate(self: &Rc<Self>, t: Target) -> bool {
        if !self.is_plus() {
            return false;
        }
        if t.zone == Z_HERO {
            let id = {
                let st = self.st.borrow();
                st.home.pool.get(st.home.hero_idx).map(|i| i.id.clone())
            };
            if let Some(id) = id {
                self.on_card_activate(&id);
            }
            return true;
        }
        if t.zone == Z_BRANDS {
            self.plus_open_brand(t.index);
            return true;
        }
        false
    }

    fn plus_open_brand(self: &Rc<Self>, index: usize) {
        let brand = BRANDS.with(|b| b.borrow().0.get(index).cloned());
        if let Some((id, name)) = brand {
            self.go(Route::Collection { id, name });
        }
    }

    /// Ereignisse der LENZ+-Oberfläche (`plus-tab`, `plus-brand`).
    pub fn plus_event(self: &Rc<Self>, name: &str, arg: i32) {
        match name {
            "plus-brand" => self.plus_open_brand(arg.max(0) as usize),
            "plus-tab" => match arg {
                1 => self.go(Route::Library {
                    kind: LibraryKind::Movies,
                    genre: None,
                    view: LibraryView::Rows,
                    sort: LibSort::Az,
                }),
                2 => self.go(Route::Library {
                    kind: LibraryKind::Series,
                    genre: None,
                    view: LibraryView::Rows,
                    sort: LibSort::Az,
                }),
                _ => self.go_reset(Route::Home),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_order_roundtrip() {
        for p in 0..5 {
            assert_eq!(idx_to_pos(pos_to_idx(p)), p);
        }
        // Position 1 = Meine Liste = gemeinsamer Index 4; Position 2 = Filme = Index 2
        assert_eq!(pos_to_idx(1), 4);
        assert_eq!(pos_to_idx(2), 2);
        // Suche, TV, Datensparmodus und Profil bleiben unverändert
        for i in 5..9 {
            assert_eq!(pos_to_idx(i), i);
            assert_eq!(idx_to_pos(i), i);
        }
    }

    #[test]
    fn graph_has_brand_zone_and_single_hero_item() {
        let l = HomeLayout {
            width: 1472.0,
            header_h: 66.0,
            gutter: 54.0,
            bb_x: 54.0,
            bb_y: 157.0,
            bb_w: 1364.0,
            bb_h: 454.0,
            play_x: 54.0,
            play_y: 157.0,
            play_w: 1364.0,
            info_x: 1418.0,
            info_w: 0.0,
            btn_h: 454.0,
            rows_top: 800.0,
            row_h: 500.0,
            top_extra: 0.0,
            pitch: 280.0,
            card_w: 260.0,
            card_h: 390.0,
            top_card_h: 390.0,
            head_h: 51.0,
        };
        let g = plus_graph(&l, true, 4, &[6, 6]);
        assert!(g.zone(Z_BRANDS).is_some());
        assert_eq!(g.zone(Z_HERO).map(Zone::len), Some(1));
        assert!(g.zone(row_zone(1)).is_some());
        let g = plus_graph(&l, true, 0, &[]);
        assert!(g.zone(Z_BRANDS).is_none());
    }
}
