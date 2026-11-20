//! Fokusgraph: Zonen (Zeilen, Spalten, Raster) mit Items, Zonen-Nachbarschaft, Fokusfalle und
//! Verlauf. Die Zonen werden von der UI nach jedem Layout mit Rechtecken versorgt.

use std::collections::BTreeMap;

use crate::action::NavAction;
use crate::geometry::{Direction, Rect, direction_score, entry_score};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ZoneId(pub u32);

/// Ein fokussierbares Item: Zone plus Index darin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub zone: ZoneId,
    pub index: usize,
}

/// Anordnung der Items in einer Zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Horizontal,
    Vertical,
    Grid { columns: usize },
}

#[derive(Clone, Debug)]
pub struct Zone {
    pub id: ZoneId,
    /// Sichtbarer Bereich der Zone (für die Zonenwahl).
    pub rect: Rect,
    pub layout: Layout,
    /// Itemzahl, wenn `items` leer ist (reine Indexlogik, Rechtecke werden aus `rect` abgeleitet).
    pub count: usize,
    /// Optionale Item-Rechtecke (gleiches Koordinatensystem wie `rect`); gewinnt gegen `count`.
    pub items: Vec<Rect>,
    /// Am Rand umbrechen statt die Zone zu verlassen.
    pub wrap: bool,
    /// Beim Wiederbetreten das zuletzt fokussierte Item wählen.
    pub remember: bool,
    /// Schrittweite für Bild auf/ab (bei Raster in Zeilen).
    pub page_size: usize,
    /// Explizite Nachbarzonen: Up, Down, Left, Right.
    pub neighbors: [Option<ZoneId>; 4],
}

impl Zone {
    pub fn new(id: ZoneId, rect: Rect, layout: Layout, count: usize) -> Self {
        Self {
            id,
            rect,
            layout,
            count,
            items: Vec::new(),
            wrap: false,
            remember: false,
            page_size: 5,
            neighbors: [None; 4],
        }
    }
    pub fn horizontal(id: ZoneId, rect: Rect, count: usize) -> Self {
        Self::new(id, rect, Layout::Horizontal, count)
    }
    pub fn vertical(id: ZoneId, rect: Rect, count: usize) -> Self {
        Self::new(id, rect, Layout::Vertical, count)
    }
    pub fn grid(id: ZoneId, rect: Rect, count: usize, columns: usize) -> Self {
        Self::new(
            id,
            rect,
            Layout::Grid {
                columns: columns.max(1),
            },
            count,
        )
    }
    pub fn with_items(mut self, items: Vec<Rect>) -> Self {
        self.items = items;
        self
    }
    pub fn wrapping(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }
    pub fn remembering(mut self, remember: bool) -> Self {
        self.remember = remember;
        self
    }
    pub fn neighbor(mut self, dir: Direction, zone: ZoneId) -> Self {
        self.neighbors[dir_slot(dir)] = Some(zone);
        self
    }

    /// Anzahl fokussierbarer Items.
    pub fn len(&self) -> usize {
        if self.items.is_empty() {
            self.count
        } else {
            self.items.len()
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Rechteck eines Items; ohne explizite Rechtecke gleichmäßig aus der Zone abgeleitet.
    pub fn item_rect(&self, index: usize) -> Rect {
        if let Some(r) = self.items.get(index) {
            return *r;
        }
        let n = self.len().max(1);
        let r = self.rect;
        match self.layout {
            Layout::Horizontal => {
                let w = r.w / n as f32;
                Rect::new(r.x + w * index as f32, r.y, w, r.h)
            }
            Layout::Vertical => {
                let h = r.h / n as f32;
                Rect::new(r.x, r.y + h * index as f32, r.w, h)
            }
            Layout::Grid { columns } => {
                let cols = columns.max(1);
                let rows = n.div_ceil(cols);
                let (w, h) = (r.w / cols as f32, r.h / rows as f32);
                Rect::new(
                    r.x + w * (index % cols) as f32,
                    r.y + h * (index / cols) as f32,
                    w,
                    h,
                )
            }
        }
    }
}

fn dir_slot(d: Direction) -> usize {
    match d {
        Direction::Up => 0,
        Direction::Down => 1,
        Direction::Left => 2,
        Direction::Right => 3,
    }
}

/// Ergebnis einer angewandten Aktion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavOutcome {
    /// Fokus bewegt; `from` ist `None`, wenn vorher nichts fokussiert war.
    Moved { from: Option<Target>, to: Target },
    /// Aktivierung des fokussierten Items.
    Activated(Target),
    /// Zurück: die UI entscheidet (Modal schließen, Route zurück, ...).
    Back,
    /// Nicht vom Graph behandelt (Rand ohne Ziel, Menü, Medientasten ...).
    Unhandled,
}

#[derive(Debug)]
struct ZoneState {
    zone: Zone,
    remembered: Option<usize>,
}

#[derive(Debug)]
struct Trap {
    zones: Vec<ZoneId>,
    saved: Option<Target>,
}

const HISTORY_MAX: usize = 32;

#[derive(Debug, Default)]
pub struct FocusGraph {
    zones: BTreeMap<ZoneId, ZoneState>,
    focus: Option<Target>,
    traps: Vec<Trap>,
    history: Vec<Target>,
}

impl FocusGraph {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- Zonenverwaltung ----

    /// Legt eine Zone an oder ersetzt sie (gemerkter Index geht dabei verloren).
    pub fn register_zone(&mut self, zone: Zone) {
        self.zones.insert(
            zone.id,
            ZoneState {
                zone,
                remembered: None,
            },
        );
        self.clamp_focus();
    }

    /// Ersetzt die Zonendaten (z. B. nach Layout/Scroll), behält den gemerkten Index.
    /// `false`, wenn die Zone unbekannt ist.
    pub fn update_zone(&mut self, zone: Zone) -> bool {
        let Some(st) = self.zones.get_mut(&zone.id) else {
            return false;
        };
        st.zone = zone;
        let len = st.zone.len();
        if let Some(r) = st.remembered {
            st.remembered = if len == 0 { None } else { Some(r.min(len - 1)) };
        }
        self.clamp_focus();
        true
    }

    pub fn remove_zone(&mut self, id: ZoneId) {
        self.zones.remove(&id);
        for t in &mut self.traps {
            t.zones.retain(|z| *z != id);
            if t.saved.is_some_and(|s| s.zone == id) {
                t.saved = None;
            }
        }
        self.history.retain(|t| t.zone != id);
        if self.focus.is_some_and(|f| f.zone == id) {
            self.focus = None;
        }
    }

    pub fn zone(&self, id: ZoneId) -> Option<&Zone> {
        self.zones.get(&id).map(|s| &s.zone)
    }

    /// Gemerkter Index einer Zone (zuletzt fokussiertes Item).
    pub fn remembered(&self, id: ZoneId) -> Option<usize> {
        self.zones.get(&id).and_then(|s| s.remembered)
    }

    fn clamp_focus(&mut self) {
        if let Some(f) = self.focus {
            match self.zones.get(&f.zone).map(|s| s.zone.len()) {
                Some(len) if len > 0 => {
                    self.focus = Some(Target {
                        zone: f.zone,
                        index: f.index.min(len - 1),
                    });
                }
                _ => self.focus = None,
            }
        }
    }

    // ---- Fokus ----

    pub fn focused(&self) -> Option<Target> {
        self.focus
    }

    fn allowed(&self, zone: ZoneId) -> bool {
        self.traps.last().is_none_or(|t| t.zones.contains(&zone))
    }

    fn valid(&self, t: Target) -> bool {
        self.allowed(t.zone)
            && self
                .zones
                .get(&t.zone)
                .is_some_and(|s| t.index < s.zone.len())
    }

    /// Setzt den Fokus; `false` bei unbekanntem Ziel oder außerhalb der Fokusfalle.
    pub fn set_focus(&mut self, t: Target) -> bool {
        if !self.valid(t) {
            return false;
        }
        self.focus = Some(t);
        if let Some(s) = self.zones.get_mut(&t.zone) {
            s.remembered = Some(t.index);
        }
        true
    }

    /// Betritt eine Zone am gemerkten bzw. ersten Item.
    pub fn focus_zone(&mut self, id: ZoneId) -> bool {
        let Some(s) = self.zones.get(&id) else {
            return false;
        };
        let len = s.zone.len();
        if len == 0 {
            return false;
        }
        let idx = s.remembered.map_or(0, |r| r.min(len - 1));
        self.set_focus(Target {
            zone: id,
            index: idx,
        })
    }

    /// Maus-Hover setzt den Fokus (wie im Web). `Moved`, wenn sich der Fokus geändert hat.
    pub fn hover(&mut self, t: Target) -> NavOutcome {
        let from = self.focus;
        if from == Some(t) {
            return NavOutcome::Unhandled;
        }
        if self.set_focus(t) {
            NavOutcome::Moved { from, to: t }
        } else {
            NavOutcome::Unhandled
        }
    }

    // ---- Fokusfalle (Modals) ----

    /// Hält den Fokus in den angegebenen Zonen; Fokus davor wird gemerkt.
    pub fn push_trap(&mut self, zones: &[ZoneId]) {
        let saved = self.focus;
        self.traps.push(Trap {
            zones: zones.to_vec(),
            saved,
        });
        if self.focus.is_none_or(|f| !zones.contains(&f.zone)) {
            self.focus = None;
            for z in zones {
                if self.focus_zone(*z) {
                    break;
                }
            }
        }
    }

    /// Hebt die oberste Falle auf und stellt den Fokus davor wieder her, falls noch gültig.
    pub fn pop_trap(&mut self) {
        let Some(t) = self.traps.pop() else {
            return;
        };
        match t.saved {
            Some(s) if self.valid(s) => {
                self.set_focus(s);
            }
            _ => {
                if self.focus.is_some_and(|f| !self.allowed(f.zone)) {
                    self.focus = None;
                }
            }
        }
    }

    pub fn trap_active(&self) -> bool {
        !self.traps.is_empty()
    }

    // ---- Verlauf für Zurück ----

    /// Merkt den aktuellen Fokus (z. B. vor dem Öffnen einer Detailseite).
    pub fn push_history(&mut self) {
        if let Some(f) = self.focus {
            if self.history.len() == HISTORY_MAX {
                self.history.remove(0);
            }
            self.history.push(f);
        }
    }

    /// Stellt den zuletzt gemerkten Fokus wieder her. `true`, wenn er noch gültig war.
    pub fn pop_history(&mut self) -> bool {
        while let Some(t) = self.history.pop() {
            if self.set_focus(t) {
                return true;
            }
        }
        false
    }

    // ---- Aktionen ----

    pub fn apply(&mut self, action: NavAction) -> NavOutcome {
        if let Some(dir) = Direction::from_action(action) {
            return self.move_dir(dir);
        }
        match action {
            NavAction::PageUp => self.page(false),
            NavAction::PageDown => self.page(true),
            NavAction::Activate => self
                .focus
                .map_or(NavOutcome::Unhandled, NavOutcome::Activated),
            NavAction::Back => NavOutcome::Back,
            _ => NavOutcome::Unhandled,
        }
    }

    fn move_to(&mut self, from: Option<Target>, to: Target) -> NavOutcome {
        if from == Some(to) || !self.set_focus(to) {
            return NavOutcome::Unhandled;
        }
        NavOutcome::Moved { from, to }
    }

    /// Erster Fokus ohne Vorgänger: oberste linke erlaubte Zone.
    fn focus_initial(&mut self) -> NavOutcome {
        let first = self
            .zones
            .values()
            .filter(|s| !s.zone.is_empty() && self.allowed(s.zone.id))
            .min_by(|a, b| {
                a.zone
                    .rect
                    .y
                    .total_cmp(&b.zone.rect.y)
                    .then(a.zone.rect.x.total_cmp(&b.zone.rect.x))
                    .then(a.zone.id.cmp(&b.zone.id))
            })
            .map(|s| s.zone.id);
        match first {
            Some(id) if self.focus_zone(id) => NavOutcome::Moved {
                from: None,
                to: self.focus.expect("gesetzt"),
            },
            _ => NavOutcome::Unhandled,
        }
    }

    fn move_dir(&mut self, dir: Direction) -> NavOutcome {
        let Some(cur) = self.focus else {
            return self.focus_initial();
        };
        let Some(st) = self.zones.get(&cur.zone) else {
            return self.focus_initial();
        };
        let zone = &st.zone;
        if let Some(idx) = step_in_zone(zone, cur.index, dir) {
            return self.move_to(
                Some(cur),
                Target {
                    zone: cur.zone,
                    index: idx,
                },
            );
        }
        let reference = zone.item_rect(cur.index);
        match self.leave_zone(cur.zone, &reference, dir) {
            Some(t) => self.move_to(Some(cur), t),
            None => NavOutcome::Unhandled,
        }
    }

    /// Zielzone (explizit, sonst geometrisch) und Item darin.
    fn leave_zone(&self, from: ZoneId, reference: &Rect, dir: Direction) -> Option<Target> {
        let explicit = self.zones[&from].zone.neighbors[dir_slot(dir)].filter(|z| {
            *z != from && self.allowed(*z) && self.zones.get(z).is_some_and(|s| !s.zone.is_empty())
        });
        let zone_id = explicit.or_else(|| {
            self.zones
                .values()
                .filter(|s| s.zone.id != from && !s.zone.is_empty() && self.allowed(s.zone.id))
                .filter_map(|s| {
                    direction_score(reference, &s.zone.rect, dir).map(|sc| (sc, s.zone.id))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, id)| id)
        })?;
        let st = &self.zones[&zone_id];
        let len = st.zone.len();
        let index = match st.remembered {
            Some(r) if st.zone.remember => r.min(len - 1),
            _ => (0..len)
                .map(|i| (entry_score(reference, &st.zone.item_rect(i), dir), i))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, i)| i)?,
        };
        Some(Target {
            zone: zone_id,
            index,
        })
    }

    fn page(&mut self, down: bool) -> NavOutcome {
        let Some(cur) = self.focus else {
            return NavOutcome::Unhandled;
        };
        let Some(st) = self.zones.get(&cur.zone) else {
            return NavOutcome::Unhandled;
        };
        let z = &st.zone;
        let len = z.len();
        let unit = match z.layout {
            Layout::Grid { columns } => columns.max(1),
            _ => 1,
        };
        let delta = z.page_size.max(1) * unit;
        let idx = if down {
            (cur.index + delta).min(len - 1)
        } else {
            cur.index.saturating_sub(delta)
        };
        self.move_to(
            Some(cur),
            Target {
                zone: cur.zone,
                index: idx,
            },
        )
    }
}

/// Schritt innerhalb der Zone; `None` am Rand (dann wird die Zone verlassen).
fn step_in_zone(z: &Zone, idx: usize, dir: Direction) -> Option<usize> {
    let len = z.len();
    let linear = |back: bool| -> Option<usize> {
        let n = if back {
            if idx > 0 {
                idx - 1
            } else if z.wrap {
                len - 1
            } else {
                return None;
            }
        } else if idx + 1 < len {
            idx + 1
        } else if z.wrap {
            0
        } else {
            return None;
        };
        (n != idx).then_some(n)
    };
    match (z.layout, dir) {
        (Layout::Horizontal, Direction::Left) | (Layout::Vertical, Direction::Up) => linear(true),
        (Layout::Horizontal, Direction::Right) | (Layout::Vertical, Direction::Down) => {
            linear(false)
        }
        (Layout::Horizontal | Layout::Vertical, _) => None,
        (Layout::Grid { columns }, d) => {
            let cols = columns.max(1);
            let (row, col) = (idx / cols, idx % cols);
            let last_row = (len - 1) / cols;
            let n = match d {
                Direction::Left => {
                    if col > 0 {
                        idx - 1
                    } else if z.wrap {
                        (row * cols + cols - 1).min(len - 1)
                    } else {
                        return None;
                    }
                }
                Direction::Right => {
                    if col + 1 < cols && idx + 1 < len {
                        idx + 1
                    } else if z.wrap {
                        row * cols
                    } else {
                        return None;
                    }
                }
                Direction::Up => {
                    if row > 0 {
                        idx - cols
                    } else if z.wrap {
                        let n = last_row * cols + col;
                        if n < len { n } else { n - cols }
                    } else {
                        return None;
                    }
                }
                Direction::Down => {
                    if idx + cols < len {
                        idx + cols
                    } else if row < last_row {
                        len - 1
                    } else if z.wrap {
                        col
                    } else {
                        return None;
                    }
                }
            };
            (n != idx).then_some(n)
        }
    }
}
