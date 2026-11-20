//! Tagesbaustein: Platzieren, Teilsummen-DP, Füller / Füllprogramm / Nachtfüllung.
//!
//! * [`slot_of`]: auf 5 min aufgerundete Platzlänge.
//! * [`age_allowed`]: Jugendschutz am Startzeitpunkt (ab 16 erst ab 22:00, ab 18 erst ab 23:00).
//! * [`pack_subset`]: Teilmenge mit maximaler Summe (Gleichstand: frühere Kandidaten).
//! * [`DayBuilder`]: baut einen lückenlosen Tagesplan.
//! * [`fill_segment`]: Kandidaten nacheinander platzieren, ab Rest <= 4 h mit Teilsummen-DP.

use std::rc::Rc;

use indexmap::IndexSet;

use crate::hash::key_hash;
use crate::jsval::round_js;
use crate::rating::Age;
use crate::time::{Bounds, GRID, HOUR, MIN, day_bounds};

const FILL_LABEL: &str = "Füllprogramm";
const NIGHT_LABEL: &str = "Füllprogramm – reguläres Programm ab 5:00";
const FILLER_MAX: i64 = 30 * MIN;
const DP_HORIZON: i64 = 4 * HOUR;
const DP_MAX_CANDS: usize = 40;

/// Art eines Planeintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Program,
    Fill,
    Nightfill,
    Filler,
    Closedown,
}

impl EntryType {
    /// Name wie im Web-Planer.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryType::Program => "program",
            EntryType::Fill => "fill",
            EntryType::Nightfill => "nightfill",
            EntryType::Filler => "filler",
            EntryType::Closedown => "closedown",
        }
    }
}

/// Premieren-Kennzeichnung eines Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremiereKind {
    Film,
    Episode,
}

impl PremiereKind {
    /// Name wie im Web-Planer.
    pub fn as_str(self) -> &'static str {
        match self {
            PremiereKind::Film => "film",
            PremiereKind::Episode => "episode",
        }
    }
}

/// Ein Eintrag im Tagesplan (Zeiten in ms, `end` exklusiv).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    pub start: i64,
    pub end: i64,
    pub kind: EntryType,
    pub reason: Option<String>,
    pub item_id: Option<String>,
    pub series_id: Option<String>,
    pub rerun: bool,
    pub premiere: Option<PremiereKind>,
    pub theme: Option<String>,
    pub age: Age,
    pub label: Option<String>,
}

impl PlanEntry {
    fn bare(start: i64, end: i64, kind: EntryType, reason: Option<&str>) -> Self {
        PlanEntry {
            start,
            end,
            kind,
            reason: reason.map(str::to_string),
            item_id: None,
            series_id: None,
            rerun: false,
            premiere: None,
            theme: None,
            age: None,
            label: None,
        }
    }
}

/// Ein platzierbares Stück (Film oder Folge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub item_id: String,
    pub dur_ms: i64,
    pub series_id: Option<String>,
    pub rerun: bool,
    pub premiere: Option<PremiereKind>,
    pub theme: Option<String>,
    pub age: Age,
    pub label: Option<String>,
}

impl Piece {
    /// Stück mit Id, Dauer und Freigabe; alles andere leer.
    pub fn new(item_id: impl Into<String>, dur_ms: i64, age: Age) -> Self {
        Piece {
            item_id: item_id.into(),
            dur_ms,
            series_id: None,
            rerun: false,
            premiere: None,
            theme: None,
            age,
            label: None,
        }
    }
}

/// Kandidat für [`fill_segment`]: Stücke, die zusammen oder gar nicht gesendet werden.
#[derive(Debug, Clone)]
pub struct Cand {
    pub id: String,
    pub movie: bool,
    pub size: i64,
    pub pieces: Vec<Piece>,
}

/// Kurzer Titel für das Füllprogramm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortItem {
    pub id: String,
    pub dur_ms: i64,
    pub age: Age,
    pub series_id: Option<String>,
}

/// Optionen für `place` / `place_block` / `can_place_block`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaceOpts {
    pub limit: Option<i64>,
    pub kind: Option<EntryType>,
}

impl PlaceOpts {
    pub fn limit(limit: i64) -> Self {
        PlaceOpts {
            limit: Some(limit),
            kind: None,
        }
    }
}

/// `slotOf` für ganzzahlige Dauern.
pub fn slot_of(dur_ms: i64) -> i64 {
    let q = dur_ms.div_euclid(GRID) + i64::from(dur_ms.rem_euclid(GRID) != 0);
    q * GRID
}

/// `slotOf` für beliebige Zahlen (`Math.ceil(d / GRID) * GRID`).
pub fn slot_of_f64(dur_ms: f64) -> f64 {
    (dur_ms / GRID as f64).ceil() * GRID as f64
}

/// Jugendschutz am Startzeitpunkt.
pub fn age_allowed(age: Age, start: i64, b: &Bounds) -> bool {
    match age {
        None => true,
        Some(a) if a <= 12 => true,
        Some(a) if a >= 18 => start >= b.a2300,
        Some(_) => start >= b.a2200,
    }
}

/// Teilsummen-DP: Indizes (aufsteigend) mit maximaler Summe <= cap; bei Gleichstand werden
/// frühere Kandidaten bevorzugt.
pub fn pack_subset(sizes: &[i64], cap: i64) -> Vec<usize> {
    let n = sizes.len();
    let cap = cap.clamp(0, 1200) as usize;
    if n == 0 || cap == 0 {
        return Vec::new();
    }
    let w = cap + 1;
    let mut suf = vec![false; (n + 1) * w];
    suf[n * w] = true;
    for i in (0..n).rev() {
        let s = sizes[i];
        for c in 0..=cap {
            let skip = suf[(i + 1) * w + c];
            let take = s <= c as i64 && suf[(i + 1) * w + c - s as usize];
            suf[i * w + c] = skip || take;
        }
    }
    let mut best = cap;
    while best > 0 && !suf[best] {
        best -= 1;
    }
    let mut out = Vec::new();
    let mut rem = best as i64;
    let mut i = 0;
    while i < n && rem > 0 {
        if sizes[i] <= rem && suf[(i + 1) * w + (rem - sizes[i]) as usize] {
            out.push(i);
            rem -= sizes[i];
        }
        i += 1;
    }
    out
}

/// Ganzer Tag Testbild.
pub fn closedown_day(d: i64, reason: &str) -> Vec<PlanEntry> {
    let b = day_bounds(d);
    vec![PlanEntry::bare(
        b.start,
        b.end,
        EntryType::Closedown,
        Some(reason),
    )]
}

/// Baut den Plan eines Sendetags. Cursor liegt immer im 5-min-Raster ab Tagesstart.
#[derive(Debug)]
pub struct DayBuilder {
    d: i64,
    b: Bounds,
    used: IndexSet<String>,
    shorts: Rc<Vec<ShortItem>>,
    fkey: String,
    entries: Vec<PlanEntry>,
    cursor: i64,
    fill_count: i64,
}

impl DayBuilder {
    /// `createDayBuilder({ D, bounds?, used?, shorts?, key? })`.
    pub fn new(
        d: i64,
        bounds: Option<Bounds>,
        used: IndexSet<String>,
        shorts: Rc<Vec<ShortItem>>,
        key: Option<&str>,
    ) -> Self {
        let b = bounds.unwrap_or_else(|| day_bounds(d));
        DayBuilder {
            d,
            b,
            used,
            shorts,
            fkey: format!("fill\u{1}{}", key.unwrap_or("")),
            entries: Vec::new(),
            cursor: b.start,
            fill_count: 0,
        }
    }

    /// Aktuelle Zeit.
    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// Tagesgrenzen.
    pub fn bounds(&self) -> Bounds {
        self.b
    }

    /// Bereits gesendete Ids (Einfügereihenfolge).
    pub fn used(&self) -> &IndexSet<String> {
        &self.used
    }

    /// Id wieder freigeben (Premieren-Titel).
    pub fn release(&mut self, id: &str) {
        self.used.shift_remove(id);
    }

    fn push_filler(&mut self, from: i64, to: i64, reason: &str) {
        if to <= from {
            return;
        }
        if let Some(last) = self.entries.last_mut()
            && last.kind == EntryType::Filler
            && last.end == from
        {
            last.end = to;
            return;
        }
        self.entries
            .push(PlanEntry::bare(from, to, EntryType::Filler, Some(reason)));
    }

    fn mk_entry(piece: &Piece, start: i64, kind: EntryType) -> PlanEntry {
        let mut e = PlanEntry::bare(start, start + piece.dur_ms, kind, None);
        e.item_id = Some(piece.item_id.clone());
        if piece.series_id.as_deref().is_some_and(|s| !s.is_empty()) {
            e.series_id = piece.series_id.clone();
        }
        e.rerun = piece.rerun;
        e.premiere = piece.premiere;
        if piece.theme.as_deref().is_some_and(|s| !s.is_empty()) {
            e.theme = piece.theme.clone();
        }
        e.age = piece.age;
        if piece.label.as_deref().is_some_and(|s| !s.is_empty()) {
            e.label = piece.label.clone();
        } else if kind == EntryType::Fill {
            e.label = Some(FILL_LABEL.to_string());
        } else if kind == EntryType::Nightfill {
            e.label = Some(NIGHT_LABEL.to_string());
        }
        e
    }

    fn lim(&self, opts: Option<PlaceOpts>) -> i64 {
        opts.and_then(|o| o.limit)
            .unwrap_or(self.b.end)
            .min(self.b.end)
    }

    /// Passt der Block (alles oder nichts) ab dem Cursor?
    pub fn can_place_block(&self, pieces: &[Piece], opts: Option<PlaceOpts>) -> bool {
        let limit = self.lim(opts);
        let mut s = self.cursor;
        let mut seen: Vec<&str> = Vec::new();
        for p in pieces {
            if p.dur_ms <= 0 || self.used.contains(&p.item_id) || seen.contains(&p.item_id.as_str())
            {
                return false;
            }
            if !age_allowed(p.age, s, &self.b) {
                return false;
            }
            s += slot_of(p.dur_ms);
            if s > limit {
                return false;
            }
            seen.push(&p.item_id);
        }
        !pieces.is_empty()
    }

    /// Block platzieren (alles oder nichts).
    pub fn place_block(&mut self, pieces: &[Piece], opts: Option<PlaceOpts>) -> bool {
        if !self.can_place_block(pieces, opts) {
            return false;
        }
        let kind = opts.and_then(|o| o.kind).unwrap_or(EntryType::Program);
        for p in pieces {
            let slot = slot_of(p.dur_ms);
            let e = Self::mk_entry(p, self.cursor, kind);
            self.entries.push(e);
            self.used.insert(p.item_id.clone());
            if slot > p.dur_ms {
                self.push_filler(self.cursor + p.dur_ms, self.cursor + slot, "grid");
            }
            self.cursor += slot;
        }
        true
    }

    /// Ein Stück prüfen.
    pub fn can_place(&self, piece: &Piece, opts: Option<PlaceOpts>) -> bool {
        self.can_place_block(std::slice::from_ref(piece), opts)
    }

    /// Ein Stück platzieren.
    pub fn place(&mut self, piece: &Piece, opts: Option<PlaceOpts>) -> bool {
        self.place_block(std::slice::from_ref(piece), opts)
    }

    fn pick_short(&self, gap: i64) -> Option<ShortItem> {
        let n = self.shorts.len();
        if n == 0 {
            return None;
        }
        let off = key_hash(&self.fkey, &format!("{}:{}", self.d, self.fill_count)) as usize % n;
        let mut best: Option<&ShortItem> = None;
        let mut best_slot = 0;
        let mut found = 0;
        let mut i = 0;
        while i < n && found < 8 {
            let it = &self.shorts[(off + i) % n];
            i += 1;
            if self.used.contains(&it.id) {
                continue;
            }
            let slot = slot_of(it.dur_ms);
            if slot > gap || !age_allowed(it.age, self.cursor, &self.b) {
                continue;
            }
            found += 1;
            if slot > best_slot {
                best = Some(it);
                best_slot = slot;
            }
        }
        best.cloned()
    }

    /// Lücke `[cursor, until)` füllen.
    pub fn fill_until(&mut self, until: i64) {
        let until = until.min(self.b.end);
        let at_end = until == self.b.end;
        while self.cursor < until {
            let gap = until - self.cursor;
            if gap <= FILLER_MAX {
                self.push_filler(self.cursor, until, if at_end { "end" } else { "grid" });
                self.cursor = until;
                break;
            }
            let Some(it) = self.pick_short(gap) else {
                self.entries.push(PlanEntry::bare(
                    self.cursor,
                    until,
                    EntryType::Closedown,
                    Some("empty"),
                ));
                self.cursor = until;
                break;
            };
            self.fill_count += 1;
            let mut p = Piece::new(it.id, it.dur_ms, it.age);
            p.series_id = it.series_id;
            let kind = if at_end {
                EntryType::Nightfill
            } else {
                EntryType::Fill
            };
            self.place_block(
                &[p],
                Some(PlaceOpts {
                    limit: Some(until),
                    kind: Some(kind),
                }),
            );
        }
    }

    /// Rest des Tages als Testbild.
    pub fn close_rest(&mut self, reason: &str) {
        if self.cursor < self.b.end {
            self.entries.push(PlanEntry::bare(
                self.cursor,
                self.b.end,
                EntryType::Closedown,
                Some(reason),
            ));
        }
        self.cursor = self.b.end;
    }

    /// Füllt bis Tagesende und liefert den Plan.
    pub fn finish(&mut self) -> Vec<PlanEntry> {
        if self.cursor < self.b.end {
            self.fill_until(self.b.end);
        }
        self.entries.clone()
    }
}

/// `fillSegment(builder, cands, { limit, strict })`.
pub fn fill_segment(builder: &mut DayBuilder, cands: &[Cand], limit: i64, strict: bool) {
    let mut pending: Vec<usize> = (0..cands.len()).collect();
    if !strict {
        while builder.cursor() < limit {
            let found = pending
                .iter()
                .position(|&i| builder.can_place_block(&cands[i].pieces, None));
            let Some(pos) = found else { break };
            let i = pending.remove(pos);
            builder.place_block(&cands[i].pieces, None);
        }
        if builder.cursor() < limit {
            builder.fill_until(limit);
        }
        return;
    }
    let opts = Some(PlaceOpts::limit(limit));
    while builder.cursor() < limit && limit - builder.cursor() > DP_HORIZON {
        let found = pending
            .iter()
            .position(|&i| builder.can_place_block(&cands[i].pieces, opts));
        let Some(pos) = found else { break };
        let i = pending.remove(pos);
        builder.place_block(&cands[i].pieces, opts);
    }
    if builder.cursor() < limit {
        let cand: Vec<usize> = pending
            .iter()
            .copied()
            .filter(|&i| builder.can_place_block(&cands[i].pieces, opts))
            .take(DP_MAX_CANDS)
            .collect();
        let sizes: Vec<i64> = cand
            .iter()
            .map(|&i| round_js(cands[i].size as f64 / GRID as f64) as i64)
            .collect();
        let idx = pack_subset(&sizes, (limit - builder.cursor()).div_euclid(GRID));
        for k in idx {
            let i = cand[k];
            builder.place_block(&cands[i].pieces, opts);
            if let Some(pos) = pending.iter().position(|&x| x == i) {
                pending.remove(pos);
            }
        }
        builder.fill_until(limit);
    }
}
