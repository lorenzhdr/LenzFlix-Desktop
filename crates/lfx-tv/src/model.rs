//! Modell: reine Funktionen auf der Planer-Schnittstelle (Port von `tvmode/model.js` und
//! `ui/overlay-util.js`). Kein Netzwerk, keine Lokalzeit; Zeit nur über `lfx-tv-schedule::time`.

use std::collections::HashMap;

use lfx_tv_schedule::fill::PremiereKind;
use lfx_tv_schedule::jsval::{is_js_ws, js_trim, num_to_string, round_js};
use lfx_tv_schedule::time::{broadcast_day_of, format_clock, format_day};
use lfx_tv_schedule::{Channel, EntryType, PlanEntry};

use crate::plan::{ItemInfo, ItemType, PlanSource, genre_key};

/// Anzeige-Art eines Planeintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Program,
    Fill,
    Filler,
    Closedown,
}

impl Kind {
    /// Name wie im Web.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Program => "program",
            Kind::Fill => "fill",
            Kind::Filler => "filler",
            Kind::Closedown => "closedown",
        }
    }
}

/// `kindOf(entry)`: `nightfill` zählt als `fill`.
pub fn kind_of(entry: &PlanEntry) -> Kind {
    match entry.kind {
        EntryType::Program => Kind::Program,
        EntryType::Fill | EntryType::Nightfill => Kind::Fill,
        EntryType::Filler => Kind::Filler,
        EntryType::Closedown => Kind::Closedown,
    }
}

/// Titel und Untertitel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TitleSub {
    pub title: String,
    pub sub: String,
}

/// Nächste Sendung nach `e` (Füller werden übersprungen, höchstens 4 Schritte). Kein Fehler: dann
/// der erste gefundene Eintrag oder `None`.
pub fn next_program<P: PlanSource + ?Sized>(
    plan: &mut P,
    no: i64,
    e: &PlanEntry,
) -> Option<PlanEntry> {
    let mut cur = e.clone();
    let mut first: Option<PlanEntry> = None;
    for _ in 0..4 {
        let Ok(n) = plan.now_on(no, cur.end) else {
            return first;
        };
        let n = n.entry;
        if n.start <= cur.start {
            return first;
        }
        if first.is_none() {
            first = Some(n.clone());
        }
        if n.kind != EntryType::Filler {
            return Some(n);
        }
        cur = n;
    }
    first
}

fn item_of<P: PlanSource + ?Sized>(plan: &P, entry: &PlanEntry) -> Option<ItemInfo> {
    entry.item_id.as_deref().and_then(|id| plan.item(id))
}

fn known(n: Option<f64>) -> Option<f64> {
    n.filter(|v| *v != 9999.0)
}

/// Serie: Serienname + `S1 E3 · Folgentitel`; Film: Titel.
pub fn title_of<P: PlanSource + ?Sized>(plan: &P, entry: &PlanEntry) -> TitleSub {
    if entry.kind == EntryType::Closedown {
        return TitleSub {
            title: "Sendepause".into(),
            sub: String::new(),
        };
    }
    let Some(it) = item_of(plan, entry) else {
        return TitleSub {
            title: entry.label.clone().unwrap_or_default(),
            sub: String::new(),
        };
    };
    if it.kind == ItemType::Episode
        && let Some(series) = it.series_title.as_deref().filter(|s| !s.is_empty())
    {
        let mut code = String::new();
        if let Some(s) = known(it.season) {
            code.push('S');
            code.push_str(&num_to_string(s));
        }
        if let Some(ep) = known(it.episode) {
            if !code.is_empty() {
                code.push(' ');
            }
            code.push('E');
            code.push_str(&num_to_string(ep));
        }
        let name = if !it.title.is_empty() && it.title != series {
            it.title.as_str()
        } else {
            ""
        };
        let sub = [code.as_str(), name]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" · ");
        return TitleSub {
            title: series.to_string(),
            sub,
        };
    }
    let title = if it.title.is_empty() {
        "Unbekannter Titel".to_string()
    } else {
        it.title
    };
    TitleSub {
        title,
        sub: String::new(),
    }
}

/// `HH:MM–HH:MM` eines Eintrags.
pub fn time_range(e: &PlanEntry) -> String {
    format!("{}–{}", format_clock(e.start), format_clock(e.end))
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// Fortschritt 0..1 der Sendung `e` zum Zeitpunkt `t`.
pub fn progress_of(t: i64, e: &PlanEntry) -> f64 {
    clamp01((t - e.start) as f64 / (e.end - e.start).max(1) as f64)
}

/// Ein Sender „jetzt“.
#[derive(Debug, Clone, PartialEq)]
pub struct NowItem {
    pub channel: Channel,
    pub entry: Option<PlanEntry>,
    /// 0..1
    pub progress: f64,
    /// Nächste Sendung ohne Füller.
    pub next: Option<PlanEntry>,
}

/// Alle 18 Sender zum Zeitpunkt `t` (Köpfe aus `channels(Sendetag von t)`).
pub fn now_all<P: PlanSource + ?Sized>(plan: &mut P, t: i64) -> Vec<NowItem> {
    let d = broadcast_day_of(t);
    let channels = plan.channels(d);
    let mut out = Vec::with_capacity(channels.len());
    for channel in channels.iter() {
        let Ok(n) = plan.now_on(channel.no, t) else {
            out.push(NowItem {
                channel: channel.clone(),
                entry: None,
                progress: 0.0,
                next: None,
            });
            continue;
        };
        let entry = n.entry;
        let progress = progress_of(t, &entry);
        let next = next_program(plan, channel.no, &entry);
        out.push(NowItem {
            channel: channel.clone(),
            entry: Some(entry),
            progress,
            next,
        });
    }
    out
}

/// Zelle im Programm-Raster.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub entry: PlanEntry,
    /// Auf das Fenster zugeschnitten.
    pub start: i64,
    /// Auf das Fenster zugeschnitten.
    pub end: i64,
    pub title: String,
    pub sub: String,
    pub kind: Kind,
    /// Laufende Zelle (nur mit `now`).
    pub now: bool,
}

/// Zeile im Programm-Raster.
#[derive(Debug, Clone, PartialEq)]
pub struct GridRow {
    pub channel: Channel,
    pub cells: Vec<Cell>,
}

fn cell_of<P: PlanSource + ?Sized>(
    plan: &mut P,
    no: i64,
    e: &PlanEntry,
    from: i64,
    to: i64,
    now: Option<i64>,
) -> Cell {
    let kind = kind_of(e);
    let (title, sub) = match kind {
        Kind::Closedown => ("Sendepause".to_string(), String::new()),
        Kind::Filler => {
            let nt = next_program(plan, no, e)
                .filter(|nx| nx.kind != EntryType::Closedown && nx.kind != EntryType::Filler)
                .map(|nx| title_of(plan, &nx).title)
                .unwrap_or_default();
            let title = if nt.is_empty() {
                "Kurze Pause".to_string()
            } else {
                format!("Gleich: {nt}")
            };
            (title, String::new())
        }
        Kind::Fill => ("Füllprogramm".to_string(), title_of(plan, e).title),
        Kind::Program => {
            let t = title_of(plan, e);
            (t.title, t.sub)
        }
    };
    Cell {
        entry: e.clone(),
        start: e.start.max(from),
        end: e.end.min(to),
        title,
        sub,
        kind,
        now: now.is_some_and(|n| e.start <= n && n < e.end),
    }
}

/// Programm-Raster für das Fenster `[from, to)`. `d` = Sendetag der Senderköpfe (Standard: Tag von
/// `from`); `now` markiert die laufende Zelle.
pub fn grid_rows<P: PlanSource + ?Sized>(
    plan: &mut P,
    from: i64,
    to: i64,
    d: Option<i64>,
    now: Option<i64>,
) -> Vec<GridRow> {
    let d0 = d.unwrap_or_else(|| broadcast_day_of(from));
    let channels = plan.channels(d0);
    let mut rows: Vec<GridRow> = channels
        .iter()
        .map(|c| GridRow {
            channel: c.clone(),
            cells: Vec::new(),
        })
        .collect();
    if to <= from {
        return rows;
    }
    let d1 = broadcast_day_of(to - 1);
    let dstart = d0.min(broadcast_day_of(from));
    for row in &mut rows {
        for day in dstart..=d1 {
            let Ok(list) = plan.plan_day(row.channel.no, day) else {
                continue;
            };
            for e in list.iter() {
                if e.end <= from || e.start >= to {
                    continue;
                }
                let c = cell_of(plan, row.channel.no, e, from, to, now);
                row.cells.push(c);
            }
        }
    }
    rows
}

/// Beschreibung für Info-Bereich und Info-Karte.
#[derive(Debug, Clone, PartialEq)]
pub struct Description {
    pub title: String,
    pub sub: String,
    /// `20:15–22:05`
    pub time: String,
    pub progress: f64,
    pub badges: Vec<String>,
    pub kind: Kind,
    pub item_id: Option<String>,
    pub series_id: Option<String>,
    /// Id, deren Bild gezeigt wird (Serie bei Folgen); `None` ohne Katalogtitel.
    pub image_item_id: Option<String>,
    pub overview: String,
    pub year: Option<i64>,
    pub genres: Vec<String>,
    pub run_min: Option<i64>,
}

/// Anzeige-Labels zu normalisierten Genres; Unbekannte werden groß geschrieben.
fn genre_labels<P: PlanSource + ?Sized>(plan: &P, norms: &[String]) -> Vec<String> {
    let map: HashMap<String, String> = plan
        .genres()
        .into_iter()
        .map(|(label, _)| (genre_key(&label), label))
        .collect();
    norms
        .iter()
        .map(|n| {
            map.get(n).cloned().unwrap_or_else(|| {
                let mut ch = n.chars();
                match ch.next() {
                    Some(c) => c.to_uppercase().collect::<String>() + ch.as_str(),
                    None => String::new(),
                }
            })
        })
        .collect()
}

/// Badge-Texte eines Eintrags.
pub fn badges(entry: &PlanEntry) -> Vec<String> {
    let mut b = Vec::new();
    match entry.premiere {
        Some(PremiereKind::Film) => b.push("Premiere".to_string()),
        Some(PremiereKind::Episode) => b.push("Neue Folge".to_string()),
        None => {}
    }
    if entry.rerun {
        b.push("Wdh.".to_string());
    }
    if let Some(a @ (16 | 18)) = entry.age {
        b.push(format!("ab {a}"));
    }
    b
}

/// `describe(scheduler, no, entry, t, { overviews })`.
pub fn describe<P: PlanSource + ?Sized>(
    plan: &mut P,
    no: i64,
    entry: &PlanEntry,
    t: i64,
    overviews: Option<&HashMap<String, String>>,
) -> Description {
    let kind = kind_of(entry);
    let it = item_of(plan, entry);
    let tt = match kind {
        Kind::Filler => TitleSub {
            title: cell_of(plan, no, entry, entry.start, entry.end, None).title,
            sub: String::new(),
        },
        Kind::Fill => TitleSub {
            title: "Füllprogramm".into(),
            sub: title_of(plan, entry).title,
        },
        _ => title_of(plan, entry),
    };
    let mut b = badges(entry);
    if let Some(theme) = entry.theme.as_deref().filter(|s| !s.is_empty()) {
        b.push(format!("Thementag: {theme}"));
    }
    if kind == Kind::Fill {
        b.push(
            entry
                .label
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Füllprogramm".into()),
        );
    }
    // Felder aus overlay-util.describe (Sendeschluss: leer)
    let (year, genres, run_min, series_id) = if entry.kind == EntryType::Closedown {
        (None, Vec::new(), None, entry.series_id.clone())
    } else if let Some(it) = &it {
        let run_min = if it.dur_ms != 0 {
            Some(round_js(it.dur_ms as f64 / 60000.0) as i64)
        } else {
            None
        };
        let sid = if it.kind == ItemType::Episode {
            entry
                .series_id
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| it.series_id.clone())
        } else {
            entry.series_id.clone()
        };
        (
            it.year.filter(|y| *y != 0),
            genre_labels(plan, &it.genres),
            run_min,
            sid,
        )
    } else {
        (None, Vec::new(), None, entry.series_id.clone())
    };
    let series_id = series_id.filter(|s| !s.is_empty());
    let has_item = it.is_some() && entry.item_id.is_some();
    let mut overview = String::new();
    if let Some(o) = it.as_ref().and_then(|i| i.overview.as_ref()) {
        overview.clone_from(o);
    } else if let (Some(map), Some(id)) = (overviews, entry.item_id.as_deref()) {
        overview = map.get(id).cloned().unwrap_or_default();
    }
    let image_item_id = if has_item {
        match &it {
            Some(i) if i.kind == ItemType::Episode && series_id.is_some() => series_id.clone(),
            _ => entry.item_id.clone(),
        }
    } else {
        None
    };
    Description {
        title: tt.title,
        sub: tt.sub,
        time: time_range(entry),
        progress: progress_of(t, entry),
        badges: b,
        kind,
        item_id: entry.item_id.clone(),
        series_id,
        image_item_id,
        overview,
        year,
        genres,
        run_min,
    }
}

/// Ein Sendetag der Tagesleiste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayRef {
    pub d: i64,
    pub label: String,
}

/// Sieben Sendetage ab dem Sendetag von `t`: `Heute`, `Morgen`, dann z. B. `Fr 14.11.`.
pub fn days(t: i64) -> Vec<DayRef> {
    let d = broadcast_day_of(t);
    (0..7)
        .map(|i| {
            let f = format_day(d + i);
            let label = match i {
                0 => "Heute".to_string(),
                1 => "Morgen".to_string(),
                _ => format!("{} {}", f.weekday, f.date),
            };
            DayRef { d: d + i, label }
        })
        .collect()
}

/// Sendername für die Anzeige: ohne das `LIVE`-Präfix des Planers (`LIVE Kino` zu `Kino`,
/// `LIVE 7` zu `Sender 7`). Das Präfix bleibt Planer-Detail.
pub fn channel_name(channel: &Channel) -> String {
    let raw = channel
        .genre_label
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&channel.name);
    let trimmed = js_trim(strip_live_prefix(raw));
    if trimmed.is_empty() || trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return if channel.no != 0 {
            format!("Sender {}", channel.no)
        } else {
            String::new()
        };
    }
    trimmed.to_string()
}

/// Entfernt `LIVE` plus Leerraum am Anfang (ohne Beachtung der Groß-/Kleinschreibung).
fn strip_live_prefix(s: &str) -> &str {
    if s.len() >= 4 && s.is_char_boundary(4) && s[..4].eq_ignore_ascii_case("LIVE") {
        let rest = &s[4..];
        let trimmed = rest.trim_start_matches(is_js_ws);
        if trimmed.len() < rest.len() {
            return trimmed;
        }
    }
    s
}
