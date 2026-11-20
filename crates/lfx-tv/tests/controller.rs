//! Controller-Tests mit Planer-Attrappe (erfundene Titel) und von Hand gestellter Uhr.
//! Semantik aus den Web-Tests `tvmode-controller*.test.js`.

use lfx_tv::controller::{AudioTicket, Card, Phase, Reason, TvController, TvOptions, TvOutput};
use lfx_tv::filler::{SecretUrl, TrailerInfo};
use lfx_tv::neighbors::ZapFlags;
use lfx_tv::request::{PlayResult, PlayerPhase, Timing, Token, TvPlayRequest, VolumeMode};
use lfx_tv::stream::FailReason;
use lfx_tv::testing::{FakePlan, closedown, entry, episode, filler, movie, program};
use lfx_tv::tracks::{Track, Tracks};
use lfx_tv::{ManualClock, PlanSource};
use lfx_tv_schedule::{EntryType, PlanEntry};

const MIN: i64 = 60_000;
const T0: i64 = 1_800_000_000_000;

struct H {
    ctl: TvController<FakePlan>,
    clock: ManualClock,
}

fn std_plan() -> FakePlan {
    let mut p = FakePlan::new();
    p.item_info(movie("m1", "Erfundener Film Eins", 90 * MIN));
    p.item_info(movie("m2", "Erfundener Film Zwei", 90 * MIN));
    p.item_info(episode(
        "e1",
        "Erfundene Serie",
        "Die Folge",
        1.0,
        3.0,
        "s1",
    ));
    p.all(&[
        program("m1", T0 - 30 * MIN, T0 + 60 * MIN),
        program("m2", T0 + 60 * MIN, T0 + 150 * MIN),
        program("m1", T0 + 150 * MIN, T0 + 240 * MIN),
    ]);
    p
}

fn harness_with(plan: FakePlan, opts: TvOptions) -> H {
    let clock = ManualClock::new(T0);
    let ctl = TvController::new(plan, Box::new(clock.clone()), opts);
    H { ctl, clock }
}

fn harness() -> H {
    harness_with(std_plan(), TvOptions::default())
}

impl H {
    fn out(&mut self) -> Vec<TvOutput> {
        self.ctl.take_output()
    }

    /// Zeit vorrücken und `poll` aufrufen.
    fn advance(&mut self, ms: i64) {
        self.clock.advance(ms);
        self.ctl.poll();
    }

    fn start_ok(&mut self) -> TvPlayRequest {
        self.ctl.start();
        let outs = self.out();
        let req = plays(&outs).into_iter().next().expect("Play nach start");
        self.ctl.on_play_result(req.token, &PlayResult::ok());
        self.out();
        req
    }
}

fn plays(o: &[TvOutput]) -> Vec<TvPlayRequest> {
    o.iter()
        .filter_map(|x| {
            if let TvOutput::Play(r) = x {
                Some(r.clone())
            } else {
                None
            }
        })
        .collect()
}

fn has(o: &[TvOutput], f: impl Fn(&TvOutput) -> bool) -> bool {
    o.iter().any(f)
}

fn count(o: &[TvOutput], f: impl Fn(&TvOutput) -> bool) -> usize {
    o.iter().filter(|x| f(x)).count()
}

#[test]
fn start_unlock_zuerst_dann_play_mit_offset_aus_der_uhr() {
    let mut h = harness();
    h.ctl.start();
    let outs = h.out();
    assert_eq!(outs[0], TvOutput::Unlock, "Unlock synchron als Erstes");
    let reqs = plays(&outs);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].item_id, "m1");
    assert_eq!(reqs[0].offset_ms, 30 * MIN);
    assert_eq!(reqs[0].entry_start, T0 - 30 * MIN);
    assert!(!reqs[0].report_progress);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
    h.ctl.on_play_result(reqs[0].token, &PlayResult::ok());
    let s = h.ctl.state();
    assert_eq!(s.phase, Phase::Playing);
    assert!(s.on);
    assert_eq!(s.channel, 1);
    assert!(s.card.is_none());
}

#[test]
fn zweites_start_ruft_nur_unlock() {
    let mut h = harness();
    h.start_ok();
    h.ctl.start();
    assert_eq!(h.out(), vec![TvOutput::Unlock]);
}

#[test]
fn letzter_sender_aus_optionen_und_gespeichert() {
    let mut h = harness_with(
        std_plan(),
        TvOptions {
            last_channel: 7,
            ..TvOptions::default()
        },
    );
    assert_eq!(h.ctl.state().channel, 7);
    h.ctl.start();
    let outs = h.out();
    assert!(has(
        &outs,
        |o| matches!(o, TvOutput::SavePrefs(p) if p.last_channel == Some(7))
    ));
    let h = harness_with(
        std_plan(),
        TvOptions {
            last_channel: 99,
            ..TvOptions::default()
        },
    );
    assert_eq!(h.ctl.state().channel, 18);
    let h = harness_with(
        std_plan(),
        TvOptions {
            last_channel: 0,
            ..TvOptions::default()
        },
    );
    assert_eq!(h.ctl.state().channel, 1);
}

#[test]
fn sendungsende_schaltet_mit_30ms_zur_naechsten() {
    let mut h = harness();
    h.start_ok();
    h.advance(60 * MIN + 29);
    assert!(plays(&h.out()).is_empty(), "noch nicht +30 ms");
    h.advance(1);
    let outs = h.out();
    let reqs = plays(&outs);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].item_id, "m2");
    assert_eq!(reqs[0].offset_ms, 30);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
}

#[test]
fn entprellung_mehrere_zaps_ergeben_genau_einen_play() {
    let mut h = harness();
    h.start_ok();
    h.ctl.zap(1);
    h.advance(100);
    h.ctl.zap(1);
    h.advance(100);
    h.ctl.zap(1);
    let outs = h.out();
    assert!(plays(&outs).is_empty());
    assert!(has(&outs, |o| matches!(o, TvOutput::StopPlayer)));
    assert_eq!(h.ctl.state().channel, 4);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
    assert!(h.ctl.state().entry.is_some(), "Vorschau des neuen Senders");
    h.advance(249);
    assert!(plays(&h.out()).is_empty());
    h.advance(1);
    let reqs = plays(&h.out());
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].item_id, "m1");
}

#[test]
fn tune_stoesst_vorausloesung_des_ziels_an_folgedruck_bricht_ab() {
    let mut h = harness();
    h.start_ok();
    h.ctl.tune(5, false);
    let o1 = h.out();
    assert!(has(
        &o1,
        |o| matches!(o, TvOutput::PrefetchTarget { item_id } if item_id == "m1")
    ));
    h.ctl.tune(6, false);
    let o2 = h.out();
    let cancel = o2
        .iter()
        .position(|o| matches!(o, TvOutput::CancelPrefetch))
        .expect("abbrechen");
    let again = o2
        .iter()
        .position(|o| matches!(o, TvOutput::PrefetchTarget { .. }))
        .expect("neu");
    assert!(cancel < again);
}

#[test]
fn umlauf_1_bis_18_in_beide_richtungen() {
    let mut h = harness();
    h.start_ok();
    h.ctl.zap(-1);
    assert_eq!(h.ctl.state().channel, 18);
    h.advance(300);
    h.ctl.zap(1);
    assert_eq!(h.ctl.state().channel, 1);
    h.ctl.tune(19, true);
    assert_eq!(
        h.ctl.state().channel,
        1,
        "19 = Umlauf auf 1 mit laufender Sendung: nichts"
    );
    h.ctl.tune(0, true);
    assert_eq!(h.ctl.state().channel, 18);
}

#[test]
fn immediate_ueberspringt_entprellung_und_stop() {
    let mut h = harness();
    h.start_ok();
    h.ctl.tune(5, true);
    let outs = h.out();
    assert!(!has(&outs, |o| matches!(o, TvOutput::StopPlayer)));
    assert_eq!(plays(&outs).len(), 1);
}

#[test]
fn gleicher_sender_mit_laufender_sendung_ist_noop() {
    let mut h = harness();
    h.start_ok();
    h.ctl.tune(1, true);
    assert!(h.out().is_empty());
}

#[test]
fn veraltete_antworten_werden_verworfen() {
    let mut h = harness();
    h.ctl.start();
    let first = plays(&h.out()).remove(0);
    h.ctl.tune(2, true);
    let second = plays(&h.out()).remove(0);
    h.ctl.on_play_result(first.token, &PlayResult::ok());
    assert_eq!(
        h.ctl.state().phase,
        Phase::Loading,
        "alte Kennung wirkt nicht"
    );
    h.ctl.on_play_result(second.token, &PlayResult::ok());
    assert_eq!(h.ctl.state().phase, Phase::Playing);
}

#[test]
fn zap_im_ausgeschalteten_zustand_tut_nichts() {
    let mut h = harness();
    h.ctl.zap(1);
    h.ctl.tune(3, true);
    h.ctl.push_digit('4');
    assert!(h.out().is_empty());
    assert_eq!(h.ctl.state().phase, Phase::Off);
}

// ---------------------------------------------------------------- Füller, Pause, Sendeschluss

fn filler_plan() -> FakePlan {
    let mut p = std_plan();
    p.all(&[
        filler(T0 - 5 * MIN, T0 + 10 * MIN),
        program("m2", T0 + 10 * MIN, T0 + 100 * MIN),
    ]);
    p
}

#[test]
fn filler_ohne_trailer_pausenkarte_und_neuer_versuch_zum_trailerstart() {
    let mut h = harness_with(filler_plan(), TvOptions::default());
    h.ctl.start();
    let outs = h.out();
    let tok = outs
        .iter()
        .find_map(|o| {
            if let TvOutput::LoadTrailer { token, item_id } = o {
                Some((*token, item_id.clone()))
            } else {
                None
            }
        })
        .expect("LoadTrailer");
    assert_eq!(tok.1, "m2");
    // Trailer von 2 min Länge: Fenster ab T0+8min, jetzt T0 → Karte mit Startzeit
    let t = TrailerInfo {
        url: SecretUrl("http://x/t?ApiKey=geheim".into()),
        dur_ms: Some(2 * MIN),
    };
    h.ctl.on_trailer(tok.0, Some(t.clone()));
    let s = h.ctl.state();
    assert_eq!(s.phase, Phase::Pause);
    match s.card {
        Some(Card::Next { title, .. }) => assert_eq!(title, "Erfundener Film Zwei"),
        c => panic!("{c:?}"),
    }
    h.out();
    h.advance(8 * MIN + 19);
    assert!(
        !has(&h.out(), |o| matches!(o, TvOutput::LoadTrailer { .. })),
        "20 ms Zuschlag"
    );
    h.advance(1);
    let outs = h.out();
    let tok2 = outs
        .iter()
        .find_map(|o| {
            if let TvOutput::LoadTrailer { token, .. } = o {
                Some(*token)
            } else {
                None
            }
        })
        .expect("neuer Versuch");
    h.ctl.on_trailer(tok2, Some(t));
    let outs = h.out();
    let url = outs
        .iter()
        .find_map(|o| {
            if let TvOutput::PlayUrl { offset_ms, url, .. } = o {
                Some((*offset_ms, url.clone()))
            } else {
                None
            }
        })
        .expect("PlayUrl");
    assert!(url.0 >= 0 && url.0 < 2 * MIN);
    assert!(
        !format!("{:?}", outs).contains("geheim"),
        "Adresse nie im Debug"
    );
}

#[test]
fn filler_mit_trailer_spielt_per_playurl_ende_fuehrt_zur_pausenkarte() {
    let mut h = harness_with(filler_plan(), TvOptions::default());
    h.clock.set(T0 + 9 * MIN);
    h.ctl.start();
    let tok = h
        .out()
        .iter()
        .find_map(|o| {
            if let TvOutput::LoadTrailer { token, .. } = o {
                Some(*token)
            } else {
                None
            }
        })
        .unwrap();
    let t = TrailerInfo {
        url: SecretUrl("u".into()),
        dur_ms: Some(2 * MIN),
    };
    h.ctl.on_trailer(tok, Some(t));
    let outs = h.out();
    assert!(has(
        &outs,
        |o| matches!(o, TvOutput::PlayUrl { offset_ms, .. } if *offset_ms == MIN)
    ));
    h.ctl.on_play_url_result(tok, &PlayResult::ok());
    assert_eq!(h.ctl.state().phase, Phase::Playing);
    h.ctl.on_player_state(PlayerPhase::Ended);
    assert_eq!(h.ctl.state().phase, Phase::Pause);
}

#[test]
fn trailer_schlaegt_fehl_pausenkarte_statt_fehler() {
    let mut h = harness_with(filler_plan(), TvOptions::default());
    h.clock.set(T0 + 9 * MIN);
    h.ctl.start();
    let tok = h
        .out()
        .iter()
        .find_map(|o| {
            if let TvOutput::LoadTrailer { token, .. } = o {
                Some(*token)
            } else {
                None
            }
        })
        .unwrap();
    h.ctl.on_trailer(
        tok,
        Some(TrailerInfo {
            url: SecretUrl("u".into()),
            dur_ms: Some(2 * MIN),
        }),
    );
    h.out();
    h.ctl
        .on_play_url_result(tok, &PlayResult::fail(FailReason::Network));
    assert_eq!(h.ctl.state().phase, Phase::Pause);
    assert!(has(&h.out(), |o| matches!(o, TvOutput::StopPlayer)));
}

#[test]
fn filler_ohne_folgetitel_direkt_pausenkarte() {
    let mut p = std_plan();
    p.all(&[
        filler(T0 - MIN, T0 + 10 * MIN),
        closedown(T0 + 10 * MIN, T0 + 20 * MIN, None),
    ]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    let outs = h.out();
    assert!(!has(&outs, |o| matches!(o, TvOutput::LoadTrailer { .. })));
    assert_eq!(h.ctl.state().phase, Phase::Pause);
    assert_eq!(
        h.ctl.state().card,
        Some(Card::Pause {
            text: "Gleich geht es weiter".into()
        })
    );
}

#[test]
fn fill_mit_itemid_wird_gespielt_banner_fuellprogramm() {
    let mut p = std_plan();
    p.all(&[entry(EntryType::Fill, Some("m1"), T0 - MIN, T0 + 60 * MIN)]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    assert_eq!(plays(&h.out()).len(), 1);
    assert_eq!(h.ctl.state().banner.as_deref(), Some("Füllprogramm"));
}

#[test]
fn nightfill_banner_label_hat_vorrang() {
    let mut e = entry(EntryType::Nightfill, Some("m1"), T0 - MIN, T0 + 60 * MIN);
    let mut p = std_plan();
    p.all(&[e.clone()]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    assert_eq!(
        h.ctl.state().banner.as_deref(),
        Some("Füllprogramm – reguläres Programm ab 5:00")
    );
    e.label = Some("Nachtprogramm".into());
    let mut p = std_plan();
    p.all(&[e]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    assert_eq!(h.ctl.state().banner.as_deref(), Some("Nachtprogramm"));
}

#[test]
fn fill_ohne_titel_pausenkarte_mit_banner_kein_play() {
    let mut p = std_plan();
    p.all(&[entry(EntryType::Fill, None, T0 - MIN, T0 + 60 * MIN)]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    let outs = h.out();
    assert!(plays(&outs).is_empty());
    let s = h.ctl.state();
    assert_eq!(s.phase, Phase::Pause);
    assert_eq!(s.banner.as_deref(), Some("Füllprogramm"));
}

#[test]
fn closedown_karte_mit_uhrzeit_kein_play() {
    let mut p = std_plan();
    p.all(&[
        closedown(T0 - MIN, T0 + 30 * MIN, None),
        program("m1", T0 + 30 * MIN, T0 + 100 * MIN),
    ]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    let outs = h.out();
    assert!(plays(&outs).is_empty());
    assert!(has(&outs, |o| matches!(o, TvOutput::StopPlayer)));
    let s = h.ctl.state();
    assert_eq!(s.phase, Phase::Closedown);
    match s.card {
        Some(Card::Closedown { text, until }) => {
            assert_eq!(text, "Sendepause");
            assert_eq!(until, lfx_tv_schedule::time::format_clock(T0 + 30 * MIN));
        }
        c => panic!("{c:?}"),
    }
}

#[test]
fn closedown_empty_auf_diesem_sender_laeuft_nichts() {
    let mut p = std_plan();
    p.all(&[closedown(T0 - MIN, T0 + 30 * MIN, Some("empty"))]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    let s = h.ctl.state();
    assert_eq!(s.phase, Phase::Closedown);
    assert_eq!(s.reason, Some(Reason::Empty));
    assert_eq!(
        s.card,
        Some(Card::Empty {
            text: "Auf diesem Sender läuft gerade nichts".into()
        })
    );
}

// ---------------------------------------------------------------- Fehler

#[test]
fn network_fehlerkarte_neuer_versuch_nach_10_s() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl
        .on_play_result(req.token, &PlayResult::fail(FailReason::Network));
    let s = h.ctl.state();
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.reason, Some(Reason::Network));
    match s.card {
        Some(Card::Error { code, title, .. }) => {
            assert_eq!(code.as_deref(), Some("NET"));
            assert_eq!(title, "Empfangsstörung – erneuter Versuch …");
        }
        c => panic!("{c:?}"),
    }
    h.out();
    h.advance(9_999);
    assert!(plays(&h.out()).is_empty());
    h.advance(1);
    assert_eq!(plays(&h.out()).len(), 1);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
}

#[test]
fn video_audio_browser_bleiben_stehen_kein_automatischer_retry() {
    for why in [FailReason::Video, FailReason::Audio, FailReason::Browser] {
        let mut h = harness();
        h.ctl.start();
        let req = plays(&h.out()).remove(0);
        h.ctl
            .on_play_result(req.token, &PlayResult::fail_code(why, "V-CODEC"));
        h.out();
        h.advance(60_000);
        assert!(plays(&h.out()).is_empty(), "{why:?}");
        assert_eq!(h.ctl.state().phase, Phase::Error);
    }
}

#[test]
fn code_und_hinweis_stehen_in_der_karte() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    let mut res = PlayResult::fail_code(FailReason::Video, "M3·F");
    res.hint = Some("Bildformat h264 HDR".into());
    h.ctl.on_play_result(req.token, &res);
    match h.ctl.state().card {
        Some(Card::Error {
            code, hint, reason, ..
        }) => {
            assert_eq!(code.as_deref(), Some("M3·F"));
            assert_eq!(
                hint.as_deref(),
                Some("Bildformat h264 HDR wird von diesem Gerät nicht unterstützt")
            );
            assert_eq!(reason, Reason::Video);
        }
        c => panic!("{c:?}"),
    }
}

#[test]
fn player_fehlerereignis_waehrend_der_wiedergabe_zeigt_fehlerkarte() {
    let mut h = harness();
    h.start_ok();
    h.ctl.on_player_error(FailReason::Video, Some("M3"), None);
    assert_eq!(h.ctl.state().phase, Phase::Error);
    // zweiter Fehler bei stehender Karte: nichts
    h.out();
    h.ctl.on_player_error(FailReason::Network, None, None);
    assert!(h.out().is_empty());
}

#[test]
fn abgebrochene_antwort_wird_ignoriert() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl
        .on_play_result(req.token, &PlayResult::fail(FailReason::Aborted));
    assert_eq!(h.ctl.state().phase, Phase::Loading);
}

#[test]
fn gesture_needs_tap_entsperrt_und_versucht_neu() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl
        .on_play_result(req.token, &PlayResult::fail(FailReason::Gesture));
    let s = h.ctl.state();
    assert!(s.needs_tap);
    assert_eq!(s.phase, Phase::Error);
    h.out();
    h.ctl.gesture();
    let outs = h.out();
    assert_eq!(outs[0], TvOutput::Unlock);
    assert_eq!(plays(&outs).len(), 1);
    assert!(!h.ctl.state().needs_tap);
}

#[test]
fn autoplay_muted_setzt_muted_by_policy_und_gesture_schaltet_ton_an() {
    let mut h = harness();
    h.start_ok();
    h.ctl.on_autoplay_muted();
    let outs = h.out();
    assert!(has(
        &outs,
        |o| matches!(o, TvOutput::Message(m) if m == "Ton aus – zum Einschalten tippen")
    ));
    let s = h.ctl.state();
    assert!(s.muted_by_policy && s.muted);
    h.ctl.gesture();
    let s = h.ctl.state();
    assert!(!s.muted_by_policy && !s.muted);
}

#[test]
fn autoplay_muted_bei_selbst_stummem_nutzer_stumm_bleibt() {
    let mut h = harness_with(
        std_plan(),
        TvOptions {
            muted: true,
            ..TvOptions::default()
        },
    );
    h.start_ok();
    h.ctl.on_autoplay_muted();
    let outs = h.out();
    assert!(!has(&outs, |o| matches!(o, TvOutput::Message(_))));
    h.ctl.gesture();
    assert!(h.ctl.state().muted);
}

#[test]
fn retry_startet_den_sender_neu() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl
        .on_play_result(req.token, &PlayResult::fail(FailReason::Video));
    h.out();
    h.ctl.retry();
    assert_eq!(plays(&h.out()).len(), 1);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
}

// ---------------------------------------------------------------- Gesehen

fn played(o: &[TvOutput]) -> Vec<String> {
    o.iter()
        .filter_map(|x| {
            if let TvOutput::SetPlayed { item_id } = x {
                Some(item_id.clone())
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn gesehen_erst_nach_90_prozent_und_nur_einmal() {
    let mut p = std_plan();
    p.all(&[
        program("m1", T0, T0 + 100 * MIN),
        program("m2", T0 + 100 * MIN, T0 + 200 * MIN),
    ]);
    let mut h = harness_with(p, TvOptions::default());
    h.start_ok();
    h.advance(89 * MIN);
    assert!(played(&h.out()).is_empty());
    h.advance(MIN + 1000);
    assert_eq!(played(&h.out()), vec!["m1".to_string()]);
    h.advance(5 * MIN);
    assert!(played(&h.out()).is_empty());
}

#[test]
fn wegzappen_vor_der_regel_zaehlt_nicht() {
    let mut p = std_plan();
    p.all(&[program("m1", T0, T0 + 100 * MIN)]);
    let mut h = harness_with(p, TvOptions::default());
    h.start_ok();
    h.advance(50 * MIN);
    h.ctl.tune(2, true);
    h.advance(60 * MIN);
    // Sender 2 läuft ebenfalls m1 ab T0, zählt aber erst nach Start: kein Ergebnis → nichts
    assert!(played(&h.out()).is_empty());
}

#[test]
fn start_ist_erst_nach_erfolgreichem_abspielen_registriert() {
    let mut p = std_plan();
    p.all(&[program("m1", T0 - 5 * MIN, T0 + 10 * MIN)]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.advance(9 * MIN);
    // kein Ergebnis → nicht registriert
    assert!(played(&h.out()).is_empty());
    h.ctl.on_play_result(req.token, &PlayResult::ok());
    h.advance(MIN);
    // Eintrag lief inzwischen ab (advance hat bereits neu gestartet)
    let _ = h.out();
}

#[test]
fn stillstand_ueber_60_s_verwirft_die_zaehlung() {
    let mut p = std_plan();
    p.all(&[program("m1", T0, T0 + 100 * MIN)]);
    let mut h = harness_with(p, TvOptions::default());
    h.start_ok();
    h.ctl.on_player_state(PlayerPhase::Stalled);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
    h.advance(125_000); // Zeitgeber verlässt nach 60 s still, die Lücke bis zum Wiedereinschalten ist > 60 s
    h.ctl.on_player_state(PlayerPhase::Playing);
    assert_eq!(h.ctl.state().phase, Phase::Playing);
    h.advance(95 * MIN);
    assert!(
        played(&h.out()).is_empty(),
        "nach Stillstand > 60 s nie gesehen"
    );
}

#[test]
fn stalled_dann_playing_kehrt_auf_playing_zurueck() {
    let mut h = harness();
    h.start_ok();
    h.ctl.on_player_state(PlayerPhase::Stalled);
    assert_eq!(h.ctl.state().phase, Phase::Loading);
    h.ctl.on_player_state(PlayerPhase::Playing);
    assert_eq!(h.ctl.state().phase, Phase::Playing);
}

// ---------------------------------------------------------------- Lautstärke

#[test]
fn setvolume_rundet_auf_5er_schritte_begrenzt_und_speichert() {
    let mut h = harness();
    h.ctl.set_volume(47.0);
    assert_eq!(h.ctl.state().volume, 45);
    let outs = h.out();
    assert!(has(
        &outs,
        |o| matches!(o, TvOutput::SavePrefs(p) if p.volume == Some(0.45) && p.muted == Some(false))
    ));
    h.ctl.set_volume(250.0);
    assert_eq!(h.ctl.state().volume, 100);
    h.ctl.set_volume(-8.0);
    assert_eq!(h.ctl.state().volume, 0);
    assert!(h.ctl.state().muted, "Pegel 0 ist stumm");
}

#[test]
fn optionen_volume_0_bis_1_oder_prozent() {
    assert_eq!(
        harness_with(
            std_plan(),
            TvOptions {
                volume: 0.5,
                ..TvOptions::default()
            }
        )
        .ctl
        .state()
        .volume,
        50
    );
    assert_eq!(
        harness_with(
            std_plan(),
            TvOptions {
                volume: 30.0,
                ..TvOptions::default()
            }
        )
        .ctl
        .state()
        .volume,
        30
    );
}

#[test]
fn togglemute_und_setmuted() {
    let mut h = harness();
    h.ctl.toggle_mute();
    assert!(h.ctl.state().muted);
    h.ctl.toggle_mute();
    assert!(!h.ctl.state().muted);
    h.ctl.set_muted(true);
    assert!(h.ctl.state().muted);
}

#[test]
fn stumm_taste_unter_autoplay_sperre_schaltet_nur_den_ton_ein() {
    let mut h = harness();
    h.start_ok();
    h.ctl.on_autoplay_muted();
    h.ctl.toggle_mute();
    assert!(!h.ctl.state().muted);
    h.ctl.toggle_mute();
    assert!(h.ctl.state().muted);
}

#[test]
fn mute_only_regler_bleibt_pegel_0_stumm_ab_5_ton_an_volume_fest_1() {
    let mut h = harness();
    h.ctl.set_volume_mode(VolumeMode::MuteOnly);
    assert!(h.ctl.volume_supported());
    h.out();
    h.ctl.set_volume(40.0);
    let outs = h.out();
    assert!(has(
        &outs,
        |o| matches!(o, TvOutput::SetVolume(v) if (*v - 1.0).abs() < 1e-9)
    ));
    assert!(has(&outs, |o| matches!(o, TvOutput::SetMuted(false))));
    h.ctl.set_volume(0.0);
    assert!(has(&h.out(), |o| matches!(o, TvOutput::SetMuted(true))));
    assert_eq!(h.ctl.volume_mode().as_str(), "mute-only");
}

#[test]
fn native_pegel_wird_durchgereicht() {
    let mut h = harness();
    h.out();
    h.ctl.set_volume(35.0);
    assert!(has(
        &h.out(),
        |o| matches!(o, TvOutput::SetVolume(v) if (*v - 0.35).abs() < 1e-9)
    ));
}

#[test]
fn start_sichert_nur_richtung_stumm_ab() {
    let mut h = harness_with(
        std_plan(),
        TvOptions {
            muted: true,
            ..TvOptions::default()
        },
    );
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl.on_play_result(req.token, &PlayResult::ok());
    assert!(has(&h.out(), |o| matches!(o, TvOutput::SetMuted(true))));
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl.on_play_result(req.token, &PlayResult::ok());
    assert!(
        !has(&h.out(), |o| matches!(o, TvOutput::SetMuted(_))),
        "nie ohne Geste entstummen"
    );
}

#[test]
fn entstummen_bei_pegel_0_stellt_letzten_pegel_wieder_her() {
    let mut h = harness();
    h.ctl.set_volume(70.0);
    h.advance(2000); // Pegel steht > 1 s
    h.ctl.set_volume(0.0);
    h.advance(100);
    h.ctl.toggle_mute(); // stumm angezeigt → entstummt
    assert_eq!(h.ctl.state().volume, 70);
    assert!(!h.ctl.state().muted);
}

#[test]
fn regler_schnell_bis_0_ziehen_nimmt_ausgangspegel() {
    let mut h = harness();
    h.ctl.set_volume(80.0);
    h.advance(2000);
    for v in [60.0, 40.0, 20.0, 0.0] {
        h.advance(50);
        h.ctl.set_volume(v);
    }
    h.ctl.set_muted(false);
    assert_eq!(h.ctl.state().volume, 80);
}

#[test]
fn entstummen_bei_pegel_0_ohne_fruehere_lautstaerke_fallback_50() {
    let mut h = harness_with(
        std_plan(),
        TvOptions {
            volume: 0.0,
            ..TvOptions::default()
        },
    );
    h.ctl.set_muted(false);
    assert_eq!(h.ctl.state().volume, 50);
}

// ---------------------------------------------------------------- Ton und Untertitel

fn tracks(active: i32) -> Tracks {
    let t = |i: i32, l: &str| Track {
        index: i,
        label: l.into(),
        lang: l.into(),
        is_default: i == 1,
        codec: "aac".into(),
        forced: false,
    };
    Tracks {
        audio: vec![t(1, "ger"), t(2, "eng")],
        subs: vec![],
        audio_index: Some(active),
        subtitle_index: -1,
    }
}

#[test]
fn tonwahl_wird_weitergereicht_aktive_spur_ist_noop() {
    let mut h = harness();
    h.start_ok();
    h.ctl.set_tracks(tracks(1));
    h.ctl.set_audio(1);
    assert!(h.out().is_empty());
    h.ctl.set_audio(2);
    let outs = h.out();
    let ticket = outs
        .iter()
        .find_map(|o| {
            if let TvOutput::SetAudio { ticket, index } = o {
                assert_eq!(*index, 2);
                Some(*ticket)
            } else {
                None
            }
        })
        .expect("SetAudio");
    h.ctl.on_audio_result(ticket, &PlayResult::ok());
    assert!(h.out().is_empty(), "Bild läuft, nichts zu übernehmen");
    h.ctl.set_subtitle(-1);
    assert!(has(&h.out(), |o| matches!(o, TvOutput::SetSubtitle(-1))));
}

#[test]
fn nicht_aufloesbare_tonspur_meldet_hinweis_ohne_fehlerkarte() {
    let mut h = harness();
    h.start_ok();
    h.ctl.set_tracks(tracks(1));
    h.ctl.set_audio(2);
    let ticket = h
        .out()
        .iter()
        .find_map(|o| {
            if let TvOutput::SetAudio { ticket, .. } = o {
                Some(*ticket)
            } else {
                None
            }
        })
        .unwrap();
    h.ctl
        .on_audio_result(ticket, &PlayResult::fail(FailReason::Video));
    let outs = h.out();
    assert!(has(
        &outs,
        |o| matches!(o, TvOutput::Message(m) if m == "Diese Tonspur ist nicht verfügbar")
    ));
    assert_eq!(h.ctl.state().phase, Phase::Playing);
    // Abbruch: kein Hinweis
    h.ctl
        .on_audio_result(ticket, &PlayResult::fail(FailReason::Aborted));
    assert!(h.out().is_empty());
}

#[test]
fn tonwechsel_im_ladefenster_uebernimmt_das_ergebnis() {
    let mut h = harness();
    h.ctl.start();
    h.out();
    h.ctl.set_audio(2);
    let ticket: AudioTicket = h
        .out()
        .iter()
        .find_map(|o| {
            if let TvOutput::SetAudio { ticket, .. } = o {
                Some(*ticket)
            } else {
                None
            }
        })
        .unwrap();
    h.ctl.on_audio_result(ticket, &PlayResult::ok());
    assert_eq!(h.ctl.state().phase, Phase::Playing);
}

#[test]
fn untertitelfehler_meldet_hinweis() {
    let mut h = harness();
    h.ctl.on_subtitle_failed();
    assert!(has(
        &h.out(),
        |o| matches!(o, TvOutput::Message(m) if m == "Untertitel nicht verfügbar")
    ));
}

#[test]
fn ohne_player_sind_spuren_leer() {
    let h = harness();
    let t = h.ctl.tracks();
    assert!(t.audio.is_empty() && t.subs.is_empty());
    assert_eq!(t.audio_index, None);
    assert_eq!(t.subtitle_index, -1);
}

// ---------------------------------------------------------------- restartTarget

#[test]
fn restart_target_film_folge_und_null_bei_filler_closedown() {
    let mut h = harness();
    assert!(h.ctl.restart_target().is_none(), "aus");
    h.start_ok();
    let t = h.ctl.restart_target().unwrap();
    assert_eq!(
        (t.item_id.as_str(), t.kind.as_str(), t.series_id.clone()),
        ("m1", "movie", None)
    );

    let mut p = std_plan();
    p.all(&[program("e1", T0 - MIN, T0 + 40 * MIN)]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    let t = h.ctl.restart_target().unwrap();
    assert_eq!(
        (t.kind.as_str(), t.series_id.as_deref()),
        ("episode", Some("s1"))
    );

    let mut h = harness_with(filler_plan(), TvOptions::default());
    h.ctl.start();
    assert!(h.ctl.restart_target().is_none());
    let mut p = std_plan();
    p.all(&[closedown(T0 - MIN, T0 + MIN, None)]);
    let mut h = harness_with(p, TvOptions::default());
    h.ctl.start();
    assert!(h.ctl.restart_target().is_none());
}

// ---------------------------------------------------------------- Takt, Tageswechsel, Aufräumen

#[test]
fn tick_jede_sekunde_syncto_alle_30_s_mit_offset() {
    let mut h = harness();
    h.start_ok();
    h.advance(1000);
    let o = h.out();
    assert_eq!(count(&o, |x| matches!(x, TvOutput::Tick(_))), 1);
    assert!(!has(&o, |x| matches!(x, TvOutput::SyncTo(_))));
    h.advance(29_000);
    let o = h.out();
    assert_eq!(count(&o, |x| matches!(x, TvOutput::Tick(_))), 29);
    let syncs: Vec<i64> = o
        .iter()
        .filter_map(|x| {
            if let TvOutput::SyncTo(m) = x {
                Some(*m)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(syncs, vec![30 * MIN + 30_000]);
}

#[test]
fn spaetes_poll_holt_alle_ticks_in_reihenfolge_nach() {
    let mut h = harness();
    h.start_ok();
    h.clock.advance(5000);
    h.ctl.poll();
    let ticks: Vec<i64> = h
        .out()
        .iter()
        .filter_map(|x| {
            if let TvOutput::Tick(t) = x {
                Some(*t)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        ticks,
        vec![T0 + 1000, T0 + 2000, T0 + 3000, T0 + 4000, T0 + 5000]
    );
}

#[test]
fn tageswechsel_um_05_uhr_plant_sieben_tage_und_meldet_day() {
    let mut h = harness();
    let d0 = lfx_tv_schedule::time::broadcast_day_of(T0);
    let end = lfx_tv_schedule::time::day_bounds(d0).end;
    h.clock.set(end - 2000);
    // Plan deckt den Wechsel nicht ab: Fehler → Fehlerkarte; für den Tick genügt das
    h.ctl.start();
    h.out();
    h.advance(3000);
    let o = h.out();
    assert!(has(&o, |x| matches!(x, TvOutput::Day(d) if *d == d0 + 1)));
    let days = o
        .iter()
        .find_map(|x| {
            if let TvOutput::PrepareDays(d) = x {
                Some(d.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(days, (d0 + 1..d0 + 8).collect::<Vec<_>>());
}

#[test]
fn stop_beendet_alles_neustart_moeglich() {
    let mut h = harness();
    h.start_ok();
    h.ctl.stop();
    let o = h.out();
    assert!(has(&o, |x| matches!(x, TvOutput::StopPlayer)));
    let s = h.ctl.state();
    assert!(!s.on);
    assert_eq!(s.phase, Phase::Off);
    assert!(s.entry.is_none() && s.card.is_none());
    h.advance(10 * MIN);
    assert!(h.out().is_empty(), "keine Zeitgeber mehr");
    assert_eq!(h.ctl.next_deadline(), None);
    h.ctl.start();
    assert_eq!(plays(&h.out()).len(), 1);
}

#[test]
fn stop_waehrend_laufendem_play_verwirft_das_ergebnis() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl.stop();
    h.out();
    h.ctl.on_play_result(req.token, &PlayResult::ok());
    assert_eq!(h.ctl.state().phase, Phase::Off);
    assert!(h.out().is_empty());
}

#[test]
fn stop_raeumt_den_netzwerk_retry_ab() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl
        .on_play_result(req.token, &PlayResult::fail(FailReason::Network));
    h.ctl.stop();
    h.out();
    h.advance(RETRY);
    assert!(plays(&h.out()).is_empty());
}

const RETRY: i64 = 10_000;

#[test]
fn dispose_ist_idempotent_und_danach_still() {
    let mut h = harness();
    h.start_ok();
    h.ctl.dispose();
    h.ctl.dispose();
    h.out();
    h.ctl.zap(1);
    h.ctl.start();
    h.advance(MIN);
    assert!(h.out().is_empty());
}

#[test]
fn stats_zeit_von_taste_bis_bild_letzte_zehn() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.advance(120);
    let mut res = PlayResult::ok();
    res.timing = Some(Timing {
        total_ms: 100.0,
        chain: "D".into(),
        ..Timing::default()
    });
    h.ctl.on_play_result(req.token, &res);
    let o = h.out();
    assert!(has(
        &o,
        |x| matches!(x, TvOutput::Zap(z) if z.key_ms == Some(120.0) && !z.failed)
    ));
    let s = h.ctl.stats();
    assert_eq!(s.last_ms, Some(120));
    assert_eq!(s.avg_ms, Some(120));
    assert_eq!(
        s.zaps[0].timing.as_ref().map(|t| t.chain.as_str()),
        Some("D")
    );
    for _ in 0..12 {
        h.ctl
            .tune(if h.ctl.state().channel == 2 { 3 } else { 2 }, true);
        let r = plays(&h.out()).remove(0);
        h.advance(50);
        h.ctl.on_play_result(r.token, &PlayResult::ok());
        h.out();
    }
    assert_eq!(h.ctl.stats().zaps.len(), 10);
}

#[test]
fn zap_ereignis_bei_fehler_mit_code() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl.on_play_result(
        req.token,
        &PlayResult::fail_code(FailReason::Video, "V-CODEC"),
    );
    let o = h.out();
    assert!(has(
        &o,
        |x| matches!(x, TvOutput::Zap(z) if z.failed && z.code.as_deref() == Some("V-CODEC") && z.reason.as_deref() == Some("video"))
    ));
}

// ---------------------------------------------------------------- Nachbarn und Ziffern

#[test]
fn nachbarn_nach_1_s_stabil_vorwaermen_umlauf_18_und_2() {
    let mut h = harness();
    h.start_ok();
    h.advance(999);
    assert!(!has(&h.out(), |o| matches!(o, TvOutput::Prewarm { .. })));
    h.advance(1);
    let o = h.out();
    let warm: Vec<(String, i64)> = o
        .iter()
        .filter_map(|x| {
            if let TvOutput::Prewarm { item_id, offset_ms } = x {
                Some((item_id.clone(), *offset_ms))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(warm.len(), 2);
    assert_eq!(warm[0], ("m1".to_string(), 30 * MIN + 1000));
    // erneut nach 35 s
    h.advance(35_000);
    assert_eq!(
        count(&h.out(), |x| matches!(x, TvOutput::Prewarm { .. })),
        2
    );
}

#[test]
fn ohne_vorwaermen_prefetch_je_sendung_einmal() {
    let mut h = harness();
    h.ctl.set_zap_flags(ZapFlags {
        zap_warm: false,
        zap_prefetch: true,
    });
    h.start_ok();
    h.advance(1000);
    let o = h.out();
    assert_eq!(count(&o, |x| matches!(x, TvOutput::Prefetch { .. })), 2);
    assert!(!has(&o, |x| matches!(x, TvOutput::Prewarm { .. })));
    h.advance(MIN);
    assert_eq!(
        count(&h.out(), |x| matches!(x, TvOutput::Prefetch { .. })),
        0
    );
}

#[test]
fn datensparmodus_kein_prefetch_kein_prewarm_und_release() {
    let mut h = harness();
    h.ctl.set_zap_flags(ZapFlags {
        zap_warm: false,
        zap_prefetch: false,
    });
    h.start_ok();
    h.advance(MIN);
    let o = h.out();
    assert!(!has(&o, |x| matches!(
        x,
        TvOutput::Prefetch { .. } | TvOutput::Prewarm { .. }
    )));
    h.ctl.release_warm();
    assert!(has(&h.out(), |x| matches!(x, TvOutput::ReleaseWarm)));
}

#[test]
fn fehlerkarte_video_waermt_nach_2_s_nachbarn() {
    let mut h = harness();
    h.ctl.start();
    let req = plays(&h.out()).remove(0);
    h.ctl
        .on_play_result(req.token, &PlayResult::fail(FailReason::Video));
    h.out();
    h.advance(1999);
    assert!(!has(&h.out(), |x| matches!(x, TvOutput::Prewarm { .. })));
    h.advance(1);
    assert_eq!(
        count(&h.out(), |x| matches!(x, TvOutput::Prewarm { .. })),
        2
    );
}

#[test]
fn ziffern_schalten_sofort_und_zweistellig() {
    let mut h = harness();
    h.start_ok();
    h.ctl.push_digit('7');
    let o = h.out();
    assert_eq!(h.ctl.state().channel, 7);
    assert_eq!(plays(&o).len(), 1, "immediate");
    h.ctl.push_digit('1');
    assert!(has(
        &h.out(),
        |x| matches!(x, TvOutput::Digits(Some(t)) if t == "1_")
    ));
    h.ctl.push_digit('2');
    let o = h.out();
    assert!(has(&o, |x| matches!(x, TvOutput::Digits(None))));
    assert_eq!(h.ctl.state().channel, 12);
    h.ctl.push_digit('1');
    h.ctl.push_digit('9');
    assert!(has(
        &h.out(),
        |x| matches!(x, TvOutput::Message(m) if m == "Dieser Sender existiert nicht")
    ));
    assert_eq!(h.ctl.state().channel, 12);
}

#[test]
fn ziffer_eins_schaltet_nach_zeitablauf_auf_sender_1() {
    let mut h = harness();
    h.start_ok();
    h.ctl.tune(5, true);
    h.out();
    h.ctl.push_digit('1');
    h.out();
    h.advance(1499);
    assert_eq!(h.ctl.state().channel, 5);
    h.advance(1);
    assert_eq!(h.ctl.state().channel, 1);
}

// ---------------------------------------------------------------- Determinismus

fn scenario() -> Vec<TvOutput> {
    let mut h = harness_with(filler_plan(), TvOptions::default());
    let mut all = Vec::new();
    h.ctl.start();
    all.extend(h.out());
    for step in 0..40 {
        h.advance(7_000 + step * 13);
        if step == 5 {
            h.ctl.zap(1);
        }
        if step == 9 {
            h.ctl.push_digit('1');
            h.ctl.push_digit('1');
        }
        if step == 20 {
            h.ctl.set_volume(35.0);
            h.ctl.toggle_mute();
        }
        all.extend(h.out());
    }
    all
}

#[test]
fn gleiche_eingaben_gleiche_ausgaben() {
    assert_eq!(scenario(), scenario());
}

#[test]
fn planer_attrappe_wird_nicht_unnoetig_befragt() {
    let mut p = FakePlan::new();
    p.set(1, vec![program("m1", T0 - MIN, T0 + MIN)]);
    let e: PlanEntry = p.now_on(1, T0).unwrap().entry;
    assert_eq!(e.item_id.as_deref(), Some("m1"));
    assert_eq!(p.now_on_calls, 1);
    let _ = Token(1);
}
