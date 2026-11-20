//! Zeit-Synchronisation mit dem Server (NTP-artig über `GET /GetUtcTime`).
//!
//! Je Messung: `t0` lokale Sendezeit, `t1` Empfang am Server, `t2` Antwort am Server, `t3` lokaler Empfang.
//! - Versatz (Server minus lokal) = `((t1 - t0) + (t2 - t3)) / 2`
//! - Laufzeit (RTT) = `(t3 - t0) - (t2 - t1)`
//!
//! Aus den letzten [`TimeSync::MAX_SAMPLES`] Messungen zählt die mit der kleinsten Laufzeit (am wenigsten
//! asymmetrische Verzögerung zu erwarten). Ping = RTT / 2.

use crate::time::{Clock, parse_rfc3339};
use lfx_api::{ApiError, Client};
use serde::Deserialize;
use std::collections::VecDeque;
use std::time::Duration;

/// Eine Messung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSample {
    /// Server minus lokal in Mikrosekunden.
    pub offset_micros: i64,
    /// Laufzeit hin und zurück in Mikrosekunden (nie negativ).
    pub rtt_micros: i64,
}

impl TimeSample {
    /// Rechnet eine Messung aus den vier Zeitstempeln (alle Mikrosekunden).
    pub fn from_timestamps(t0: i64, t1: i64, t2: i64, t3: i64) -> Self {
        Self {
            offset_micros: ((t1 - t0) + (t2 - t3)) / 2,
            rtt_micros: ((t3 - t0) - (t2 - t1)).max(0),
        }
    }
}

/// Sammelt Messungen und rechnet zwischen Server- und Lokalzeit um.
#[derive(Debug, Clone, Default)]
pub struct TimeSync {
    samples: VecDeque<TimeSample>,
}

impl TimeSync {
    /// So viele Messungen werden behalten.
    pub const MAX_SAMPLES: usize = 8;

    /// Leer (Versatz 0).
    pub fn new() -> Self {
        Self::default()
    }

    /// Nimmt eine Messung auf.
    pub fn add(&mut self, sample: TimeSample) {
        if self.samples.len() == Self::MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Gibt es mindestens eine Messung?
    pub fn is_synced(&self) -> bool {
        !self.samples.is_empty()
    }

    /// Anzahl der Messungen im Fenster.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Keine Messung?
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Verwendete Messung (kleinste Laufzeit; bei Gleichstand die neuere).
    pub fn best(&self) -> Option<TimeSample> {
        self.samples
            .iter()
            .rev()
            .copied()
            .min_by_key(|s| s.rtt_micros)
    }

    /// Versatz Server minus lokal (0 ohne Messung).
    pub fn offset_micros(&self) -> i64 {
        self.best().map_or(0, |s| s.offset_micros)
    }

    /// Einfache Laufzeit (RTT / 2) in Mikrosekunden.
    pub fn ping_micros(&self) -> i64 {
        self.best().map_or(0, |s| s.rtt_micros / 2)
    }

    /// Serverzeit → lokale Zeit.
    pub fn server_to_local(&self, server_micros: i64) -> i64 {
        server_micros - self.offset_micros()
    }

    /// Lokale Zeit → Serverzeit.
    pub fn local_to_server(&self, local_micros: i64) -> i64 {
        local_micros + self.offset_micros()
    }
}

/// Zeitplan der Messungen: zuerst schnell hintereinander, danach selten.
#[derive(Debug, Clone, Copy)]
pub struct TimeSyncPolicy {
    /// Anzahl der schnellen ersten Messungen.
    pub greedy_count: u32,
    /// Abstand der schnellen Messungen.
    pub greedy_interval: Duration,
    /// Abstand danach.
    pub steady_interval: Duration,
}

impl Default for TimeSyncPolicy {
    fn default() -> Self {
        Self {
            greedy_count: 3,
            greedy_interval: Duration::from_secs(1),
            steady_interval: Duration::from_secs(60),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UtcTimeResponse {
    request_reception_time: String,
    response_transmission_time: String,
}

/// Eine Messung gegen `GET /GetUtcTime`.
pub async fn measure_once(client: &Client, clock: &dyn Clock) -> Result<TimeSample, ApiError> {
    let t0 = clock.now_micros();
    let resp: UtcTimeResponse = client.get_json("/GetUtcTime", &[]).await?;
    let t3 = clock.now_micros();
    let parse = |s: &str| parse_rfc3339(s).ok_or_else(|| ApiError::Decode("Zeitstempel".into()));
    Ok(TimeSample::from_timestamps(
        t0,
        parse(&resp.request_reception_time)?,
        parse(&resp.response_transmission_time)?,
        t3,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::ManualClock;

    /// Simuliert einen Server, dessen Uhr `skew` Mikrosekunden vorgeht, mit getrennten Hin- und Rückwegzeiten.
    fn simulate(clock: &ManualClock, skew: i64, up: i64, down: i64, proc: i64) -> TimeSample {
        let t0 = clock.now_micros();
        clock.advance(up);
        let t1 = clock.now_micros() + skew;
        clock.advance(proc);
        let t2 = clock.now_micros() + skew;
        clock.advance(down);
        let t3 = clock.now_micros();
        TimeSample::from_timestamps(t0, t1, t2, t3)
    }

    #[test]
    fn symmetric_path_is_exact() {
        let clock = ManualClock::new(1_000_000_000);
        let s = simulate(&clock, 2_500_000, 20_000, 20_000, 1_000);
        assert_eq!(s.offset_micros, 2_500_000);
        assert_eq!(s.rtt_micros, 40_000);
    }

    #[test]
    fn asymmetry_error_is_bounded_by_half_rtt() {
        let clock = ManualClock::new(0);
        let s = simulate(&clock, -700_000, 10_000, 50_000, 0);
        assert!((s.offset_micros + 700_000).abs() <= s.rtt_micros / 2);
    }

    #[test]
    fn best_sample_has_smallest_rtt() {
        let mut t = TimeSync::new();
        assert!(!t.is_synced());
        assert_eq!(t.offset_micros(), 0);
        t.add(TimeSample {
            offset_micros: 900,
            rtt_micros: 300_000,
        });
        t.add(TimeSample {
            offset_micros: 1_000,
            rtt_micros: 20_000,
        });
        t.add(TimeSample {
            offset_micros: 1_200,
            rtt_micros: 90_000,
        });
        assert_eq!(t.offset_micros(), 1_000);
        assert_eq!(t.ping_micros(), 10_000);
        assert_eq!(t.server_to_local(5_000), 4_000);
        assert_eq!(t.local_to_server(4_000), 5_000);
    }

    #[test]
    fn window_drops_old_samples() {
        let mut t = TimeSync::new();
        t.add(TimeSample {
            offset_micros: 1,
            rtt_micros: 1,
        });
        for i in 0..TimeSync::MAX_SAMPLES {
            t.add(TimeSample {
                offset_micros: 100 + i as i64,
                rtt_micros: 1000,
            });
        }
        assert_eq!(t.len(), TimeSync::MAX_SAMPLES);
        assert_ne!(t.offset_micros(), 1);
    }

    #[test]
    fn negative_rtt_clamped() {
        let s = TimeSample::from_timestamps(0, 10, 100, 50);
        assert_eq!(s.rtt_micros, 0);
    }
}
