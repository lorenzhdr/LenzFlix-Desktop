//! Vorwärm-Kandidaten: Nachbarsender ±1 vorwärmen bzw. vorauflösen (Port von
//! `player/neighbors.js`), ohne Timer und ohne Player: der Controller ruft [`Neighbors::poll`] auf und
//! führt die gelieferten [`NeighborAction`]s aus.
//!
//! * `arm(Play)`: nach [`NEIGHBOR_MS`] stabilem Spielen mit Vorwärmen `Prewarm` für die Nachbarn
//!   (danach alle [`WARM_TICK_MS`], höchstens [`WARM_TICK_MAX`]-mal), sonst `Prefetch` je Sendung
//!   einmal. Steht der Sender kurz im Puffern, jede Sekunde neu (höchstens 20-mal).
//! * `arm(Error)`: nach [`ERROR_WARM_MS`] Fehlerkarte, nur mit Vorwärmen.
//! * Die Schalter (`zap_warm`, `zap_prefetch`) werden bei jedem Schritt frisch gelesen.

use indexmap::IndexSet;
use lfx_tv_schedule::{EntryType, PlanEntry};

use crate::plan::PlanSource;

/// Stabiles Spielen, bis die Nachbarn vorgewärmt werden.
pub const NEIGHBOR_MS: i64 = 1000;
/// Wartezeit nach einer Fehlerkarte.
pub const ERROR_WARM_MS: i64 = 2000;
/// Auffrischung der Vorwärm-Sitzungen.
pub const WARM_TICK_MS: i64 = 35_000;
/// Höchstzahl der Auffrischungen.
pub const WARM_TICK_MAX: u32 = 10;
/// Fehlergründe, nach denen vorgewärmt wird (nicht bei Netzwerk-/Tipp-Fehlern).
pub const WARM_AFTER_ERROR: [&str; 3] = ["video", "browser", "audio"];
const RETRY_MS: i64 = 1000;
const RETRY_MAX: u32 = 20;
const DONE_MAX: usize = 200;

/// Betriebsart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Play,
    Error,
}

/// Phase der Wiedergabe aus Sicht des Vorwärmers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmPhase {
    /// Sendung läuft stabil.
    Playing,
    /// Fehlerkarte steht.
    Error,
    /// Puffern, Pause, aus.
    Other,
}

/// Schalter (Netzregel, Datensparmodus); jederzeit änderbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZapFlags {
    /// Vorwärmen erlaubt.
    pub zap_warm: bool,
    /// Vorauflösen erlaubt (Datensparmodus aus).
    pub zap_prefetch: bool,
}

impl Default for ZapFlags {
    fn default() -> Self {
        Self {
            zap_warm: true,
            zap_prefetch: true,
        }
    }
}

/// Was der Controller ausführen soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeighborAction {
    /// Sitzung des Titels vorwärmen.
    Prewarm { item_id: String, offset_ms: i64 },
    /// PlaybackInfo des Titels vorauflösen.
    Prefetch { item_id: String },
    /// Notiz für die Messanzeige (`filler`, `phase`).
    Note(&'static str),
}

/// Umgebung eines Schritts.
pub struct NeighborCtx<'a> {
    pub plan: &'a mut dyn PlanSource,
    pub channel: i64,
    pub phase: WarmPhase,
    pub flags: ZapFlags,
    pub channel_count: i64,
}

#[derive(Debug, Clone, Copy)]
struct Timer {
    at: i64,
    mode: Mode,
    tries: u32,
}

/// Vorwärm-Takt.
#[derive(Debug, Default)]
pub struct Neighbors {
    timer: Option<Timer>,
    ticks: u32,
    done: IndexSet<String>,
}

impl Neighbors {
    /// Neuer Takt (aus).
    pub fn new() -> Self {
        Self::default()
    }

    /// Nächster Zeitpunkt.
    pub fn next_deadline(&self) -> Option<i64> {
        self.timer.map(|t| t.at)
    }

    /// Takt läuft?
    pub fn is_armed(&self) -> bool {
        self.timer.is_some()
    }

    /// Stoppt (Zappen, neue Sendung, Pause, Aus).
    pub fn stop(&mut self) {
        self.timer = None;
    }

    /// Startet den Takt. `Error` braucht Vorwärmen, sonst passiert nichts.
    pub fn arm(&mut self, mode: Mode, flags: ZapFlags, now: i64) {
        if mode == Mode::Error && !flags.zap_warm {
            return;
        }
        self.stop();
        self.ticks = 0;
        let wait = if mode == Mode::Error {
            ERROR_WARM_MS
        } else {
            NEIGHBOR_MS
        };
        self.timer = Some(Timer {
            at: now + wait,
            mode,
            tries: 0,
        });
    }

    fn neighbors(ctx: &mut NeighborCtx<'_>, now: i64) -> Vec<(i64, PlanEntry, i64)> {
        let n = ctx.channel_count.max(1);
        let ch = ctx.channel;
        let mut out = Vec::new();
        for no in [((ch - 2 + n) % n) + 1, (ch % n) + 1] {
            if no == ch {
                continue;
            }
            if let Ok(cur) = ctx.plan.now_on(no, now)
                && cur.entry.kind == EntryType::Program
                && cur.entry.item_id.is_some()
            {
                out.push((no, cur.entry, cur.offset_ms));
            }
        }
        out
    }

    /// Lässt einen fälligen Zeitgeber wirken; `now` ist der Fälligkeitszeitpunkt.
    pub fn poll(&mut self, now: i64, ctx: &mut NeighborCtx<'_>) -> Vec<NeighborAction> {
        let mut out = Vec::new();
        let Some(t) = self.timer else { return out };
        if now < t.at {
            return out;
        }
        self.timer = None;
        self.step(t.mode, t.tries, now, ctx, &mut out);
        out
    }

    fn step(
        &mut self,
        mode: Mode,
        tries: u32,
        now: i64,
        ctx: &mut NeighborCtx<'_>,
        out: &mut Vec<NeighborAction>,
    ) {
        let want = if mode == Mode::Error {
            WarmPhase::Error
        } else {
            WarmPhase::Playing
        };
        if ctx.phase != want {
            if mode == Mode::Error {
                return; // Karte weg: nichts mehr vorwärmen
            }
            out.push(NeighborAction::Note("phase"));
            if tries < RETRY_MAX {
                self.timer = Some(Timer {
                    at: now + RETRY_MS,
                    mode,
                    tries: tries + 1,
                });
            }
            return;
        }
        if !ctx.flags.zap_warm {
            if mode != Mode::Play {
                return;
            }
            // Ohne Vorwärmen: je Sendung einmal die PlaybackInfo; im Datensparmodus im Takt erneut
            if !ctx.flags.zap_prefetch {
                if self.ticks < WARM_TICK_MAX {
                    self.ticks += 1;
                    self.timer = Some(Timer {
                        at: now + WARM_TICK_MS,
                        mode,
                        tries: 0,
                    });
                }
                return;
            }
            for (no, e, _) in Self::neighbors(ctx, now) {
                let item = e.item_id.clone().unwrap_or_default();
                let key = format!("{no}:{}:{item}", e.start);
                if self.done.contains(&key) {
                    continue;
                }
                self.done.insert(key);
                if self.done.len() > DONE_MAX {
                    self.done.shift_remove_index(0);
                }
                out.push(NeighborAction::Prefetch { item_id: item });
            }
            return;
        }
        let list = Self::neighbors(ctx, now);
        if list.is_empty() {
            out.push(NeighborAction::Note("filler"));
        }
        for (_, e, off) in list {
            out.push(NeighborAction::Prewarm {
                item_id: e.item_id.unwrap_or_default(),
                offset_ms: off,
            });
        }
        if self.ticks >= WARM_TICK_MAX {
            return;
        }
        self.ticks += 1;
        self.timer = Some(Timer {
            at: now + WARM_TICK_MS,
            mode,
            tries: 0,
        });
    }
}
