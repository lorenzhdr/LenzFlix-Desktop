//! Info-Karte und Inhaltsangaben (Port von `tvmode/info-card.js` und `tvmode/details.js`).
//!
//! Die Karte zeigt das **Kalenderdatum des Beginns** (Planzone, Wochentag und `DD.MM.`), nicht den
//! Sendetag: eine Sendung um 02:00 gehört zum Vortag-Sendetag, steht aber unter dem Datum des
//! Beginns. Die Inhaltsangabe lädt [`Details`] entprellt nach.

use std::collections::{HashMap, HashSet};

use lfx_tv_schedule::PlanEntry;
use lfx_tv_schedule::time::{DAY_MS, broadcast_day_of, format_day, offset_ms};

use crate::model::{channel_name, describe};
use crate::plan::PlanSource;

/// Entprellzeit für Inhaltsangaben der Infoleiste.
pub const DETAILS_DELAY_MS: i64 = 400;

/// Zustand des Inhaltsangaben-Textes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overview {
    /// Kein Titel oder kein Dienst: Feld ausblenden.
    Hidden,
    /// „Inhaltsangabe wird geladen …“
    Loading,
    /// „Keine Inhaltsangabe vorhanden.“
    Empty,
    /// Text.
    Text(String),
}

impl Overview {
    /// Anzeigetext (leer bei `Hidden`).
    pub fn text(&self) -> &str {
        match self {
            Overview::Hidden => "",
            Overview::Loading => "Inhaltsangabe wird geladen …",
            Overview::Empty => "Keine Inhaltsangabe vorhanden.",
            Overview::Text(t) => t,
        }
    }
}

/// Knopf der Karte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAction {
    /// „Jetzt einschalten“ (nur bei laufender Sendung): Karte schließen, Sender sofort wählen.
    TuneNow(i64),
    /// „Details“: Detailseite des Titels.
    Details(String),
    /// „In der Mediathek ansehen“: TV stoppen, Titel von vorn abspielen.
    PlayFromStart(String),
}

impl CardAction {
    /// Beschriftung.
    pub fn label(&self) -> &'static str {
        match self {
            CardAction::TuneNow(_) => "Jetzt einschalten",
            CardAction::Details(_) => "Details",
            CardAction::PlayFromStart(_) => "In der Mediathek ansehen",
        }
    }
}

/// Inhalt der Info-Karte.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoCard {
    pub no: i64,
    pub entry: PlanEntry,
    /// `5 · Kino`
    pub channel: String,
    pub title: String,
    pub sub: String,
    /// `Fr 14.11. · 20:15–22:05`
    pub time_line: String,
    /// `1999 · 95 Min. · Komödie`
    pub meta: Option<String>,
    pub badges: Vec<String>,
    pub image_item_id: Option<String>,
    pub item_id: Option<String>,
    pub overview: Overview,
    pub running: bool,
    pub actions: Vec<CardAction>,
}

/// Läuft die Sendung zum Zeitpunkt `t`?
pub fn is_running(entry: &PlanEntry, t: i64) -> bool {
    entry.start <= t && t < entry.end
}

/// Datum des Beginns: `Fr 14.11.` (Kalendertag in der Planzone, nicht der Sendetag).
pub fn start_date_label(start: i64) -> String {
    let f = format_day((start + offset_ms(start)).div_euclid(DAY_MS));
    format!("{} {}", f.weekday, f.date)
}

fn overview_state(
    overview: &str,
    id: Option<&str>,
    details: Option<&HashMap<String, String>>,
) -> Overview {
    let Some(id) = id else {
        return Overview::Hidden;
    };
    let text = if overview.is_empty() {
        details.and_then(|d| d.get(id)).map(String::as_str)
    } else {
        Some(overview)
    };
    match text {
        Some("") => Overview::Empty,
        Some(t) => Overview::Text(t.to_string()),
        None if details.is_some() => Overview::Loading,
        None => Overview::Hidden,
    }
}

fn actions_of(no: i64, running: bool, item_id: Option<&str>) -> Vec<CardAction> {
    let mut a = Vec::new();
    if running {
        a.push(CardAction::TuneNow(no));
    }
    if let Some(id) = item_id {
        a.push(CardAction::Details(id.to_string()));
        a.push(CardAction::PlayFromStart(id.to_string()));
    }
    a
}

/// Baut die Karte. `details` = bekannte Inhaltsangaben (`None`: kein Nachlade-Dienst).
pub fn build_info_card<P: PlanSource + ?Sized>(
    plan: &mut P,
    no: i64,
    entry: &PlanEntry,
    t: i64,
    details: Option<&HashMap<String, String>>,
) -> InfoCard {
    let info = describe(plan, no, entry, t, details);
    let d = broadcast_day_of(entry.start);
    let name = plan
        .channels(d)
        .iter()
        .find(|c| c.no == no)
        .map(channel_name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("Sender {no}"));
    let mut meta: Vec<String> = Vec::new();
    if let Some(y) = info.year {
        meta.push(y.to_string());
    }
    if let Some(m) = info.run_min {
        meta.push(format!("{m} Min."));
    }
    if !info.genres.is_empty() {
        meta.push(info.genres.join(", "));
    }
    let running = is_running(entry, t);
    InfoCard {
        no,
        entry: entry.clone(),
        channel: format!("{no} · {name}"),
        time_line: format!("{} · {}", start_date_label(entry.start), info.time),
        meta: if meta.is_empty() {
            None
        } else {
            Some(meta.join(" · "))
        },
        overview: overview_state(&info.overview, info.item_id.as_deref(), details),
        actions: actions_of(no, running, info.item_id.as_deref()),
        title: info.title,
        sub: info.sub,
        badges: info.badges,
        image_item_id: info.image_item_id,
        item_id: info.item_id,
        running,
    }
}

impl InfoCard {
    /// Takt: ändert sich „läuft gerade“, baut sich die Knopfreihe neu. Liefert `true` bei Änderung.
    pub fn tick(&mut self, t: i64) -> bool {
        let r = is_running(&self.entry, t);
        if r == self.running {
            return false;
        }
        self.running = r;
        self.actions = actions_of(self.no, r, self.item_id.as_deref());
        true
    }

    /// Inhaltsangabe wurde geladen: Text neu bestimmen.
    pub fn set_details(&mut self, details: Option<&HashMap<String, String>>) {
        self.overview = overview_state("", self.item_id.as_deref(), details);
    }
}

/// Nachladen der Inhaltsangaben (Port von `details.js`): Infoleiste entprellt, Info-Karte sofort.
/// Das Laden selbst macht der Aufrufer; `request`/`poll` nennen die Id, die zu laden ist.
#[derive(Debug)]
pub struct Details {
    texts: HashMap<String, String>,
    inflight: HashSet<String>,
    wait: Option<(String, i64)>,
    delay_ms: i64,
    can: bool,
}

impl Details {
    /// Neuer Zwischenspeicher. `can` = es gibt einen Dienst zum Nachladen.
    pub fn new(can: bool) -> Self {
        Self {
            texts: HashMap::new(),
            inflight: HashSet::new(),
            wait: None,
            delay_ms: DETAILS_DELAY_MS,
            can,
        }
    }

    /// Text oder `None` (noch nicht geladen). `Some("")` = geladen, ohne Inhaltsangabe.
    pub fn get(&self, id: &str) -> Option<&String> {
        self.texts.get(id)
    }

    /// Alle bekannten Texte.
    pub fn map(&self) -> &HashMap<String, String> {
        &self.texts
    }

    fn start(&mut self, id: &str) -> Option<String> {
        if !self.can || self.texts.contains_key(id) || self.inflight.contains(id) {
            return None;
        }
        self.inflight.insert(id.to_string());
        Some(id.to_string())
    }

    /// Fordert eine Inhaltsangabe an. `immediate`: sofort laden (Info-Karte); sonst entprellt, eine
    /// noch wartende andere Id wird verworfen. Liefert die Id, die sofort zu laden ist.
    pub fn request(&mut self, id: &str, immediate: bool, now: i64) -> Option<String> {
        if id.is_empty() || !self.can || self.texts.contains_key(id) || self.inflight.contains(id) {
            return None;
        }
        if immediate {
            return self.start(id);
        }
        self.wait = Some((id.to_string(), now + self.delay_ms));
        None
    }

    /// Fälligkeit der entprellten Anfrage.
    pub fn next_deadline(&self) -> Option<i64> {
        self.wait.as_ref().map(|w| w.1)
    }

    /// Lässt die entprellte Anfrage wirken; liefert die Id, die zu laden ist.
    pub fn poll(&mut self, now: i64) -> Option<String> {
        let (id, at) = self.wait.clone()?;
        if now < at {
            return None;
        }
        self.wait = None;
        self.start(&id)
    }

    /// Ergebnis des Ladens (Fehler: leerer Text).
    pub fn provide(&mut self, id: &str, text: &str) {
        self.inflight.remove(id);
        self.texts.insert(id.to_string(), text.to_string());
    }

    /// Beendet den Dienst.
    pub fn dispose(&mut self) {
        self.can = false;
        self.wait = None;
    }
}
