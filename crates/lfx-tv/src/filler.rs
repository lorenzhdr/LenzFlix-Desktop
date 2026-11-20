//! Füller (Lücke bis 30 min): Trailer der nächsten Sendung am Ende der Lücke, davor Pausenbild
//! (Port von `player/filler.js`). Zweistufig, weil das Laden des Trailers IO ist:
//! [`trailer_target`] nennt den Titel, dessen Trailer zu laden ist, [`pick_filler`] entscheidet.

use std::fmt;

use lfx_tv_schedule::{EntryType, PlanEntry};

/// Adresse mit Schlüssel: Debug zeigt sie nie.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretUrl(pub String);

impl fmt::Debug for SecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretUrl(***)")
    }
}

/// Geladener Trailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailerInfo {
    pub url: SecretUrl,
    pub dur_ms: Option<i64>,
}

/// Entscheidung für einen Füller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillerPick {
    /// Trailer spielen; `offset_ms` relativ zum Trailer.
    Trailer { url: SecretUrl, offset_ms: i64 },
    /// Pausenkarte; `trailer_start_ms` = Zeitpunkt des Trailerstarts, dann neu fragen.
    Card { trailer_start_ms: Option<i64> },
}

/// Titel (Id), dessen Trailer für diesen Füller zu laden ist.
pub fn trailer_target<'a>(entry: &PlanEntry, next: Option<&'a PlanEntry>) -> Option<&'a str> {
    if entry.kind != EntryType::Filler {
        return None;
    }
    next.and_then(|n| n.item_id.as_deref())
}

/// Der Trailer läuft deterministisch im Fenster `[entry.end - dur, entry.end)`.
pub fn pick_filler(entry: &PlanEntry, trailer: Option<&TrailerInfo>, now: i64) -> FillerPick {
    let card = FillerPick::Card {
        trailer_start_ms: None,
    };
    if entry.kind != EntryType::Filler {
        return card;
    }
    let Some(t) = trailer else { return card };
    let Some(dur) = t.dur_ms.filter(|d| *d > 0) else {
        return card;
    };
    if t.url.0.is_empty() || dur > entry.end - entry.start {
        return card;
    }
    let start_at = entry.end - dur;
    if now >= start_at && now < entry.end {
        FillerPick::Trailer {
            url: t.url.clone(),
            offset_ms: now - start_at,
        }
    } else if now < start_at {
        FillerPick::Card {
            trailer_start_ms: Some(start_at),
        }
    } else {
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filler(start: i64, end: i64) -> PlanEntry {
        PlanEntry {
            start,
            end,
            kind: EntryType::Filler,
            reason: None,
            item_id: None,
            series_id: None,
            rerun: false,
            premiere: None,
            theme: None,
            age: None,
            label: None,
        }
    }

    fn trailer(dur: Option<i64>) -> TrailerInfo {
        TrailerInfo {
            url: SecretUrl("http://x/t?ApiKey=geheim".into()),
            dur_ms: dur,
        }
    }

    #[test]
    fn trailer_im_fenster_mit_offset() {
        let e = filler(0, 600_000);
        let t = trailer(Some(120_000));
        assert_eq!(
            pick_filler(&e, Some(&t), 500_000),
            FillerPick::Trailer {
                url: t.url.clone(),
                offset_ms: 20_000
            }
        );
        assert_eq!(
            pick_filler(&e, Some(&t), 480_000),
            FillerPick::Trailer {
                url: t.url.clone(),
                offset_ms: 0
            }
        );
    }

    #[test]
    fn vor_dem_fenster_karte_mit_startzeit_danach_karte() {
        let e = filler(0, 600_000);
        let t = trailer(Some(120_000));
        assert_eq!(
            pick_filler(&e, Some(&t), 100_000),
            FillerPick::Card {
                trailer_start_ms: Some(480_000)
            }
        );
        assert_eq!(
            pick_filler(&e, Some(&t), 600_000),
            FillerPick::Card {
                trailer_start_ms: None
            }
        );
    }

    #[test]
    fn karte_ohne_oder_zu_langen_trailer() {
        let e = filler(0, 100_000);
        for t in [
            None,
            Some(trailer(None)),
            Some(trailer(Some(0))),
            Some(trailer(Some(200_000))),
        ] {
            assert_eq!(
                pick_filler(&e, t.as_ref(), 50_000),
                FillerPick::Card {
                    trailer_start_ms: None
                }
            );
        }
        let mut prog = filler(0, 100_000);
        prog.kind = EntryType::Program;
        assert_eq!(
            pick_filler(&prog, Some(&trailer(Some(10))), 99_990),
            FillerPick::Card {
                trailer_start_ms: None
            }
        );
    }

    #[test]
    fn ziel_nur_bei_fuellern_mit_folgetitel() {
        let e = filler(0, 10);
        let mut n = filler(10, 20);
        assert_eq!(trailer_target(&e, Some(&n)), None);
        n.item_id = Some("m2".into());
        assert_eq!(trailer_target(&e, Some(&n)), Some("m2"));
        let mut prog = n.clone();
        prog.kind = EntryType::Program;
        assert_eq!(trailer_target(&prog, Some(&n)), None);
        assert_eq!(trailer_target(&e, None), None);
    }

    #[test]
    fn debug_zeigt_adresse_nicht() {
        let t = trailer(Some(1));
        assert!(!format!("{t:?}").contains("geheim"));
    }
}
