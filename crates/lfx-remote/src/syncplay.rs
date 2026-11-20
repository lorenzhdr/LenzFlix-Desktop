//! SyncPlay-Kern ohne I/O und ohne UI (Sans-IO): Gruppenzustand, Ausführungszeit der Befehle,
//! Drift-Korrektur sowie Ready/Buffering-Meldungen.
//!
//! Alle Zeiten kommen als Parameter herein (`now_local`, Mikrosekunden seit 1970 auf der lokalen Uhr). Der Kern
//! rechnet mit dem Versatz aus [`TimeSync`] auf Serverzeit um. Die Laufzeit ([`crate::session`]) führt die
//! zurückgegebenen [`Scheduled`]-Einträge zur angegebenen Zeit aus.
//!
//! # Schwellen der Drift-Korrektur ([`DriftConfig`], Standardwerte)
//! Abweichung `d` = Ist-Position minus erwartete Position (Anker + vergangene Serverzeit), nur bei laufender,
//! nicht puffernder Wiedergabe und nach der Beruhigungszeit:
//! - `|d| < 60 ms`: nichts.
//! - `60 ms <= |d| < 2000 ms` und Geschwindigkeit erlaubt: Rate `1 - d/1000 ms` (begrenzt auf 0,5 bis 2,0)
//!   für 1 s, danach zurück auf 1,0.
//! - Geschwindigkeit nicht erlaubt: ab `|d| >= 400 ms` Sprung, darunter nichts.
//! - `|d| >= 2000 ms`: Sprung auf die erwartete Position.
//! - Nach Sprung oder Befehl 2 s Beruhigungszeit ohne Korrektur.
//!
//! Befehle führen nur dann einen Sprung aus, wenn der Player mehr als 200 ms von der Zielposition abweicht.

use crate::enums::{GroupStateType, PlayCommand, PlayQueueUpdateReason, SendCommandType};
use crate::message::{GroupInfo, GroupUpdate, PlayQueueUpdate, SendCommand};
use crate::player::{PlayerAction, PlayerSnapshot};
use crate::time::{TICKS_PER_MICRO, rfc3339};
use crate::timesync::TimeSync;
use serde::Serialize;

/// Schwellen der Drift-Korrektur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftConfig {
    /// Darunter wird nichts korrigiert (ms).
    pub dead_zone_ms: f64,
    /// Ab hier Sprung statt Geschwindigkeit (ms).
    pub speed_max_ms: f64,
    /// Dauer der Geschwindigkeitsänderung (ms).
    pub speed_duration_ms: f64,
    /// Kleinste Geschwindigkeit.
    pub rate_min: f64,
    /// Größte Geschwindigkeit.
    pub rate_max: f64,
    /// Ohne Geschwindigkeitsregelung: ab hier Sprung (ms).
    pub skip_min_ms: f64,
    /// Ruhezeit nach Sprung oder Befehl (ms).
    pub settle_ms: i64,
    /// Befehle springen nur bei größerer Abweichung (ms).
    pub seek_tolerance_ms: f64,
    /// Darf die Wiedergabegeschwindigkeit verändert werden?
    pub allow_rate: bool,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            dead_zone_ms: 60.0,
            speed_max_ms: 2000.0,
            speed_duration_ms: 1000.0,
            rate_min: 0.5,
            rate_max: 2.0,
            skip_min_ms: 400.0,
            settle_ms: 2000,
            seek_tolerance_ms: 200.0,
            allow_rate: true,
        }
    }
}

/// Eine zeitgesteuerte Aktion. `at_local_micros <= now` heißt: sofort.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheduled {
    /// Ausführungszeit auf der lokalen Uhr.
    pub at_local_micros: i64,
    /// Aktion.
    pub action: PlayerAction,
}

/// Anfrage-Körper für `POST /SyncPlay/Ready` und `/SyncPlay/Buffering`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReadyRequest {
    /// Zeitpunkt (Serveruhr).
    #[serde(with = "rfc3339")]
    pub when: i64,
    /// Position in Ticks.
    pub position_ticks: i64,
    /// Würde die Wiedergabe laufen?
    pub is_playing: bool,
    /// Listeneintrag des laufenden Elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_item_id: Option<String>,
}

/// Meldung an die Gruppe.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncReport {
    /// Puffert, Gruppe soll warten.
    Buffering(ReadyRequest),
    /// Bereit.
    Ready(ReadyRequest),
}

#[derive(Debug, Clone, Copy)]
struct Anchor {
    playing: bool,
    position_ticks: i64,
    when_server: i64,
}

/// Zustand eines SyncPlay-Clients.
#[derive(Debug, Clone)]
pub struct SyncCore {
    cfg: DriftConfig,
    time: TimeSync,
    group: Option<GroupInfo>,
    state: Option<GroupStateType>,
    queue: Option<PlayQueueUpdate>,
    anchor: Option<Anchor>,
    last_cmd: Option<(i64, SendCommandType, Option<i64>)>,
    rate_until: Option<i64>,
    settle_until: i64,
    pending_ready: bool,
    buffering_sent: bool,
    last_drift_micros: Option<i64>,
}

fn ms_to_micros(ms: f64) -> i64 {
    (ms * 1000.0) as i64
}

impl SyncCore {
    /// Neu und ohne Gruppe.
    pub fn new(cfg: DriftConfig) -> Self {
        Self {
            cfg,
            time: TimeSync::new(),
            group: None,
            state: None,
            queue: None,
            anchor: None,
            last_cmd: None,
            rate_until: None,
            settle_until: 0,
            pending_ready: false,
            buffering_sent: false,
            last_drift_micros: None,
        }
    }

    /// Zeit-Synchronisation (lesen).
    pub fn time(&self) -> &TimeSync {
        &self.time
    }

    /// Zeit-Synchronisation (Messungen eintragen).
    pub fn time_mut(&mut self) -> &mut TimeSync {
        &mut self.time
    }

    /// Aktuelle Gruppe.
    pub fn group(&self) -> Option<&GroupInfo> {
        self.group.as_ref()
    }

    /// Zuletzt gemeldeter Gruppenzustand.
    pub fn group_state(&self) -> Option<&GroupStateType> {
        self.state.as_ref()
    }

    /// Zuletzt bekannte Wiedergabeliste.
    pub fn queue(&self) -> Option<&PlayQueueUpdate> {
        self.queue.as_ref()
    }

    /// Letzte gemessene Abweichung in Mikrosekunden (positiv = Player ist voraus).
    pub fn last_drift_micros(&self) -> Option<i64> {
        self.last_drift_micros
    }

    /// Ist der Client in einer Gruppe?
    pub fn in_group(&self) -> bool {
        self.group.is_some()
    }

    /// Setzt alles zurück (Gruppe verlassen). Liefert ein nötiges `SetRate(1.0)`.
    pub fn reset(&mut self) -> Vec<PlayerAction> {
        let restore = self.rate_until.is_some();
        *self = Self {
            time: std::mem::take(&mut self.time),
            ..Self::new(self.cfg)
        };
        if restore {
            vec![PlayerAction::SetRate(1.0)]
        } else {
            Vec::new()
        }
    }

    fn current_playlist_item(&self) -> Option<String> {
        let q = self.queue.as_ref()?;
        let idx = usize::try_from(q.playing_item_index).ok()?;
        q.playlist.get(idx).map(|i| i.playlist_item_id.clone())
    }

    fn expected_ticks(&self, server_now: i64) -> Option<i64> {
        let a = self.anchor?;
        Some(if a.playing {
            a.position_ticks + (server_now - a.when_server) * TICKS_PER_MICRO
        } else {
            a.position_ticks
        })
    }

    fn seek_if_needed(&self, snap: &PlayerSnapshot, target: i64, out: &mut Vec<PlayerAction>) {
        let tol = ms_to_micros(self.cfg.seek_tolerance_ms) * TICKS_PER_MICRO;
        if (snap.position_ticks - target).abs() > tol {
            out.push(PlayerAction::SeekTicks(target));
        }
    }

    /// Verarbeitet eine Gruppenänderung. Liefert Aktionen (z. B. Start einer neuen Wiedergabeliste).
    pub fn on_group_update(&mut self, update: &GroupUpdate, now_local: i64) -> Vec<Scheduled> {
        let now = |action| Scheduled {
            at_local_micros: now_local,
            action,
        };
        match update {
            GroupUpdate::GroupJoined { info, .. } => {
                self.group = Some(info.clone());
                self.state = info.state.clone();
                Vec::new()
            }
            GroupUpdate::GroupLeft { .. }
            | GroupUpdate::NotInGroup { .. }
            | GroupUpdate::GroupDoesNotExist { .. }
            | GroupUpdate::LibraryAccessDenied { .. } => {
                self.reset().into_iter().map(now).collect()
            }
            GroupUpdate::StateUpdate { state, .. } => {
                self.state = Some(state.clone());
                if let Some(g) = self.group.as_mut() {
                    g.state = Some(state.clone());
                }
                Vec::new()
            }
            GroupUpdate::UserJoined { user, .. } => {
                if let Some(g) = self.group.as_mut()
                    && !g.participants.contains(user)
                {
                    g.participants.push(user.clone());
                }
                Vec::new()
            }
            GroupUpdate::UserLeft { user, .. } => {
                if let Some(g) = self.group.as_mut() {
                    g.participants.retain(|p| p != user);
                }
                Vec::new()
            }
            GroupUpdate::PlayQueue { update, .. } => self.on_play_queue(update, now_local),
            GroupUpdate::Unknown { .. } => Vec::new(),
        }
    }

    fn on_play_queue(&mut self, q: &PlayQueueUpdate, now_local: i64) -> Vec<Scheduled> {
        self.queue = Some(q.clone());
        let starts = matches!(
            q.reason,
            Some(
                PlayQueueUpdateReason::NewPlaylist
                    | PlayQueueUpdateReason::SetCurrentItem
                    | PlayQueueUpdateReason::NextItem
                    | PlayQueueUpdateReason::PreviousItem
            )
        );
        if !starts || q.playlist.is_empty() {
            return Vec::new();
        }
        let server_now = self.time.local_to_server(now_local);
        let mut start = q.start_position_ticks;
        if q.is_playing && q.last_update > 0 {
            start += (server_now - q.last_update).max(0) * TICKS_PER_MICRO;
        }
        // Die Gruppe wartet nach einer neuen Liste, bis alle bereit sind; danach kommt ein Unpause-Befehl.
        self.anchor = Some(Anchor {
            playing: false,
            position_ticks: start,
            when_server: server_now,
        });
        self.pending_ready = true;
        self.settle_until = now_local + self.cfg.settle_ms * 1000;
        vec![Scheduled {
            at_local_micros: now_local,
            action: PlayerAction::Play {
                item_ids: q.playlist.iter().map(|i| i.item_id.clone()).collect(),
                start_index: Some(q.playing_item_index),
                start_position_ticks: Some(start),
                command: PlayCommand::PlayNow,
                media_source_id: None,
                audio_stream_index: None,
                subtitle_stream_index: None,
            },
        }]
    }

    /// Verarbeitet ein SyncPlay-Kommando; berechnet Ausführungszeit auf der lokalen Uhr.
    pub fn on_command(
        &mut self,
        cmd: &SendCommand,
        now_local: i64,
        snap: &PlayerSnapshot,
    ) -> Vec<Scheduled> {
        let key = (cmd.when, cmd.command.clone(), cmd.position_ticks);
        if self.last_cmd.as_ref() == Some(&key) {
            return Vec::new(); // Wiederholung desselben Befehls ignorieren
        }
        self.last_cmd = Some(key);

        let when_local = self.time.server_to_local(cmd.when);
        let at = when_local.max(now_local);
        let pos = cmd.position_ticks.unwrap_or(snap.position_ticks);
        let settle = self.cfg.settle_ms * 1000;
        self.settle_until = at + settle;
        let mut out = Vec::new();
        let mut push = |at_local: i64, action| {
            out.push(Scheduled {
                at_local_micros: at_local,
                action,
            })
        };
        let mut actions = Vec::new();

        match &cmd.command {
            SendCommandType::Unpause => {
                self.anchor = Some(Anchor {
                    playing: true,
                    position_ticks: pos,
                    when_server: cmd.when,
                });
                if when_local <= now_local {
                    let target = pos + (now_local - when_local) * TICKS_PER_MICRO;
                    self.seek_if_needed(snap, target, &mut actions);
                    actions.push(PlayerAction::Unpause);
                    for a in actions {
                        push(now_local, a);
                    }
                } else {
                    self.seek_if_needed(snap, pos, &mut actions);
                    for a in actions {
                        push(now_local, a);
                    }
                    push(when_local, PlayerAction::Unpause);
                }
            }
            SendCommandType::Pause => {
                self.anchor = Some(Anchor {
                    playing: false,
                    position_ticks: pos,
                    when_server: cmd.when,
                });
                actions.push(PlayerAction::Pause);
                self.seek_if_needed(snap, pos, &mut actions);
                for a in actions {
                    push(at, a);
                }
            }
            SendCommandType::Stop => {
                self.anchor = None;
                push(at, PlayerAction::Stop);
            }
            SendCommandType::Seek => {
                self.anchor = Some(Anchor {
                    playing: false,
                    position_ticks: pos,
                    when_server: cmd.when,
                });
                self.pending_ready = true;
                push(at, PlayerAction::SeekTicks(pos));
            }
            SendCommandType::Unknown(_) => {}
        }
        out
    }

    /// Regelmäßiger Aufruf (z. B. alle 500 ms): Geschwindigkeit zurücksetzen, Drift ausgleichen.
    pub fn tick(&mut self, now_local: i64, snap: &PlayerSnapshot) -> Vec<PlayerAction> {
        let mut out = Vec::new();
        if let Some(until) = self.rate_until {
            if now_local >= until {
                self.rate_until = None;
                out.push(PlayerAction::SetRate(1.0));
            }
            return out;
        }
        let Some(anchor) = self.anchor.filter(|a| a.playing) else {
            return out;
        };
        if self.group.is_none()
            || snap.item_id.is_none()
            || !snap.playing
            || snap.buffering
            || now_local < self.settle_until
        {
            return out;
        }
        let server_now = self.time.local_to_server(now_local);
        if server_now < anchor.when_server {
            return out;
        }
        let expected = self
            .expected_ticks(server_now)
            .unwrap_or(snap.position_ticks);
        let diff_micros = (snap.position_ticks - expected) / TICKS_PER_MICRO;
        self.last_drift_micros = Some(diff_micros);
        let diff_ms = diff_micros as f64 / 1000.0;
        let abs = diff_ms.abs();
        let c = self.cfg;
        if abs < c.dead_zone_ms {
            return out;
        }
        if c.allow_rate && abs < c.speed_max_ms {
            let rate = (1.0 - diff_ms / c.speed_duration_ms).clamp(c.rate_min, c.rate_max);
            self.rate_until = Some(now_local + ms_to_micros(c.speed_duration_ms));
            out.push(PlayerAction::SetRate(rate));
        } else if abs >= c.skip_min_ms || abs >= c.speed_max_ms {
            self.settle_until = now_local + c.settle_ms * 1000;
            out.push(PlayerAction::SeekTicks(expected));
        }
        out
    }

    /// Ready/Buffering-Meldung fällig? Aufruf zusammen mit [`SyncCore::tick`].
    pub fn poll_report(&mut self, now_local: i64, snap: &PlayerSnapshot) -> Option<SyncReport> {
        if self.group.is_none() || snap.item_id.is_none() {
            return None;
        }
        let request = |core: &Self| ReadyRequest {
            when: core.time.local_to_server(now_local),
            position_ticks: snap.position_ticks,
            is_playing: core.anchor.map_or(snap.playing, |a| a.playing),
            playlist_item_id: core.current_playlist_item(),
        };
        if snap.buffering && !self.buffering_sent {
            self.buffering_sent = true;
            Some(SyncReport::Buffering(request(self)))
        } else if !snap.buffering && (self.buffering_sent || self.pending_ready) {
            self.buffering_sent = false;
            self.pending_ready = false;
            Some(SyncReport::Ready(request(self)))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::SyncPlayQueueItem;
    use crate::time::{Clock, ManualClock, TICKS_PER_SECOND};
    use crate::timesync::TimeSample;

    const SEC: i64 = 1_000_000;

    fn core_with_offset(offset: i64) -> SyncCore {
        let mut c = SyncCore::new(DriftConfig::default());
        c.time_mut().add(TimeSample {
            offset_micros: offset,
            rtt_micros: 10_000,
        });
        c.on_group_update(
            &GroupUpdate::GroupJoined {
                group_id: "g".into(),
                info: GroupInfo {
                    group_id: "g".into(),
                    group_name: "Test".into(),
                    ..GroupInfo::default()
                },
            },
            0,
        );
        c
    }

    fn snap(pos_s: f64, playing: bool) -> PlayerSnapshot {
        PlayerSnapshot {
            item_id: Some("i1".into()),
            position_ticks: (pos_s * TICKS_PER_SECOND as f64) as i64,
            playing,
            buffering: false,
            rate: 1.0,
        }
    }

    fn cmd(kind: SendCommandType, when_server: i64, pos_s: f64) -> SendCommand {
        SendCommand {
            group_id: "g".into(),
            playlist_item_id: "p".into(),
            when: when_server,
            position_ticks: Some((pos_s * TICKS_PER_SECOND as f64) as i64),
            command: kind,
            emitted_at: when_server - SEC,
        }
    }

    #[test]
    fn future_unpause_scheduled_on_local_clock_despite_skew() {
        // Serveruhr geht 3 s vor.
        let clock = ManualClock::new(1_000 * SEC);
        let mut core = core_with_offset(3 * SEC);
        let server_now = clock.now_micros() + 3 * SEC;
        let out = core.on_command(
            &cmd(SendCommandType::Unpause, server_now + 2 * SEC, 10.0),
            clock.now_micros(),
            &snap(10.0, false),
        );
        // Position stimmt -> kein Sprung, nur zeitgesteuertes Unpause nach genau 2 s lokaler Zeit.
        assert_eq!(
            out,
            vec![Scheduled {
                at_local_micros: clock.now_micros() + 2 * SEC,
                action: PlayerAction::Unpause
            }]
        );
    }

    #[test]
    fn late_unpause_compensates_elapsed_time() {
        let clock = ManualClock::new(500 * SEC);
        let mut core = core_with_offset(0);
        // Befehl war für 1,5 s in der Vergangenheit geplant, Player steht bei 10 s.
        let out = core.on_command(
            &cmd(
                SendCommandType::Unpause,
                clock.now_micros() - 1_500_000,
                10.0,
            ),
            clock.now_micros(),
            &snap(10.0, false),
        );
        let actions: Vec<_> = out.iter().map(|s| s.action.clone()).collect();
        assert_eq!(
            actions,
            vec![PlayerAction::SeekTicks(115_000_000), PlayerAction::Unpause]
        );
        assert!(out.iter().all(|s| s.at_local_micros == clock.now_micros()));
    }

    #[test]
    fn pause_seeks_when_position_differs() {
        let mut core = core_with_offset(0);
        let now = 100 * SEC;
        let out = core.on_command(
            &cmd(SendCommandType::Pause, now, 30.0),
            now,
            &snap(31.0, true),
        );
        let a: Vec<_> = out.iter().map(|s| s.action.clone()).collect();
        assert_eq!(
            a,
            vec![PlayerAction::Pause, PlayerAction::SeekTicks(300_000_000)]
        );
        // Kleine Abweichung (100 ms) -> kein Sprung.
        let mut core = core_with_offset(0);
        let out = core.on_command(
            &cmd(SendCommandType::Pause, now, 30.0),
            now,
            &snap(30.1, true),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn duplicate_command_ignored() {
        let mut core = core_with_offset(0);
        let c = cmd(SendCommandType::Pause, 50 * SEC, 1.0);
        assert!(!core.on_command(&c, 50 * SEC, &snap(1.0, true)).is_empty());
        assert!(core.on_command(&c, 50 * SEC, &snap(1.0, true)).is_empty());
    }

    #[test]
    fn seek_command_marks_ready_pending() {
        let mut core = core_with_offset(0);
        let now = 10 * SEC;
        let out = core.on_command(
            &cmd(SendCommandType::Seek, now, 60.0),
            now,
            &snap(5.0, true),
        );
        assert_eq!(out[0].action, PlayerAction::SeekTicks(600_000_000));
        let r = core.poll_report(now, &snap(60.0, false));
        assert!(
            matches!(r, Some(SyncReport::Ready(ref q)) if q.position_ticks == 600_000_000 && !q.is_playing)
        );
        assert!(core.poll_report(now, &snap(60.0, false)).is_none());
    }

    /// Simulierter Ablauf: Unpause bei 0, Player läuft zu schnell/langsam.
    fn running_core(clock: &ManualClock, offset: i64) -> SyncCore {
        let mut core = core_with_offset(offset);
        let when = clock.now_micros() + offset;
        core.on_command(
            &cmd(SendCommandType::Unpause, when, 0.0),
            clock.now_micros(),
            &snap(0.0, false),
        );
        clock.advance(10 * SEC); // Beruhigungszeit ist vorbei
        core
    }

    #[test]
    fn drift_within_dead_zone_does_nothing() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = running_core(&clock, 0);
        // Erwartet 10 s, Ist 10,03 s.
        assert!(core.tick(clock.now_micros(), &snap(10.03, true)).is_empty());
        assert_eq!(core.last_drift_micros(), Some(30_000));
    }

    #[test]
    fn behind_speeds_up_then_resets_rate() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = running_core(&clock, 5 * SEC);
        // 500 ms hinterher -> Rate 1,5.
        let out = core.tick(clock.now_micros(), &snap(9.5, true));
        assert_eq!(out, vec![PlayerAction::SetRate(1.5)]);
        // Während der Regelung keine weitere Korrektur.
        clock.advance(500_000);
        assert!(core.tick(clock.now_micros(), &snap(10.4, true)).is_empty());
        clock.advance(600_000);
        assert_eq!(
            core.tick(clock.now_micros(), &snap(11.0, true)),
            vec![PlayerAction::SetRate(1.0)]
        );
    }

    #[test]
    fn ahead_slows_down() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = running_core(&clock, 0);
        let out = core.tick(clock.now_micros(), &snap(10.25, true));
        assert_eq!(out, vec![PlayerAction::SetRate(0.75)]);
    }

    #[test]
    fn large_drift_seeks_to_expected_position() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = running_core(&clock, 0);
        let out = core.tick(clock.now_micros(), &snap(5.0, true));
        assert_eq!(out, vec![PlayerAction::SeekTicks(100_000_000)]);
        // Danach Beruhigungszeit.
        assert!(
            core.tick(clock.now_micros() + SEC, &snap(5.0, true))
                .is_empty()
        );
    }

    #[test]
    fn without_rate_support_small_drift_ignored_medium_seeks() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = SyncCore::new(DriftConfig {
            allow_rate: false,
            ..DriftConfig::default()
        });
        core.on_group_update(
            &GroupUpdate::GroupJoined {
                group_id: "g".into(),
                info: GroupInfo::default(),
            },
            0,
        );
        core.on_command(
            &cmd(SendCommandType::Unpause, clock.now_micros(), 0.0),
            clock.now_micros(),
            &snap(0.0, false),
        );
        clock.advance(10 * SEC);
        assert!(core.tick(clock.now_micros(), &snap(9.8, true)).is_empty());
        assert_eq!(
            core.tick(clock.now_micros(), &snap(9.0, true)),
            vec![PlayerAction::SeekTicks(100_000_000)]
        );
    }

    #[test]
    fn no_correction_while_buffering_paused_or_unsynced() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = running_core(&clock, 0);
        let mut s = snap(1.0, true);
        s.buffering = true;
        assert!(core.tick(clock.now_micros(), &s).is_empty());
        assert!(core.tick(clock.now_micros(), &snap(1.0, false)).is_empty());
        let mut core = SyncCore::new(DriftConfig::default());
        assert!(core.tick(clock.now_micros(), &snap(1.0, true)).is_empty());
    }

    #[test]
    fn buffering_and_ready_reports() {
        let mut core = core_with_offset(2 * SEC);
        let mut s = snap(12.0, true);
        s.buffering = true;
        let r = core.poll_report(100 * SEC, &s).unwrap();
        let SyncReport::Buffering(q) = r else {
            panic!()
        };
        assert_eq!(q.when, 102 * SEC); // lokal -> Serverzeit
        assert!(core.poll_report(100 * SEC, &s).is_none()); // nur einmal
        s.buffering = false;
        assert!(matches!(
            core.poll_report(101 * SEC, &s),
            Some(SyncReport::Ready(_))
        ));
        assert!(core.poll_report(102 * SEC, &s).is_none());
    }

    #[test]
    fn ready_body_serializes_for_server() {
        let body = serde_json::to_value(ReadyRequest {
            when: 1_795_178_096_500_000,
            position_ticks: 42,
            is_playing: true,
            playlist_item_id: Some("pl".into()),
        })
        .unwrap();
        assert_eq!(body["When"], "2026-11-20T12:34:56.500000Z");
        assert_eq!(body["PositionTicks"], 42);
        assert_eq!(body["IsPlaying"], true);
        assert_eq!(body["PlaylistItemId"], "pl");
    }

    #[test]
    fn new_playlist_starts_playback_with_elapsed_offset() {
        let mut core = core_with_offset(SEC);
        let now_local = 100 * SEC;
        let q = PlayQueueUpdate {
            reason: Some(PlayQueueUpdateReason::NewPlaylist),
            last_update: now_local + SEC - 2 * SEC, // vor 2 s (Serveruhr)
            playlist: vec![
                SyncPlayQueueItem {
                    item_id: "a".into(),
                    playlist_item_id: "pa".into(),
                },
                SyncPlayQueueItem {
                    item_id: "b".into(),
                    playlist_item_id: "pb".into(),
                },
            ],
            playing_item_index: 1,
            start_position_ticks: 10 * TICKS_PER_SECOND,
            is_playing: true,
            ..PlayQueueUpdate::default()
        };
        let out = core.on_group_update(
            &GroupUpdate::PlayQueue {
                group_id: "g".into(),
                update: q,
            },
            now_local,
        );
        let PlayerAction::Play {
            item_ids,
            start_index,
            start_position_ticks,
            ..
        } = &out[0].action
        else {
            panic!()
        };
        assert_eq!(item_ids, &["a", "b"]);
        assert_eq!(*start_index, Some(1));
        assert_eq!(*start_position_ticks, Some(12 * TICKS_PER_SECOND));
        let mut s = snap(12.0, false);
        s.buffering = false;
        let SyncReport::Ready(req) = core.poll_report(now_local, &s).unwrap() else {
            panic!()
        };
        assert_eq!(req.playlist_item_id.as_deref(), Some("pb"));
    }

    #[test]
    fn leaving_group_resets_rate_and_state() {
        let clock = ManualClock::new(2_000 * SEC);
        let mut core = running_core(&clock, 0);
        core.tick(clock.now_micros(), &snap(9.5, true)); // Rate aktiv
        let out = core.on_group_update(
            &GroupUpdate::GroupLeft {
                group_id: "g".into(),
            },
            clock.now_micros(),
        );
        assert_eq!(out[0].action, PlayerAction::SetRate(1.0));
        assert!(!core.in_group());
        assert!(core.time().is_synced()); // Zeitversatz bleibt
    }

    #[test]
    fn participants_tracked() {
        let mut core = core_with_offset(0);
        core.on_group_update(
            &GroupUpdate::UserJoined {
                group_id: "g".into(),
                user: "Eva".into(),
            },
            0,
        );
        assert_eq!(core.group().unwrap().participants, ["Eva"]);
        core.on_group_update(
            &GroupUpdate::UserLeft {
                group_id: "g".into(),
                user: "Eva".into(),
            },
            0,
        );
        assert!(core.group().unwrap().participants.is_empty());
    }
}
