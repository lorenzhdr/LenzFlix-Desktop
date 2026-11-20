//! Modell-Tests (Titel, Raster, Info-Karte, Nachbarn) mit Planer-Attrappe und erfundenen Titeln.
//! Semantik aus `tvmode-model.test.js`, `tvmode-info-card.test.js`, `tv-player-neighbors.test.js`.

use std::collections::HashMap;

use lfx_tv::epg::{CellAction, CellSize, EpgCell, EpgGrid, classify};
use lfx_tv::info_card::{CardAction, Details, Overview, build_info_card, start_date_label};
use lfx_tv::model::{Kind, badges, channel_name, describe, grid_rows, next_program, title_of};
use lfx_tv::neighbors::{
    ERROR_WARM_MS, Mode, NEIGHBOR_MS, NeighborAction, NeighborCtx, Neighbors, WARM_TICK_MAX,
    WARM_TICK_MS, WarmPhase, ZapFlags,
};
use lfx_tv::testing::{FakePlan, closedown, entry, episode, filler, movie, program};
use lfx_tv_schedule::fill::PremiereKind;
use lfx_tv_schedule::time::{days_from_civil, format_clock, wall_to_utc};
use lfx_tv_schedule::{Channel, EntryType};

const MIN: i64 = 60_000;
const T0: i64 = 1_800_000_000_000;

fn plan() -> FakePlan {
    let mut p = FakePlan::new();
    p.item_info(movie("m1", "Erfundener Film Eins", 95 * MIN));
    p.item_info(movie("m2", "Erfundener Film Zwei", 90 * MIN));
    p.item_info(episode(
        "e1",
        "Erfundene Serie",
        "Die Folge",
        1.0,
        3.0,
        "s1",
    ));
    p.genre("Komödie", 10);
    p.all(&[
        program("m1", T0 - 30 * MIN, T0 + 60 * MIN),
        filler(T0 + 60 * MIN, T0 + 65 * MIN),
        program("e1", T0 + 65 * MIN, T0 + 110 * MIN),
        closedown(T0 + 110 * MIN, T0 + 200 * MIN, None),
    ]);
    p
}

fn ch(no: i64, name: &str, genre: Option<&str>) -> Channel {
    Channel {
        no,
        key: format!("k{no}"),
        name: name.into(),
        kind: "genre",
        genre_label: genre.map(str::to_string),
    }
}

#[test]
fn titel_film_folge_und_unbekannt() {
    let p = plan();
    let film = program("m1", 0, 1);
    assert_eq!(title_of(&p, &film).title, "Erfundener Film Eins");
    let ep = program("e1", 0, 1);
    let t = title_of(&p, &ep);
    assert_eq!(t.title, "Erfundene Serie");
    assert_eq!(t.sub, "S1 E3 · Die Folge");
    assert_eq!(title_of(&p, &program("nix", 0, 1)).title, "");
    let mut lab = program("nix", 0, 1);
    lab.label = Some("Nachtprogramm".into());
    assert_eq!(title_of(&p, &lab).title, "Nachtprogramm");
    assert_eq!(title_of(&p, &closedown(0, 1, None)).title, "Sendepause");
}

#[test]
fn folge_ohne_staffel_und_folgennummer_9999_unbekannt() {
    let mut p = FakePlan::new();
    p.item_info(episode("x", "Serie", "Titel", 9999.0, 9999.0, "s"));
    assert_eq!(title_of(&p, &program("x", 0, 1)).sub, "Titel");
    p.item_info(episode("y", "Serie", "Serie", 2.0, 9999.0, "s"));
    assert_eq!(title_of(&p, &program("y", 0, 1)).sub, "S2");
}

#[test]
fn naechste_sendung_ueberspringt_fueller() {
    let mut p = plan();
    let cur = program("m1", T0 - 30 * MIN, T0 + 60 * MIN);
    let nx = next_program(&mut p, 1, &cur).unwrap();
    assert_eq!(nx.item_id.as_deref(), Some("e1"), "Füller übersprungen");
    // am Ende des Plans: kein Fehler, kein Eintrag
    let last = closedown(T0 + 110 * MIN, T0 + 200 * MIN, None);
    assert_eq!(next_program(&mut p, 1, &last), None);
}

#[test]
fn raster_titel_fuer_filler_fill_closedown() {
    let mut p = plan();
    p.set(
        2,
        vec![
            entry(EntryType::Fill, Some("m2"), T0, T0 + 10 * MIN),
            filler(T0 + 10 * MIN, T0 + 12 * MIN),
            closedown(T0 + 12 * MIN, T0 + 50 * MIN, None),
        ],
    );
    let rows = grid_rows(&mut p, T0, T0 + 50 * MIN, Some(0), Some(T0 + MIN));
    let r = &rows[1];
    assert_eq!(r.cells[0].title, "Füllprogramm");
    assert_eq!(r.cells[0].sub, "Erfundener Film Zwei");
    assert_eq!(r.cells[0].kind, Kind::Fill);
    assert!(r.cells[0].now && !r.cells[1].now);
    assert_eq!(
        r.cells[1].title, "Kurze Pause",
        "Folge ist Sendepause: kein Gleich-Titel"
    );
    assert_eq!(r.cells[2].title, "Sendepause");
    let r0 = &rows[0];
    let filler_cell = r0.cells.iter().find(|c| c.kind == Kind::Filler);
    assert!(
        filler_cell.is_none(),
        "Fenster endet vor dem Füller auf Sender 1"
    );
    let rows = grid_rows(&mut p, T0 + 55 * MIN, T0 + 80 * MIN, Some(0), None);
    let f = rows[0]
        .cells
        .iter()
        .find(|c| c.kind == Kind::Filler)
        .unwrap();
    assert_eq!(f.title, "Gleich: Erfundene Serie");
    assert_eq!(f.start, T0 + 60 * MIN);
}

#[test]
fn zellen_werden_auf_das_fenster_zugeschnitten() {
    let mut p = plan();
    let rows = grid_rows(&mut p, T0, T0 + 30 * MIN, Some(0), None);
    let c = &rows[0].cells[0];
    assert_eq!((c.start, c.end), (T0, T0 + 30 * MIN));
    assert_eq!(
        c.entry.start,
        T0 - 30 * MIN,
        "Originalzeiten bleiben im Eintrag"
    );
    assert!(
        grid_rows(&mut p, 5, 5, Some(0), None)
            .iter()
            .all(|r| r.cells.is_empty())
    );
}

#[test]
fn beschreibung_badges_thementag_und_fill() {
    let mut p = plan();
    let mut e = program("m1", T0, T0 + 95 * MIN);
    e.premiere = Some(PremiereKind::Film);
    e.rerun = true;
    e.age = Some(16);
    e.theme = Some("Komödientag".into());
    let d = describe(&mut p, 1, &e, T0 + 10 * MIN, None);
    assert_eq!(
        d.badges,
        vec!["Premiere", "Wdh.", "ab 16", "Thementag: Komödientag"]
    );
    assert_eq!(
        d.time,
        format!("{}–{}", format_clock(T0), format_clock(T0 + 95 * MIN))
    );
    assert!((d.progress - 10.0 / 95.0).abs() < 1e-9);
    assert_eq!(d.run_min, Some(95));
    assert_eq!(
        d.genres,
        vec!["Komödie"],
        "Anzeigelabel aus den Katalog-Genres"
    );
    assert_eq!(d.year, Some(1999));
    let mut f = entry(EntryType::Fill, Some("m2"), T0, T0 + 90 * MIN);
    f.label = Some("Nachtprogramm".into());
    let d = describe(&mut p, 1, &f, T0, None);
    assert_eq!(d.title, "Füllprogramm");
    assert_eq!(d.sub, "Erfundener Film Zwei");
    assert!(d.badges.contains(&"Nachtprogramm".to_string()));
    assert_eq!(badges(&program("m1", 0, 1)).len(), 0);
    let mut e18 = program("m1", 0, 1);
    e18.age = Some(12);
    e18.premiere = Some(PremiereKind::Episode);
    assert_eq!(badges(&e18), vec!["Neue Folge"]);
}

#[test]
fn bild_id_serie_bei_folgen_sonst_titel_none_ohne_katalog() {
    let mut p = plan();
    let d = describe(&mut p, 1, &program("e1", T0, T0 + 45 * MIN), T0, None);
    assert_eq!(d.image_item_id.as_deref(), Some("s1"));
    assert_eq!(d.series_id.as_deref(), Some("s1"));
    let d = describe(&mut p, 1, &program("m1", T0, T0 + 45 * MIN), T0, None);
    assert_eq!(d.image_item_id.as_deref(), Some("m1"));
    let d = describe(&mut p, 1, &filler(T0, T0 + MIN), T0, None);
    assert_eq!(d.image_item_id, None);
    let d = describe(&mut p, 1, &closedown(T0, T0 + MIN, None), T0, None);
    assert_eq!(d.title, "Sendepause");
    assert_eq!(d.image_item_id, None);
}

#[test]
fn inhaltsangabe_aus_karte_map() {
    let mut p = plan();
    let mut ov = HashMap::new();
    ov.insert("m1".to_string(), "Eine erfundene Handlung.".to_string());
    let d = describe(&mut p, 1, &program("m1", T0, T0 + MIN), T0, Some(&ov));
    assert_eq!(d.overview, "Eine erfundene Handlung.");
}

#[test]
fn sendernamen_ohne_live_praefix() {
    assert_eq!(channel_name(&ch(1, "LIVE Kino", None)), "Kino");
    assert_eq!(channel_name(&ch(7, "LIVE 7", None)), "Sender 7");
    assert_eq!(
        channel_name(&ch(5, "LIVE Action", Some("Action"))),
        "Action"
    );
    assert_eq!(channel_name(&ch(3, "live   Serie", None)), "Serie");
    assert_eq!(
        channel_name(&ch(3, "LIVERPOOL", None)),
        "LIVERPOOL",
        "Präfix nur mit Leerraum"
    );
    assert_eq!(channel_name(&ch(9, "9", None)), "Sender 9");
    assert_eq!(channel_name(&ch(0, "", None)), "");
}

// ---------------------------------------------------------------- Info-Karte

#[test]
fn info_karte_zeigt_kalenderdatum_des_beginns_nicht_den_sendetag() {
    // Beginn 02:00 am 21.11. gehört zum Sendetag 20.11., die Karte zeigt aber den 21.11.
    let d21 = days_from_civil(2026, 11, 21);
    let start = wall_to_utc(d21, 2 * 60);
    let label = start_date_label(start);
    let f = lfx_tv_schedule::time::format_day(d21);
    assert_eq!(label, format!("{} {}", f.weekday, f.date));
    assert!(label.ends_with("21.11."));
    // 20:15 am 20.11.: gleiches Datum wie der Sendetag
    assert!(
        start_date_label(wall_to_utc(days_from_civil(2026, 11, 20), 20 * 60 + 15))
            .ends_with("20.11.")
    );
    // vor 05:00 und 23:30
    assert!(start_date_label(wall_to_utc(d21, 4 * 60 + 59)).ends_with("21.11."));
    assert!(start_date_label(wall_to_utc(d21, 23 * 60 + 30)).ends_with("21.11."));
}

#[test]
fn info_karte_inhalt_aktionen_und_tick() {
    let mut p = plan();
    let e = program("m1", T0 + 30 * MIN, T0 + 60 * MIN);
    let c = build_info_card(&mut p, 5, &e, T0, None);
    assert_eq!(c.channel, "5 · Sender 5");
    assert!(!c.running);
    assert_eq!(c.title, "Erfundener Film Eins");
    assert_eq!(c.meta.as_deref(), Some("1999 · 95 Min. · Komödie"));
    assert_eq!(
        c.actions,
        vec![
            CardAction::Details("m1".into()),
            CardAction::PlayFromStart("m1".into())
        ],
        "nicht laufend: kein Einschalten"
    );
    assert_eq!(CardAction::TuneNow(1).label(), "Jetzt einschalten");
    let mut c = c;
    assert!(!c.tick(T0 + 29 * MIN));
    assert!(c.tick(T0 + 31 * MIN));
    assert_eq!(c.actions[0], CardAction::TuneNow(5));
    assert!(c.tick(T0 + 61 * MIN));
    assert!(
        c.actions
            .iter()
            .all(|a| !matches!(a, CardAction::TuneNow(_)))
    );
}

#[test]
fn info_karte_filler_ohne_titel_hat_keine_details() {
    let mut p = plan();
    let c = build_info_card(&mut p, 1, &filler(T0, T0 + MIN), T0, None);
    assert!(
        c.actions
            .iter()
            .all(|a| matches!(a, CardAction::TuneNow(_)))
    );
    assert_eq!(c.overview, Overview::Hidden);
    assert_eq!(c.meta, None);
}

#[test]
fn inhaltsangabe_zustaende() {
    let mut p = plan();
    let e = program("m1", T0, T0 + MIN);
    // kein Dienst: ausgeblendet
    assert_eq!(
        build_info_card(&mut p, 1, &e, T0, None).overview,
        Overview::Hidden
    );
    let mut map = HashMap::new();
    // Dienst da, noch nichts geladen
    let mut c = build_info_card(&mut p, 1, &e, T0, Some(&map));
    assert_eq!(c.overview, Overview::Loading);
    assert_eq!(c.overview.text(), "Inhaltsangabe wird geladen …");
    map.insert("m1".into(), String::new());
    c.set_details(Some(&map));
    assert_eq!(c.overview, Overview::Empty);
    assert_eq!(c.overview.text(), "Keine Inhaltsangabe vorhanden.");
    map.insert("m1".into(), "Text".into());
    c.set_details(Some(&map));
    assert_eq!(c.overview, Overview::Text("Text".into()));
    assert_eq!(
        build_info_card(&mut p, 1, &e, T0, Some(&map)).overview,
        Overview::Text("Text".into())
    );
}

#[test]
fn details_entprellt_sofort_und_einmal() {
    let mut d = Details::new(true);
    assert_eq!(d.request("a", false, 0), None);
    assert_eq!(
        d.request("b", false, 100),
        None,
        "wartende andere Id wird verworfen"
    );
    assert_eq!(d.next_deadline(), Some(500));
    assert_eq!(d.poll(499), None);
    assert_eq!(d.poll(500).as_deref(), Some("b"));
    assert_eq!(d.request("b", true, 600), None, "läuft schon");
    assert_eq!(d.request("c", true, 600).as_deref(), Some("c"));
    d.provide("c", "Text C");
    assert_eq!(d.get("c").map(String::as_str), Some("Text C"));
    assert_eq!(d.request("c", true, 700), None, "schon geladen");
    d.provide("b", "");
    assert_eq!(d.get("b").map(String::as_str), Some(""));
    let mut off = Details::new(false);
    assert_eq!(off.request("a", true, 0), None);
    d.dispose();
    assert_eq!(d.request("z", true, 0), None);
}

// ---------------------------------------------------------------- EPG-Zellen

fn cell(width_min: f64, kind: Kind) -> EpgCell {
    let mut p = plan();
    let g = EpgGrid::build(&mut p, T0, None);
    let mut c = g.rows()[0].cells[0].clone();
    c.width_min = width_min;
    c.kind = kind;
    c
}

#[test]
fn zellen_groessenklassen_fueller_frueher_klein() {
    assert_eq!(classify(&cell(5.0, Kind::Program), 5.0), CellSize::Tiny); // 25 px
    assert_eq!(classify(&cell(12.0, Kind::Program), 5.0), CellSize::Short); // 60 px
    assert_eq!(classify(&cell(30.0, Kind::Program), 5.0), CellSize::Normal);
    assert_eq!(
        classify(&cell(12.0, Kind::Filler), 5.0),
        CellSize::Tiny,
        "Füller unter 90 px ohne Text"
    );
}

#[test]
fn zellenklick_laufend_schaltet_sonst_info() {
    let mut p = plan();
    let g = EpgGrid::build(&mut p, T0, None);
    let cur = g.rows()[0].cells.iter().find(|c| c.current).unwrap();
    assert_eq!(EpgGrid::on_cell(cur, T0), CellAction::Tune(1));
    let other = g.rows()[0].cells.iter().find(|c| !c.current).unwrap();
    assert!(matches!(
        EpgGrid::on_cell(other, T0),
        CellAction::Info { no: 1, .. }
    ));
}

// ---------------------------------------------------------------- Nachbarn

fn ctx<'a>(
    p: &'a mut FakePlan,
    channel: i64,
    phase: WarmPhase,
    flags: ZapFlags,
) -> NeighborCtx<'a> {
    NeighborCtx {
        plan: p,
        channel,
        phase,
        flags,
        channel_count: 18,
    }
}

#[test]
fn nachbarn_umlauf_sender_1_waermt_18_und_2_sender_18_waermt_17_und_1() {
    for (channel, want) in [(1, [18, 2]), (18, [17, 1])] {
        let mut p = FakePlan::new();
        for no in 1..=18 {
            p.set(
                no,
                vec![program(&format!("m{no}"), T0 - MIN, T0 + 60 * MIN)],
            );
        }
        let mut nb = Neighbors::new();
        nb.arm(Mode::Play, ZapFlags::default(), T0);
        assert_eq!(nb.next_deadline(), Some(T0 + NEIGHBOR_MS));
        let acts = nb.poll(
            T0 + NEIGHBOR_MS,
            &mut ctx(&mut p, channel, WarmPhase::Playing, ZapFlags::default()),
        );
        let ids: Vec<String> = acts
            .iter()
            .filter_map(|a| {
                if let NeighborAction::Prewarm { item_id, offset_ms } = a {
                    assert_eq!(*offset_ms, MIN + NEIGHBOR_MS);
                    Some(item_id.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            ids,
            want.iter().map(|n| format!("m{n}")).collect::<Vec<_>>()
        );
    }
}

#[test]
fn nachbarn_takt_35_s_hoechstens_10_mal() {
    let mut p = plan();
    let mut nb = Neighbors::new();
    nb.arm(Mode::Play, ZapFlags::default(), 0);
    let mut t = NEIGHBOR_MS;
    let mut runs = 0;
    while nb.next_deadline().is_some() {
        assert_eq!(nb.next_deadline(), Some(t));
        let acts = nb.poll(
            t,
            &mut ctx(&mut p, 1, WarmPhase::Playing, ZapFlags::default()),
        );
        // plan(): Programm läuft bei T0 nicht; Zeit t ist klein → Füller-Notiz, kein Programm
        assert!(!acts.is_empty());
        runs += 1;
        t += WARM_TICK_MS;
        if runs > 20 {
            break;
        }
    }
    assert_eq!(
        runs,
        WARM_TICK_MAX as usize + 1,
        "erster Lauf plus 10 Auffrischungen"
    );
}

#[test]
fn nachbarn_ohne_programm_notiz_filler() {
    let mut p = FakePlan::new();
    for no in 1..=18 {
        p.set(no, vec![filler(T0 - MIN, T0 + MIN)]);
    }
    let mut nb = Neighbors::new();
    nb.arm(Mode::Play, ZapFlags::default(), T0);
    let acts = nb.poll(
        T0 + NEIGHBOR_MS,
        &mut ctx(&mut p, 5, WarmPhase::Playing, ZapFlags::default()),
    );
    assert_eq!(acts, vec![NeighborAction::Note("filler")]);
}

#[test]
fn nachbarn_puffern_erneut_jede_sekunde_max_20_grund_phase() {
    let mut p = plan();
    let mut nb = Neighbors::new();
    nb.arm(Mode::Play, ZapFlags::default(), 0);
    let mut t = NEIGHBOR_MS;
    let mut notes = 0;
    while let Some(d) = nb.next_deadline() {
        assert_eq!(d, t);
        let acts = nb.poll(
            t,
            &mut ctx(&mut p, 1, WarmPhase::Other, ZapFlags::default()),
        );
        assert_eq!(acts, vec![NeighborAction::Note("phase")]);
        notes += 1;
        t += 1000;
    }
    assert_eq!(notes, 21, "erster Versuch plus 20 Wiederholungen");
}

#[test]
fn nachbarn_fehlerkarte_nach_2_s_nur_mit_vorwaermen_karte_weg_schluss() {
    let mut p = plan();
    let mut nb = Neighbors::new();
    nb.arm(
        Mode::Error,
        ZapFlags {
            zap_warm: false,
            zap_prefetch: true,
        },
        0,
    );
    assert_eq!(nb.next_deadline(), None, "ohne Vorwärmen nichts");
    nb.arm(Mode::Error, ZapFlags::default(), 0);
    assert_eq!(nb.next_deadline(), Some(ERROR_WARM_MS));
    let acts = nb.poll(
        ERROR_WARM_MS,
        &mut ctx(&mut p, 1, WarmPhase::Other, ZapFlags::default()),
    );
    assert!(acts.is_empty(), "Karte weg: Schluss");
    assert_eq!(nb.next_deadline(), None);
}

#[test]
fn nachbarn_schalter_werden_je_schritt_gelesen() {
    let mut p = FakePlan::new();
    for no in 1..=18 {
        p.set(
            no,
            vec![program(&format!("m{no}"), T0 - MIN, T0 + 600 * MIN)],
        );
    }
    let mut nb = Neighbors::new();
    nb.arm(Mode::Play, ZapFlags::default(), T0);
    // Datensparmodus an: nichts, Takt bleibt (erneut versuchen)
    let off = ZapFlags {
        zap_warm: false,
        zap_prefetch: false,
    };
    let acts = nb.poll(
        T0 + NEIGHBOR_MS,
        &mut ctx(&mut p, 2, WarmPhase::Playing, off),
    );
    assert!(acts.is_empty());
    assert_eq!(nb.next_deadline(), Some(T0 + NEIGHBOR_MS + WARM_TICK_MS));
    // während der Sendung wieder an: Vorwärmen
    let acts = nb.poll(
        T0 + NEIGHBOR_MS + WARM_TICK_MS,
        &mut ctx(&mut p, 2, WarmPhase::Playing, ZapFlags::default()),
    );
    assert_eq!(
        acts.iter()
            .filter(|a| matches!(a, NeighborAction::Prewarm { .. }))
            .count(),
        2
    );
    nb.stop();
    assert_eq!(nb.next_deadline(), None);
}

#[test]
fn nachbarn_prefetch_je_sendung_einmal_neue_sendung_erneut() {
    let mut p = FakePlan::new();
    for no in 1..=18 {
        p.set(
            no,
            vec![
                program(&format!("m{no}"), T0 - MIN, T0 + 10 * MIN),
                program(&format!("n{no}"), T0 + 10 * MIN, T0 + 100 * MIN),
            ],
        );
    }
    let flags = ZapFlags {
        zap_warm: false,
        zap_prefetch: true,
    };
    let mut nb = Neighbors::new();
    let mut total = Vec::new();
    for now in [T0, T0 + 20 * MIN] {
        nb.arm(Mode::Play, flags, now);
        let acts = nb.poll(
            now + NEIGHBOR_MS,
            &mut ctx(&mut p, 3, WarmPhase::Playing, flags),
        );
        total.extend(acts);
        // zweites Arming derselben Sendung: nichts Neues
        nb.arm(Mode::Play, flags, now + 5000);
        let again = nb.poll(
            now + 5000 + NEIGHBOR_MS,
            &mut ctx(&mut p, 3, WarmPhase::Playing, flags),
        );
        assert!(again.is_empty());
    }
    assert_eq!(total.len(), 4);
    assert!(
        total
            .iter()
            .all(|a| matches!(a, NeighborAction::Prefetch { .. }))
    );
}
