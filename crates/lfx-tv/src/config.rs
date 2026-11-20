//! Senderbelegung der Plätze 5 bis 16 (Port von `features/tvdata/config.js`).
//!
//! Quellen in dieser Reihenfolge, die erste mit nicht leerem Ergebnis gewinnt: `tvChannels`,
//! `liveChannels` (Rückfall alter Konfigurationen), CSS-Variable `--lfx-tv-channels`,
//! `--lfx-live-channels`. Format `"5=Action;6=Komödie"` oder Objekt `{ "5": "Action" }`. Das
//! Auslesen der Quellen (Einstellungen, Custom-CSS) macht die App; hier liegt Auswahl und
//! Umwandlung für den Planer.

use std::collections::BTreeMap;

use lfx_tv_schedule::JsVal;
use lfx_tv_schedule::config::parse_channel_config;

/// Rohwerte der Quellen (`None` = nicht gesetzt).
#[derive(Debug, Clone, Default)]
pub struct ChannelConfigSources {
    /// `LenzFlixConfig.tvChannels`
    pub tv_channels: Option<JsVal>,
    /// `LenzFlixConfig.liveChannels` (Rückfall)
    pub live_channels: Option<JsVal>,
    /// CSS-Variable `--lfx-tv-channels`
    pub css_tv_channels: Option<String>,
    /// CSS-Variable `--lfx-live-channels` (Rückfall)
    pub css_live_channels: Option<String>,
}

fn is_set(v: &JsVal) -> bool {
    !matches!(v, JsVal::Undefined | JsVal::Null) && *v != JsVal::Str(String::new())
}

/// Wählt die Belegung `{ Platz: Genre-Label }`.
pub fn resolve_channel_config(src: &ChannelConfigSources) -> BTreeMap<i64, String> {
    for cfg in [&src.tv_channels, &src.live_channels].into_iter().flatten() {
        if is_set(cfg) {
            let parsed = parse_channel_config(cfg);
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }
    for css in [&src.css_tv_channels, &src.css_live_channels]
        .into_iter()
        .flatten()
    {
        let parsed = parse_channel_config(&JsVal::Str(css.clone()));
        if !parsed.is_empty() {
            return parsed;
        }
    }
    BTreeMap::new()
}

/// Belegung als Objekt für `Scheduler::new` (Plätze aufsteigend).
pub fn to_planner_config(map: &BTreeMap<i64, String>) -> JsVal {
    JsVal::Obj(
        map.iter()
            .map(|(no, label)| (no.to_string(), JsVal::Str(label.clone())))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> JsVal {
        JsVal::Str(x.into())
    }

    fn obj(pairs: &[(&str, &str)]) -> JsVal {
        JsVal::Obj(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), s(v)))
                .collect(),
        )
    }

    #[test]
    fn string_plaetze_5_bis_16_quotes_und_leerraum() {
        let src = ChannelConfigSources {
            tv_channels: Some(s(
                " \"5=Action ; 6 = Komödie;4=zu klein;17=zu gross;x=1;7\" ",
            )),
            ..Default::default()
        };
        let m = resolve_channel_config(&src);
        assert_eq!(m.get(&5).map(String::as_str), Some("Action"));
        assert_eq!(m.get(&6).map(String::as_str), Some("Komödie"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn objekt_und_vorrang_vor_css() {
        let src = ChannelConfigSources {
            tv_channels: Some(obj(&[("5", "Action")])),
            css_tv_channels: Some("\"6=Krimi\"".into()),
            ..Default::default()
        };
        let m = resolve_channel_config(&src);
        assert_eq!(m.keys().copied().collect::<Vec<_>>(), vec![5]);
    }

    #[test]
    fn css_variable_als_quelle() {
        let src = ChannelConfigSources {
            css_tv_channels: Some("\"5=Action;6=Komödie\"".into()),
            ..Default::default()
        };
        assert_eq!(resolve_channel_config(&src).len(), 2);
    }

    #[test]
    fn rueckfall_alte_namen_neuer_hat_vorrang() {
        let src = ChannelConfigSources {
            live_channels: Some(s("5=Alt")),
            ..Default::default()
        };
        assert_eq!(
            resolve_channel_config(&src).get(&5).map(String::as_str),
            Some("Alt")
        );
        let src = ChannelConfigSources {
            live_channels: Some(obj(&[("8", "Alt")])),
            ..Default::default()
        };
        assert_eq!(
            resolve_channel_config(&src).get(&8).map(String::as_str),
            Some("Alt")
        );
        let src = ChannelConfigSources {
            css_live_channels: Some("5=AltCss".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_channel_config(&src).get(&5).map(String::as_str),
            Some("AltCss")
        );
        let src = ChannelConfigSources {
            tv_channels: Some(s("5=Neu")),
            live_channels: Some(s("5=Alt")),
            css_tv_channels: Some("5=Css".into()),
            css_live_channels: Some("5=AltCss".into()),
        };
        assert_eq!(
            resolve_channel_config(&src).get(&5).map(String::as_str),
            Some("Neu")
        );
        // leere/ungültige Config fällt zur nächsten Quelle durch
        let src = ChannelConfigSources {
            tv_channels: Some(s("kaputt")),
            css_tv_channels: Some("9=Doku".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_channel_config(&src).get(&9).map(String::as_str),
            Some("Doku")
        );
    }

    #[test]
    fn ohne_quelle_leer_und_planerobjekt() {
        assert!(resolve_channel_config(&ChannelConfigSources::default()).is_empty());
        let mut m = BTreeMap::new();
        m.insert(6, "Krimi".to_string());
        m.insert(5, "Action".to_string());
        let JsVal::Obj(o) = to_planner_config(&m) else {
            panic!()
        };
        assert_eq!(o[0].0, "5");
    }
}
