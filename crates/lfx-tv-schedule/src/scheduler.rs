//! Öffentliche Planer-API (Port von `index.js`): rein, deterministisch, ohne Uhr und IO.
//!
//! Die Uhrzeit liefert der Aufrufer (`t` in ms, UTC). Die Inhalte der Filmreihen für Sender 18
//! lädt der Aufrufer (siehe [`Scheduler::next_boxset_request`] / [`Scheduler::provide_boxset`]
//! oder die bequeme Form [`Scheduler::prepare`]).

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use crate::catalog::{Catalog, Item};
use crate::fill::{EntryType, PlanEntry, closedown_day};
use crate::jsval::JsVal;
use crate::plan::Context;
use crate::time::{Bounds, broadcast_day_of, day_bounds};

pub use crate::plan::Channel;

const LOOKAHEAD_DAYS: i64 = 3;
const MEMO_MAX: usize = 18 * 8;

/// Ungültiger Sender (nicht 1 bis 18) oder Tag (`RangeError` im Web-Planer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError;

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("planDay: ungültiger Sender oder Tag")
    }
}

impl std::error::Error for PlanError {}

/// Ergebnis von [`Scheduler::now_on`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowOn {
    pub entry: PlanEntry,
    /// Abstand des Zeitpunkts zum Beginn des Eintrags.
    pub offset_ms: i64,
    /// Sendetag.
    pub d: i64,
}

fn valid_plan(list: &[PlanEntry], b: &Bounds) -> bool {
    let (Some(first), Some(last)) = (list.first(), list.last()) else {
        return false;
    };
    if first.start != b.start || last.end != b.end {
        return false;
    }
    list.iter()
        .enumerate()
        .all(|(i, e)| e.end > e.start && (i == 0 || e.start == list[i - 1].end))
}

/// Der Planer.
pub struct Scheduler {
    ctx: Context,
    memo: HashMap<(i64, i64), Rc<Vec<PlanEntry>>>,
    memo_order: VecDeque<(i64, i64)>,
}

impl Scheduler {
    /// `createScheduler(catalog, { channelConfig })`; `channel_config` ist ein Objekt
    /// (`{5: 'Action'}`), ein Text (`5=Action;6=Komödie`) oder `JsVal::Undefined`.
    pub fn new(catalog: Arc<Catalog>, channel_config: &JsVal) -> Self {
        Scheduler {
            ctx: Context::new(catalog, channel_config),
            memo: HashMap::new(),
            memo_order: VecDeque::new(),
        }
    }

    /// Die 18 Sender des Sendetags `d`.
    pub fn channels(&mut self, d: i64) -> Rc<Vec<Channel>> {
        self.ctx.channels(d)
    }

    /// Nächste Filmreihe, deren Inhalte für `days` noch fehlen (siehe Moduldoku).
    pub fn next_boxset_request(&mut self, days: &[i64]) -> Option<String> {
        crate::themeday::next_boxset_request(&mut self.ctx, days)
    }

    /// Inhalte einer Filmreihe melden (`None` = Laden fehlgeschlagen oder keine Liste).
    pub fn provide_boxset(&mut self, id: &str, ids: Option<Vec<String>>) {
        crate::themeday::provide_boxset(&mut self.ctx, id, ids);
        self.memo.clear();
        self.memo_order.clear();
    }

    /// Lädt die Filmreihen-Inhalte für `days` sequentiell über `loader` (`Err` = Fehler,
    /// `Ok(None)` = keine Liste); gleichwertig zum `prepare` des Web-Planers.
    pub fn prepare(
        &mut self,
        days: &[i64],
        mut loader: impl FnMut(&str) -> Result<Option<Vec<String>>, ()>,
    ) {
        while let Some(id) = self.next_boxset_request(days) {
            let ids = loader(&id).unwrap_or(None);
            self.provide_boxset(&id, ids);
        }
    }

    /// Plan des Senders `no` (1 bis 18) am Sendetag `d`.
    pub fn plan_day(&mut self, no: i64, d: i64) -> Result<Rc<Vec<PlanEntry>>, PlanError> {
        if !(1..=18).contains(&no) {
            return Err(PlanError);
        }
        let key = (no, d);
        if let Some(p) = self.memo.get(&key) {
            return Ok(p.clone());
        }
        let b = day_bounds(d);
        let plan: Vec<PlanEntry> = if no >= 17 {
            let list = if no == 17 {
                crate::premiere::plan_day(&mut self.ctx, d)
            } else {
                crate::themeday::plan_day(&mut self.ctx, d)
            };
            if valid_plan(&list, &b) {
                list
            } else {
                closedown_day(d, "empty")
            }
        } else {
            self.ctx.plan_builtin(no, d)
        };
        if self.memo.len() >= MEMO_MAX
            && let Some(old) = self.memo_order.pop_front()
        {
            self.memo.remove(&old);
        }
        let p = Rc::new(plan);
        self.memo.insert(key, p.clone());
        self.memo_order.push_back(key);
        Ok(p)
    }

    /// Was läuft auf Sender `no` zum Zeitpunkt `t`?
    pub fn now_on(&mut self, no: i64, t: i64) -> Result<NowOn, PlanError> {
        let d = broadcast_day_of(t);
        let list = self.plan_day(no, d)?;
        let i = Self::index_at(&list, t);
        let entry = list[i].clone();
        Ok(NowOn {
            offset_ms: t - entry.start,
            entry,
            d,
        })
    }

    fn index_at(list: &[PlanEntry], t: i64) -> usize {
        let (mut lo, mut hi) = (0usize, list.len() - 1);
        while lo < hi {
            let m = (lo + hi).div_ceil(2);
            if list[m].start <= t {
                lo = m;
            } else {
                hi = m - 1;
            }
        }
        lo
    }

    /// Aktueller Eintrag plus die `n` folgenden Sendungen (Art program, fill oder nightfill;
    /// Füller und Testbild werden übersprungen), höchstens 3 Tage voraus.
    pub fn upcoming(&mut self, no: i64, t: i64, n: usize) -> Result<Vec<PlanEntry>, PlanError> {
        let now = self.now_on(no, t)?;
        let d = now.d;
        let mut list = self.plan_day(no, d)?;
        let mut i = Self::index_at(&list, t) + 1;
        let mut out = vec![now.entry];
        let mut day = d;
        while out.len() <= n {
            if i >= list.len() {
                day += 1;
                if day > d + LOOKAHEAD_DAYS {
                    break;
                }
                list = self.plan_day(no, day)?;
                i = 0;
                continue;
            }
            let e = &list[i];
            i += 1;
            if matches!(
                e.kind,
                EntryType::Program | EntryType::Fill | EntryType::Nightfill
            ) {
                out.push(e.clone());
            }
        }
        Ok(out)
    }

    /// Titel, Folge oder Serie zur Id.
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.ctx.catalog.items.get(id)
    }

    /// Genres mit Anzeigelabel und Titelzahl.
    pub fn genres(&self) -> Vec<(String, i64)> {
        self.ctx
            .catalog
            .genres
            .iter()
            .map(|g| (g.label.clone(), g.count))
            .collect()
    }
}
