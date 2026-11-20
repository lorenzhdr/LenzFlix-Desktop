//! Sender 18 „Thementag“.
//!
//! Arten: Filmreihe (`Boxset`, Inhalte vorab über [`next_boxset_request`] / [`provide_boxset`]
//! geladen), Jahrzehnt (mindestens 6 h), Pilotfolgen, Überlänge (Filme ab 150 min). Art des
//! Tages: Permutation der verfügbaren Arten per Hash der Woche, zyklisch über die 7 Tage. Ist
//! die Art am Tag nicht planbar, gilt die nächste der Permutation; ist keine planbar, folgt nur
//! Füllprogramm/Nachtfüllung. Der Tagesplan nutzt nur den Zwischenspeicher der Sammlungen.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use indexmap::{IndexMap, IndexSet};

use crate::catalog::{Boxset, Item, PlayItem};
use crate::fill::{Piece, PlanEntry, slot_of};
use crate::hash::{cmp_str, key_hash};
use crate::plan::Context;
use crate::pool::{
    Stream, bucket_count, make_entries, rotation, rotation_with_borrow, stream_of_age,
};
use crate::time::{HOUR, day_bounds, week_of, week_start};

const OVERLENGTH_MS: i64 = 150 * 60_000;
const DECADE_MIN_MS: i64 = 6 * HOUR;
const MIN_ITEMS: usize = 2;
const MIN_PILOTS: usize = 3;
const MAX_LOADS_PER_DAY: usize = 12;
const DAY_CAP_MS: i64 = 24 * HOUR;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Boxset,
    Decade,
    Pilot,
    Overlength,
}

const KINDS: [Kind; 4] = [Kind::Boxset, Kind::Decade, Kind::Pilot, Kind::Overlength];

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Boxset => "boxset",
            Kind::Decade => "decade",
            Kind::Pilot => "pilot",
            Kind::Overlength => "overlength",
        }
    }
}

/// Ein berechnetes Thema (Name und Titel).
#[derive(Debug)]
pub(crate) struct Theme {
    name: String,
    items: Vec<PlayItem>,
}

/// Zwischenspeicher je Kontext: geladene Sammlungs-Inhalte und berechnete Themen.
#[derive(Default)]
pub struct ThemeState {
    boxsets: HashMap<String, Option<Vec<String>>>,
    themes: HashMap<(Kind, i64), Option<Rc<Theme>>>,
}

fn num_cmp(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Chronologisch: Jahr, Premierendatum, Staffel, Folge, Id (fehlende Werte zuletzt).
fn chrono(a: &PlayItem, b: &PlayItem) -> Ordering {
    num_cmp(
        a.year().map_or(9999.0, |y| y as f64),
        b.year().map_or(9999.0, |y| y as f64),
    )
    .then_with(|| num_cmp(a.premiere(), b.premiere()))
    .then_with(|| num_cmp(a.season(), b.season()))
    .then_with(|| num_cmp(a.episode(), b.episode()))
    .then_with(|| cmp_str(a.id(), b.id()))
}

/// Alle Kandidaten in Rotationsreihenfolge für Tag `d`: eigener Eimer zuerst, dann die folgenden.
fn rotated<T: Clone>(key: &str, items: &[T], d: i64, id_of: impl Fn(&T) -> String) -> Vec<T> {
    if items.is_empty() {
        return Vec::new();
    }
    let sorted = make_entries(key, items, id_of, |_| 1);
    let c = bucket_count(sorted.len() as i64, 2);
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for i in 0..c {
        for e in rotation(&sorted, c, d + i) {
            if seen.insert(e.id.clone()) {
                out.push(e.it);
            }
        }
    }
    out
}

/// Teilmenge für den Tag (Hash-Rotation, bis etwa 24 h), chronologisch sortiert.
fn pick_day(items: &[PlayItem], key: &str, d: i64) -> Vec<PlayItem> {
    let ms = |it: &PlayItem| slot_of(it.dur_ms());
    let total: i64 = items.iter().map(ms).sum();
    let mut sel: Vec<PlayItem> = items.to_vec();
    if total > DAY_CAP_MS {
        let sorted = make_entries(key, items, |it| it.id().to_string(), ms);
        let c = bucket_count(total, DAY_CAP_MS);
        let (home, borrow) = rotation_with_borrow(&sorted, c, d);
        sel = Vec::new();
        let mut sum = 0;
        for e in home.into_iter().chain(borrow) {
            if sum >= DAY_CAP_MS {
                break;
            }
            sum += ms(&e.it);
            sel.push(e.it);
        }
    }
    sel.sort_by(chrono);
    sel
}

struct DayItems {
    movies: Vec<PlayItem>,
    eps: Vec<PlayItem>,
    pilots: Vec<PlayItem>,
}

/// Pool zum Tag `d`: Filme und Serienfolgen mit `added_at` vor dem Tagesbeginn.
fn pool_of(ctx: &mut Context, d: i64) -> DayItems {
    let movies: Vec<PlayItem> = ctx.movies_at(d).into_iter().map(PlayItem::Movie).collect();
    let mut eps = Vec::new();
    let mut pilots = Vec::new();
    for sv in ctx.series_at(d).iter() {
        for e in &sv.v.eps {
            eps.push(PlayItem::Episode(e.clone()));
        }
        let pilot =
            sv.v.eps
                .iter()
                .find(|e| e.season == 1.0 && e.episode == 1.0)
                .or_else(|| sv.v.eps.first());
        if let Some(p) = pilot {
            pilots.push(PlayItem::Episode(p.clone()));
        }
    }
    DayItems {
        movies,
        eps,
        pilots,
    }
}

fn decade_name(dec: i64) -> String {
    if !(1900..2000).contains(&dec) {
        format!("Die {dec}er")
    } else {
        format!("Die {}er", dec - 1900)
    }
}

/// Planbare Titel einer Sammlung zum Tag `d` (Filme/Folgen im Pool, einmalig).
fn boxset_items(ctx: &mut Context, ids: &[String], d: i64) -> Vec<PlayItem> {
    let cut = ctx.day_start(d) as f64;
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            continue;
        }
        match ctx.catalog.items.get(id) {
            Some(Item::Movie(m)) => {
                if m.added_at < cut {
                    out.push(PlayItem::Movie(m.clone()));
                }
            }
            Some(Item::Episode(e)) => {
                if let Some(s) = ctx.catalog.items.get(&e.series_id)
                    && e.added_at.max(s.added_at()) < cut
                {
                    out.push(PlayItem::Episode(e.clone()));
                }
            }
            _ => {}
        }
    }
    out
}

/// Zulässig sind nur die ersten 12 Kandidaten der Rotation (Position, nicht Cache-Zustand).
fn boxset_cands(ctx: &Context, d: i64) -> Vec<Boxset> {
    let list: Vec<Boxset> = ctx
        .catalog
        .boxsets
        .iter()
        .filter(|x| x.child_count >= 2.0)
        .cloned()
        .collect();
    let mut r = rotated("themeday:boxset", &list, d, |b| b.id.clone());
    r.truncate(MAX_LOADS_PER_DAY);
    r
}

fn compute_theme(ctx: &mut Context, kind: Kind, d: i64) -> Option<Theme> {
    if kind == Kind::Boxset {
        for c in boxset_cands(ctx, d) {
            let Some(Some(ids)) = ctx.themes.boxsets.get(&c.id).cloned() else {
                continue;
            };
            let mut items = boxset_items(ctx, &ids, d);
            if items.len() >= MIN_ITEMS {
                items.sort_by(chrono);
                return Some(Theme {
                    name: format!("Filmreihe: {}", c.name),
                    items,
                });
            }
        }
        return None;
    }
    let p = pool_of(ctx, d);
    match kind {
        Kind::Overlength => {
            let items: Vec<PlayItem> = p
                .movies
                .iter()
                .filter(|m| m.dur_ms() >= OVERLENGTH_MS)
                .cloned()
                .collect();
            (items.len() >= MIN_ITEMS).then(|| Theme {
                name: "Überlänge".into(),
                items: pick_day(&items, "themeday:overlength", d),
            })
        }
        Kind::Pilot => (p.pilots.len() >= MIN_PILOTS).then(|| Theme {
            name: "Pilotfolgen-Tag".into(),
            items: pick_day(&p.pilots, "themeday:pilot", d),
        }),
        Kind::Decade => {
            struct Group {
                dec: i64,
                items: Vec<PlayItem>,
                ms: i64,
            }
            let mut by_dec: IndexMap<i64, Group> = IndexMap::new();
            for it in p.movies.iter().chain(p.eps.iter()) {
                let Some(y) = it.year() else { continue };
                let dec = y.div_euclid(10) * 10;
                let g = by_dec.entry(dec).or_insert_with(|| Group {
                    dec,
                    items: Vec::new(),
                    ms: 0,
                });
                g.items.push(it.clone());
                g.ms += it.dur_ms();
            }
            let cands: Vec<(String, i64, Vec<PlayItem>)> = by_dec
                .into_values()
                .filter(|g| g.ms >= DECADE_MIN_MS)
                .map(|g| (g.dec.to_string(), g.dec, g.items))
                .collect();
            let first = rotated("themeday:decade", &cands, d, |g| g.0.clone())
                .into_iter()
                .next();
            first.map(|(_, dec, items)| Theme {
                name: decade_name(dec),
                items: pick_day(&items, &format!("themeday:decade:{dec}"), d),
            })
        }
        Kind::Boxset => None,
    }
}

fn theme_for(ctx: &mut Context, kind: Kind, d: i64) -> Option<Rc<Theme>> {
    if let Some(t) = ctx.themes.themes.get(&(kind, d)) {
        return t.clone();
    }
    let t = compute_theme(ctx, kind, d).map(Rc::new);
    ctx.themes.themes.insert((kind, d), t.clone());
    t
}

/// Verfügbare Arten der Woche `w` in Permutationsreihenfolge (Hash der Woche).
fn week_order(ctx: &mut Context, w: i64) -> Vec<Kind> {
    let d0 = week_start(w);
    let mut avail: Vec<(Kind, u32)> = Vec::new();
    for k in KINDS {
        if theme_for(ctx, k, d0).is_some() {
            avail.push((k, key_hash("themeday", &format!("{w}:{}", k.name()))));
        }
    }
    avail.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| cmp_str(a.0.name(), b.0.name())));
    avail.into_iter().map(|x| x.0).collect()
}

/// Themen des Tages: erstes und gegebenenfalls zweites.
fn themes_for(ctx: &mut Context, d: i64) -> Vec<Rc<Theme>> {
    let order = week_order(ctx, week_of(d));
    let n = order.len() as i64;
    if n == 0 {
        return Vec::new();
    }
    let i = d - week_start(week_of(d));
    let mut out: Vec<Rc<Theme>> = Vec::new();
    let mut j = 0;
    while j < n && out.len() < 2 {
        let th = theme_for(ctx, order[((i + j).rem_euclid(n)) as usize], d);
        if let Some(t) = &th {
            out.push(t.clone());
        }
        if out.len() == 1 {
            let b = day_bounds(d);
            if let Some(t) = &th {
                let sum: i64 = t.items.iter().map(|it| slot_of(it.dur_ms())).sum();
                if sum >= b.end - b.start - HOUR {
                    break;
                }
            }
        }
        j += 1;
    }
    out
}

fn piece_of(it: &PlayItem, theme: &str) -> Piece {
    let mut p = Piece::new(it.id(), it.dur_ms(), it.age());
    p.series_id = it.series_id().map(str::to_string);
    p.theme = Some(theme.to_string());
    p
}

fn plan_themes(ctx: &mut Context, d: i64, themes: &[Rc<Theme>]) -> Vec<PlanEntry> {
    let mut bld = ctx.builder(18, d, IndexSet::new());
    let b = bld.bounds();
    let mut a: Vec<(PlayItem, String)> = Vec::new();
    let mut bq: Vec<(PlayItem, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for th in themes {
        for it in &th.items {
            if !seen.insert(it.id().to_string()) {
                continue;
            }
            let target = if stream_of_age(it.age()) == Stream::A {
                &mut a
            } else {
                &mut bq
            };
            target.push((it.clone(), th.name.clone()));
        }
    }
    while bld.cursor() < b.end && (!a.is_empty() || !bq.is_empty()) {
        let a_first = bld.cursor() < b.a2200;
        let mut done = false;
        for which in 0..2 {
            // vor 22:00 erst A, danach B; ab 22:00 erst B, danach A
            let use_a = (which == 0) == a_first;
            let list = if use_a { &mut a } else { &mut bq };
            let pos = list
                .iter()
                .position(|x| bld.can_place(&piece_of(&x.0, &x.1), None));
            if let Some(i) = pos {
                let x = list.remove(i);
                bld.place(&piece_of(&x.0, &x.1), None);
                done = true;
                break;
            }
        }
        if done {
            continue;
        }
        let gate = if bq.is_empty() {
            0
        } else if bld.cursor() < b.a2200 {
            b.a2200
        } else if bld.cursor() < b.a2300 {
            b.a2300
        } else {
            0
        };
        if gate == 0 {
            break;
        }
        bld.fill_until(gate);
    }
    bld.finish()
}

/// Plan von Sender 18 für den Sendetag `d`.
pub(crate) fn plan_day(ctx: &mut Context, d: i64) -> Vec<PlanEntry> {
    let themes = themes_for(ctx, d);
    plan_themes(ctx, d, &themes)
}

/// Nächste Sammlung, deren Inhalte für die Tage `days` noch geladen werden müssen (`None`, wenn
/// alles Nötige im Zwischenspeicher liegt). Der Aufrufer lädt die Inhalte (IO ist nicht Sache
/// des Planers) und meldet sie mit [`provide_boxset`]; danach erneut aufrufen. Die Abfolge
/// entspricht dem sequentiellen `prepare` des Web-Planers.
pub fn next_boxset_request(ctx: &mut Context, days: &[i64]) -> Option<String> {
    let mut all: Vec<i64> = days.to_vec();
    all.extend(days.iter().map(|d| week_start(week_of(*d))));
    all.sort_unstable();
    all.dedup();
    for d in all {
        for c in boxset_cands(ctx, d) {
            match ctx.themes.boxsets.get(&c.id).cloned() {
                None => return Some(c.id),
                Some(Some(ids)) => {
                    if boxset_items(ctx, &ids, d).len() >= MIN_ITEMS {
                        break;
                    }
                }
                Some(None) => {}
            }
        }
    }
    None
}

/// Inhalte einer Sammlung melden (`None` = Laden fehlgeschlagen oder keine Liste).
pub fn provide_boxset(ctx: &mut Context, id: &str, ids: Option<Vec<String>>) {
    ctx.themes.boxsets.insert(id.to_string(), ids);
    ctx.themes.themes.clear(); // neue Daten können früher berechnete Themen ändern
}
