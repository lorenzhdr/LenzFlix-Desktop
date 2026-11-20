//! W4-media: reine Logik der Medienseiten (Musik, Bücher, Fotos, Live-TV) ohne Oberfläche und ohne Netzwerk.
//!
//! Enthält die Routen der Medienseiten, die Warteschlange des Audio-Players (Zufall, Wiederholen, Springen),
//! die Zeitrechnung des Programmführers und die Zoom-/Schiebe-Rechnung des Foto-Betrachters. Alles ist
//! deterministisch testbar.

use lfx_api::BaseItem;

// ---------------------------------------------------------------- Routen

/// Welche Medienseite gemeint ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    /// Musikbibliothek (Alben, Künstler, Titel, Genres, Wiedergabelisten).
    Music,
    /// Album.
    Album,
    /// Künstler.
    Artist,
    /// Wiedergabeliste.
    Playlist,
    /// Bücherliste.
    Books,
    /// Buchdetail.
    Book,
    /// Fotoordner oder Fotoraster.
    Photos,
    /// Live-TV.
    LiveTv,
}

/// Eine Medienseite: Art, Element-/Ansichts-Id, Titel und Reiter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRoute {
    pub kind: MediaKind,
    /// Id der Bibliotheksansicht bzw. des Elements (Album, Künstler, Liste, Buch, Fotoordner).
    pub id: String,
    /// Anzeigetitel (Bibliotheksname, Albumname, …).
    pub name: String,
    /// Reiter (Musik: 0 Alben … 4 Wiedergabelisten; Live-TV: 0 Kanäle … 3 Timer).
    pub tab: usize,
}

impl MediaRoute {
    pub fn new(kind: MediaKind, id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            name: name.into(),
            tab: 0,
        }
    }

    pub fn with_tab(mut self, tab: usize) -> Self {
        self.tab = tab;
        self
    }

    /// Kennung für Automation und Protokoll.
    pub fn slug(&self) -> &'static str {
        match self.kind {
            MediaKind::Music => "music",
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
            MediaKind::Books => "books",
            MediaKind::Book => "book",
            MediaKind::Photos => "photos",
            MediaKind::LiveTv => "livetv",
        }
    }
}

/// Reiter der Musikbibliothek.
pub const MUSIC_TABS: [&str; 5] = ["Alben", "Künstler", "Titel", "Genres", "Wiedergabelisten"];
/// Reiter von Live-TV.
pub const LIVE_TABS: [&str; 4] = ["Kanäle", "Programm", "Aufnahmen", "Timer"];

/// Ordnet einen Sammlungstyp (`CollectionType`) einer Medienseite zu. `homevideos` sind in Jellyfin die
/// Foto-/Heimvideo-Bibliotheken; sie öffnen den Fotobereich.
pub fn kind_for_collection(ctype: &str) -> Option<MediaKind> {
    match ctype {
        "music" => Some(MediaKind::Music),
        "books" => Some(MediaKind::Books),
        "photos" | "homevideos" => Some(MediaKind::Photos),
        "livetv" => Some(MediaKind::LiveTv),
        _ => None,
    }
}

// ---------------------------------------------------------------- Warteschlange

/// Wiederholen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::All => 1,
            Self::One => 2,
        }
    }
}

/// Warteschlange des Audio-Players. `order` ist die Abspielreihenfolge (Indizes in `items`); `pos` zeigt
/// hinein. Zufall mischt deterministisch aus einem Startwert (Tests) und hält den laufenden Titel vorn.
#[derive(Clone, Debug, Default)]
pub struct AudioQueue {
    pub items: Vec<BaseItem>,
    order: Vec<usize>,
    pos: usize,
    pub shuffle: bool,
    pub repeat: Repeat,
    seed: u64,
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = (*state).max(1);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

impl AudioQueue {
    pub fn new(seed: u64) -> Self {
        Self {
            seed: seed.max(1),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Index des laufenden Titels in `items`.
    pub fn current_index(&self) -> Option<usize> {
        self.order.get(self.pos).copied()
    }

    pub fn current(&self) -> Option<&BaseItem> {
        self.current_index().and_then(|i| self.items.get(i))
    }

    /// Setzt die Warteschlange neu und beginnt bei `start` (Index in `items`).
    pub fn set(&mut self, items: Vec<BaseItem>, start: usize) {
        self.items = items;
        let start = start.min(self.items.len().saturating_sub(1));
        self.rebuild_order(start);
    }

    fn rebuild_order(&mut self, first: usize) {
        let n = self.items.len();
        self.order = (0..n).collect();
        self.pos = first.min(n.saturating_sub(1));
        if self.shuffle && n > 1 {
            // Fisher-Yates ohne den laufenden Titel, der steht vorn.
            let mut rest: Vec<usize> = (0..n).filter(|i| *i != first).collect();
            for i in (1..rest.len()).rev() {
                let j = (xorshift(&mut self.seed) % (i as u64 + 1)) as usize;
                rest.swap(i, j);
            }
            self.order = std::iter::once(first).chain(rest).collect();
            self.pos = 0;
        }
    }

    /// Zufall ein-/ausschalten; der laufende Titel bleibt der laufende.
    pub fn set_shuffle(&mut self, on: bool) {
        let cur = self.current_index().unwrap_or(0);
        self.shuffle = on;
        self.rebuild_order(cur);
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// Nächster Titel: `auto` = natürliches Ende (bei „Einen wiederholen“ derselbe Titel). Gibt den neuen
    /// Index in `items` zurück oder `None` am Ende der Liste ohne Wiederholen.
    pub fn next(&mut self, auto: bool) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        if auto && self.repeat == Repeat::One {
            return self.current_index();
        }
        if self.pos + 1 < self.order.len() {
            self.pos += 1;
        } else if self.repeat == Repeat::All || (!auto && self.repeat != Repeat::Off) {
            self.pos = 0;
        } else {
            return None;
        }
        self.current_index()
    }

    /// Vorheriger Titel (am Anfang: Anfang bzw. bei „Alle wiederholen“ der letzte Titel).
    pub fn previous(&mut self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        if self.pos > 0 {
            self.pos -= 1;
        } else if self.repeat == Repeat::All {
            self.pos = self.order.len() - 1;
        }
        self.current_index()
    }

    /// Springt auf einen Titel (Index in `items`).
    pub fn jump_to(&mut self, index: usize) -> Option<usize> {
        let p = self.order.iter().position(|i| *i == index)?;
        self.pos = p;
        self.current_index()
    }

    /// Hängt Titel hinten an.
    pub fn enqueue(&mut self, more: Vec<BaseItem>) {
        let start = self.items.len();
        self.items.extend(more);
        self.order.extend(start..self.items.len());
    }

    /// Fügt Titel direkt hinter dem laufenden ein.
    pub fn play_next(&mut self, more: Vec<BaseItem>) {
        let start = self.items.len();
        let count = more.len();
        self.items.extend(more);
        let at = (self.pos + 1).min(self.order.len());
        for k in 0..count {
            self.order.insert(at + k, start + k);
        }
    }

    /// Entfernt einen Titel (Index in `items`). Der laufende Titel kann entfernt werden; dann rückt der
    /// nächste nach (`pos` bleibt, begrenzt).
    pub fn remove(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }
        let was_before = self
            .order
            .iter()
            .position(|i| *i == index)
            .is_some_and(|p| p < self.pos);
        self.items.remove(index);
        self.order.retain(|i| *i != index);
        for i in &mut self.order {
            if *i > index {
                *i -= 1;
            }
        }
        if was_before && self.pos > 0 {
            self.pos -= 1;
        }
        self.pos = self.pos.min(self.order.len().saturating_sub(1));
    }

    /// Titel in Abspielreihenfolge (für die Anzeige der Warteschlange): (Index in `items`, ist laufend).
    pub fn upcoming(&self) -> Vec<(usize, bool)> {
        self.order
            .iter()
            .enumerate()
            .map(|(p, i)| (*i, p == self.pos))
            .collect()
    }
}

// ---------------------------------------------------------------- Programmführer

/// Pixel je Minute im Programmführer.
pub const GUIDE_PX_PER_MIN: f64 = 4.0;
/// Breite des Zeitfensters in Stunden.
pub const GUIDE_HOURS: i64 = 6;
const HOUR_MS: i64 = 3_600_000;

/// Beginn des Zeitfensters: die volle Stunde vor `now_ms` (eine Stunde Rückblick).
pub fn guide_window_start(now_ms: i64) -> i64 {
    now_ms.div_euclid(HOUR_MS) * HOUR_MS - HOUR_MS
}

/// Lage einer Sendung im Fenster: (x, Breite) in Pixeln; `None`, wenn sie ganz außerhalb liegt. Sendungen,
/// die vor dem Fenster beginnen, werden vorn abgeschnitten.
pub fn guide_cell(win_start: i64, start_ms: i64, end_ms: i64) -> Option<(f64, f64)> {
    let win_end = win_start + GUIDE_HOURS * HOUR_MS;
    if end_ms <= win_start || start_ms >= win_end || end_ms <= start_ms {
        return None;
    }
    let s = start_ms.max(win_start);
    let e = end_ms.min(win_end);
    let per_ms = GUIDE_PX_PER_MIN / 60_000.0;
    Some((
        (s - win_start) as f64 * per_ms,
        ((e - s) as f64 * per_ms).max(8.0),
    ))
}

/// Spalte der aktuellen Zeit in Pixeln.
pub fn guide_now_x(win_start: i64, now_ms: i64) -> f64 {
    (now_ms - win_start) as f64 * GUIDE_PX_PER_MIN / 60_000.0
}

// ---------------------------------------------------------------- Foto-Betrachter

/// Kleinster und größter Zoom.
pub const ZOOM_MIN: f32 = 1.0;
pub const ZOOM_MAX: f32 = 8.0;

/// Zoomstufe nach einem Schritt (ein Schritt = Faktor 1,25), begrenzt.
pub fn zoom_step(zoom: f32, steps: i32) -> f32 {
    (zoom * 1.25f32.powi(steps)).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Begrenzt die Verschiebung (Anteile der Bildfläche, 0 = Mitte) so, dass bei Zoom `z` kein Rand sichtbar wird.
pub fn clamp_pan(zoom: f32, pan: f32) -> f32 {
    let limit = (zoom - 1.0) / 2.0;
    pan.clamp(-limit, limit)
}

/// Endung der Datei zu einem Buch (nur Buchstaben und Ziffern, höchstens 5 Zeichen).
pub fn book_extension(container: Option<&str>) -> String {
    let ext: String = container
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(5)
        .collect::<String>()
        .to_ascii_lowercase();
    if ext.is_empty() {
        "bin".to_owned()
    } else {
        ext
    }
}

/// Dateiname eines geladenen Buchs: nur aus der Id und der Endung, nie aus dem Servertext.
pub fn book_file_name(id: &str, container: Option<&str>) -> String {
    let safe: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    format!("buch-{safe}.{}", book_extension(container))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<BaseItem> {
        (0..n)
            .map(|i| BaseItem {
                id: format!("t{i}"),
                name: format!("Titel {i}"),
                ..BaseItem::default()
            })
            .collect()
    }

    fn id(q: &AudioQueue) -> String {
        q.current().map(|i| i.id.clone()).unwrap_or_default()
    }

    #[test]
    fn plain_order_and_end() {
        let mut q = AudioQueue::new(7);
        q.set(items(3), 1);
        assert_eq!(id(&q), "t1");
        assert_eq!(q.next(true), Some(2));
        assert_eq!(q.next(true), None);
        assert_eq!(id(&q), "t2");
        assert_eq!(q.previous(), Some(1));
    }

    #[test]
    fn repeat_modes() {
        let mut q = AudioQueue::new(7);
        q.set(items(2), 1);
        q.set_repeat(Repeat::All);
        assert_eq!(q.next(true), Some(0));
        q.set_repeat(Repeat::One);
        assert_eq!(q.next(true), Some(0));
        // Von Hand weiter: bei „Einen wiederholen“ geht es zum nächsten
        assert_eq!(q.next(false), Some(1));
        q.set_repeat(Repeat::Off);
        assert_eq!(q.next(true), None);
        assert_eq!(Repeat::Off.cycle().cycle().cycle(), Repeat::Off);
    }

    #[test]
    fn shuffle_keeps_current_and_covers_all() {
        let mut q = AudioQueue::new(42);
        q.set(items(8), 3);
        q.set_shuffle(true);
        assert_eq!(id(&q), "t3");
        let mut seen = vec![id(&q)];
        while let Some(i) = q.next(true) {
            seen.push(q.items[i].id.clone());
        }
        assert_eq!(seen.len(), 8);
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 8);
        // Zufall aus: Reihenfolge wieder aufsteigend, laufender Titel bleibt
        q.set_shuffle(false);
        let cur = q.current_index().unwrap();
        assert_eq!(q.next(false), Some((cur + 1) % 8).filter(|n| *n > cur));
    }

    #[test]
    fn shuffle_is_deterministic_per_seed() {
        let order = |seed| {
            let mut q = AudioQueue::new(seed);
            q.shuffle = true;
            q.set(items(10), 0);
            q.upcoming().into_iter().map(|(i, _)| i).collect::<Vec<_>>()
        };
        assert_eq!(order(5), order(5));
        assert_ne!(order(5), order(6));
    }

    #[test]
    fn enqueue_play_next_remove() {
        let mut q = AudioQueue::new(1);
        q.set(items(3), 0);
        let extra = vec![BaseItem {
            id: "x".into(),
            ..BaseItem::default()
        }];
        q.play_next(extra.clone());
        assert_eq!(
            q.next(false).map(|i| q.items[i].id.clone()),
            Some("x".into())
        );
        q.enqueue(extra);
        assert_eq!(q.len(), 5);
        // Vorherigen entfernen: der laufende bleibt
        let cur = id(&q);
        q.remove(0);
        assert_eq!(id(&q), cur);
        // Laufenden entfernen: der nächste rückt nach
        q.remove(q.current_index().unwrap());
        assert!(q.current().is_some());
        q.set(Vec::new(), 0);
        assert!(q.next(true).is_none() && q.current().is_none());
    }

    #[test]
    fn jump_finds_item_in_shuffled_order() {
        let mut q = AudioQueue::new(9);
        q.shuffle = true;
        q.set(items(6), 0);
        assert_eq!(q.jump_to(4), Some(4));
        assert_eq!(id(&q), "t4");
        assert_eq!(q.jump_to(99), None);
    }

    #[test]
    fn guide_geometry() {
        let now = 1_700_000_000_000i64 + 25 * 60_000;
        let ws = guide_window_start(now);
        assert_eq!(ws % HOUR_MS, 0);
        assert!(ws < now && now - ws <= 2 * HOUR_MS);
        // Sendung genau eine Stunde nach Fensterbeginn, 30 min lang
        let (x, w) = guide_cell(ws, ws + HOUR_MS, ws + HOUR_MS + 30 * 60_000).unwrap();
        assert_eq!(x, 60.0 * GUIDE_PX_PER_MIN);
        assert_eq!(w, 30.0 * GUIDE_PX_PER_MIN);
        // Beginnt vor dem Fenster: vorn abgeschnitten
        let (x, w) = guide_cell(ws, ws - HOUR_MS, ws + 30 * 60_000).unwrap();
        assert_eq!(x, 0.0);
        assert_eq!(w, 30.0 * GUIDE_PX_PER_MIN);
        // Ganz außerhalb
        assert!(guide_cell(ws, ws - 2 * HOUR_MS, ws - HOUR_MS).is_none());
        assert!(guide_cell(ws, ws + 7 * HOUR_MS, ws + 8 * HOUR_MS).is_none());
        assert!(guide_cell(ws, ws + 10, ws + 10).is_none());
        assert!(guide_now_x(ws, now) > 0.0);
    }

    #[test]
    fn zoom_and_pan_limits() {
        assert_eq!(zoom_step(1.0, -1), 1.0);
        assert!((zoom_step(1.0, 2) - 1.5625).abs() < 1e-5);
        assert_eq!(zoom_step(7.9, 5), ZOOM_MAX);
        assert_eq!(clamp_pan(1.0, 0.7), 0.0);
        assert_eq!(clamp_pan(3.0, 5.0), 1.0);
        assert_eq!(clamp_pan(3.0, -5.0), -1.0);
    }

    #[test]
    fn collections_map_to_pages() {
        assert_eq!(kind_for_collection("music"), Some(MediaKind::Music));
        assert_eq!(kind_for_collection("homevideos"), Some(MediaKind::Photos));
        assert_eq!(kind_for_collection("livetv"), Some(MediaKind::LiveTv));
        assert_eq!(kind_for_collection("movies"), None);
    }

    #[test]
    fn book_file_names_ignore_server_text() {
        assert_eq!(
            book_file_name("ab-12/../x", Some("epub")),
            "buch-ab12x.epub"
        );
        assert_eq!(book_extension(Some("../../evil.exe;")), "exe");
        assert_eq!(book_extension(None), "bin");
        assert_eq!(book_extension(Some("PDF")), "pdf");
    }
}
