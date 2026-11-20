//! Fortschrittsmeldungen an den Server (`Playing`, `Progress`, `Stopped`).
//!
//! * [`ReportPlanner`]: reine Zeitlogik (wann wird gemeldet), mit simulierter Zeit testbar.
//! * [`ReportSink`]: Ziel der Meldungen. Standard [`ApiSink`] (direkt über `lfx-api`); später
//!   kann der Offline-Speicher (Outbox) das Trait implementieren.
//! * [`ProgressReporter`]: Aufgabe, die Player-Ereignisse in Meldungen übersetzt.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use lfx_api::{Client, PlaybackReport, PlaybackStop, TICKS_PER_SECOND};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::player::Player;
use crate::request::{PlayRequest, StreamKind};
use crate::state::{EndKind, PlayerEvent, PlayerState, TrackKind};

/// Eine Meldung an den Server.
#[derive(Debug, Clone)]
pub enum Report {
    Playing(PlaybackReport),
    Progress(PlaybackReport),
    Stopped(PlaybackStop),
}

/// Fehler beim Zustellen (nur Text; nie mit Token).
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ReportError(pub String);

impl From<lfx_api::ApiError> for ReportError {
    fn from(e: lfx_api::ApiError) -> Self {
        Self(e.to_string())
    }
}

/// Zielort der Meldungen.
pub trait ReportSink: Send + Sync + 'static {
    fn send(
        &self,
        report: Report,
    ) -> Pin<Box<dyn Future<Output = Result<(), ReportError>> + Send + '_>>;
}

/// Meldet direkt an den Server.
pub struct ApiSink {
    client: Client,
}

impl ApiSink {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl ReportSink for ApiSink {
    fn send(
        &self,
        report: Report,
    ) -> Pin<Box<dyn Future<Output = Result<(), ReportError>> + Send + '_>> {
        Box::pin(async move {
            match report {
                Report::Playing(r) => self.client.report_playing(&r).await,
                Report::Progress(r) => self.client.report_progress(&r).await,
                Report::Stopped(r) => self.client.report_stopped(&r).await,
            }
            .map_err(ReportError::from)
        })
    }
}

/// Auslöser für die Planung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Loaded,
    Tick { paused: bool },
    Pause,
    Seeked,
    TrackChanged,
    Eof,
    Ended { failed: bool },
}

/// Was gemeldet werden soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Planned {
    Playing,
    Progress,
    Stopped { failed: bool },
}

/// Entscheidet, wann welche Meldung fällig ist. Zeit kommt von außen (`now` als Dauer seit
/// einem beliebigen Nullpunkt), damit Tests sie simulieren können.
#[derive(Debug)]
pub struct ReportPlanner {
    interval: Duration,
    last: Option<Duration>,
    started: bool,
    stopped: bool,
}

impl ReportPlanner {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
            started: false,
            stopped: false,
        }
    }

    /// Wurde schon `Stopped` geplant (oder nie gestartet und beendet)?
    pub fn is_finished(&self) -> bool {
        self.stopped
    }

    pub fn on(&mut self, now: Duration, trigger: Trigger) -> Option<Planned> {
        if self.stopped {
            return None;
        }
        match trigger {
            Trigger::Loaded => {
                if self.started {
                    return None;
                }
                self.started = true;
                self.last = Some(now);
                Some(Planned::Playing)
            }
            Trigger::Ended { failed } => {
                self.stopped = true;
                // Ohne `Playing` gibt es serverseitig nichts zu beenden.
                self.started.then_some(Planned::Stopped { failed })
            }
            Trigger::Eof => {
                self.stopped = true;
                self.started.then_some(Planned::Stopped { failed: false })
            }
            _ if !self.started => None,
            Trigger::Tick { paused } => {
                let due = self
                    .last
                    .is_none_or(|l| now.saturating_sub(l) >= self.interval);
                if paused || !due {
                    return None;
                }
                self.last = Some(now);
                Some(Planned::Progress)
            }
            Trigger::Pause | Trigger::Seeked | Trigger::TrackChanged => {
                self.last = Some(now);
                Some(Planned::Progress)
            }
        }
    }
}

/// Meldungsbau aus Zustand und Auftrag.
pub fn build_report(request: &PlayRequest, state: &PlayerState, planned: Planned) -> Report {
    let ticks = |secs: f64| (secs.max(0.0) * TICKS_PER_SECOND as f64) as i64;
    let method = request.play_method;
    let item = request.item_id.clone();
    let source = request.media_source_id.clone();
    let session = request.play_session_id.clone().unwrap_or_default();
    match planned {
        Planned::Stopped { failed } => Report::Stopped(PlaybackStop {
            item_id: item,
            media_source_id: source,
            play_session_id: session,
            position_ticks: ticks(state.position),
            failed,
        }),
        Planned::Playing | Planned::Progress => {
            let position = if planned == Planned::Playing && state.position <= 0.0 {
                request.start_seconds
            } else {
                state.position
            };
            let audio = state
                .selected(TrackKind::Audio)
                .and_then(|t| request.jellyfin_index(StreamKind::Audio, t.id))
                .or(request.audio_stream_index);
            let subtitle = match state.selected(TrackKind::Subtitle) {
                Some(t) => request.jellyfin_index(StreamKind::Subtitle, t.id),
                None if request.track_mapping => Some(-1),
                None => None,
            };
            let mut r = PlaybackReport::direct(item, source, session, ticks(position));
            r.is_paused = state.paused;
            r.is_muted = state.muted;
            r.volume_level = state.volume.clamp(0.0, 100.0).round() as u8;
            r.play_method = method;
            r.audio_stream_index = audio;
            r.subtitle_stream_index = subtitle;
            if planned == Planned::Playing {
                Report::Playing(r)
            } else {
                Report::Progress(r)
            }
        }
    }
}

/// Einstellungen des Melders.
#[derive(Debug, Clone)]
pub struct ReporterConfig {
    /// `false` (TV-Modus): gar nichts melden, keine Spuren in „Weiterschauen“.
    pub report_progress: bool,
    /// Abstand der Fortschrittsmeldungen (Standard 10 s).
    pub interval: Duration,
}

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
            report_progress: true,
            interval: Duration::from_secs(10),
        }
    }
}

/// Laufender Melder. Endet von selbst nach `Stopped`.
pub struct ProgressReporter {
    task: Option<JoinHandle<()>>,
}

impl ProgressReporter {
    /// Startet den Melder. Muss in einer Tokio-Laufzeit und **vor** [`Player::open`] des
    /// zugehörigen Auftrags aufgerufen werden (er bindet sich an das passende `Opened`).
    pub fn spawn(
        player: &Player,
        request: PlayRequest,
        sink: Arc<dyn ReportSink>,
        config: ReporterConfig,
    ) -> Self {
        if !config.report_progress {
            return Self { task: None };
        }
        let mut events = player.subscribe_events();
        let player = player.clone();
        let task = tokio::spawn(async move {
            let t0 = Instant::now();
            let mut planner = ReportPlanner::new(config.interval);
            let mut tick = tokio::time::interval(config.interval.min(Duration::from_secs(1)));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut generation: Option<u64> = None;

            let deliver = |planned: Planned| {
                let report = build_report(&request, &player.state(), planned);
                let sink = sink.clone();
                async move {
                    if let Err(e) = sink.send(report).await {
                        tracing::warn!("Fortschrittsmeldung fehlgeschlagen: {e}");
                    }
                }
            };

            loop {
                let trigger = tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => map_event(&ev, &request, &mut generation),
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => {
                            // Player weg: laufende Wiedergabe abschließen, sonst beenden.
                            if generation.is_none() {
                                break;
                            }
                            Some(Trigger::Ended { failed: false })
                        }
                    },
                    _ = tick.tick() => {
                        if generation.is_some() {
                            let paused = player.state().paused;
                            Some(Trigger::Tick { paused })
                        } else {
                            None
                        }
                    }
                };
                let Some(trigger) = trigger else { continue };
                if let Some(planned) = planner.on(t0.elapsed(), trigger) {
                    deliver(planned).await;
                }
                if planner.is_finished() {
                    break;
                }
            }
        });
        Self { task: Some(task) }
    }

    /// Wartet, bis der Melder fertig ist (nach `Stopped`).
    pub async fn join(self) {
        if let Some(t) = self.task {
            let _ = t.await;
        }
    }

    /// Bricht ab, ohne `Stopped` zu melden.
    pub fn abort(&self) {
        if let Some(t) = &self.task {
            t.abort();
        }
    }
}

fn map_event(ev: &PlayerEvent, req: &PlayRequest, generation: &mut Option<u64>) -> Option<Trigger> {
    if let PlayerEvent::Opened {
        generation: g,
        item_id,
        media_source_id,
    } = ev
    {
        if generation.is_none()
            && *item_id == req.item_id
            && *media_source_id == req.media_source_id
        {
            *generation = Some(*g);
        }
        return None;
    }
    if Some(ev.generation()) != *generation {
        return None;
    }
    Some(match ev {
        PlayerEvent::FileLoaded { .. } => Trigger::Loaded,
        PlayerEvent::PauseChanged { .. } => Trigger::Pause,
        PlayerEvent::Seeked { .. } => Trigger::Seeked,
        PlayerEvent::TrackChanged { .. } => Trigger::TrackChanged,
        PlayerEvent::Eof { .. } => Trigger::Eof,
        PlayerEvent::Ended { kind, .. } => Trigger::Ended {
            failed: matches!(kind, EndKind::Error(_)),
        },
        PlayerEvent::Opened { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn plans_playing_progress_and_stopped_with_simulated_time() {
        let mut p = ReportPlanner::new(s(10));
        assert_eq!(p.on(s(0), Trigger::Tick { paused: false }), None);
        assert_eq!(p.on(s(1), Trigger::Loaded), Some(Planned::Playing));
        assert_eq!(p.on(s(1), Trigger::Loaded), None);
        assert_eq!(p.on(s(5), Trigger::Tick { paused: false }), None);
        assert_eq!(
            p.on(s(11), Trigger::Tick { paused: false }),
            Some(Planned::Progress)
        );
        assert_eq!(p.on(s(15), Trigger::Tick { paused: false }), None);
        assert_eq!(
            p.on(s(21), Trigger::Tick { paused: false }),
            Some(Planned::Progress)
        );
        // Pause meldet sofort und setzt den Takt zurück; in der Pause kein Takt-Fortschritt.
        assert_eq!(p.on(s(23), Trigger::Pause), Some(Planned::Progress));
        assert_eq!(p.on(s(60), Trigger::Tick { paused: true }), None);
        assert_eq!(p.on(s(61), Trigger::Seeked), Some(Planned::Progress));
        assert_eq!(p.on(s(62), Trigger::TrackChanged), Some(Planned::Progress));
        assert_eq!(
            p.on(s(63), Trigger::Ended { failed: false }),
            Some(Planned::Stopped { failed: false })
        );
        assert!(p.is_finished());
        assert_eq!(p.on(s(64), Trigger::Pause), None);
        assert_eq!(p.on(s(65), Trigger::Ended { failed: false }), None);
    }

    #[test]
    fn failed_load_without_playing_reports_nothing() {
        let mut p = ReportPlanner::new(s(10));
        assert_eq!(p.on(s(1), Trigger::Ended { failed: true }), None);
        assert!(p.is_finished());
    }

    #[test]
    fn eof_stops_once() {
        let mut p = ReportPlanner::new(s(10));
        p.on(s(0), Trigger::Loaded);
        assert_eq!(
            p.on(s(100), Trigger::Eof),
            Some(Planned::Stopped { failed: false })
        );
        assert_eq!(p.on(s(101), Trigger::Ended { failed: false }), None);
    }

    #[test]
    fn report_uses_ticks_and_mapped_indices() {
        use crate::request::StreamMap;
        use crate::state::Track;
        let mut req = PlayRequest::new("item", "src", "http://h/x");
        req.play_session_id = Some("sess".into());
        req.track_mapping = true;
        req.streams = vec![
            StreamMap {
                index: 1,
                kind: StreamKind::Audio,
                external: false,
            },
            StreamMap {
                index: 2,
                kind: StreamKind::Audio,
                external: false,
            },
        ];
        let track = |id, kind, selected| Track {
            id,
            kind,
            lang: None,
            title: None,
            codec: None,
            default: false,
            forced: false,
            external: false,
            selected,
        };
        let st = PlayerState {
            position: 12.5,
            paused: true,
            volume: 150.0,
            tracks: vec![track(2, TrackKind::Audio, true)],
            ..Default::default()
        };
        let Report::Progress(r) = build_report(&req, &st, Planned::Progress) else {
            panic!("kein Progress");
        };
        assert_eq!(r.position_ticks, 125_000_000);
        assert!(r.is_paused);
        assert_eq!(r.volume_level, 100);
        assert_eq!(r.audio_stream_index, Some(2));
        assert_eq!(r.subtitle_stream_index, Some(-1));
        assert_eq!(r.play_session_id, "sess");
        let Report::Stopped(stop) = build_report(&req, &st, Planned::Stopped { failed: true })
        else {
            panic!("kein Stopped");
        };
        assert!(stop.failed);
        assert_eq!(stop.position_ticks, 125_000_000);
    }
}
