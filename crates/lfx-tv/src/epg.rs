//! EPG-Modell (Port der Logik von `tvmode/epg-grid.js`): Raster je Sender für einen Sendetag,
//! Zeitskala, „jetzt“-Linie, laufende und vergangene Zellen, Tagesleiste. Die UI liest nur dieses
//! Modell; Pixel und Scrollen sind Sache der Ansicht (Hilfen: [`classify`], [`reveal_left`]).

use lfx_tv_schedule::time::{MIN, broadcast_day_of, day_bounds, format_clock};
use lfx_tv_schedule::{Channel, PlanEntry};

use crate::model::{DayRef, Kind, days, grid_rows};
use crate::plan::PlanSource;

/// Schrittweite der Zeitskala.
pub const SCALE_STEP_MS: i64 = 30 * MIN;
/// Schmaler als dies (px): kein Text.
pub const TINY_PX: f64 = 48.0;
/// Schmaler als dies (px): nur Titel ohne Uhrzeit.
pub const SHORT_PX: f64 = 90.0;
/// Die „jetzt“-Linie steht nach `reveal` so viele Minuten nach dem linken Rand.
pub const LEAD_MIN: f64 = 30.0;
/// Standard-Pixel je Minute.
pub const DEFAULT_PPM: f64 = 5.0;

/// Marke der Zeitskala.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleMark {
    pub at: i64,
    /// `HH:MM`
    pub label: String,
    /// Halbe Stunde (nicht `:00`).
    pub half: bool,
    /// Minuten ab Tagesbeginn.
    pub offset_min: f64,
}

/// Größenklasse einer Zelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellSize {
    Normal,
    /// Nur Titel.
    Short,
    /// Kein Text.
    Tiny,
}

/// Zelle des Rasters.
#[derive(Debug, Clone, PartialEq)]
pub struct EpgCell {
    pub entry: PlanEntry,
    pub no: i64,
    pub kind: Kind,
    pub title: String,
    pub sub: String,
    /// `20:15–22:05` (Originalzeiten, nicht zugeschnitten).
    pub time: String,
    /// Minuten ab Tagesbeginn (zugeschnitten).
    pub start_min: f64,
    /// Breite in Minuten (zugeschnitten).
    pub width_min: f64,
    /// Läuft gerade.
    pub current: bool,
    /// Ist vorbei.
    pub past: bool,
    /// Fortschritt in Prozent, nur bei laufender Zelle.
    pub progress_pct: Option<f64>,
    /// Tooltip.
    pub tooltip: String,
    /// Sprachausgabe.
    pub aria_label: String,
}

/// Zeile des Rasters.
#[derive(Debug, Clone, PartialEq)]
pub struct EpgRow {
    pub channel: Channel,
    pub cells: Vec<EpgCell>,
}

/// Was ein Klick auf eine Zelle bewirkt.
#[derive(Debug, Clone, PartialEq)]
pub enum CellAction {
    /// Laufende Sendung: Sender einschalten (sofort).
    Tune(i64),
    /// Andere Sendung: Info-Karte öffnen.
    Info { no: i64, entry: PlanEntry },
}

/// Zustand des Rasters.
#[derive(Debug, Clone)]
pub struct EpgGrid {
    today: i64,
    selected: i64,
    day_start: i64,
    day_end: i64,
    days: Vec<DayRef>,
    scale: Vec<ScaleMark>,
    rows: Vec<EpgRow>,
}

impl EpgGrid {
    /// Baut das Raster für den Sendetag `selected` (Standard: heute) zum Zeitpunkt `t`.
    pub fn build<P: PlanSource + ?Sized>(plan: &mut P, t: i64, selected: Option<i64>) -> Self {
        let today = broadcast_day_of(t);
        let mut g = Self {
            today,
            selected: selected.unwrap_or(today),
            day_start: 0,
            day_end: 0,
            days: days(t),
            scale: Vec::new(),
            rows: Vec::new(),
        };
        g.render_day(plan, t);
        g
    }

    fn render_day<P: PlanSource + ?Sized>(&mut self, plan: &mut P, t: i64) {
        let b = day_bounds(self.selected);
        self.day_start = b.start;
        self.day_end = b.end;
        self.scale = (0..)
            .map(|i| b.start + i * SCALE_STEP_MS)
            .take_while(|at| *at < b.end)
            .map(|at| {
                let label = format_clock(at);
                ScaleMark {
                    at,
                    half: !label.ends_with(":00"),
                    label,
                    offset_min: (at - b.start) as f64 / MIN as f64,
                }
            })
            .collect();
        let rows = grid_rows(plan, b.start, b.end, Some(self.selected), Some(t));
        self.rows = rows
            .into_iter()
            .map(|r| {
                let no = r.channel.no;
                let cells = r
                    .cells
                    .into_iter()
                    .map(|c| {
                        let time = format!(
                            "{}–{}",
                            format_clock(c.entry.start),
                            format_clock(c.entry.end)
                        );
                        EpgCell {
                            no,
                            kind: c.kind,
                            tooltip: format!("{} · {time}", c.title),
                            aria_label: format!("Sender {no}, {}, {time}", c.title),
                            title: c.title,
                            sub: c.sub,
                            start_min: (c.start - b.start) as f64 / MIN as f64,
                            width_min: (c.end - c.start) as f64 / MIN as f64,
                            current: false,
                            past: false,
                            progress_pct: None,
                            time,
                            entry: c.entry,
                        }
                    })
                    .collect();
                EpgRow {
                    channel: r.channel,
                    cells,
                }
            })
            .collect();
        self.apply_time(t);
    }

    /// Setzt `current`, `past` und Fortschritt der Zellen für `t`.
    pub fn apply_time(&mut self, t: i64) {
        for row in &mut self.rows {
            for c in &mut row.cells {
                c.current = c.entry.start <= t && t < c.entry.end;
                c.past = c.entry.end <= t;
                c.progress_pct = c.current.then(|| {
                    (t - c.entry.start) as f64 / (c.entry.end - c.entry.start).max(1) as f64 * 100.0
                });
            }
        }
    }

    /// Wählt einen anderen Sendetag (nur wenn er sich ändert oder das Raster leer ist).
    pub fn select_day<P: PlanSource + ?Sized>(&mut self, plan: &mut P, d: i64, t: i64) -> bool {
        if d == self.selected && !self.rows.iter().all(|r| r.cells.is_empty()) {
            return false;
        }
        self.selected = d;
        self.render_day(plan, t);
        true
    }

    /// Zurück zu heute („Jetzt“-Knopf). Liefert, ob neu aufgebaut wurde.
    pub fn select_today<P: PlanSource + ?Sized>(&mut self, plan: &mut P, t: i64) -> bool {
        if self.selected == self.today {
            return false;
        }
        self.selected = self.today;
        self.render_day(plan, t);
        true
    }

    /// Takt: bei Tageswechsel (05:00) neu aufbauen und dem Tag folgen, sonst nur Zeit anwenden.
    /// Liefert `true` bei Neuaufbau.
    pub fn tick<P: PlanSource + ?Sized>(&mut self, plan: &mut P, t: i64) -> bool {
        let d = broadcast_day_of(t);
        if d != self.today {
            let follow = self.selected == self.today;
            self.today = d;
            if follow || self.selected < d || self.selected > d + 6 {
                self.selected = d;
            }
            self.days = days(t);
            self.render_day(plan, t);
            return true;
        }
        self.apply_time(t);
        false
    }

    /// Heutiger Sendetag.
    pub fn today(&self) -> i64 {
        self.today
    }

    /// Gewählter Sendetag.
    pub fn selected(&self) -> i64 {
        self.selected
    }

    /// Fenster `[start, end)` des gewählten Tages.
    pub fn bounds(&self) -> (i64, i64) {
        (self.day_start, self.day_end)
    }

    /// Tagesleiste (7 Tage).
    pub fn days(&self) -> &[DayRef] {
        &self.days
    }

    /// Zeitskala.
    pub fn scale(&self) -> &[ScaleMark] {
        &self.scale
    }

    /// Zeilen.
    pub fn rows(&self) -> &[EpgRow] {
        &self.rows
    }

    /// Gesamtbreite in Minuten.
    pub fn total_min(&self) -> f64 {
        (self.day_end - self.day_start) as f64 / MIN as f64
    }

    /// Position der „jetzt“-Linie in Minuten ab Tagesbeginn (2 Nachkommastellen) oder `None`,
    /// wenn `t` außerhalb des gewählten Tages liegt (Linie versteckt).
    pub fn now_line(&self, t: i64) -> Option<f64> {
        if t >= self.day_start && t < self.day_end {
            Some(((t - self.day_start) as f64 / MIN as f64 * 100.0).round() / 100.0)
        } else {
            None
        }
    }

    /// Klick auf eine Zelle.
    pub fn on_cell(cell: &EpgCell, t: i64) -> CellAction {
        if cell.entry.start <= t && t < cell.entry.end {
            CellAction::Tune(cell.no)
        } else {
            CellAction::Info {
                no: cell.no,
                entry: cell.entry.clone(),
            }
        }
    }
}

/// Größenklasse einer Zelle bei `ppm` Pixeln je Minute; Füller sind früher „tiny“.
pub fn classify(cell: &EpgCell, ppm: f64) -> CellSize {
    let px = cell.width_min * ppm;
    let filler = cell.kind == Kind::Filler;
    if px < TINY_PX || (filler && px < SHORT_PX) {
        CellSize::Tiny
    } else if px < SHORT_PX {
        CellSize::Short
    } else {
        CellSize::Normal
    }
}

/// Linke Scrollposition (px), damit die „jetzt“-Linie ca. 30 min nach dem linken Rand steht.
/// Nur für heute und wenn `t` im Tag liegt.
pub fn reveal_left(grid: &EpgGrid, t: i64, ppm: f64) -> Option<f64> {
    if grid.selected != grid.today || t < grid.day_start || t >= grid.day_end {
        return None;
    }
    Some((((t - grid.day_start) as f64 / MIN as f64 - LEAD_MIN) * ppm).max(0.0))
}
