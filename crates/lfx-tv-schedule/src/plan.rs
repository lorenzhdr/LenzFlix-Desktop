//! Planer für die Sender 1 bis 16 und Kontext für die Spezialsender 17 und 18.
//!
//! * Sender 1, 2, 5 bis 16: Mischsender mit Eimer-Rotation (`plan_mixed`).
//! * Sender 3: feste Wochen-Sendeplätze (`plan_serie`).
//! * Sender 4: Marathon (`plan_marathon`).
//! * Sender 17 (Premiere) und 18 (Thementag): `crate::premiere` und `crate::themeday`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};

use crate::catalog::{Catalog, Episode, Item, Movie, Series, norm_genre};
use crate::fill::{
    Cand, DayBuilder, Piece, PlanEntry, ShortItem, closedown_day, fill_segment, slot_of,
};
use crate::hash::{bitrev32, cmp_str, key_hash};
use crate::jsval::{JsVal, js_number, js_trim, object_entries_ordered};
use crate::pool::{
    CAP_A, CAP_B, Entry, Stream, bucket, bucket_count, make_entries, rotation_with_borrow,
    stream_of_age,
};
use crate::rating::Age;
use crate::series::{SeriesTools, View};
use crate::time::{HOUR, MIN, week_of, week_start};

const SHORT_MAX: i64 = 50 * MIN;
const MIN_POOL: i64 = 2 * HOUR;
const T_A: i64 = 16 * HOUR;
const T_B: i64 = 5 * HOUR;
/// Sender 3: feste Sendeplätze nur in `[dayStart, dayStart + 21 h)`.
const SLOT_END: i64 = 21 * HOUR;
/// 22:00 bis 02:00.
const B_SLOT_CAP: i64 = 4 * HOUR;

/// Sender in der Liste `channels(D)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub no: i64,
    pub key: String,
    pub name: String,
    /// `movies`, `series`, `special` oder `genre`.
    pub kind: &'static str,
    pub genre_label: Option<String>,
}

/// Serie im Pool eines Tages zusammen mit ihrer Sicht.
#[derive(Debug, Clone)]
pub struct SeriesAt {
    pub s: Arc<Series>,
    pub v: Rc<View>,
}

/// Pool-Eintrag eines Mischsenders: Film oder Serie.
#[derive(Debug, Clone)]
pub enum Work {
    Movie(Arc<Movie>),
    Series(SeriesAt),
}

impl Work {
    fn id(&self) -> String {
        match self {
            Work::Movie(m) => m.id.clone(),
            Work::Series(sv) => sv.s.id.clone(),
        }
    }
    fn ms(&self) -> i64 {
        match self {
            Work::Movie(m) => m.dur_ms,
            Work::Series(sv) => sv.v.auf_ms,
        }
    }
    fn age(&self) -> Age {
        match self {
            Work::Movie(m) => m.age,
            Work::Series(sv) => sv.v.age,
        }
    }
}

struct DayPool {
    movies: Vec<Arc<Movie>>,
    median_year: Option<i64>,
    genres: Vec<Option<String>>,
    g_movies: HashMap<i64, Vec<Arc<Movie>>>,
    g_series: HashMap<i64, Vec<SeriesAt>>,
}

struct ChPool {
    total_ms: i64,
    a: Vec<Entry<Work>>,
    b: Vec<Entry<Work>>,
}

/// Wochen-Sendeplatz von Sender 3.
#[derive(Debug, Clone)]
struct Slot {
    s: Arc<Series>,
    v: Rc<View>,
    off: i64,
    cw: i64,
    admit_w: i64,
}

struct MStream {
    eps: Vec<Arc<Episode>>,
    age: Age,
}

/// Planungskontext: Katalog, Senderbelegung und alle Zwischenspeicher. Die Zwischenspeicher
/// ändern keine Ergebnisse (die Pläne sind rein), sie sparen nur Rechenzeit.
pub struct Context {
    pub catalog: Arc<Catalog>,
    pub(crate) tools: SeriesTools,
    config: BTreeMap<i64, String>,
    known_genres: HashSet<String>,
    all_shorts: Vec<ShortItem>,
    all_eff: Vec<f64>,
    pools: HashMap<i64, Rc<DayPool>>,
    shorts_cache: HashMap<i64, Rc<Vec<ShortItem>>>,
    series_cache: HashMap<i64, Rc<Vec<SeriesAt>>>,
    ch_pools: HashMap<(i64, i64), Rc<ChPool>>,
    ch_lists: HashMap<i64, Rc<Vec<Channel>>>,
    week_tables: HashMap<i64, Rc<Vec<Slot>>>,
    pub(crate) premiere: Option<Rc<crate::premiere::Info>>,
    pub(crate) themes: crate::themeday::ThemeState,
}

/// Senderbelegung aus `channelConfig` (Objekt `{5: 'Action'}` oder Text `5=Action;6=Komödie`).
/// Plätze 5 bis 16, Genre normalisiert. Eigener Parser des Planers (anders als
/// [`crate::config::parse_channel_config`]: kein Entfernen von Anführungszeichen).
pub fn parse_plan_config(cfg: &JsVal) -> BTreeMap<i64, String> {
    let mut out = BTreeMap::new();
    let pairs: Vec<(String, JsVal)> = match cfg {
        JsVal::Str(s) => s
            .split(';')
            .filter_map(|p| {
                p.find('=')
                    .map(|i| (p[..i].to_string(), JsVal::Str(p[i + 1..].to_string())))
            })
            .collect(),
        JsVal::Obj(o) => object_entries_ordered(o)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        JsVal::Arr(a) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        _ => Vec::new(),
    };
    for (k, v) in pairs {
        let no = js_number(&JsVal::Str(js_trim(&k).to_string()));
        let g = norm_genre(&v);
        if no.is_finite() && no.fract() == 0.0 && (5.0..=16.0).contains(&no) && !g.is_empty() {
            out.insert(no as i64, g);
        }
    }
    out
}

fn fixed_channel(no: i64) -> Option<(&'static str, &'static str, &'static str)> {
    match no {
        1 => Some(("kino", "LIVE Kino", "movies")),
        2 => Some(("klassik", "LIVE Klassik", "movies")),
        3 => Some(("serie", "LIVE Serie", "series")),
        4 => Some(("marathon", "LIVE Marathon", "series")),
        17 => Some(("premiere", "LIVE Premiere", "special")),
        18 => Some(("thema", "LIVE Thementag", "special")),
        _ => None,
    }
}

fn pool_ms<T>(entries: &[Entry<T>]) -> i64 {
    entries.iter().map(|e| e.ms).sum()
}

impl Context {
    /// `createContext(catalog, { channelConfig })`.
    pub fn new(catalog: Arc<Catalog>, channel_config: &JsVal) -> Self {
        let known_genres = catalog.genres.iter().map(|g| g.norm.clone()).collect();
        // Statische Hash-Reihenfolge der kurzen Titel (bitrev32(h); Gleichstand: Katalogreihenfolge).
        let mut list: Vec<ShortItem> = Vec::new();
        for m in &catalog.movies {
            if m.dur_ms <= SHORT_MAX {
                list.push(ShortItem {
                    id: m.id.clone(),
                    dur_ms: m.dur_ms,
                    age: m.age,
                    series_id: None,
                });
            }
        }
        let mut effs: Vec<f64> = catalog
            .movies
            .iter()
            .filter(|m| m.dur_ms <= SHORT_MAX)
            .map(|m| m.added_at)
            .collect();
        for s in &catalog.series {
            for e in &s.eps {
                if e.dur_ms <= SHORT_MAX {
                    list.push(ShortItem {
                        id: e.id.clone(),
                        dur_ms: e.dur_ms,
                        age: e.age,
                        series_id: Some(e.series_id.clone()),
                    });
                    let sa = catalog
                        .items
                        .get(&e.series_id)
                        .map_or(f64::NEG_INFINITY, Item::added_at);
                    effs.push(e.added_at.max(sa));
                }
            }
        }
        let mut order: Vec<(u32, usize)> = list
            .iter()
            .enumerate()
            .map(|(i, it)| (bitrev32(key_hash("fill", &it.id)), i))
            .collect();
        order.sort_unstable();
        let all_shorts = order.iter().map(|&(_, i)| list[i].clone()).collect();
        let all_eff = order.iter().map(|&(_, i)| effs[i]).collect();
        Context {
            catalog,
            tools: SeriesTools::new(),
            config: parse_plan_config(channel_config),
            known_genres,
            all_shorts,
            all_eff,
            pools: HashMap::new(),
            shorts_cache: HashMap::new(),
            series_cache: HashMap::new(),
            ch_pools: HashMap::new(),
            ch_lists: HashMap::new(),
            week_tables: HashMap::new(),
            premiere: None,
            themes: crate::themeday::ThemeState::default(),
        }
    }

    /// Sendetagsbeginn (ms), gecacht.
    pub fn day_start(&mut self, d: i64) -> i64 {
        self.tools.day_start(d)
    }

    /// Kurze Titel (höchstens 50 min) mit `added_at` vor dem Tagesbeginn, in Hash-Reihenfolge.
    pub fn shorts(&mut self, d: i64) -> Rc<Vec<ShortItem>> {
        if let Some(v) = self.shorts_cache.get(&d) {
            return v.clone();
        }
        let cut = self.day_start(d) as f64;
        let v: Vec<ShortItem> = self
            .all_shorts
            .iter()
            .zip(&self.all_eff)
            .filter(|(_, eff)| **eff < cut)
            .map(|(it, _)| it.clone())
            .collect();
        let v = Rc::new(v);
        self.shorts_cache.insert(d, v.clone());
        v
    }

    /// Tagesbaustein für Sender `no` (Hash-Schlüssel `String(no)`).
    pub fn builder(&mut self, no: i64, d: i64, used: IndexSet<String>) -> DayBuilder {
        let shorts = self.shorts(d);
        DayBuilder::new(d, None, used, shorts, Some(&no.to_string()))
    }

    /// Serien im Pool zum Stichtag `day_start(D)`.
    pub fn series_at(&mut self, d: i64) -> Rc<Vec<SeriesAt>> {
        if let Some(v) = self.series_cache.get(&d) {
            return v.clone();
        }
        let cut = self.day_start(d);
        let mut v = Vec::new();
        let catalog = self.catalog.clone();
        for s in &catalog.series {
            if let Some(view) = self.tools.view(s, cut) {
                v.push(SeriesAt {
                    s: s.clone(),
                    v: view,
                });
            }
        }
        let v = Rc::new(v);
        self.series_cache.insert(d, v.clone());
        v
    }

    /// Filme mit `added_at` vor dem Tagesbeginn von `d`.
    pub fn movies_at(&mut self, d: i64) -> Vec<Arc<Movie>> {
        let cut = self.day_start(d) as f64;
        self.catalog
            .movies
            .iter()
            .filter(|m| m.added_at < cut)
            .cloned()
            .collect()
    }

    /// Pool zum Stichtag, Genre-Belegung der Plätze 5 bis 16 und Genre-Zuordnung (jeder Titel
    /// auf genau einem Genre-Sender).
    fn day_pool(&mut self, d: i64) -> Rc<DayPool> {
        if let Some(p) = self.pools.get(&d) {
            return p.clone();
        }
        let movies = self.movies_at(d);
        let series = self.series_at(d);
        let mut years: Vec<i64> = movies.iter().filter_map(|m| m.year).collect();
        years.sort_unstable();
        let median_year = if years.is_empty() {
            None
        } else {
            Some(years[(years.len() - 1) / 2])
        };
        let mut counts: IndexMap<String, i64> = IndexMap::new();
        for m in &movies {
            for g in &m.genres {
                *counts.entry(g.clone()).or_insert(0) += 1;
            }
        }
        for sv in series.iter() {
            for g in &sv.s.genres {
                *counts.entry(g.clone()).or_insert(0) += 1;
            }
        }
        let mut top: Vec<(&String, &i64)> = counts.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then_with(|| cmp_str(a.0, b.0)));
        let top: Vec<String> = top.into_iter().map(|x| x.0.clone()).collect();
        let mut genres: Vec<Option<String>> = vec![None; 17];
        let mut reserved: HashSet<String> = HashSet::new();
        for p in 5..=16i64 {
            if let Some(g) = self.config.get(&p)
                && self.known_genres.contains(g)
                && !reserved.contains(g)
            {
                genres[p as usize] = Some(g.clone());
                reserved.insert(g.clone());
            }
        }
        let mut ti = 0;
        for slot in &mut genres[5..=16] {
            if slot.is_some() {
                continue;
            }
            while ti < top.len() && reserved.contains(&top[ti]) {
                ti += 1;
            }
            if ti < top.len() {
                *slot = Some(top[ti].clone());
                ti += 1;
            }
        }
        let places: Vec<i64> = (5..=16i64)
            .filter(|p| genres[*p as usize].is_some())
            .collect();
        let mut g_movies: HashMap<i64, Vec<Arc<Movie>>> = HashMap::new();
        let mut g_series: HashMap<i64, Vec<SeriesAt>> = HashMap::new();
        for p in &places {
            g_movies.insert(*p, Vec::new());
            g_series.insert(*p, Vec::new());
        }
        let assign = |id: &str, t_genres: &[String]| -> i64 {
            let fit: Vec<i64> = places
                .iter()
                .copied()
                .filter(|p| {
                    genres[*p as usize]
                        .as_ref()
                        .is_some_and(|g| t_genres.contains(g))
                })
                .collect();
            if fit.is_empty() {
                0
            } else {
                fit[key_hash("genre", id) as usize % fit.len()]
            }
        };
        for m in &movies {
            let p = assign(&m.id, &m.genres);
            if p != 0 {
                g_movies.get_mut(&p).expect("Platz belegt").push(m.clone());
            }
        }
        for sv in series.iter() {
            let p = assign(&sv.s.id, &sv.s.genres);
            if p != 0 {
                g_series.get_mut(&p).expect("Platz belegt").push(sv.clone());
            }
        }
        let pool = Rc::new(DayPool {
            movies,
            median_year,
            genres,
            g_movies,
            g_series,
        });
        self.pools.insert(d, pool.clone());
        pool
    }

    /// Die 18 Sender des Tages `d`.
    pub fn channels(&mut self, d: i64) -> Rc<Vec<Channel>> {
        if let Some(l) = self.ch_lists.get(&d) {
            return l.clone();
        }
        let p = self.day_pool(d);
        let mut list = Vec::new();
        for no in 1..=18i64 {
            if let Some((key, name, kind)) = fixed_channel(no) {
                list.push(Channel {
                    no,
                    key: key.into(),
                    name: name.into(),
                    kind,
                    genre_label: None,
                });
            } else if let Some(g) = &p.genres[no as usize] {
                let label = self.catalog.genre_label(g);
                list.push(Channel {
                    no,
                    key: format!("genre:{g}"),
                    name: format!("LIVE {label}"),
                    kind: "genre",
                    genre_label: Some(label),
                });
            } else {
                list.push(Channel {
                    no,
                    key: "genre:".into(),
                    name: format!("LIVE {no}"),
                    kind: "genre",
                    genre_label: None,
                });
            }
        }
        let l = Rc::new(list);
        self.ch_lists.insert(d, l.clone());
        l
    }

    // ---- Sender 1, 2, 5-16: Mischsender mit Eimer-Rotation ---------------------------------

    fn pool_of(&mut self, no: i64, d: i64) -> Rc<ChPool> {
        if let Some(c) = self.ch_pools.get(&(d, no)) {
            return c.clone();
        }
        let p = self.day_pool(d);
        let key: String;
        let mut movies: Vec<Arc<Movie>> = Vec::new();
        let mut series: Vec<SeriesAt> = Vec::new();
        if no == 1 || no == 2 {
            key = if no == 1 { "kino" } else { "klassik" }.to_string();
            let med = p.median_year;
            movies = p
                .movies
                .iter()
                .filter(|m| (m.year.is_none() || med.is_none() || m.year >= med) == (no == 1))
                .cloned()
                .collect();
        } else {
            let g = p.genres[no as usize].clone();
            key = format!("genre:{}", g.as_deref().unwrap_or(""));
            if g.is_some() {
                movies = p.g_movies.get(&no).cloned().unwrap_or_default();
                series = p.g_series.get(&no).cloned().unwrap_or_default();
            }
        }
        let mut items: Vec<Work> = movies.iter().map(|m| Work::Movie(m.clone())).collect();
        items.extend(series.iter().map(|sv| Work::Series(sv.clone())));
        let a: Vec<Work> = items
            .iter()
            .filter(|w| stream_of_age(w.age()) == Stream::A)
            .cloned()
            .collect();
        let b: Vec<Work> = items
            .iter()
            .filter(|w| stream_of_age(w.age()) == Stream::B)
            .cloned()
            .collect();
        let mut total_ms: i64 = movies.iter().map(|m| m.dur_ms).sum();
        total_ms += series.iter().map(|sv| sv.v.total_ms).sum::<i64>();
        let cp = Rc::new(ChPool {
            total_ms,
            a: make_entries(&key, &a, Work::id, Work::ms),
            b: make_entries(&key, &b, Work::id, Work::ms),
        });
        self.ch_pools.insert((d, no), cp.clone());
        cp
    }

    fn cand_of(&mut self, d: i64, w: &Work, c: i64, borrowed: bool) -> Cand {
        match w {
            Work::Movie(m) => Cand {
                id: m.id.clone(),
                movie: true,
                size: slot_of(m.dur_ms),
                pieces: vec![Piece::new(m.id.clone(), m.dur_ms, m.age)],
            },
            Work::Series(sv) => {
                let (s, v) = (&sv.s, &sv.v);
                let base = v.admit.div_euclid(c);
                let k0 = d.div_euclid(c) - base;
                let k = if borrowed { (k0 - 1).max(0) } else { k0 };
                let wkey = format!("{}|m|{}|{}|{}", s.id, c, v.admit, v.b);
                let admit = v.admit;
                let eps = self
                    .tools
                    .block(s, &wkey, k, v.b, &|kk| admit.max((base + kk) * c));
                let pieces: Vec<Piece> = eps
                    .iter()
                    .map(|e| {
                        let mut p = Piece::new(e.id.clone(), e.dur_ms, v.age);
                        p.series_id = Some(s.id.clone());
                        p.rerun = borrowed;
                        p
                    })
                    .collect();
                let size = pieces.iter().map(|p| slot_of(p.dur_ms)).sum();
                Cand {
                    id: s.id.clone(),
                    movie: false,
                    size,
                    pieces,
                }
            }
        }
    }

    fn stream_cands(&mut self, d: i64, sorted: &[Entry<Work>], cap: i64) -> Vec<Cand> {
        if sorted.is_empty() {
            return Vec::new();
        }
        let c = bucket_count(pool_ms(sorted), cap);
        let (home, borrow) = rotation_with_borrow(sorted, c, d);
        let mut out: Vec<Cand> = home
            .iter()
            .map(|e| self.cand_of(d, &e.it, c, false))
            .collect();
        out.extend(borrow.iter().map(|e| self.cand_of(d, &e.it, c, true)));
        out
    }

    fn plan_mixed(&mut self, no: i64, d: i64) -> Vec<PlanEntry> {
        let cp = self.pool_of(no, d);
        if cp.total_ms < MIN_POOL {
            return closedown_day(d, "empty");
        }
        let mut bld = self.builder(no, d, IndexSet::new());
        let b = bld.bounds();
        let cands_a = self.stream_cands(d, &cp.a, CAP_A);
        let cands_b = self.stream_cands(d, &cp.b, CAP_B);
        let anchor = cands_a
            .iter()
            .position(|c| c.movie && b.a2015 + c.size <= b.end);
        if let Some(ai) = anchor {
            let rest: Vec<Cand> = cands_a
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != ai)
                .map(|(_, c)| c.clone())
                .collect();
            fill_segment(&mut bld, &rest, b.a2015, true);
            bld.place_block(&cands_a[ai].pieces, None);
        }
        fill_segment(&mut bld, &cands_a, b.a2200, false);
        fill_segment(&mut bld, &cands_b, b.end, true);
        bld.finish()
    }

    // ---- Sender 3: feste Wochen-Sendeplätze ------------------------------------------------

    /// Slot-Tabelle der Woche `w` (A zuerst, dann B; gleiche Wanduhrzeit Mo bis So).
    fn week_table(&mut self, w: i64) -> Rc<Vec<Slot>> {
        if let Some(t) = self.week_tables.get(&w) {
            return t.clone();
        }
        let ws = week_start(w);
        let svs = self.series_at(ws);
        let mut slots: Vec<Slot> = Vec::new();
        let mut off = 0i64;
        for st in [Stream::A, Stream::B] {
            let list_src: Vec<SeriesAt> = svs
                .iter()
                .filter(|sv| stream_of_age(sv.v.age) == st)
                .cloned()
                .collect();
            let sorted = make_entries("serie", &list_src, |x| x.s.id.clone(), |x| x.v.slot_len);
            let cap = if st == Stream::A { CAP_A } else { B_SLOT_CAP };
            let cw = if sorted.is_empty() {
                1
            } else {
                bucket_count(pool_ms(&sorted), cap)
            };
            let (home, borrow) = rotation_with_borrow(&sorted, cw, w);
            let list: Vec<Entry<SeriesAt>> = home.into_iter().chain(borrow).collect();
            if st == Stream::B {
                off = off.max(CAP_A);
            }
            let mut deferred: Vec<Entry<SeriesAt>> = Vec::new();
            for e in list {
                let x = &e.it;
                if st == Stream::B && x.v.age == Some(18) && off < 18 * HOUR {
                    deferred.push(e);
                    continue;
                }
                let lim = if st == Stream::A { CAP_A } else { SLOT_END };
                if off < lim && off + x.v.slot_len <= SLOT_END {
                    slots.push(Slot {
                        s: x.s.clone(),
                        v: x.v.clone(),
                        off,
                        cw,
                        admit_w: week_of(x.v.admit),
                    });
                    off += x.v.slot_len;
                }
            }
            if st == Stream::B {
                off = off.max(18 * HOUR);
                for e in deferred {
                    let x = &e.it;
                    if off + x.v.slot_len <= SLOT_END {
                        slots.push(Slot {
                            s: x.s.clone(),
                            v: x.v.clone(),
                            off,
                            cw,
                            admit_w: week_of(x.v.admit),
                        });
                        off += x.v.slot_len;
                    }
                }
            }
        }
        let t = Rc::new(slots);
        self.week_tables.insert(w, t.clone());
        t
    }

    fn slot_pieces(&mut self, slot: &Slot, d: i64) -> Vec<Piece> {
        let (s, v, cw, admit_w) = (&slot.s, &slot.v, slot.cw, slot.admit_w);
        let w = week_of(d);
        let base_w = admit_w.div_euclid(cw);
        let k = 7 * (w.div_euclid(cw) - base_w) + (d - week_start(w));
        let wkey = format!("{}|w|{}|{}|{}", s.id, cw, v.admit, v.b);
        let admit = v.admit;
        let eps = self.tools.block(s, &wkey, k.max(0), v.b, &|kk| {
            admit.max(week_start((base_w + kk.div_euclid(7)) * cw) + kk.rem_euclid(7))
        });
        eps.iter()
            .map(|e| {
                let mut p = Piece::new(e.id.clone(), e.dur_ms, v.age);
                p.series_id = Some(s.id.clone());
                p
            })
            .collect()
    }

    fn plan_serie(&mut self, no: i64, d: i64) -> Vec<PlanEntry> {
        let table = self.week_table(week_of(d));
        if table.is_empty() {
            return closedown_day(d, "empty");
        }
        let mut bld = self.builder(no, d, IndexSet::new());
        let b = bld.bounds();
        for slot in table.iter() {
            let start = b.start + slot.off;
            if bld.cursor() < start {
                bld.fill_until(start);
            }
            let pieces = self.slot_pieces(slot, d);
            bld.place_block(&pieces, None);
        }
        let night_start = b.start + SLOT_END;
        if bld.cursor() < night_start {
            bld.fill_until(night_start);
        }
        // Nacht bis 05:00: Wiederholung der Folgen von gestern
        let prev = self.week_table(week_of(d - 1));
        let order: Vec<Slot> = prev
            .iter()
            .filter(|x| x.off >= CAP_A)
            .chain(prev.iter().filter(|x| x.off < CAP_A))
            .cloned()
            .collect();
        let mut cands: Vec<Cand> = Vec::new();
        for slot in &order {
            let mut pieces = self.slot_pieces(slot, d - 1);
            for p in &mut pieces {
                p.rerun = true;
            }
            if !pieces.is_empty() {
                let size = pieces.iter().map(|p| slot_of(p.dur_ms)).sum();
                cands.push(Cand {
                    id: slot.s.id.clone(),
                    movie: false,
                    pieces,
                    size,
                });
            }
        }
        fill_segment(&mut bld, &cands, b.end, true);
        bld.finish()
    }

    // ---- Sender 4: Marathon ----------------------------------------------------------------

    /// Folgen des Marathon-Tags eines Stroms.
    fn marathon_stream(&mut self, d: i64, st: Stream, t: i64) -> Option<MStream> {
        let svs: Vec<SeriesAt> = self
            .series_at(d)
            .iter()
            .filter(|x| stream_of_age(x.v.age) == st)
            .cloned()
            .collect();
        if svs.is_empty() {
            return None;
        }
        let sorted = make_entries("marathon", &svs, |x| x.s.id.clone(), |_| 0);
        let m = sorted.len() as i64;
        let mut c = 1i64;
        while c * 2 <= m {
            c *= 2;
        }
        let mut bk = bucket(&sorted, c, d);
        if bk.is_empty() && c > 1 {
            bk = bucket(&sorted, c, d + c / 2);
        }
        let mut i = 1;
        while bk.is_empty() && i < c {
            bk = bucket(&sorted, c, d + i);
            i += 1;
        }
        if bk.is_empty() {
            bk = sorted.clone();
        }
        let q = i64::from((d as u32) >> c.trailing_zeros());
        let blen = bk.len() as i64;
        let first = bk[(q % blen) as usize].it.clone();
        let n = q / blen;
        let mut out: Vec<Arc<Episode>> = Vec::new();
        let v0 = first.v.clone();
        if v0.total_ms > t {
            let lo = n * t;
            let hi = lo + t;
            let mut r = lo.div_euclid(v0.total_ms);
            while r * v0.total_ms < hi {
                let mut g = r * v0.total_ms;
                for e in &v0.eps {
                    if g >= lo && g < hi {
                        out.push(e.clone());
                    }
                    g += e.dur_ms;
                }
                r += 1;
            }
        } else {
            let order: Vec<&SeriesAt> = sorted.iter().map(|e| &e.it).collect();
            let mut at = order
                .iter()
                .position(|w| Arc::ptr_eq(&w.s, &first.s))
                .unwrap_or(0);
            let mut total = 0i64;
            let mut i = 0;
            while i < order.len() && total < t {
                let v = &order[at].v;
                let mut off = 0i64;
                for e in &v.eps {
                    if total + off >= t {
                        break;
                    }
                    out.push(e.clone());
                    off += e.dur_ms;
                }
                total += off;
                i += 1;
                at = (at + 1) % order.len();
            }
        }
        Some(MStream {
            eps: out,
            age: v0.age,
        })
    }

    fn plan_marathon(&mut self, no: i64, d: i64) -> Vec<PlanEntry> {
        if self.series_at(d).is_empty() {
            return closedown_day(d, "empty");
        }
        let mut bld = self.builder(no, d, IndexSet::new());
        let b = bld.bounds();
        let run = |bld: &mut DayBuilder, st: &MStream| {
            for e in &st.eps {
                let mut p = Piece::new(e.id.clone(), e.dur_ms, st.age);
                p.series_id = Some(e.series_id.clone());
                if !bld.place_block(&[p], None) {
                    break;
                }
            }
        };
        if let Some(a) = self.marathon_stream(d, Stream::A, T_A) {
            run(&mut bld, &a);
        }
        if bld.cursor() < b.a2200 {
            bld.fill_until(b.a2200);
        }
        if let Some(s) = self.marathon_stream(d, Stream::B, T_B) {
            if s.age == Some(18) && bld.cursor() < b.a2300 {
                bld.fill_until(b.a2300);
            }
            run(&mut bld, &s);
        }
        bld.finish()
    }

    /// Plan der Sender 1 bis 16 (17 und 18 siehe Spezialsender).
    pub fn plan_builtin(&mut self, no: i64, d: i64) -> Vec<PlanEntry> {
        if no == 1 || no == 2 || (5..=16).contains(&no) {
            self.plan_mixed(no, d)
        } else if no == 3 {
            self.plan_serie(no, d)
        } else if no == 4 {
            self.plan_marathon(no, d)
        } else {
            closedown_day(d, "empty")
        }
    }
}
