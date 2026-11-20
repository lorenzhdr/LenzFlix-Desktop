//! Sender 17 „Premiere“.
//!
//! Einheiten: ein Film = 1 Einheit; neue Folgen einer Serie vom selben Sendetag (`added_at`) in
//! Folgenreihenfolge zu Blöcken (höchstens 3 Folgen, etwa 2 h). Premieren-Kette (unabhängig vom
//! berechneten Tag, je Katalog gemerkt): alle Einheiten mit `added_day >= EPOCH` nach
//! (`added_at`, Id); Kandidat = max(`added_day + 1`, vorherige Premiere + 1); liegt er mehr als
//! 15 Tage nach dem frühesten Tag, wird die Einheit verworfen. Tagesplan: Premiere 20:15
//! (Freigabe 16: 22:00, 18: 23:00, davor um 20:15 eine Wiederholung), Rest = Wiederholungen aus
//! dem Pool der Neuzugänge im Fenster W (30, 60, … 365 Tage, kleinstes mit mindestens 24 h),
//! Eimer-Rotation (Schlüssel `premiere`).

use std::collections::HashMap;
use std::rc::Rc;

use indexmap::{IndexMap, IndexSet};

use crate::catalog::PlayItem;
use crate::fill::{Cand, Piece, PlaceOpts, PlanEntry, PremiereKind, fill_segment, slot_of};
use crate::hash::cmp_str;
use crate::plan::Context;
use crate::pool::{
    Stream, bucket_count, cap_of, make_entries, rotation_with_borrow, stream_of_age,
};
use crate::rating::Age;
use crate::time::{HOUR, broadcast_day_of, day_of_date};

const MAX_WAIT_DAYS: i64 = 15;
const BLOCK_MAX_EPS: usize = 3;
const BLOCK_MAX_MS: i64 = 2 * HOUR;
const MIN_POOL_MS: i64 = 24 * HOUR;
const WINDOWS: [i64; 13] = [30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330, 360, 365];

/// Eine Einheit: Film oder Folgenblock.
pub(crate) struct Unit {
    id: String,
    film: bool,
    added_at: f64,
    day: i64,
    items: Vec<PlayItem>,
    age: Age,
    ms: i64,
}

/// Kette der Premieren (je Katalog einmal berechnet).
pub(crate) struct Info {
    units: Vec<Rc<Unit>>,
    by_day: HashMap<i64, Rc<Unit>>,
    pday: HashMap<String, i64>,
}

fn mk_unit(id: &str, film: bool, added_at: f64, items: Vec<PlayItem>) -> Unit {
    let mut age: Age = None;
    let mut ms = 0;
    for it in &items {
        if let Some(a) = it.age()
            && age.is_none_or(|x| a > x)
        {
            age = Some(a);
        }
        ms += it.dur_ms();
    }
    Unit {
        id: id.to_string(),
        film,
        added_at,
        day: broadcast_day_of(added_at as i64),
        items,
        age,
        ms,
    }
}

fn build_info(ctx: &Context) -> Info {
    let epoch = day_of_date(2026.0, 10.0, 1.0);
    let mut units: Vec<Unit> = Vec::new();
    for m in &ctx.catalog.movies {
        units.push(mk_unit(
            &m.id,
            true,
            m.added_at,
            vec![PlayItem::Movie(m.clone())],
        ));
    }
    for s in &ctx.catalog.series {
        let mut by_day: IndexMap<i64, Vec<(PlayItem, f64)>> = IndexMap::new();
        for e in &s.eps {
            let at = e.added_at.max(s.added_at);
            by_day
                .entry(broadcast_day_of(at as i64))
                .or_default()
                .push((PlayItem::Episode(e.clone()), at));
        }
        for g in by_day.values() {
            let mut cur: Vec<&(PlayItem, f64)> = Vec::new();
            let mut sum = 0i64;
            let flush = |cur: &mut Vec<&(PlayItem, f64)>, units: &mut Vec<Unit>| {
                if cur.is_empty() {
                    return;
                }
                let at = cur.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
                units.push(mk_unit(
                    cur[0].0.id(),
                    false,
                    at,
                    cur.iter().map(|x| x.0.clone()).collect(),
                ));
                cur.clear();
            };
            for x in g {
                let slot = slot_of(x.0.dur_ms());
                if !cur.is_empty() && (cur.len() >= BLOCK_MAX_EPS || sum + slot > BLOCK_MAX_MS) {
                    flush(&mut cur, &mut units);
                    sum = 0;
                }
                cur.push(x);
                sum += slot;
            }
            flush(&mut cur, &mut units);
        }
    }
    units.sort_by(|a, b| {
        a.added_at
            .partial_cmp(&b.added_at)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| cmp_str(&a.id, &b.id))
    });
    let units: Vec<Rc<Unit>> = units.into_iter().map(Rc::new).collect();
    let mut by_day: HashMap<i64, Rc<Unit>> = HashMap::new();
    let mut pday: HashMap<String, i64> = HashMap::new();
    let mut prev = i64::MIN / 4;
    for u in &units {
        if u.day < epoch {
            continue;
        }
        let cand = (u.day + 1).max(prev + 1);
        if cand > u.day + MAX_WAIT_DAYS {
            continue; // definiert verworfen
        }
        by_day.insert(cand, u.clone());
        pday.insert(u.id.clone(), cand);
        prev = cand;
    }
    Info {
        units,
        by_day,
        pday,
    }
}

/// Pool der Wiederholungen zum Tag `d` (nur bereits sendbare Einheiten, keine künftigen
/// Premieren), mit Fenster-Verlängerung.
fn rerun_pool(info: &Info, d: i64) -> Vec<Rc<Unit>> {
    let avail: Vec<&Rc<Unit>> = info
        .units
        .iter()
        .filter(|u| u.day < d && !info.pday.get(&u.id).is_some_and(|p| *p >= d))
        .collect();
    for w in WINDOWS {
        let ms: i64 = avail.iter().filter(|u| u.day >= d - w).map(|u| u.ms).sum();
        if ms >= MIN_POOL_MS || w == 365 {
            return avail
                .into_iter()
                .filter(|u| u.day >= d - w)
                .cloned()
                .collect();
        }
    }
    Vec::new()
}

fn pieces_of(
    u: &Unit,
    premiere: Option<PremiereKind>,
    label: Option<&str>,
    rerun: bool,
) -> Vec<Piece> {
    u.items
        .iter()
        .map(|it| {
            let mut p = Piece::new(it.id(), it.dur_ms(), it.age());
            p.rerun = rerun;
            p.premiere = premiere;
            p.label = label.map(str::to_string);
            p.series_id = it.series_id().map(str::to_string);
            p
        })
        .collect()
}

fn cand_of(u: &Unit) -> Cand {
    let pieces = pieces_of(u, None, None, true);
    let size = pieces.iter().map(|p| slot_of(p.dur_ms)).sum();
    Cand {
        id: u.id.clone(),
        movie: u.film,
        size,
        pieces,
    }
}

fn stream_cands(pool: &[Rc<Unit>], s: Stream, d: i64) -> Vec<Cand> {
    if pool.is_empty() {
        return Vec::new();
    }
    let sorted = make_entries("premiere", pool, |u| u.id.clone(), |u| u.ms);
    let c = bucket_count(sorted.iter().map(|e| e.ms).sum(), cap_of(s));
    let (home, borrow) = rotation_with_borrow(&sorted, c, d);
    home.iter()
        .chain(borrow.iter())
        .map(|e| cand_of(&e.it))
        .collect()
}

/// Plan von Sender 17 für den Sendetag `d`.
pub(crate) fn plan_day(ctx: &mut Context, d: i64) -> Vec<PlanEntry> {
    let info = match &ctx.premiere {
        Some(i) => i.clone(),
        None => {
            let i = Rc::new(build_info(ctx));
            ctx.premiere = Some(i.clone());
            i
        }
    };
    let prem: Option<Rc<Unit>> = info.by_day.get(&d).cloned();
    let pool = rerun_pool(&info, d);
    let pool_a: Vec<Rc<Unit>> = pool
        .iter()
        .filter(|u| stream_of_age(u.age) == Stream::A)
        .cloned()
        .collect();
    let pool_b: Vec<Rc<Unit>> = pool
        .iter()
        .filter(|u| stream_of_age(u.age) == Stream::B)
        .cloned()
        .collect();
    let cands_a = stream_cands(&pool_a, Stream::A, d);
    let cands_b = stream_cands(&pool_b, Stream::B, d);
    let prem_pieces: Option<Vec<Piece>> = prem.as_ref().map(|p| {
        if p.film {
            pieces_of(p, Some(PremiereKind::Film), None, false)
        } else {
            pieces_of(p, Some(PremiereKind::Episode), Some("Neue Folge"), false)
        }
    });
    // Premieren-Titel vorab sperren: das Füllprogramm darf sie nicht vorher senden;
    // erst beim Platzieren freigeben.
    let used: IndexSet<String> = prem_pieces
        .as_ref()
        .map(|ps| ps.iter().map(|p| p.item_id.clone()).collect())
        .unwrap_or_default();
    let mut bld = ctx.builder(17, d, used);
    let b = bld.bounds();
    let place_prem = |bld: &mut crate::fill::DayBuilder| {
        if let Some(ps) = &prem_pieces {
            for p in ps {
                bld.release(&p.item_id);
            }
            bld.place_block(ps, None);
        }
    };
    let without = |cands: &[Cand], idx: Option<usize>| -> Vec<Cand> {
        cands
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != idx)
            .map(|(_, c)| c.clone())
            .collect()
    };
    let late = prem.as_ref().is_some_and(|p| p.age.is_some_and(|a| a > 12));
    if late {
        let p_age = prem.as_ref().and_then(|p| p.age).unwrap_or(0);
        let at = if p_age >= 18 { b.a2300 } else { b.a2200 };
        let anchor = cands_a.iter().position(|c| c.size <= b.a2200 - b.a2015);
        fill_segment(&mut bld, &without(&cands_a, anchor), b.a2015, true);
        if let Some(ai) = anchor {
            bld.place_block(&cands_a[ai].pieces, Some(PlaceOpts::limit(b.a2200)));
        }
        fill_segment(&mut bld, &cands_a, b.a2200, true);
        if at > b.a2200 {
            fill_segment(&mut bld, &cands_b, at, true);
        }
        if bld.cursor() < at {
            bld.fill_until(at);
        }
        place_prem(&mut bld);
        fill_segment(&mut bld, &cands_b, b.end, true);
        return bld.finish();
    }
    let anchor_movie = if prem.is_some() {
        None
    } else {
        cands_a
            .iter()
            .position(|c| c.movie && b.a2015 + c.size <= b.end)
    };
    fill_segment(&mut bld, &without(&cands_a, anchor_movie), b.a2015, true);
    if prem.is_some() {
        place_prem(&mut bld);
    } else if let Some(ai) = anchor_movie {
        bld.place_block(&cands_a[ai].pieces, None);
    }
    fill_segment(&mut bld, &cands_a, b.a2200, false);
    fill_segment(&mut bld, &cands_b, b.end, true);
    bld.finish()
}
