//! Messanzeige fürs Umschalten (Port der Textlogik von `ui/zapstats.js`): vier Textzeilen und der
//! Durchschnitt der letzten 10 erfolgreichen Starts. Kein DOM; nie URLs, Schlüssel oder Item-Ids.

use std::collections::VecDeque;

use lfx_tv_schedule::jsval::round_js;

use crate::format::{js_to_fixed, sec};
use crate::request::Timing;

const AVG_MAX: usize = 10;

fn sec1(ms: Option<f64>) -> String {
    ms.map_or_else(|| "–".to_string(), sec)
}

fn sgn(s: Option<f64>) -> String {
    match s {
        Some(v) if v.is_finite() => {
            format!(
                "{}{}",
                if v < 0.0 { '-' } else { '+' },
                js_to_fixed(v.abs(), 1).replace('.', ",")
            )
        }
        _ => "–".to_string(),
    }
}

fn range(r: Option<[f64; 2]>) -> String {
    r.map_or_else(
        || "–".to_string(),
        |[a, b]| format!("{}…{}", sgn(Some(a)), sgn(Some(b))),
    )
}

/// Zeile 3: Quelle, Grund, Sprung, Puffer, Warten. Leer, wenn `timing` keine Zusatzfelder trägt.
pub fn detail_line(t: Option<&Timing>) -> String {
    let Some(t) = t else { return String::new() };
    let Some(jumped) = t.jumped else {
        return String::new();
    };
    let mut p: Vec<String> = Vec::new();
    if let Some(s) = &t.src {
        let q = [s.container.as_str(), s.v.as_str(), s.a.as_str()]
            .iter()
            .filter(|x| !x.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("·");
        if !q.is_empty() {
            p.push(format!(
                "Quelle {q}{}",
                if s.fmt.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", s.fmt)
                }
            ));
        }
        if !s.reasons.is_empty() {
            p.push(format!("Grund {}", s.reasons.join(",")));
        }
    }
    if t.forced {
        p.push("HLS erzwungen".into());
    }
    p.push(format!("Sprung {}", if jumped { "ja" } else { "nein" }));
    if t.dev_s.is_some_and(f64::is_finite) {
        p.push(format!("Abw {}", sgn(t.dev_s)));
    }
    p.push(format!("Seek {}/{}", sec1(t.seeking_ms), sec1(t.seeked_ms)));
    p.push(format!("canplay {}", sec1(t.canplay_ms)));
    p.push(format!(
        "Puffer {} → {}",
        range(t.buf_seeked),
        range(t.buf_playing)
    ));
    let wait = if t.wait_count > 0 {
        format!(" {} s", sec(t.wait_ms))
    } else {
        String::new()
    };
    p.push(format!("Warten {}×{wait}", t.wait_count));
    p.join(" · ")
}

/// Zeile 4 (Vorwärmen), nur wenn `timing.warm` vorhanden.
pub fn warm_line(t: Option<&Timing>) -> String {
    let Some(t) = t else { return String::new() };
    let Some(w) = &t.warm else {
        return String::new();
    };
    let res = match &t.res {
        Some(r) => format!(
            "Res {}/{}",
            r.n,
            r.first_seg_ms
                .map_or_else(|| "–".to_string(), |ms| format!("{} s", sec(ms)))
        ),
        None => "Res –".to_string(),
    };
    if !w.on {
        return format!("Vorwärmen aus · {res}");
    }
    let vw: Vec<String> = w.started.map_or_else(Vec::new, |s| {
        vec![format!("Vw {}/{s}", w.done.unwrap_or(0))]
    });
    let mut p: Vec<String> = vec!["Vorwärmen an".into()];
    if !w.used {
        p.push("genutzt nein".into());
        p.extend(vw);
        p.push(format!(
            "Grund {}",
            w.skip.as_deref().filter(|s| !s.is_empty()).unwrap_or("–")
        ));
        p.push(res);
        return p.join(" · ");
    }
    p.push(
        if w.partial {
            "genutzt teilweise"
        } else {
            "genutzt ja"
        }
        .into(),
    );
    p.push(format!(
        "Alter {}",
        w.age_ms.map_or_else(
            || "–".to_string(),
            |a| format!("{} s", round_js(a / 1000.0) as i64)
        )
    ));
    p.push(format!("Liste {}", sec1(w.playlist_ms)));
    p.push(format!("Init {}", sec1(w.init_ms)));
    p.push(format!("Seg 1. Byte {}", sec1(w.seg_first_byte_ms)));
    p.extend(vw);
    p.push(res);
    p.join(" · ")
}

/// Meldung eines Umschaltens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZapInfo {
    pub channel: Option<i64>,
    /// Ab Tastendruck inklusive Entprellung.
    pub key_ms: Option<f64>,
    pub timing: Option<Timing>,
    /// `false` bei Fehler (Standard: Erfolg).
    pub failed: bool,
    pub code: Option<String>,
    pub reason: Option<String>,
    pub hint: Option<String>,
    /// Gesamtzeit, falls `timing` fehlt.
    pub total_ms: Option<f64>,
}

/// Ton-Diagnose.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioInfo {
    pub mode: String,
    /// Reglerwert 0 bis 100.
    pub level: Option<f64>,
    pub muted: Option<bool>,
    pub volume: Option<f64>,
}

/// Zustand der Messanzeige.
#[derive(Debug, Default)]
pub struct ZapStats {
    hist: VecDeque<f64>,
    audio: AudioInfo,
    l1: String,
    l3: String,
    l4: String,
}

impl ZapStats {
    /// Leere Anzeige.
    pub fn new() -> Self {
        Self::default()
    }

    /// Meldet ein Umschalten.
    pub fn update(&mut self, z: &ZapInfo) {
        let t = z.timing.as_ref();
        let mut parts: Vec<String> = vec![format!(
            "Sender {}",
            z.channel.map_or_else(|| "?".to_string(), |c| c.to_string())
        )];
        if z.failed {
            // Der Code trägt bei M<n>/B-PLAY die Kette schon: nicht doppelt anhängen.
            let code = z.code.clone().unwrap_or_default();
            let ch = t
                .map(|t| t.chain.clone())
                .filter(|c| !c.is_empty() && !code.contains(c.as_str()))
                .unwrap_or_default();
            let both = [code.as_str(), ch.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            parts.push("FEHLER".into());
            parts.push(if both.is_empty() {
                z.reason
                    .clone()
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| "?".into())
            } else {
                both
            });
            if let Some(h) = z
                .hint
                .clone()
                .filter(|h| !h.is_empty())
                .or_else(|| t.and_then(|t| t.hint.clone()).filter(|h| !h.is_empty()))
            {
                parts.push(h);
            }
            if let Some(t) = t {
                parts.push(format!("{} s", sec(t.total_ms)));
            }
        } else {
            let total = t.map(|t| t.total_ms).or(z.total_ms);
            if let Some(tot) = total {
                self.hist.push_back(tot);
                if self.hist.len() > AVG_MAX {
                    self.hist.pop_front();
                }
            }
            parts.push(format!("{} s", total.map_or_else(|| "?".to_string(), sec)));
            if let Some(t) = t {
                if !t.chain.is_empty() {
                    parts.push(t.chain.clone());
                }
                parts.push(format!(
                    "Info {}{}",
                    sec(t.info_ms.unwrap_or(0.0)),
                    if t.cached { " Cache" } else { "" }
                ));
                if let Some(l) = t.load_ms {
                    parts.push(format!("Laden {}", sec(l)));
                }
                if let Some(s) = t.start_ms {
                    parts.push(format!("Start {}", sec(s)));
                }
            }
            if let Some(k) = z.key_ms {
                parts.push(format!("ab Taste {} s", sec(k)));
            }
            if !self.hist.is_empty() {
                let avg = self.hist.iter().sum::<f64>() / self.hist.len() as f64;
                parts.push(format!("Ø {} s ({})", sec(avg), self.hist.len()));
            }
        }
        self.l1 = parts.join(" · ");
        self.l3 = detail_line(t);
        self.l4 = warm_line(t);
    }

    /// Setzt die Ton-Diagnose.
    pub fn update_audio(&mut self, info: AudioInfo) {
        if !info.mode.is_empty() {
            self.audio.mode = info.mode;
        }
        if info.level.is_some() {
            self.audio.level = info.level;
        }
        self.audio.muted = info.muted;
        self.audio.volume = info.volume;
    }

    /// Zeile 2: Ton.
    pub fn audio_line(&self) -> String {
        let mode = if self.audio.mode.is_empty() {
            "?"
        } else {
            self.audio.mode.as_str()
        };
        let muted = self
            .audio
            .muted
            .map_or("?", |m| if m { "ja" } else { "nein" });
        let vol = self
            .audio
            .volume
            .map_or_else(|| "?".to_string(), |v| js_to_fixed(v, 2).replace('.', ","));
        let level = self
            .audio
            .level
            .map_or_else(|| "?".to_string(), |l| (round_js(l) as i64).to_string());
        format!("Ton: {mode} · muted {muted} · vol {vol} · Regler {level}")
    }

    /// Alle vier Zeilen.
    pub fn lines(&self) -> [String; 4] {
        [
            self.l1.clone(),
            self.audio_line(),
            self.l3.clone(),
            self.l4.clone(),
        ]
    }

    /// Durchschnitt der letzten erfolgreichen Starts (ms) und Anzahl.
    pub fn average(&self) -> Option<(f64, usize)> {
        if self.hist.is_empty() {
            None
        } else {
            Some((
                self.hist.iter().sum::<f64>() / self.hist.len() as f64,
                self.hist.len(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{ResInfo, SrcInfo, WarmInfo};

    fn timing(total: f64) -> Timing {
        Timing {
            total_ms: total,
            info_ms: Some(300.0),
            load_ms: Some(1100.0),
            start_ms: Some(200.0),
            chain: "D".into(),
            cached: true,
            mode: "direct".into(),
            ..Timing::default()
        }
    }

    #[test]
    fn zeile_1_mit_phasen_cache_taste_und_durchschnitt() {
        let mut z = ZapStats::new();
        z.update(&ZapInfo {
            channel: Some(4),
            key_ms: Some(2100.0),
            timing: Some(timing(1800.0)),
            ..ZapInfo::default()
        });
        assert_eq!(
            z.lines()[0],
            "Sender 4 · 1,8 s · D · Info 0,3 Cache · Laden 1,1 · Start 0,2 · ab Taste 2,1 s · Ø 1,8 s (1)"
        );
    }

    #[test]
    fn durchschnitt_der_letzten_zehn_fehler_zaehlen_nicht() {
        let mut z = ZapStats::new();
        for i in 0..12 {
            z.update(&ZapInfo {
                channel: Some(1),
                total_ms: Some(1000.0 + f64::from(i) * 100.0),
                ..ZapInfo::default()
            });
        }
        assert_eq!(z.average().unwrap().1, 10);
        z.update(&ZapInfo {
            channel: Some(2),
            failed: true,
            code: Some("M3·F>T>T".into()),
            timing: Some(Timing {
                chain: "F>T>T".into(),
                total_ms: 5000.0,
                ..Timing::default()
            }),
            ..ZapInfo::default()
        });
        assert_eq!(z.average().unwrap().1, 10);
        assert_eq!(z.lines()[0], "Sender 2 · FEHLER · M3·F>T>T · 5,0 s");
    }

    #[test]
    fn fehler_ohne_code_haengt_kette_an() {
        let mut z = ZapStats::new();
        z.update(&ZapInfo {
            failed: true,
            code: Some("V-CODEC".into()),
            hint: Some("Bildformat h264 HDR".into()),
            timing: Some(Timing {
                chain: "F".into(),
                total_ms: 1500.0,
                ..Timing::default()
            }),
            ..ZapInfo::default()
        });
        assert_eq!(
            z.lines()[0],
            "Sender ? · FEHLER · V-CODEC F · Bildformat h264 HDR · 1,5 s"
        );
    }

    #[test]
    fn zeile_2_ton() {
        let mut z = ZapStats::new();
        assert_eq!(z.audio_line(), "Ton: ? · muted ? · vol ? · Regler ?");
        z.update_audio(AudioInfo {
            mode: "native".into(),
            level: Some(45.0),
            muted: Some(false),
            volume: Some(0.45),
        });
        assert_eq!(
            z.audio_line(),
            "Ton: native · muted nein · vol 0,45 · Regler 45"
        );
    }

    #[test]
    fn zeile_3_quelle_sprung_puffer() {
        let mut t = timing(1000.0);
        assert_eq!(detail_line(Some(&t)), "", "ohne Zusatzfelder leer");
        t.jumped = Some(false);
        t.src = Some(SrcInfo {
            container: "mkv".into(),
            v: "hevc".into(),
            a: "eac3".into(),
            fmt: "Main 10 L153 10 Bit 3840×1600".into(),
            reasons: vec!["Container".into()],
        });
        t.dev_s = Some(0.3);
        t.seeking_ms = Some(0.0);
        t.seeked_ms = Some(300.0);
        t.canplay_ms = Some(500.0);
        t.buf_seeked = Some([0.0, 2.0]);
        t.buf_playing = Some([0.0, 12.0]);
        t.wait_count = 2;
        t.wait_ms = 1800.0;
        assert_eq!(
            detail_line(Some(&t)),
            "Quelle mkv·hevc·eac3 · Main 10 L153 10 Bit 3840×1600 · Grund Container · Sprung nein · Abw +0,3 · Seek 0,0/0,3 · canplay 0,5 · Puffer +0,0…+2,0 → +0,0…+12,0 · Warten 2× 1,8 s"
        );
        t.forced = true;
        t.wait_count = 0;
        t.seeking_ms = None;
        assert!(detail_line(Some(&t)).contains("HLS erzwungen"));
        assert!(detail_line(Some(&t)).contains("Seek –/0,3"));
        assert!(detail_line(Some(&t)).ends_with("Warten 0×"));
    }

    #[test]
    fn zeile_4_vorwaermen() {
        let mut t = timing(1000.0);
        assert_eq!(warm_line(Some(&t)), "");
        t.warm = Some(WarmInfo {
            on: false,
            ..WarmInfo::default()
        });
        assert_eq!(warm_line(Some(&t)), "Vorwärmen aus · Res –");
        t.warm = Some(WarmInfo {
            on: true,
            used: false,
            skip: Some("kette".into()),
            started: Some(3),
            done: Some(2),
            ..WarmInfo::default()
        });
        t.res = Some(ResInfo {
            n: 3,
            first_seg_ms: Some(5100.0),
        });
        assert_eq!(
            warm_line(Some(&t)),
            "Vorwärmen an · genutzt nein · Vw 2/3 · Grund kette · Res 3/5,1 s"
        );
        t.warm = Some(WarmInfo {
            on: true,
            used: true,
            age_ms: Some(12_400.0),
            playlist_ms: Some(300.0),
            init_ms: Some(200.0),
            seg_first_byte_ms: Some(4800.0),
            ..WarmInfo::default()
        });
        assert_eq!(
            warm_line(Some(&t)),
            "Vorwärmen an · genutzt ja · Alter 12 s · Liste 0,3 · Init 0,2 · Seg 1. Byte 4,8 · Res 3/5,1 s"
        );
        if let Some(w) = t.warm.as_mut() {
            w.partial = true;
        }
        assert!(warm_line(Some(&t)).contains("genutzt teilweise"));
    }

    #[test]
    fn keine_adressen_in_der_ausgabe() {
        let mut z = ZapStats::new();
        z.update(&ZapInfo {
            channel: Some(1),
            timing: Some(timing(10.0)),
            ..ZapInfo::default()
        });
        for l in z.lines() {
            assert!(!l.contains("http") && !l.contains("ApiKey"));
        }
    }
}
