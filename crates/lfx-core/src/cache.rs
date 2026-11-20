//! Cache-Regeln der Startseite (Port von `core/cache.js`, `core/homecache.js`): Laufzeiten, Schlüssel,
//! Ablauf und Invalidierung als reine Funktionen. Die Uhr kommt von außen (`now_ms`); das Zusammenfassen
//! gleichzeitiger Abrufe (In-flight-Deduplizierung) ist Sache der asynchronen Schicht.

use indexmap::IndexMap;

/// Startseiten-Cache (Zeilen, Billboard): 10 Minuten.
pub const HOME_TTL_MS: u64 = 10 * 60_000;
/// Ansichten (`/UserViews`): 5 Minuten.
pub const VIEWS_TTL_MS: u64 = 5 * 60_000;
/// „Neue Folgen“-Set: 10 Minuten.
pub const NEW_SERIES_TTL_MS: u64 = 10 * 60_000;
/// „Neu und beliebt“-Abfragen: 5 Minuten.
pub const NEW_POPULAR_TTL_MS: u64 = 5 * 60_000;
/// Neu-Info (Badges nach Staffel/Folge): 5 Minuten.
pub const NEW_CONTENT_TTL_MS: u64 = 5 * 60_000;
/// Standard-Laufzeit von `cached()`.
pub const DEFAULT_TTL_MS: u64 = 60_000;
/// Obergrenze der Einträge; die ältesten fallen zuerst heraus.
pub const MAX_ENTRIES: usize = 500;
/// Nach Verlassen des Players wird nach so langer Zeit ein zweites Mal geleert (Stopp-Bericht kommt spät).
pub const PLAYER_LEFT_REPEAT_MS: u64 = 2_000;

/// `homeKey`: `home:<userId|anon>:<teile…>`.
pub fn home_key(user_id: &str, parts: &[&str]) -> String {
    let uid = if user_id.is_empty() { "anon" } else { user_id };
    let mut out = format!("home:{uid}");
    for p in parts {
        out.push(':');
        out.push_str(p);
    }
    out
}

/// Laufzeit einer Zeile (`wrapRow`): Ersatzzeilen (`no_cache`) werden nicht gehalten.
pub fn row_ttl_ms(no_cache: bool) -> u64 {
    if no_cache { 0 } else { HOME_TTL_MS }
}

/// Eine Zeilenliste mit fehlenden Bibliotheken/Genres/Charts („degraded“) wird nicht gecacht.
pub fn should_cache_defs(degraded: bool) -> bool {
    !degraded
}

/// Ablauf-Cache mit expliziter Uhr.
#[derive(Debug, Clone)]
pub struct TtlCache<V> {
    entries: IndexMap<String, (V, u64)>,
}

impl<V> Default for TtlCache<V> {
    fn default() -> Self {
        Self {
            entries: IndexMap::new(),
        }
    }
}

impl<V> TtlCache<V> {
    /// Leerer Cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Treffer, solange `expires > now`; abgelaufene Einträge werden entfernt (`peekCached`).
    pub fn get(&mut self, key: &str, now_ms: u64) -> Option<&V> {
        if self.entries.get(key).is_some_and(|(_, exp)| *exp <= now_ms) {
            self.entries.shift_remove(key);
            return None;
        }
        self.entries.get(key).map(|(v, _)| v)
    }

    /// Speichert mit Laufzeit; erneutes Setzen rückt den Eintrag ans Ende, über [`MAX_ENTRIES`] fallen
    /// die ältesten heraus. Laufzeit 0 speichert nichts (der Eintrag wäre sofort abgelaufen).
    pub fn insert(&mut self, key: &str, value: V, ttl_ms: u64, now_ms: u64) {
        self.entries.shift_remove(key);
        if ttl_ms == 0 {
            return;
        }
        self.entries
            .insert(key.to_owned(), (value, now_ms + ttl_ms));
        while self.entries.len() > MAX_ENTRIES {
            self.entries.shift_remove_index(0);
        }
    }

    /// `clearCache(prefix)`: ohne Präfix alles, sonst Schlüssel mit Präfix.
    pub fn clear(&mut self, prefix: Option<&str>) {
        match prefix {
            None => self.entries.clear(),
            Some(p) => self.entries.retain(|k, _| !k.starts_with(p)),
        }
    }

    /// Anzahl der Einträge (auch abgelaufene, bis sie angefragt werden).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Leer?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Anlässe, die den Startseiten-Cache betreffen (`initHomeCache`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Wiedergabe startet (`lfx:play`).
    Play,
    /// Player-Seite betreten.
    PlayerEntered,
    /// Player-Seite verlassen.
    PlayerLeft,
    /// Anderer Benutzer.
    UserChanged,
    /// Titel geändert oder gelöscht.
    ItemsChanged,
    /// Favorit/Gesehen/Fortschritt außerhalb (Server-Nachricht `UserDataChanged`).
    UserDataChanged,
    /// Favorit gesetzt (leert vor dem Aufruf und nach der Antwort).
    FavoriteSet,
    /// Gesehen gesetzt (leert vor dem Aufruf und nach der Antwort).
    PlayedSet,
}

/// Was bei einem Anlass zu tun ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalidation {
    /// Präfix zu leeren; `None` = alles.
    pub prefix: Option<&'static str>,
    /// Zusätzlich den Titel-Speicher leeren (Benutzerwechsel).
    pub clear_item_store: bool,
    /// Nach so vielen Millisekunden erneut leeren.
    pub repeat_after_ms: Option<u64>,
    /// Nach der Serverantwort noch einmal leeren (ein Abruf im Fenster hat den alten Stand gespeichert).
    pub again_after_response: bool,
}

/// Invalidierungsregel eines Anlasses.
pub fn invalidation(trigger: Trigger) -> Invalidation {
    let home = Invalidation {
        prefix: Some("home:"),
        clear_item_store: false,
        repeat_after_ms: None,
        again_after_response: false,
    };
    match trigger {
        Trigger::Play
        | Trigger::PlayerEntered
        | Trigger::ItemsChanged
        | Trigger::UserDataChanged => home,
        Trigger::PlayerLeft => Invalidation {
            repeat_after_ms: Some(PLAYER_LEFT_REPEAT_MS),
            ..home
        },
        Trigger::UserChanged => Invalidation {
            prefix: None,
            clear_item_store: true,
            ..home
        },
        Trigger::FavoriteSet | Trigger::PlayedSet => Invalidation {
            again_after_response: true,
            ..home
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(home_key("u1", &["row", "resume"]), "home:u1:row:resume");
        assert_eq!(home_key("", &["billboard"]), "home:anon:billboard");
    }

    #[test]
    fn ttl_semantics() {
        let mut c = TtlCache::new();
        c.insert("home:u:a", 1, HOME_TTL_MS, 1_000);
        assert_eq!(c.get("home:u:a", 1_000 + HOME_TTL_MS - 1), Some(&1));
        assert_eq!(
            c.get("home:u:a", 1_000 + HOME_TTL_MS),
            None,
            "expires > now"
        );
        assert!(c.is_empty(), "abgelaufen: entfernt");
        c.insert("x", 2, row_ttl_ms(true), 0);
        assert!(c.is_empty(), "no_cache speichert nicht");
    }

    #[test]
    fn evicts_oldest_and_clears_by_prefix() {
        let mut c = TtlCache::new();
        for i in 0..=MAX_ENTRIES {
            c.insert(&format!("k{i}"), i, 1_000, 0);
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        assert!(c.get("k0", 1).is_none() && c.get("k1", 1).is_some());
        c.insert("home:u:a", 1, 1_000, 0);
        c.insert("lib:u:a", 1, 1_000, 0);
        c.clear(Some("home:"));
        assert!(c.get("home:u:a", 1).is_none() && c.get("lib:u:a", 1).is_some());
        c.clear(None);
        assert!(c.is_empty());
    }

    #[test]
    fn invalidation_rules() {
        assert_eq!(invalidation(Trigger::Play).prefix, Some("home:"));
        assert_eq!(
            invalidation(Trigger::PlayerLeft).repeat_after_ms,
            Some(2_000)
        );
        let u = invalidation(Trigger::UserChanged);
        assert!(u.prefix.is_none() && u.clear_item_store);
        assert!(invalidation(Trigger::FavoriteSet).again_after_response);
        assert!(should_cache_defs(false) && !should_cache_defs(true));
    }
}
