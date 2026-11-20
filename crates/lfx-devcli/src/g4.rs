//! `g4`: Live-Prüflauf gegen einen Server (Gate G4).
//!
//! Je Check eine Zeile `OK`/`FAIL`/`SKIP` mit Dauer. URLs erscheinen nur über [`lfx_api::redact_url`],
//! nie das Token und nie der Server-Host. Der Fortschrittstest stellt Position und „gesehen“ des benutzten
//! Titels am Ende wieder her und prüft das.

use crate::branding::{Branding, parse_branding};
use crate::commands::{Ctx, open_session};
use anyhow::Result;
use lfx_api::{
    Client, ItemKind, ItemsQuery, PlaybackInfoRequest, PlaybackReport, PlaybackStop, SortBy,
    TICKS_PER_SECOND, redact_url,
};
use std::io::Write;
use std::time::{Duration, Instant};

/// Gemeldete Position des Fortschrittstests.
const TEST_POSITION_SECS: i64 = 30;
/// Zulässige Abweichung beim Zurücklesen.
const TOLERANCE_TICKS: i64 = 5 * TICKS_PER_SECOND;
/// Zeitlimit für die Aufrufe außerhalb von Jellyfin (Branding, Charts-Worker).
const EXTERN_TIMEOUT: Duration = Duration::from_secs(10);

/// Zusammenfassung des Laufs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Bestandene Checks.
    pub ok: u32,
    /// Fehlgeschlagene Checks.
    pub fail: u32,
    /// Übersprungene Checks.
    pub skipped: u32,
}

impl Summary {
    /// Kein Fehlschlag?
    pub fn passed(&self) -> bool {
        self.fail == 0
    }
}

enum Outcome {
    Ok(String),
    Fail(String),
    Skip(String),
}

struct Recorder<'a> {
    out: &'a mut dyn Write,
    summary: Summary,
}

impl Recorder<'_> {
    fn record(&mut self, name: &str, started: Instant, outcome: Outcome) {
        let ms = started.elapsed().as_millis();
        let (tag, detail) = match outcome {
            Outcome::Ok(d) => {
                self.summary.ok += 1;
                ("OK  ", d)
            }
            Outcome::Fail(d) => {
                self.summary.fail += 1;
                ("FAIL", d)
            }
            Outcome::Skip(d) => {
                self.summary.skipped += 1;
                ("SKIP", d)
            }
        };
        let _ = writeln!(self.out, "[{tag}] {name:<22} {ms:>5} ms  {detail}");
    }

    fn result(&mut self, name: &str, started: Instant, r: Result<String, String>) {
        self.record(
            name,
            started,
            match r {
                Ok(d) => Outcome::Ok(d),
                Err(d) => Outcome::Fail(d),
            },
        );
    }
}

fn api_err(e: lfx_api::ApiError) -> String {
    e.to_string()
}

/// Führt alle Checks aus. `Err` nur, wenn gar keine Sitzung vorhanden ist.
pub async fn run(ctx: &Ctx, out: &mut dyn Write) -> Result<Summary> {
    let session = open_session(ctx)?;
    let client = session.client;
    let total = Instant::now();
    let mut rec = Recorder {
        out,
        summary: Summary::default(),
    };
    writeln!(
        rec.out,
        "LENZFLIX g4 - Konto {} auf {}",
        session.user_name, session.server_name
    )?;

    // 1-6: Lesende Grundabfragen.
    let t = Instant::now();
    let r = client
        .public_info()
        .await
        .map(|i| {
            format!(
                "{} {} (GET <server>/System/Info/Public)",
                i.product_name, i.version
            )
        })
        .map_err(api_err);
    rec.result("system_info_public", t, r);

    let t = Instant::now();
    let r = client
        .me()
        .await
        .map(|u| format!("Benutzer {} (GET <server>/Users/Me)", u.name))
        .map_err(api_err);
    rec.result("users_me", t, r);

    let t = Instant::now();
    let r = client
        .views()
        .await
        .and_then(|v| {
            if v.items.is_empty() {
                Err(lfx_api::ApiError::Decode("keine Bibliotheken".into()))
            } else {
                Ok(format!("{} Bibliotheken", v.items.len()))
            }
        })
        .map_err(api_err);
    rec.result("user_views", t, r);

    let t = Instant::now();
    let q = ItemsQuery::new()
        .include_types(&[ItemKind::Movie])
        .recursive(true)
        .sort_by(&[SortBy::SortName])
        .limit(5);
    let r = client.items(&q).await.map_err(api_err).and_then(|res| {
        if res.items.is_empty() {
            Err("keine Filme".to_owned())
        } else {
            Ok(format!(
                "{} von {} Filmen",
                res.items.len(),
                res.total_record_count
            ))
        }
    });
    rec.result("items_movies", t, r);

    let t = Instant::now();
    let r = client
        .resume(5, &[], &[])
        .await
        .map(|v| format!("{} Einträge", v.items.len()))
        .map_err(api_err);
    rec.result("resume", t, r);

    let t = Instant::now();
    let r = client
        .next_up(None, 5, &[])
        .await
        .map(|v| format!("{} Einträge", v.items.len()))
        .map_err(api_err);
    rec.result("next_up", t, r);

    // 7-9: Wiedergabe-Metadaten und Fortschritt.
    playback_checks(&client, &mut rec).await;

    // 10-12: Branding und Charts-Worker.
    extern_checks(&client, &mut rec).await;

    let s = rec.summary;
    writeln!(
        rec.out,
        "Ergebnis: {} OK, {} FAIL, {} SKIP in {} ms",
        s.ok,
        s.fail,
        s.skipped,
        total.elapsed().as_millis()
    )?;
    Ok(s)
}

struct Candidate {
    id: String,
    name: String,
    source_id: String,
    played: bool,
    position: i64,
}

async fn pick_movie(client: &Client) -> Result<Candidate, String> {
    let q = ItemsQuery::new()
        .include_types(&[ItemKind::Movie])
        .recursive(true)
        .sort_by(&[SortBy::SortName])
        .fields(&["MediaSources"])
        .limit(100);
    let res = client.items(&q).await.map_err(api_err)?;
    let mut h264: Vec<Candidate> = Vec::new();
    for item in res.items {
        let Some(source) = item.media_sources.iter().find(|s| {
            s.media_streams
                .iter()
                .any(|m| m.kind == "Video" && m.codec.as_deref() == Some("h264"))
        }) else {
            continue;
        };
        let ud = item.user_data.clone().unwrap_or_default();
        h264.push(Candidate {
            id: item.id.clone(),
            name: item.name.clone(),
            source_id: source.id.clone(),
            played: ud.played,
            position: ud.playback_position_ticks,
        });
    }
    // Möglichst ein unberührter Titel (ungesehen, ohne Position): weniger Spuren.
    let idx = h264
        .iter()
        .position(|c| !c.played && c.position == 0)
        .unwrap_or(0);
    if h264.is_empty() {
        return Err("kein Film mit h264-Videostream unter den ersten 100".to_owned());
    }
    Ok(h264.swap_remove(idx))
}

async fn user_data(client: &Client, id: &str) -> Result<(i64, bool), String> {
    let item = client.item(id, &[]).await.map_err(api_err)?;
    let ud = item.user_data.unwrap_or_default();
    Ok((ud.playback_position_ticks, ud.played))
}

async fn playback_checks(client: &Client, rec: &mut Recorder<'_>) {
    let t = Instant::now();
    let cand = match pick_movie(client).await {
        Ok(c) => {
            rec.record(
                "pick_movie",
                t,
                Outcome::Ok("Film mit h264 gefunden".into()),
            );
            c
        }
        Err(e) => {
            rec.record("pick_movie", t, Outcome::Fail(e));
            rec.record(
                "playback_info",
                Instant::now(),
                Outcome::Skip("kein Titel".into()),
            );
            rec.record(
                "progress_roundtrip",
                Instant::now(),
                Outcome::Skip("kein Titel".into()),
            );
            rec.record(
                "restore_userdata",
                Instant::now(),
                Outcome::Skip("kein Titel".into()),
            );
            return;
        }
    };
    let _ = &cand.name; // Titelname wird bewusst nicht ausgegeben.

    // PlaybackInfo mit Direktwiedergabe-Profil.
    let t = Instant::now();
    let user_id = client.user_id().unwrap_or_default();
    let info = client
        .playback_info(&cand.id, &PlaybackInfoRequest::direct_play(user_id))
        .await;
    let session_id = match info {
        Ok(resp) => {
            let source = resp
                .media_sources
                .iter()
                .find(|s| s.id == cand.source_id)
                .or(resp.media_sources.first());
            match (source, resp.play_session_id) {
                (Some(s), Some(sid)) if s.supports_direct_play => {
                    rec.record(
                        "playback_info",
                        t,
                        Outcome::Ok(format!(
                            "SupportsDirectPlay=true, Container {}",
                            s.container.as_deref().unwrap_or("?")
                        )),
                    );
                    Some(sid)
                }
                (Some(_), _) => {
                    rec.record(
                        "playback_info",
                        t,
                        Outcome::Fail(format!(
                            "SupportsDirectPlay=false oder keine Sitzungs-Id (ErrorCode: {})",
                            resp.error_code.as_deref().unwrap_or("-")
                        )),
                    );
                    None
                }
                (None, _) => {
                    rec.record(
                        "playback_info",
                        t,
                        Outcome::Fail("keine Medienquelle".into()),
                    );
                    None
                }
            }
        }
        Err(e) => {
            rec.record("playback_info", t, Outcome::Fail(api_err(e)));
            None
        }
    };
    let Some(session_id) = session_id else {
        rec.record(
            "progress_roundtrip",
            Instant::now(),
            Outcome::Skip("ohne PlaybackInfo".into()),
        );
        rec.record(
            "restore_userdata",
            Instant::now(),
            Outcome::Skip("nichts verändert".into()),
        );
        return;
    };

    // Zustand vor dem Test merken.
    let t = Instant::now();
    let (before_pos, before_played) = match user_data(client, &cand.id).await {
        Ok(v) => v,
        Err(e) => {
            rec.record(
                "progress_roundtrip",
                t,
                Outcome::Fail(format!("UserData vorher: {e}")),
            );
            rec.record(
                "restore_userdata",
                Instant::now(),
                Outcome::Skip("nichts verändert".into()),
            );
            return;
        }
    };

    // Playing -> Progress -> Stopped (30 s), zurücklesen.
    let t = Instant::now();
    let report = |pos: i64| PlaybackReport::direct(&cand.id, &cand.source_id, &session_id, pos);
    let test_ticks = TEST_POSITION_SECS * TICKS_PER_SECOND;
    let round: Result<String, String> = async {
        client
            .report_playing(&report(0))
            .await
            .map_err(|e| format!("Playing: {}", api_err(e)))?;
        client
            .report_progress(&report(test_ticks))
            .await
            .map_err(|e| format!("Progress: {}", api_err(e)))?;
        let (after_progress, _) = user_data(client, &cand.id).await?;
        client
            .report_stopped(&PlaybackStop {
                item_id: cand.id.clone(),
                media_source_id: cand.source_id.clone(),
                play_session_id: session_id.clone(),
                position_ticks: test_ticks,
                failed: false,
            })
            .await
            .map_err(|e| format!("Stopped: {}", api_err(e)))?;
        let (after_stop, _) = user_data(client, &cand.id).await?;
        let near = |v: i64| (v - test_ticks).abs() <= TOLERANCE_TICKS;
        let secs = |v: i64| v / TICKS_PER_SECOND;
        if near(after_progress) || near(after_stop) {
            Ok(format!(
                "Position nach Progress {} s, nach Stopped {} s (Soll {} s)",
                secs(after_progress),
                secs(after_stop),
                TEST_POSITION_SECS
            ))
        } else {
            Err(format!(
                "Position nach Progress {} s, nach Stopped {} s, erwartet {} s",
                secs(after_progress),
                secs(after_stop),
                TEST_POSITION_SECS
            ))
        }
    }
    .await;
    rec.result("progress_roundtrip", t, round);

    // Alten Zustand wiederherstellen (läuft immer, auch nach Fehlern oben).
    let t = Instant::now();
    let restored: Result<String, String> = async {
        client
            .report_stopped(&PlaybackStop {
                item_id: cand.id.clone(),
                media_source_id: cand.source_id.clone(),
                play_session_id: session_id.clone(),
                position_ticks: before_pos,
                failed: false,
            })
            .await
            .map_err(|e| format!("Stopped (zurück): {}", api_err(e)))?;
        let (_, played_now) = user_data(client, &cand.id).await?;
        if played_now != before_played {
            client
                .set_played(&cand.id, before_played)
                .await
                .map_err(|e| format!("Gesehen-Status: {}", api_err(e)))?;
        }
        let (pos_now, played_now) = user_data(client, &cand.id).await?;
        let pos_ok = (pos_now - before_pos).abs() <= TICKS_PER_SECOND;
        if pos_ok && played_now == before_played {
            Ok(format!(
                "Position {} s, gesehen={} wie vorher",
                pos_now / TICKS_PER_SECOND,
                played_now
            ))
        } else {
            Err(format!(
                "nicht wiederhergestellt: Position {} s statt {} s, gesehen={} statt {}",
                pos_now / TICKS_PER_SECOND,
                before_pos / TICKS_PER_SECOND,
                played_now,
                before_played
            ))
        }
    }
    .await;
    rec.result("restore_userdata", t, restored);
}

async fn fetch_branding(http: &reqwest::Client, base: &str) -> Result<(Branding, usize), String> {
    for path in ["/Branding/Css", "/Branding/Css.css"] {
        let url = format!("{base}{path}");
        let resp = match http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return Err(e.without_url().to_string()),
        };
        if !resp.status().is_success() {
            continue;
        }
        let text = resp.text().await.map_err(|e| e.without_url().to_string())?;
        if text.trim().is_empty() {
            continue;
        }
        return Ok((parse_branding(&text), text.len()));
    }
    Err("kein Branding-CSS (beide Pfade leer oder nicht vorhanden)".to_owned())
}

async fn extern_checks(client: &Client, rec: &mut Recorder<'_>) {
    let http = match reqwest::Client::builder()
        .timeout(EXTERN_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none()) // das Token geht nie an einen Umleitungsziel-Host
        .build()
    {
        Ok(h) => h,
        Err(e) => {
            let msg = e.without_url().to_string();
            rec.record("branding_css", Instant::now(), Outcome::Fail(msg));
            rec.record(
                "charts",
                Instant::now(),
                Outcome::Skip("ohne HTTP-Client".into()),
            );
            rec.record(
                "me_stats",
                Instant::now(),
                Outcome::Skip("ohne HTTP-Client".into()),
            );
            return;
        }
    };

    let t = Instant::now();
    let branding = match fetch_branding(&http, client.base_url()).await {
        Ok((b, bytes)) => {
            rec.record(
                "branding_css",
                t,
                Outcome::Ok(format!(
                    "{} Bytes (GET <server>/Branding/Css); charts-url {}, stats-url {}, {} Senderplätze",
                    bytes,
                    yes_no(b.charts_url.is_some()),
                    yes_no(b.stats_url.is_some()),
                    b.tv_channels.len()
                )),
            );
            b
        }
        Err(e) => {
            rec.record("branding_css", t, Outcome::Fail(e));
            Branding::default()
        }
    };

    let Some(charts) = branding.charts_url.clone() else {
        let why = "keine --lfx-charts-url im Branding-CSS";
        rec.record("charts", Instant::now(), Outcome::Skip(why.into()));
        rec.record("me_stats", Instant::now(), Outcome::Skip(why.into()));
        return;
    };
    let token = client
        .session()
        .map(|s| s.token)
        .map(|t| t.expose().to_owned())
        .unwrap_or_default();

    let t = Instant::now();
    let url = format!("{charts}/charts");
    let r = worker_get(&http, &url, &token)
        .await
        .and_then(|(status, json)| {
            let list = |k: &str| json.get(k).and_then(|v| v.as_array()).map(Vec::len);
            match (
                json.get("version").and_then(|v| v.as_i64()),
                list("movies"),
                list("series"),
                list("all"),
            ) {
                (Some(1), Some(m), Some(s), Some(a)) => Ok(format!(
                    "HTTP {status}, {m} Filme, {s} Serien, {a} gesamt (GET {})",
                    redact_url(&url)
                )),
                _ => Err(format!(
                    "HTTP {status}, ungültige Antwort (GET {})",
                    redact_url(&url)
                )),
            }
        });
    rec.result("charts", t, r);

    let t = Instant::now();
    let url = format!("{charts}/me/stats");
    let r = worker_get(&http, &url, &token)
        .await
        .map(|(status, _)| format!("HTTP {status} (GET {})", redact_url(&url)));
    rec.result("me_stats", t, r);
}

fn yes_no(b: bool) -> &'static str {
    if b { "ja" } else { "nein" }
}

/// GET mit `X-Emby-Token`; `Ok` nur bei 2xx und lesbarem JSON.
async fn worker_get(
    http: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<(u16, serde_json::Value), String> {
    let resp = http
        .get(url)
        .header("X-Emby-Token", token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("{} (GET {})", e.without_url(), redact_url(url)))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        return Err(format!("HTTP {status} (GET {})", redact_url(url)));
    }
    let json = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("HTTP {status}, Antwort nicht lesbar: {}", e.without_url()))?;
    Ok((status, json))
}
