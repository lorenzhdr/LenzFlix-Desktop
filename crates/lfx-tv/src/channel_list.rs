//! Senderliste „Jetzt im TV“ als Modell (Port der Logik von `tvmode/channel-list.js`): 18 Zeilen mit
//! Logo (Farbe, Schriftfarbe, Kurzname), Titel, Zeit, „Danach“, Fortschritt und Bild-Id.

use lfx_tv_schedule::PlanEntry;
use lfx_tv_schedule::time::format_clock;

use crate::logos::{channel_color, channel_ink, short_name};
use crate::model::{Kind, channel_name, describe, now_all, progress_of, title_of};
use crate::plan::PlanSource;

/// Logo eines Senders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    /// `#rrggbb`
    pub color: &'static str,
    /// Schriftfarbe mit gutem Kontrast.
    pub ink: &'static str,
    /// Kurzname (`KINO`, höchstens 9 Zeichen).
    pub short: String,
    /// Voller Anzeigename (ohne `LIVE`-Präfix).
    pub name: String,
}

/// Zeile der Senderliste.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRow {
    pub no: i64,
    pub badge: Badge,
    /// `Kein Programm`, wenn der Planer nichts liefert.
    pub title: String,
    /// `20:15–22:05 · S1 E2 · Folge`
    pub sub: String,
    /// `Danach: Titel · 22:05`
    pub next: Option<String>,
    /// Gedämpft (Füllprogramm, Füller, Sendepause).
    pub dim: bool,
    /// Bild-Id (Serie bei Folgen) oder `None`.
    pub image_item_id: Option<String>,
    /// Fortschritt 0..1.
    pub progress: f64,
    pub aria_label: String,
    pub entry: Option<PlanEntry>,
}

/// Senderliste.
#[derive(Debug, Clone, Default)]
pub struct ChannelList {
    rows: Vec<ChannelRow>,
    current: i64,
}

fn badge_of(c: &lfx_tv_schedule::Channel) -> Badge {
    Badge {
        color: channel_color(c.no),
        ink: channel_ink(c.no),
        short: short_name(c),
        name: channel_name(c),
    }
}

fn fill_row<P: PlanSource + ?Sized>(
    plan: &mut P,
    item: crate::model::NowItem,
    t: i64,
) -> ChannelRow {
    let no = item.channel.no;
    let badge = badge_of(&item.channel);
    let (title, sub, dim, image) = match &item.entry {
        Some(e) => {
            let d = describe(plan, no, e, t, None);
            let sub = [d.time.as_str(), d.sub.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" · ");
            (d.title, sub, d.kind != Kind::Program, d.image_item_id)
        }
        None => ("Kein Programm".to_string(), String::new(), false, None),
    };
    let next = item.next.as_ref().map(|n| {
        format!(
            "Danach: {} · {}",
            title_of(plan, n).title,
            format_clock(n.start)
        )
    });
    let label_name = if badge.name.is_empty() {
        badge.short.clone()
    } else {
        badge.name.clone()
    };
    ChannelRow {
        no,
        aria_label: format!("Sender {no}, {label_name}: {title}"),
        progress: item.entry.as_ref().map_or(0.0, |e| progress_of(t, e)),
        entry: item.entry,
        badge,
        title,
        sub,
        next,
        dim,
        image_item_id: image,
    }
}

impl ChannelList {
    /// Baut die Liste zum Zeitpunkt `t`.
    pub fn build<P: PlanSource + ?Sized>(plan: &mut P, t: i64) -> Self {
        let rows = now_all(plan, t)
            .into_iter()
            .map(|i| fill_row(plan, i, t))
            .collect();
        Self { rows, current: 0 }
    }

    /// Zeilen.
    pub fn rows(&self) -> &[ChannelRow] {
        &self.rows
    }

    /// Hervorgehobener Sender (0 = keiner, wenn der TV aus ist).
    pub fn current(&self) -> i64 {
        self.current
    }

    /// Markiert den laufenden Sender (`None` = aus).
    pub fn set_current(&mut self, no: Option<i64>) {
        self.current = no.unwrap_or(0);
    }

    /// Takt: ist eine Zeile veraltet (Sendung vorbei), wird alles neu gelesen; sonst nur der
    /// Fortschritt. Liefert `true`, wenn Zeilen neu aufgebaut wurden.
    pub fn tick<P: PlanSource + ?Sized>(&mut self, plan: &mut P, t: i64) -> bool {
        let stale = self
            .rows
            .iter()
            .any(|r| r.entry.as_ref().is_none_or(|e| t >= e.end || t < e.start));
        if stale {
            let cur = self.current;
            *self = Self::build(plan, t);
            self.current = cur;
            return true;
        }
        for r in &mut self.rows {
            if let Some(e) = &r.entry {
                r.progress = progress_of(t, e);
            }
        }
        false
    }
}

/// Breite des Fortschrittsbalkens als Text (`12.3%`, wie `toFixed(1)` im Web).
pub fn progress_percent_text(p: f64) -> String {
    format!(
        "{}%",
        crate::format::js_to_fixed(p.clamp(0.0, 1.0) * 100.0, 1)
    )
}
