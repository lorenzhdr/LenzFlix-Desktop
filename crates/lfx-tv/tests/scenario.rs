//! Szenario-Tests mit dem echten Planer und einem erfundenen Katalog (nur erfundene Titel).

use std::rc::Rc;
use std::sync::Arc;

use lfx_tv::catalog::RawCatalog;
use lfx_tv::channel_list::ChannelList;
use lfx_tv::controller::{Phase, TvController, TvOptions, TvOutput};
use lfx_tv::epg::{EpgGrid, reveal_left};
use lfx_tv::model::{channel_name, days, describe, grid_rows, now_all};
use lfx_tv::request::PlayResult;
use lfx_tv::{ManualClock, PlanSource};
use lfx_tv_schedule::time::{broadcast_day_of, day_bounds, days_from_civil, wall_to_utc};
use lfx_tv_schedule::{Scheduler, normalize_catalog};
use serde_json::{Value, json};

const MIN: i64 = 60_000;
const TICKS_PER_MIN: i64 = 600_000_000;

/// Erfundener Katalog: 80 Filme, 12 Serien mit je 16 Folgen.
fn raw_catalog(loaded_at: i64) -> RawCatalog {
    let genres = [
        "Komödie",
        "Drama",
        "Action",
        "Krimi",
        "Thriller",
        "Dokumentation",
    ];
    let movies: Vec<Value> = (0..80)
        .map(|i| {
            json!({
                "Id": format!("mv{i:03}"),
                "Name": format!("Erfundener Film {i}"),
                "RunTimeTicks": (80 + (i % 50)) * TICKS_PER_MIN,
                "Genres": [genres[(i % 6) as usize], genres[((i + 2) % 6) as usize]],
                "OfficialRating": if i % 7 == 0 { "FSK 16" } else { "FSK 12" },
                "ProductionYear": 1980 + (i % 40),
                "PremiereDate": format!("{}-03-1{}T00:00:00.0000000Z", 1980 + (i % 40), i % 9),
                "DateCreated": format!("2020-0{}-1{}T10:00:00.0000000Z", 1 + (i % 9), i % 9),
                "LocalTrailerCount": i % 2,
            })
        })
        .collect();
    let series: Vec<Value> = (0..12)
        .map(|i| {
            json!({
                "Id": format!("sr{i:02}"),
                "Name": format!("Erfundene Serie {i}"),
                "Genres": [genres[(i % 6) as usize]],
                "OfficialRating": "FSK 12",
                "ProductionYear": 1995 + i,
                "DateCreated": "2020-02-01T10:00:00.0000000Z",
            })
        })
        .collect();
    let mut episodes = Vec::new();
    for s in 0..12 {
        for e in 0..16 {
            episodes.push(json!({
                "Id": format!("ep{s:02}{e:02}"),
                "Name": format!("Folge {e}"),
                "SeriesId": format!("sr{s:02}"),
                "SeriesName": format!("Erfundene Serie {s}"),
                "ParentIndexNumber": 1 + e / 8,
                "IndexNumber": 1 + e % 8,
                "RunTimeTicks": 45 * TICKS_PER_MIN,
                "PremiereDate": format!("2001-0{}-0{}T00:00:00.0000000Z", 1 + e % 9, 1 + e % 8),
                "DateCreated": "2020-02-02T10:00:00.0000000Z",
                "OfficialRating": "FSK 12",
            }));
        }
    }
    RawCatalog {
        movies,
        series,
        episodes,
        boxsets: Vec::new(),
        loaded_at,
    }
}

fn t0() -> i64 {
    wall_to_utc(days_from_civil(2026, 11, 20), 20 * 60 + 15)
}

fn scheduler() -> Scheduler {
    let raw = raw_catalog(t0());
    let cat = Arc::new(normalize_catalog(&raw.to_jsval()));
    Scheduler::new(cat, &lfx_tv_schedule::JsVal::Undefined)
}

/// Läuft `steps` mal `step_ms` weiter, antwortet jedem Play mit Erfolg; liefert die Ausgaben.
fn run(channel: i64, steps: usize, step_ms: i64) -> (Vec<TvOutput>, Vec<(String, i64)>) {
    let clock = ManualClock::new(t0());
    let mut ctl = TvController::new(
        scheduler(),
        Box::new(clock.clone()),
        TvOptions {
            last_channel: channel,
            ..TvOptions::default()
        },
    );
    let mut all = Vec::new();
    let mut plays = Vec::new();
    ctl.start();
    for step in 0..=steps {
        for o in ctl.take_output() {
            if let TvOutput::Play(req) = &o {
                plays.push((req.item_id.clone(), req.offset_ms));
                ctl.on_play_result(req.token, &PlayResult::ok());
            }
            all.push(o);
        }
        // Antworten erzeugen Ausgaben (Nachbarn-Vorwärmen): mitnehmen
        all.extend(ctl.take_output());
        if step < steps {
            clock.advance(step_ms);
            ctl.poll();
        }
    }
    (all, plays)
}

#[test]
fn drei_stunden_laufen_luckenlos_ohne_fehlerkarte() {
    let (outs, plays) = run(3, 360, 30_000);
    assert!(!plays.is_empty());
    // Folgestart nach jedem Sendungsende mit kleinem Offset (30 ms Zuschlag)
    for (_, off) in plays.iter().skip(1) {
        assert_eq!(*off, 30, "Folgesendung startet 30 ms nach Beginn");
    }
    assert!(
        plays.len() >= 2,
        "in 3 h mindestens ein Sendungswechsel: {plays:?}"
    );
    for o in &outs {
        if let TvOutput::Change(s) = o {
            assert_ne!(s.phase, Phase::Error);
        }
    }
}

#[test]
fn echter_planer_ist_deterministisch_im_controller() {
    let a = run(7, 120, 45_000);
    let b = run(7, 120, 45_000);
    assert_eq!(a.0, b.0);
    assert_eq!(a.1, b.1);
}

#[test]
fn alle_18_sender_liefern_ein_programm_jetzt() {
    let mut s = scheduler();
    let t = t0();
    let all = now_all(&mut s, t);
    assert_eq!(all.len(), 18);
    for (i, n) in all.iter().enumerate() {
        assert_eq!(n.channel.no, i as i64 + 1);
        let e = n.entry.as_ref().expect("Eintrag");
        assert!(e.start <= t && t < e.end);
        assert!((0.0..=1.0).contains(&n.progress));
        if let Some(nx) = &n.next {
            assert!(nx.start >= e.end);
        }
    }
}

#[test]
fn raster_deckt_den_ganzen_sendetag_luckenlos() {
    let mut s = scheduler();
    let d = broadcast_day_of(t0());
    let b = day_bounds(d);
    let rows = grid_rows(&mut s, b.start, b.end, None, Some(t0()));
    assert_eq!(rows.len(), 18);
    for r in &rows {
        assert_eq!(r.cells.first().map(|c| c.start), Some(b.start));
        assert_eq!(r.cells.last().map(|c| c.end), Some(b.end));
        for w in r.cells.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        assert_eq!(r.cells.iter().filter(|c| c.now).count(), 1);
        assert!(r.cells.iter().all(|c| !c.title.is_empty()));
    }
}

#[test]
fn raster_fenster_ueber_05_uhr_nutzt_beide_tage_und_schneidet_zu() {
    let mut s = scheduler();
    let d = broadcast_day_of(t0());
    let b = day_bounds(d);
    let from = b.end - 60 * MIN;
    let to = b.end + 60 * MIN;
    let rows = grid_rows(&mut s, from, to, Some(d), None);
    let r = &rows[3];
    assert_eq!(r.cells.first().map(|c| c.start), Some(from));
    assert_eq!(r.cells.last().map(|c| c.end), Some(to));
    for w in r.cells.windows(2) {
        assert_eq!(w[0].end, w[1].start);
    }
}

#[test]
fn senderkoepfe_ohne_live_praefix_und_kurznamen_hoechstens_9_zeichen() {
    let mut s = scheduler();
    let d = broadcast_day_of(t0());
    let ch = s.channels(d);
    assert_eq!(ch.len(), 18);
    for c in ch.iter() {
        let n = channel_name(c);
        assert!(!n.to_uppercase().starts_with("LIVE"), "{n}");
        assert!(!n.is_empty());
        assert!(lfx_tv::logos::short_name(c).chars().count() <= 9);
    }
}

#[test]
fn beschreibung_von_film_und_folge_aus_dem_katalog() {
    let mut s = scheduler();
    let t = t0();
    let mut saw_movie = false;
    let mut saw_episode = false;
    for n in now_all(&mut s, t) {
        let e = n.entry.unwrap();
        let d = describe(&mut s, n.channel.no, &e, t, None);
        assert!(!d.title.is_empty());
        if lfx_tv::model::kind_of(&e) != lfx_tv::model::Kind::Program {
            continue; // Füllprogramm: Titel "Füllprogramm", Sendung im Untertitel
        }
        assert!(d.time.contains('–'));
        if let Some(id) = &e.item_id {
            let it = PlanSource::item(&s, id).unwrap();
            if let Some(series_title) = it.series_title.clone() {
                saw_episode = true;
                assert_eq!(d.title, series_title);
                assert!(d.sub.starts_with('S'), "{}", d.sub);
                assert!(
                    d.image_item_id
                        .as_deref()
                        .is_some_and(|i| i.starts_with("sr")),
                    "Bild der Serie"
                );
            } else {
                saw_movie = true;
                assert_eq!(d.image_item_id.as_deref(), Some(id.as_str()));
                assert!(d.year.is_some() && d.run_min.is_some());
                assert!(!d.genres.is_empty());
                assert!(
                    d.genres
                        .iter()
                        .all(|g| g.chars().next().is_some_and(char::is_uppercase)),
                    "Anzeigelabels: {:?}",
                    d.genres
                );
            }
        }
    }
    assert!(saw_movie || saw_episode);
}

#[test]
fn epg_raster_skala_jetzt_linie_und_tageswechsel() {
    let mut s = scheduler();
    let t = t0();
    let mut g = EpgGrid::build(&mut s, t, None);
    assert_eq!(g.scale().len(), (g.total_min() / 30.0) as usize);
    assert!(
        g.scale().len() >= 46 && g.scale().len() <= 50,
        "MEZ-Tag 48, Umstellungstage 46/50"
    );
    assert_eq!(g.scale()[0].label, "05:00");
    assert!(g.scale().iter().any(|m| m.half));
    assert_eq!(g.days().len(), 7);
    assert_eq!(g.days()[0].label, "Heute");
    assert_eq!(g.days()[1].label, "Morgen");
    let line = g.now_line(t).unwrap();
    assert!((line - (t - g.bounds().0) as f64 / MIN as f64).abs() < 0.01);
    assert_eq!(g.rows().len(), 18);
    for r in g.rows() {
        assert_eq!(r.cells.iter().filter(|c| c.current).count(), 1);
        assert!(r.cells.iter().filter(|c| c.past).all(|c| c.entry.end <= t));
        let cur = r.cells.iter().find(|c| c.current).unwrap();
        assert!(cur.progress_pct.is_some());
    }
    assert!(reveal_left(&g, t, 5.0).unwrap() > 0.0);
    // anderer Tag: keine Linie
    let d1 = g.today() + 1;
    assert!(g.select_day(&mut s, d1, t));
    assert_eq!(g.now_line(t), None);
    assert!(reveal_left(&g, t, 5.0).is_none());
    assert!(!g.select_day(&mut s, d1, t), "gleicher Tag: nichts neu");
    assert!(g.select_today(&mut s, t));
    // Tageswechsel folgt heute
    let next_day_t = g.bounds().1 + 5 * MIN;
    assert!(g.tick(&mut s, next_day_t));
    assert_eq!(g.today(), broadcast_day_of(next_day_t));
    assert_eq!(g.selected(), g.today());
    assert!(!g.tick(&mut s, next_day_t + 1000));
}

#[test]
fn sieben_tage_leiste_beginnt_am_sendetag() {
    let d = days(t0());
    assert_eq!(d.len(), 7);
    assert_eq!(d[0].d, broadcast_day_of(t0()));
    // vor 05:00 gehört t zum Vortag
    let early = wall_to_utc(days_from_civil(2026, 11, 21), 3 * 60);
    assert_eq!(days(early)[0].d, broadcast_day_of(t0()));
}

#[test]
fn senderliste_18_zeilen_mit_logos_und_tick() {
    let mut s = scheduler();
    let t = t0();
    let mut l = ChannelList::build(&mut s, t);
    assert_eq!(l.rows().len(), 18);
    let colors: std::collections::HashSet<_> = l.rows().iter().map(|r| r.badge.color).collect();
    assert_eq!(colors.len(), 18);
    for r in l.rows() {
        assert!(r.aria_label.starts_with(&format!("Sender {}", r.no)));
        assert!(r.title != "Kein Programm");
    }
    l.set_current(Some(4));
    assert_eq!(l.current(), 4);
    assert!(
        !l.tick(&mut s, t + MIN),
        "innerhalb der Sendung nur Fortschritt"
    );
    let end = l
        .rows()
        .iter()
        .filter_map(|r| r.entry.as_ref().map(|e| e.end))
        .min()
        .unwrap();
    assert!(l.tick(&mut s, end), "Sendung vorbei: neu gelesen");
    assert_eq!(l.current(), 4, "Markierung bleibt");
}

#[test]
fn senderbelegung_aus_dem_admin_wirkt_im_planer() {
    use lfx_tv::config::{ChannelConfigSources, resolve_channel_config, to_planner_config};
    let raw = raw_catalog(t0());
    let cat = Arc::new(normalize_catalog(&raw.to_jsval()));
    let src = ChannelConfigSources {
        css_tv_channels: Some("\"5=Krimi;6=Action\"".into()),
        ..Default::default()
    };
    let cfg = to_planner_config(&resolve_channel_config(&src));
    let mut s = Scheduler::new(cat, &cfg);
    let ch = s.channels(broadcast_day_of(t0()));
    let c5 = ch.iter().find(|c| c.no == 5).unwrap();
    assert_eq!(channel_name(c5), "Krimi");
    assert_eq!(lfx_tv::logos::short_name(c5), "KRIMI");
    let _ = Rc::strong_count(&ch);
}
