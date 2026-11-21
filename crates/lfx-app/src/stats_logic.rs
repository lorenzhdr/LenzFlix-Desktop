//! W5-stats: reine Logik der Statistik-Seite (Formate, Zeitreihe füllen, Heatmap, Anteile) ohne Oberfläche und Netz.

use std::collections::HashMap;

use lfx_api::charts::{MeStats, StatsMethod, StatsRange};

const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
];
/// Index 0 = Sonntag (wie `dow` des Dienstes).
const WEEKDAYS: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];

/// Zeiträume in Reihenfolge der Reiter.
pub const RANGES: [StatsRange; 4] = [
    StatsRange::Days7,
    StatsRange::Days30,
    StatsRange::Year,
    StatsRange::All,
];

/// Überschrift des Zeitraums.
pub fn range_title(range: StatsRange) -> &'static str {
    match range {
        StatsRange::Days7 => "Letzte 7 Tage",
        StatsRange::Days30 => "Letzte 30 Tage",
        StatsRange::Year => "Dieses Jahr",
        StatsRange::All => "Gesamtzeit",
    }
}

/// Ganze Zahl mit Tausenderpunkt.
pub fn fmt_int(n: i64) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Dauer in Sekunden als „2 T 3 Std.“, „1 Std. 30 Min.“, „5 Min.“.
pub fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("{d} T {h} Std.")
    } else if h > 0 {
        format!("{h} Std. {m} Min.")
    } else if m > 0 {
        format!("{m} Min.")
    } else if secs > 0 {
        format!("{secs} Sek.")
    } else {
        "0 Min.".to_owned()
    }
}

// ---------------------------------------------------------------- Kalender (ohne Uhr, rein arithmetisch)

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-MM-DD` (weitere Zeichen werden ignoriert) → (Jahr, Monat, Tag).
fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let head = s.get(..10)?;
    let mut it = head.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// „2. Januar 2026“; unlesbare Eingabe kommt unverändert zurück.
pub fn fmt_date(s: &str) -> String {
    match parse_ymd(s) {
        Some((y, m, d)) => format!("{d}. {} {y}", MONTHS[(m - 1) as usize]),
        None => s.to_owned(),
    }
}

/// „2. Januar 2026, 18:00“ aus `YYYY-MM-DD[T ]HH:MM…`.
pub fn fmt_datetime(s: &str) -> String {
    let time = s.get(11..16).filter(|t| t.as_bytes().get(2) == Some(&b':'));
    match time {
        Some(t) => format!("{}, {t}", fmt_date(s)),
        None => fmt_date(s),
    }
}

fn weekday_of(days: i64) -> usize {
    (days + 4).rem_euclid(7) as usize // 1970-01-01 war ein Donnerstag; 0 = Sonntag
}

// ---------------------------------------------------------------- Ergebnisobjekte

/// Ein Balken des Verlaufs.
#[derive(Clone, Debug, PartialEq)]
pub struct BarRow {
    pub frac: f32,
    pub label: String,
    pub tip: String,
    pub hot: bool,
}

/// Eine Zeile mit Anteilsbalken.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareRow {
    pub name: String,
    pub meta: String,
    pub frac: f32,
}

/// Anzeigewerte der fertigen Seite.
#[derive(Clone, Debug, Default)]
pub struct StatsView {
    pub hero_value: String,
    pub hero_sub: String,
    pub kpis: Vec<(String, String)>,
    pub notice: String,
    pub top_series: Vec<ShareRow>,
    pub top_movies: Vec<ShareRow>,
    pub bars: Vec<BarRow>,
    pub line: String,
    pub range_title: String,
    pub heat: Vec<f32>,
    pub heat_best: String,
    pub binge: Option<(String, String, String)>,
    pub devices: Vec<ShareRow>,
    pub methods: Vec<ShareRow>,
    /// Ids der Titel, die für die Namen nachgeladen werden müssen.
    pub title_ids: Vec<String>,
}

/// Id wie im Dienst: 32 Hex ohne Bindestriche, klein.
pub fn norm_id(id: &str) -> String {
    id.replace('-', "").to_lowercase()
}

/// Ids, deren Namen die Seite braucht (Top-Listen, Marathon-Serie).
pub fn title_ids(s: &MeStats) -> Vec<String> {
    let mut ids: Vec<String> = s
        .top_series
        .iter()
        .map(|t| t.id.as_str())
        .chain(s.top_movies.iter().map(|t| t.id.as_str()))
        .chain(s.binge.series_day.iter().map(|d| d.series_id.as_str()))
        .filter(|i| !i.is_empty())
        .map(norm_id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

// ---------------------------------------------------------------- Verlauf

/// Zeitreihe mit gefüllten Lücken: (Schlüssel, Sekunden, Beschriftung, Detailtext).
fn timeline_rows(s: &MeStats) -> Vec<(String, i64, String, String)> {
    let unit = s.timeline.unit.as_str();
    let mut map: HashMap<&str, i64> = HashMap::new();
    for b in &s.timeline.buckets {
        *map.entry(b.key.as_str()).or_default() += b.seconds;
    }
    let mut keys: Vec<&str> = map.keys().copied().collect();
    keys.sort_unstable();
    let (Some(first), Some(last)) = (keys.first().copied(), keys.last().copied()) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    match unit {
        "day" => {
            let end = parse_ymd(&s.computed_at)
                .or_else(|| parse_ymd(last))
                .map(|(y, m, d)| days_from_civil(y, m, d));
            let n = match s.range.as_str() {
                "7d" => 7,
                "30d" => 30,
                _ => 0,
            };
            let (Some(end), Some(f)) = (end, parse_ymd(first)) else {
                return Vec::new();
            };
            let start = if n > 0 {
                end - (n - 1)
            } else {
                days_from_civil(f.0, f.1, f.2)
            };
            let end = end.max(
                parse_ymd(last)
                    .map(|(y, m, d)| days_from_civil(y, m, d))
                    .unwrap_or(end),
            );
            for (i, day) in (start..=end).take(400).enumerate() {
                let (y, m, d) = civil_from_days(day);
                let key = format!("{y:04}-{m:02}-{d:02}");
                let secs = map.get(key.as_str()).copied().unwrap_or(0);
                let label = if n == 7 || i % 5 == 0 {
                    format!("{d}.{m}.")
                } else {
                    String::new()
                };
                let date = format!("{}, {}", WEEKDAYS[weekday_of(day)], fmt_date(&key));
                rows.push((key, secs, label, date));
            }
        }
        "week" => {
            let parse = |k: &str| -> Option<(i64, i64)> {
                let (y, w) = k.split_once('-')?;
                Some((y.parse().ok()?, w.parse().ok()?))
            };
            let (Some(mut cur), Some(end)) = (parse(first), parse(last)) else {
                return Vec::new();
            };
            for i in 0..120 {
                let key = format!("{:04}-{:02}", cur.0, cur.1);
                let secs = map.get(key.as_str()).copied().unwrap_or(0);
                let label = if i % 4 == 0 {
                    format!("KW {}", cur.1)
                } else {
                    String::new()
                };
                rows.push((
                    key,
                    secs,
                    label,
                    format!("Woche {} im Jahr {}", cur.1, cur.0),
                ));
                if cur >= end {
                    break;
                }
                cur = if cur.1 >= 53 {
                    (cur.0 + 1, 0)
                } else {
                    (cur.0, cur.1 + 1)
                };
            }
        }
        _ => {
            let parse = |k: &str| -> Option<(i64, i64)> {
                let (y, m) = k.split_once('-')?;
                Some((y.parse().ok()?, m.parse().ok()?))
            };
            let (Some(mut cur), Some(end)) = (parse(first), parse(last)) else {
                return Vec::new();
            };
            for i in 0..240 {
                if !(1..=12).contains(&cur.1) {
                    break;
                }
                let key = format!("{:04}-{:02}", cur.0, cur.1);
                let secs = map.get(key.as_str()).copied().unwrap_or(0);
                let name = MONTHS_SHORT[(cur.1 - 1) as usize];
                let label = if cur.1 == 1 || i == 0 {
                    format!("{name} {:02}", cur.0 % 100)
                } else if i % 3 == 0 {
                    name.to_owned()
                } else {
                    String::new()
                };
                rows.push((
                    key,
                    secs,
                    label,
                    format!("{} {}", MONTHS[(cur.1 - 1) as usize], cur.0),
                ));
                if cur >= end {
                    break;
                }
                cur = if cur.1 >= 12 {
                    (cur.0 + 1, 1)
                } else {
                    (cur.0, cur.1 + 1)
                };
            }
        }
    }
    rows
}

/// Balken und Linie (gleitender Durchschnitt als Path-Befehle in einer 1000x100-Viewbox).
pub fn build_bars(s: &MeStats) -> (Vec<BarRow>, String) {
    let rows = timeline_rows(s);
    let max = rows.iter().map(|r| r.1).max().unwrap_or(0);
    if max <= 0 {
        return (Vec::new(), String::new());
    }
    let hot = rows.iter().position(|r| r.1 == max);
    let bars: Vec<BarRow> = rows
        .iter()
        .enumerate()
        .map(|(i, (_, secs, label, date))| BarRow {
            frac: *secs as f32 / max as f32,
            label: label.clone(),
            tip: if *secs > 0 {
                format!("{date}: {}", fmt_duration(*secs))
            } else {
                format!("{date}: keine Wiedergabe")
            },
            hot: Some(i) == hot,
        })
        .collect();
    let n = rows.len();
    let mut line = String::new();
    if n >= 4 {
        let w = if s.timeline.unit == "day" { 7 } else { 3 };
        for i in 0..n {
            let from = (i + 1).saturating_sub(w);
            let slice = &rows[from..=i];
            let avg = slice.iter().map(|r| r.1).sum::<i64>() as f32 / slice.len() as f32;
            let x = (i as f32 + 0.5) / n as f32 * 1000.0;
            let y = 100.0 - (avg / max as f32 * 100.0).min(100.0);
            line.push_str(&format!(
                "{}{x:.1} {y:.1} ",
                if i == 0 { "M " } else { "L " }
            ));
        }
    }
    (bars, line.trim_end().to_owned())
}

// ---------------------------------------------------------------- Heatmap

/// 7 Zeilen (Mo..So) x 24 Spalten, normiert auf 0..1; dazu der Satz zur stärksten Zelle.
pub fn build_heat(s: &MeStats) -> (Vec<f32>, String) {
    let mut m = [[0i64; 24]; 7];
    for c in &s.heatmap {
        if c.dow < 7 && c.hour < 24 {
            m[c.dow as usize][c.hour as usize] += c.seconds;
        }
    }
    let max = m.iter().flatten().copied().max().unwrap_or(0);
    if max <= 0 {
        return (vec![0.0; 168], String::new());
    }
    let mut out = Vec::with_capacity(168);
    let mut best = (0usize, 0usize);
    for r in 0..7 {
        let dow = (r + 1) % 7;
        for h in 0..24 {
            out.push(m[dow][h] as f32 / max as f32);
            if m[dow][h] == max && m[best.0][best.1] != max {
                best = (dow, h);
            }
        }
    }
    let text = format!(
        "Am meisten {} um {} Uhr, {}",
        WEEKDAYS[best.0],
        best.1,
        fmt_duration(max)
    );
    (out, text)
}

// ---------------------------------------------------------------- Anteile und Ranglisten

fn percent(frac: f32) -> String {
    let p = (frac * 100.0).round() as i64;
    if p == 0 && frac > 0.0 {
        "< 1 %".to_owned()
    } else {
        format!("{p} %")
    }
}

/// Wiedergabearten (nach Sekunden, sonst nach Wiedergaben).
pub fn method_shares(s: &MeStats) -> Vec<ShareRow> {
    let m = &s.methods;
    let list: [(&str, &StatsMethod); 4] = [
        ("Direkt abspielen", &m.direct_play),
        ("Direkt streamen", &m.direct_stream),
        ("Transkodiert", &m.transcode),
        ("Sonstige", &m.other),
    ];
    let by_seconds = list.iter().any(|(_, v)| v.seconds > 0);
    let value = |v: &StatsMethod| if by_seconds { v.seconds } else { v.plays };
    let total: i64 = list.iter().map(|(_, v)| value(v)).sum();
    list.iter()
        .filter(|(_, v)| value(v) > 0 && total > 0)
        .map(|(name, v)| {
            let frac = value(v) as f32 / total as f32;
            ShareRow {
                name: (*name).to_owned(),
                meta: format!("{} · {}", percent(frac), fmt_duration(v.seconds)),
                frac,
            }
        })
        .collect()
}

/// Die fünf Geräte mit der meisten Watchtime.
pub fn device_shares(s: &MeStats) -> Vec<ShareRow> {
    let mut list: Vec<_> = s.devices.iter().collect();
    list.sort_by_key(|d| std::cmp::Reverse(d.seconds));
    let total: i64 = list.iter().map(|d| d.seconds.max(0)).sum();
    list.iter()
        .take(5)
        .map(|d| {
            let name = if d.device.is_empty() {
                if d.client.is_empty() {
                    "Unbekannt"
                } else {
                    &d.client
                }
            } else {
                &d.device
            };
            let label = if !d.client.is_empty() && !d.device.is_empty() && d.client != d.device {
                format!("{name} · {}", d.client)
            } else {
                name.to_owned()
            };
            let frac = if total > 0 {
                d.seconds.max(0) as f32 / total as f32
            } else {
                0.0
            };
            ShareRow {
                name: label,
                meta: format!("{} · {}", percent(frac), fmt_duration(d.seconds)),
                frac,
            }
        })
        .collect()
}

fn top_rows(items: &[(String, i64, String)], titles: &HashMap<String, String>) -> Vec<ShareRow> {
    let max = items.iter().map(|i| i.1).max().unwrap_or(0).max(1);
    items
        .iter()
        .map(|(id, secs, meta)| ShareRow {
            name: titles
                .get(&norm_id(id))
                .cloned()
                .unwrap_or_else(|| "Unbekannter Titel".to_owned()),
            meta: meta.clone(),
            frac: *secs as f32 / max as f32,
        })
        .collect()
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{} {}", fmt_int(n), if n == 1 { one } else { many })
}

/// Baut alle Anzeigewerte; `titles` bildet normalisierte Ids auf Namen ab.
pub fn build_view(s: &MeStats, titles: &HashMap<String, String>) -> StatsView {
    let t = &s.totals;
    let range = RANGES
        .iter()
        .copied()
        .find(|r| r.as_str() == s.range)
        .unwrap_or_default();
    let (bars, line) = build_bars(s);
    let (heat, heat_best) = build_heat(s);
    let mut notices = Vec::new();
    if s.ignored {
        notices.push("Für dein Konto werden keine Statistiken erfasst.".to_owned());
    }
    if s.stale {
        notices.push(format!(
            "Stand: {} – Server gerade nicht erreichbar.",
            fmt_datetime(&s.computed_at)
        ));
    }
    if s.approx {
        notices
            .push("Die Serien-Zuordnung ist bei sehr vielen Folgen nur näherungsweise.".to_owned());
    }
    let series: Vec<_> = s
        .top_series
        .iter()
        .take(10)
        .map(|x| {
            let eps = x.episodes.unwrap_or(0);
            (
                x.id.clone(),
                x.seconds,
                format!(
                    "{} · {}",
                    plural(eps, "Folge", "Folgen"),
                    fmt_duration(x.seconds)
                ),
            )
        })
        .collect();
    let movies: Vec<_> = s
        .top_movies
        .iter()
        .take(10)
        .map(|x| {
            let plays = x.plays.unwrap_or(1);
            let meta = if plays > 1 {
                format!("{plays}× · {}", fmt_duration(x.seconds))
            } else {
                fmt_duration(x.seconds)
            };
            (x.id.clone(), x.seconds, meta)
        })
        .collect();
    let binge = match (&s.binge.top_day, &s.binge.series_day) {
        (None, None) => None,
        (day, sd) => {
            let big = day.as_ref().map_or(String::new(), |d| {
                format!("{} · {}", fmt_date(&d.date), fmt_duration(d.seconds))
            });
            let sub = if day.is_some() {
                "Dein Tag mit der meisten Watchtime.".to_owned()
            } else {
                String::new()
            };
            let line = sd.as_ref().map_or(String::new(), |d| {
                let name = titles
                    .get(&norm_id(&d.series_id))
                    .cloned()
                    .unwrap_or_else(|| "einer Serie".to_owned());
                format!(
                    "{} von {name} am {}",
                    plural(d.episodes, "Folge", "Folgen"),
                    fmt_date(&d.date)
                )
            });
            Some((big, sub, line))
        }
    };
    StatsView {
        hero_value: fmt_duration(t.seconds),
        hero_sub: s.compare.as_ref().map_or(String::new(), |c| {
            format!(
                "Platz {} von {} · Ø {} (Dein Platz nach Watchtime unter allen aktiven Nutzern)",
                c.rank,
                c.of,
                fmt_duration(c.avg_seconds)
            )
        }),
        kpis: vec![
            ("Folgen".into(), fmt_int(t.episodes)),
            ("Filme".into(), fmt_int(t.movies)),
            ("Serien".into(), fmt_int(t.series)),
            ("Aktive Tage".into(), fmt_int(t.active_days)),
        ],
        notice: notices.join(" "),
        top_series: top_rows(&series, titles),
        top_movies: top_rows(&movies, titles),
        bars,
        line,
        range_title: range_title(range).to_owned(),
        heat,
        heat_best,
        binge,
        devices: device_shares(s),
        methods: method_shares(s),
        title_ids: title_ids(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MeStats {
        serde_json::from_str(
            r#"{"version":1,"range":"7d","computedAt":"2026-03-10T18:00:00Z","stale":false,"approx":false,"ignored":false,
              "totals":{"seconds":9000,"plays":4,"episodes":3,"movies":1,"series":1,"activeDays":2},
              "topSeries":[{"id":"AA-bb","seconds":6000,"episodes":3}],"topMovies":[{"id":"cc","seconds":3000,"plays":2}],
              "timeline":{"unit":"day","buckets":[{"key":"2026-03-08","seconds":3000},{"key":"2026-03-10","seconds":6000}]},
              "heatmap":[{"dow":1,"hour":20,"seconds":6000},{"dow":0,"hour":9,"seconds":3000}],
              "binge":{"topDay":{"date":"2026-03-10","seconds":6000},"seriesDay":{"date":"2026-03-10","seriesId":"aabb","episodes":3}},
              "devices":[{"client":"A","device":"PC","plays":4,"seconds":9000}],
              "methods":{"directPlay":{"plays":3,"seconds":6000},"directStream":{"plays":0,"seconds":0},"transcode":{"plays":1,"seconds":3000},"other":{"plays":0,"seconds":0}},
              "compare":{"rank":2,"of":4,"avgSeconds":5400}}"#,
        )
        .unwrap()
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_int(1234567), "1.234.567");
        assert_eq!(fmt_int(12), "12");
        assert_eq!(fmt_duration(0), "0 Min.");
        assert_eq!(fmt_duration(45), "45 Sek.");
        assert_eq!(fmt_duration(5400), "1 Std. 30 Min.");
        assert_eq!(fmt_duration(90_000), "1 T 1 Std.");
        assert_eq!(fmt_date("2026-03-10"), "10. März 2026");
        assert_eq!(fmt_datetime("2026-03-10T18:00:00Z"), "10. März 2026, 18:00");
        assert_eq!(fmt_date("kaputt"), "kaputt");
    }

    #[test]
    fn calendar_round_trip_and_weekday() {
        for d in [-1000, 0, 19_000, 20_513] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        // 10. März 2026 ist ein Dienstag
        let d = days_from_civil(2026, 3, 10);
        assert_eq!(WEEKDAYS[weekday_of(d)], "Dienstag");
    }

    #[test]
    fn day_timeline_fills_gaps_and_ends_at_compute_day() {
        let (bars, line) = build_bars(&sample());
        assert_eq!(bars.len(), 7);
        assert!((bars[6].frac - 1.0).abs() < 1e-6 && bars[6].hot);
        assert!((bars[4].frac - 0.5).abs() < 1e-6);
        assert!(bars[0].frac == 0.0 && bars[0].tip.contains("keine Wiedergabe"));
        assert!(bars[6].tip.contains("Dienstag") && bars[6].tip.contains("1 Std. 40 Min."));
        assert!(line.starts_with("M ") && line.contains(" L "));
    }

    #[test]
    fn empty_timeline_gives_no_bars() {
        let s: MeStats = serde_json::from_str(r#"{"range":"30d"}"#).unwrap();
        assert!(build_bars(&s).0.is_empty());
        let (heat, best) = build_heat(&s);
        assert_eq!(heat.len(), 168);
        assert!(best.is_empty());
    }

    #[test]
    fn month_and_week_units() {
        let mut s = sample();
        s.timeline.unit = "month".into();
        s.timeline.buckets = vec![
            lfx_api::charts::StatsBucket {
                key: "2025-11".into(),
                seconds: 100,
            },
            lfx_api::charts::StatsBucket {
                key: "2026-02".into(),
                seconds: 300,
            },
        ];
        let (bars, _) = build_bars(&s);
        assert_eq!(bars.len(), 4);
        s.timeline.unit = "week".into();
        s.timeline.buckets = vec![
            lfx_api::charts::StatsBucket {
                key: "2026-05".into(),
                seconds: 100,
            },
            lfx_api::charts::StatsBucket {
                key: "2026-08".into(),
                seconds: 100,
            },
        ];
        assert_eq!(build_bars(&s).0.len(), 4);
    }

    #[test]
    fn heatmap_rows_start_on_monday() {
        let (heat, best) = build_heat(&sample());
        assert_eq!(heat.len(), 168);
        assert!((heat[20] - 1.0).abs() < 1e-6, "Montag 20 Uhr");
        assert!((heat[6 * 24 + 9] - 0.5).abs() < 1e-6, "Sonntag 9 Uhr");
        assert_eq!(best, "Am meisten Montag um 20 Uhr, 1 Std. 40 Min.");
    }

    #[test]
    fn shares_and_view() {
        let s = sample();
        let m = method_shares(&s);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].meta, "67 % · 1 Std. 40 Min.");
        let titles: HashMap<String, String> =
            [("aabb".to_owned(), "Erfundene Serie".to_owned())].into();
        let v = build_view(&s, &titles);
        assert_eq!(v.title_ids, vec!["aabb", "cc"]);
        assert_eq!(v.top_series[0].name, "Erfundene Serie");
        assert_eq!(v.top_movies[0].name, "Unbekannter Titel");
        assert_eq!(v.top_movies[0].meta, "2× · 50 Min.");
        assert_eq!(v.kpis[0], ("Folgen".to_owned(), "3".to_owned()));
        assert!(v.hero_sub.starts_with("Platz 2 von 4"));
        let b = v.binge.unwrap();
        assert!(b.2.contains("3 Folgen von Erfundene Serie"));
        assert_eq!(v.range_title, "Letzte 7 Tage");
    }

    #[test]
    fn notices() {
        let mut s = sample();
        s.stale = true;
        s.approx = true;
        let v = build_view(&s, &HashMap::new());
        assert!(
            v.notice.contains("Stand: 10. März 2026, 18:00") && v.notice.contains("näherungsweise")
        );
        s.ignored = true;
        assert!(
            build_view(&s, &HashMap::new())
                .notice
                .contains("keine Statistiken")
        );
    }
}
