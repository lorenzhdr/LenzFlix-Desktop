//! Kommandozeile: `--home <dir>`, `--server <url>`, `--design flix|plus|prime|retro`, `--keyring`.

use std::path::PathBuf;

/// Design-Wahl (entspricht dem Slint-`Design`-Enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DesignId {
    #[default]
    Flix,
    Plus,
    Prime,
    Retro,
}

impl DesignId {
    /// Aus der Kennung (`flix`, `plus`, `prime`, `retro`).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "flix" => Self::Flix,
            "plus" => Self::Plus,
            "prime" => Self::Prime,
            "retro" => Self::Retro,
            _ => return None,
        })
    }

    /// Kennung für Einstellungen und Automation.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Flix => "flix",
            Self::Plus => "plus",
            Self::Prime => "prime",
            Self::Retro => "retro",
        }
    }

    /// Index wie im Slint-Enum (`flix, plus, prime, retro`).
    pub fn index(self) -> i32 {
        self as i32
    }

    /// Aus dem Index der Einstellungsseite.
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Plus,
            2 => Self::Prime,
            3 => Self::Retro,
            _ => Self::Flix,
        }
    }
}

/// Aufrufoptionen.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Isolierter App-Ordner (`config/`, `cache/`, `data/` darunter); ohne Angabe die OS-Ordner.
    pub home: Option<PathBuf>,
    /// Server vorbelegen.
    pub server: Option<String>,
    /// Design festlegen (sonst gespeicherte Wahl, sonst LENZFLIX).
    pub design: Option<DesignId>,
    /// Echten Schlüsselbund auch bei `--home` benutzen (sonst im Speicher, nichts bleibt liegen).
    pub keyring: bool,
    /// Abstand der Fortschrittsmeldungen (nur Entwicklung; Standard 10 s).
    pub report_interval: Option<std::time::Duration>,
    /// Fenster maximiert starten (Tests, Screenshots).
    pub maximized: bool,
    /// Fenstergröße in logischen Pixeln (Standard 1280x800).
    pub size: Option<(f32, f32)>,
}

/// Wertet die Argumente aus (ohne Programmnamen). Fehler als deutscher Text.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .ok_or_else(|| format!("{name} braucht einen Wert"))
        };
        match a.as_str() {
            "--home" => o.home = Some(PathBuf::from(value("--home")?)),
            "--server" => o.server = Some(value("--server")?),
            "--design" => {
                let v = value("--design")?;
                o.design = Some(DesignId::parse(&v).ok_or_else(|| {
                    format!("unbekanntes Design „{v}“ (flix, plus, prime, retro)")
                })?);
            }
            "--keyring" => o.keyring = true,
            "--maximized" => o.maximized = true,
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| format!("ungültige Größe „{v}“ (z. B. 1100x640)"))?;
                let (w, h): (f32, f32) = (
                    w.parse().map_err(|_| format!("ungültige Breite „{w}“"))?,
                    h.parse().map_err(|_| format!("ungültige Höhe „{h}“"))?,
                );
                o.size = Some((w.max(640.0), h.max(420.0)));
            }
            "--report-interval-ms" => {
                let v = value("--report-interval-ms")?;
                let ms: u64 = v.parse().map_err(|_| format!("ungültige Zahl „{v}“"))?;
                o.report_interval = Some(std::time::Duration::from_millis(ms.max(100)));
            }
            other => return Err(format!("unbekanntes Argument: {other}")),
        }
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn full_line() {
        let o = parse(args(&[
            "--home",
            "x/y",
            "--server",
            "http://a",
            "--design",
            "retro",
            "--keyring",
        ]))
        .unwrap();
        assert_eq!(o.home.unwrap(), PathBuf::from("x/y"));
        assert_eq!(o.server.as_deref(), Some("http://a"));
        assert_eq!(o.design, Some(DesignId::Retro));
        assert!(o.keyring);
    }

    #[test]
    fn errors() {
        assert!(parse(args(&["--design", "blau"])).is_err());
        assert!(parse(args(&["--home"])).is_err());
        assert!(parse(args(&["--quatsch"])).is_err());
        assert!(parse(args(&[])).unwrap().home.is_none());
    }

    #[test]
    fn design_indices() {
        for d in [
            DesignId::Flix,
            DesignId::Plus,
            DesignId::Prime,
            DesignId::Retro,
        ] {
            assert_eq!(DesignId::from_index(d.index()), d);
            assert_eq!(DesignId::parse(d.slug()), Some(d));
        }
    }
}
