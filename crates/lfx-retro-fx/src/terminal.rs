//! Geheimes Terminal (Port von `terminal-commands.js` und dem Verlauf aus `terminal.js`).
//! Parser, Vervollständigung und Befehle sind UI-frei: [`execute`] liefert eine Liste von
//! [`TermStep`] (Ausgabe, Wartezeit, Aktion), die die UI nacheinander abspielt. Schließt der
//! Benutzer das Terminal, verwirft die UI die restlichen Schritte (im Web liefert `sleep` dann
//! `false`). Alles ist Großschrift-Text; nie Tokens, IDs oder URLs mit Schlüsseln ausgeben.

use crate::rng::Rng;

/// Eingabeaufforderung.
pub const PROMPT: &str = "LENZ84:\\>";
/// Höchstzahl Zeilen im Terminal und Einträge im Verlauf.
pub const MAX_LINES: usize = 400;
pub const MAX_HISTORY: usize = 50;

/// Befehlsname -> kanonischer Name (Web: `ALIASES`).
pub const ALIASES: &[(&str, &str)] = &[
    ("hilfe", "hilfe"),
    ("help", "hilfe"),
    ("?", "hilfe"),
    ("whoami", "whoami"),
    ("datum", "datum"),
    ("date", "datum"),
    ("uhr", "uhr"),
    ("time", "uhr"),
    ("ls", "ls"),
    ("dir", "ls"),
    ("suche", "suche"),
    ("search", "suche"),
    ("spiele", "spiele"),
    ("play", "spiele"),
    ("zufall", "zufall"),
    ("random", "zufall"),
    ("design", "design"),
    ("theme", "design"),
    ("sound", "sound"),
    ("crt", "crt"),
    ("regen", "regen"),
    ("rain", "regen"),
    ("overdrive", "overdrive"),
    ("hack", "hack"),
    ("reboot", "reboot"),
    ("clear", "clear"),
    ("cls", "clear"),
    ("exit", "exit"),
    ("quit", "exit"),
    ("lenz", "lenz"),
    ("sudo", "sudo"),
    ("42", "42"),
    ("hallo", "hallo"),
    ("hello", "hallo"),
];

/// Befehle der Hilfe (versteckte fehlen absichtlich).
pub const HELP_LINES: &[(&str, &str)] = &[
    ("HILFE", "DIESE LISTE"),
    ("WHOAMI", "WER BIN ICH"),
    ("DATUM / UHR", "DATUM UND UHRZEIT"),
    ("LS", "BIBLIOTHEKEN AUFLISTEN"),
    ("SUCHE <TEXT>", "MEDIATHEK DURCHSUCHEN"),
    ("SPIELE <TITEL>", "KASSETTE LADEN"),
    ("ZUFALL", "ZUFÄLLIGER FILM"),
    ("DESIGN <ID>", "FLIX | PLUS | PRIME | RETRO"),
    ("SOUND AN|AUS", "TON SCHALTEN"),
    ("CRT AN|AUS", "BILDSCHIRM-EFFEKT"),
    ("REGEN", "CODE-REGEN STARTEN"),
    ("OVERDRIVE", "FARBRAUSCH"),
    ("HACK", "ZUGRIFF ERZWINGEN (SPIEL)"),
    ("REBOOT", "NEU STARTEN"),
    ("LENZ", "LOGO"),
    ("CLEAR", "BILDSCHIRM LEEREN"),
    ("EXIT", "TERMINAL SCHLIESSEN"),
];

/// ASCII-Logo des Befehls `lenz`.
pub const ASCII_LOGO: &[&str] = &[
    " _      _____  _   _  _____    __    __  ___   _  _",
    "| |    | ____|| \\ | ||__  /   / /   / / ( _ ) | || |",
    "| |    |  _|  |  \\| |  / /   / /   / /  / _ \\ | || |_",
    "| |___ | |___ | |\\  | / /_  / /   / /  | (_) ||__   _|",
    "|_____||_____||_| \\_|/____|/_/   /_/    \\___/    |_|",
];

const WEEKDAYS: [&str; 7] = [
    "SONNTAG",
    "MONTAG",
    "DIENSTAG",
    "MITTWOCH",
    "DONNERSTAG",
    "FREITAG",
    "SAMSTAG",
];

/// Zerlegte Ortszeit (die UI liefert sie; hier keine Zeitzonenlogik).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateParts {
    /// 0 = Sonntag.
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
    pub year: i32,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

pub fn format_date(d: &DateParts) -> String {
    format!(
        "{}, {:02}.{:02}.{}",
        WEEKDAYS[usize::from(d.weekday % 7)],
        d.day,
        d.month,
        d.year
    )
}

pub fn format_time(d: &DateParts) -> String {
    format!("{:02}:{:02}:{:02}", d.hour, d.minute, d.second)
}

/// Hex-Zeile für den Hack-Effekt (Adresse, dann 8 Bytes).
pub fn hex_line(rng: &mut Rng) -> String {
    let addr = (rng.next_f64() * f64::from(0xffff_u32)).floor() as u32;
    let bytes: Vec<String> = (0..8)
        .map(|_| format!("{:02X}", (rng.next_f64() * 256.0).floor() as u32))
        .collect();
    format!("0x{addr:04X}  {}", bytes.join(" "))
}

/// Fortschrittsbalken `[####....]  40%`.
pub fn progress_bar(pct: f64, width: usize) -> String {
    let p = pct.clamp(0.0, 100.0);
    let full = ((p / 100.0) * width as f64).round() as usize;
    format!(
        "[{}{}] {:>3}%",
        "#".repeat(full),
        ".".repeat(width - full),
        pct.round() as i64
    )
}

/// Zerlegte Eingabezeile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub raw: String,
    /// Kanonischer Name oder `None`, wenn unbekannt.
    pub cmd: Option<&'static str>,
    /// Eingegebener Name (klein).
    pub name: String,
    /// Rest der Zeile.
    pub args: String,
    pub argv: Vec<String>,
}

pub fn parse_command(input: &str) -> ParsedCommand {
    let raw = input.trim().to_string();
    let mut parts = raw.split_whitespace();
    let Some(first) = parts.next() else {
        return ParsedCommand {
            raw,
            cmd: None,
            name: String::new(),
            args: String::new(),
            argv: vec![],
        };
    };
    let name = first.to_lowercase();
    let argv: Vec<String> = parts.map(str::to_string).collect();
    let args = raw[first.len()..].trim().to_string();
    let cmd = ALIASES.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
    ParsedCommand {
        raw: raw.clone(),
        cmd,
        name,
        args,
        argv,
    }
}

/// Tab-Vervollständigung: Befehle (außer versteckten), die mit `prefix` beginnen, sortiert.
pub fn complete(prefix: &str) -> Vec<&'static str> {
    let p = prefix.to_lowercase();
    if p.is_empty() || p.chars().any(char::is_whitespace) {
        return vec![];
    }
    let mut out: Vec<&'static str> = ALIASES
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| !["sudo", "42", "hallo", "hello", "?"].contains(k) && k.starts_with(&p))
        .collect();
    out.sort_unstable();
    out
}

/// Eingabe nach Tab: eindeutig -> `name `, mehrere -> längster gemeinsamer Anfang (wenn länger als
/// die Eingabe), sonst `None` (die UI listet dann [`complete`] auf).
pub fn tab_complete(input: &str) -> Option<String> {
    let t = input.trim();
    let matches = complete(t);
    match matches.len() {
        0 => None,
        1 => Some(format!("{} ", matches[0])),
        _ => {
            let mut prefix = matches[0].to_string();
            for m in &matches {
                while !m.starts_with(&prefix) {
                    prefix.pop();
                }
            }
            (prefix.chars().count() > t.chars().count()).then_some(prefix)
        }
    }
}

/// Darstellung einer Zeile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    Normal,
    /// Eingegebener Befehl.
    Cmd,
    Err,
    Hint,
    Ok,
}

/// Aktionen, die die App ausführt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermAction {
    SetTheme(String),
    SetPref { name: String, on: bool },
    UnlockSound,
    Regen,
    Overdrive,
    Reboot,
}

/// Ein Schritt der Befehlsausführung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermStep {
    Print {
        text: String,
        style: LineStyle,
    },
    /// Fehlerzeile (rot) plus Klang `FxSound::Error`.
    Error(String),
    /// Letzte Zeile ersetzen (Fortschrittsbalken).
    ReplaceLast(String),
    Clear,
    Close,
    /// Zu einer Adresse navigieren (die UI wartet vorher ca. 700 ms und schließt das Terminal).
    Go(String),
    /// Warten in Millisekunden.
    Sleep(u32),
    Action(TermAction),
}

/// Treffer einer Titelsuche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleHit {
    pub id: String,
    pub name: String,
    pub year: Option<i32>,
}

/// Datenquelle für die Befehle (App-Seite). Die Aufrufe dürfen blockieren; die UI ruft
/// [`execute`] in einem Hintergrundthread auf.
pub trait TermHost {
    fn user_name(&self) -> Option<String>;
    fn libraries(&self) -> Vec<String>;
    fn search_titles(&self, query: &str) -> Vec<TitleHit>;
    fn random_title(&self) -> Option<TitleHit>;
    /// Wählbare Design-IDs (klein).
    fn themes(&self) -> Vec<String>;
    fn theme_name(&self, id: &str) -> String;
    fn current_theme(&self) -> String;
    fn pref(&self, name: &str) -> bool;
    fn now(&self) -> DateParts;
    fn search_href(&self, query: &str) -> String;
    fn title_href(&self, id: &str) -> String;
}

struct Out(Vec<TermStep>);

impl Out {
    fn say(&mut self, t: impl Into<String>) {
        self.0.push(TermStep::Print {
            text: t.into(),
            style: LineStyle::Normal,
        });
    }
    fn styled(&mut self, t: impl Into<String>, style: LineStyle) {
        self.0.push(TermStep::Print {
            text: t.into(),
            style,
        });
    }
    fn err(&mut self, t: impl Into<String>) {
        self.0.push(TermStep::Error(t.into()));
    }
    fn sleep(&mut self, ms: u32) {
        self.0.push(TermStep::Sleep(ms));
    }
}

/// Befehlszeile ausführen. Liefert zuerst die Zeile mit der Eingabeaufforderung, dann die Schritte.
/// Leere Zeilen liefern nichts.
pub fn run_line(line: &str, host: &dyn TermHost, rng: &mut Rng) -> Vec<TermStep> {
    let raw = line.trim();
    if raw.is_empty() {
        return vec![];
    }
    let mut steps = vec![TermStep::Print {
        text: format!("{PROMPT} {raw}"),
        style: LineStyle::Cmd,
    }];
    steps.extend(execute(raw, host, rng));
    steps
}

/// Befehl ohne Aufforderungszeile ausführen (Web: `execute`).
pub fn execute(line: &str, host: &dyn TermHost, rng: &mut Rng) -> Vec<TermStep> {
    let p = parse_command(line);
    let mut o = Out(Vec::new());
    if p.raw.is_empty() {
        return o.0;
    }
    match p.cmd {
        Some("hilfe") => {
            o.say("VERFÜGBARE BEFEHLE:");
            for (a, b) in HELP_LINES {
                o.say(format!("  {a:<16} {b}"));
            }
            o.styled(
                "TAB = VERVOLLSTÄNDIGEN, PFEIL AUF/AB = VERLAUF.",
                LineStyle::Hint,
            );
        }
        Some("whoami") => {
            o.say(
                host.user_name()
                    .map_or_else(|| "GAST".into(), |n| n.to_uppercase()),
            );
        }
        Some("datum") => o.say(format_date(&host.now())),
        Some("uhr") => o.say(format_time(&host.now())),
        Some("ls") => {
            let libs = host.libraries();
            if libs.is_empty() {
                o.say("KEINE BIBLIOTHEKEN GEFUNDEN.");
            } else {
                let n = libs.len();
                o.say(format!("{n} BIBLIOTHEK{}:", if n == 1 { "" } else { "EN" }));
                for l in libs {
                    o.say(format!("  <DIR>  {}", l.to_uppercase()));
                }
            }
        }
        Some("suche") => {
            if p.args.is_empty() {
                o.err("GIB EINEN SUCHBEGRIFF AN: SUCHE <TEXT>");
            } else {
                o.say(format!("SUCHE NACH \"{}\" ...", p.args));
                o.0.push(TermStep::Go(host.search_href(&p.args)));
            }
        }
        Some("spiele") => {
            if p.args.is_empty() {
                o.err("WELCHE KASSETTE? SPIELE <TITEL>");
            } else {
                o.say(format!("SUCHE KASSETTE \"{}\" ...", p.args));
                match host.search_titles(&p.args).first() {
                    None => o.err(format!("KEINE KASSETTE GEFUNDEN: {}", p.args)),
                    Some(best) => {
                        let year = best.year.map_or(String::new(), |y| format!(" ({y})"));
                        o.say(format!("GEFUNDEN: {}{year}", best.name.to_uppercase()));
                        o.say("LADE KASSETTE ...");
                        o.0.push(TermStep::Go(host.title_href(&best.id)));
                    }
                }
            }
        }
        Some("zufall") => {
            o.say("ZIEHE ZUFALLSKASSETTE ...");
            match host.random_title() {
                None => o.err("NICHTS GEFUNDEN. IST DIE MEDIATHEK LEER?"),
                Some(pick) => {
                    o.say(format!("GEZOGEN: {}", pick.name.to_uppercase()));
                    o.say("LADE KASSETTE ...");
                    o.0.push(TermStep::Go(host.title_href(&pick.id)));
                }
            }
        }
        Some("design") => {
            let themes = host.themes();
            let id = p.argv.first().map(|s| s.to_lowercase()).unwrap_or_default();
            if id.is_empty() {
                o.say(format!(
                    "AKTUELL: {}",
                    host.theme_name(&host.current_theme())
                ));
                o.say(format!("VERFÜGBAR: {}", themes.join(" | ").to_uppercase()));
            } else if !themes.contains(&id) {
                o.err(format!(
                    "UNBEKANNTES DESIGN: {id}. VERFÜGBAR: {}",
                    themes.join(", ")
                ));
            } else if id == host.current_theme() {
                o.say("DIESES DESIGN IST BEREITS AKTIV.");
            } else {
                o.say(format!(
                    "WECHSLE ZU {} ... DAS TERMINAL WIRD GESCHLOSSEN.",
                    host.theme_name(&id)
                ));
                o.sleep(500);
                o.0.push(TermStep::Close);
                o.0.push(TermStep::Action(TermAction::SetTheme(id)));
            }
        }
        Some(c @ ("sound" | "crt")) => {
            let arg = p.argv.first().map(|s| s.to_lowercase()).unwrap_or_default();
            let label = c.to_uppercase();
            let on = ["an", "on", "1", "ein"].contains(&arg.as_str());
            let off = ["aus", "off", "0"].contains(&arg.as_str());
            if !on && !off {
                o.say(format!(
                    "{label}: {}. NUTZUNG: {label} AN|AUS",
                    if host.pref(c) { "AN" } else { "AUS" }
                ));
            } else {
                o.0.push(TermStep::Action(TermAction::SetPref { name: c.into(), on }));
                if c == "sound" && on {
                    o.0.push(TermStep::Action(TermAction::UnlockSound));
                }
                o.say(format!("{label}: {}", if on { "AN" } else { "AUS" }));
            }
        }
        Some("regen") => {
            o.say("CODE-REGEN STARTET ... TASTE ZUM BEENDEN.");
            o.sleep(400);
            o.0.push(TermStep::Close);
            o.0.push(TermStep::Action(TermAction::Regen));
        }
        Some("overdrive") => {
            o.say(">> OVERDRIVE <<");
            o.0.push(TermStep::Action(TermAction::Overdrive));
        }
        Some("hack") => hack(&mut o, rng),
        Some("reboot") => {
            o.say("SYSTEM WIRD NEU GESTARTET ...");
            o.sleep(500);
            o.0.push(TermStep::Close);
            o.0.push(TermStep::Action(TermAction::Reboot));
        }
        Some("clear") => o.0.push(TermStep::Clear),
        Some("exit") => o.0.push(TermStep::Close),
        Some("lenz") => {
            for l in ASCII_LOGO {
                o.say(*l);
            }
            o.say("LENZ//84 :: MEDIATHEK-TERMINAL");
        }
        Some("sudo") => o.err("NETTER VERSUCH."),
        Some("42") => o.say("DIE ANTWORT. NUR WIE LAUTET DIE FRAGE?"),
        Some("hallo") => {
            let n = host
                .user_name()
                .map_or(String::new(), |n| format!(", {}", n.to_uppercase()));
            o.say(format!("HALLO{n}. SCHÖN, DASS DU DA BIST."));
        }
        _ => o.err(format!("BEFEHL NICHT GEFUNDEN: {}. TIPPE 'HILFE'.", p.name)),
    }
    o.0
}

/// Rein kosmetische Fake-Hacking-Animation.
fn hack(o: &mut Out, rng: &mut Rng) {
    o.say("VERBINDE MIT MAINFRAME ...");
    o.sleep(500);
    for _ in 0..14 {
        o.say(hex_line(rng));
        o.sleep(60);
    }
    o.say(progress_bar(0.0, 24));
    for i in 0..=10 {
        o.0.push(TermStep::ReplaceLast(progress_bar(f64::from(i * 10), 24)));
        o.sleep(110);
    }
    for _ in 0..8 {
        o.say(hex_line(rng));
        o.sleep(45);
    }
    o.styled(">> ZUGRIFF GEWÄHRT <<", LineStyle::Ok);
    o.say("(ALLES NUR SPIELEREI. NIEMAND WURDE GEHACKT.)");
}

/// Eingabeverlauf mit Pfeiltasten (Web: Verlauf in `terminal.js`).
#[derive(Debug, Clone, Default)]
pub struct History {
    items: Vec<String>,
    index: usize,
    draft: String,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Befehl merken (gleiche Folgeeinträge nur einmal, höchstens 50).
    pub fn push(&mut self, line: &str) {
        let raw = line.trim();
        if raw.is_empty() {
            return;
        }
        if self.items.last().map(String::as_str) != Some(raw) {
            self.items.push(raw.to_string());
        }
        if self.items.len() > MAX_HISTORY {
            self.items.remove(0);
        }
        self.index = self.items.len();
        self.draft.clear();
    }

    /// Pfeil auf: `current` ist der Text im Feld (wird beim ersten Mal als Entwurf gemerkt).
    pub fn up(&mut self, current: &str) -> String {
        if self.index == self.items.len() {
            self.draft = current.to_string();
        }
        if self.index > 0 {
            self.index -= 1;
        }
        self.items.get(self.index).cloned().unwrap_or_default()
    }

    /// Pfeil ab: zurück bis zum Entwurf.
    pub fn down(&mut self) -> String {
        if self.index < self.items.len() {
            self.index += 1;
        }
        if self.index >= self.items.len() {
            self.draft.clone()
        } else {
            self.items[self.index].clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        hits: Vec<TitleHit>,
    }

    impl TermHost for Fake {
        fn user_name(&self) -> Option<String> {
            Some("anna".into())
        }
        fn libraries(&self) -> Vec<String> {
            vec!["Filme".into(), "Serien".into()]
        }
        fn search_titles(&self, _q: &str) -> Vec<TitleHit> {
            self.hits.clone()
        }
        fn random_title(&self) -> Option<TitleHit> {
            self.hits.first().cloned()
        }
        fn themes(&self) -> Vec<String> {
            ["flix", "plus", "prime", "retro"]
                .map(String::from)
                .to_vec()
        }
        fn theme_name(&self, id: &str) -> String {
            id.to_uppercase()
        }
        fn current_theme(&self) -> String {
            "retro".into()
        }
        fn pref(&self, name: &str) -> bool {
            name == "crt"
        }
        fn now(&self) -> DateParts {
            DateParts {
                weekday: 5,
                day: 4,
                month: 9,
                year: 2026,
                hour: 7,
                minute: 5,
                second: 3,
            }
        }
        fn search_href(&self, q: &str) -> String {
            format!("/suche?q={q}")
        }
        fn title_href(&self, id: &str) -> String {
            format!("/titel/{id}")
        }
    }

    fn host() -> Fake {
        Fake {
            hits: vec![TitleHit {
                id: "t1".into(),
                name: "Testfilm".into(),
                year: Some(1999),
            }],
        }
    }

    fn texts(steps: &[TermStep]) -> Vec<String> {
        steps
            .iter()
            .filter_map(|s| match s {
                TermStep::Print { text, .. } | TermStep::Error(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn run(line: &str) -> Vec<TermStep> {
        execute(line, &host(), &mut Rng::new(1))
    }

    #[test]
    fn parser() {
        let p = parse_command("  SPIELE  Der  Film ");
        assert_eq!(p.cmd, Some("spiele"));
        assert_eq!(p.name, "spiele");
        assert_eq!(p.args, "Der  Film");
        assert_eq!(p.argv, vec!["Der", "Film"]);
        assert_eq!(parse_command("play x").cmd, Some("spiele"));
        assert_eq!(parse_command("quatsch").cmd, None);
        assert_eq!(parse_command("   ").name, "");
        assert_eq!(parse_command("toString").cmd, None);
    }

    #[test]
    fn vervollstaendigung() {
        assert_eq!(complete("he"), vec!["help"]);
        assert_eq!(complete("r"), vec!["rain", "random", "reboot", "regen"]);
        assert!(complete("sud").is_empty(), "sudo ist versteckt");
        assert!(complete("4").is_empty());
        assert!(complete("a b").is_empty());
        assert_eq!(tab_complete("wh"), Some("whoami ".into()));
        assert_eq!(tab_complete("re"), None, "gemeinsamer Anfang = Eingabe");
        assert_eq!(tab_complete("cle"), Some("clear ".into()));
        assert_eq!(tab_complete("c"), None);
    }

    #[test]
    fn hilfe_und_logo() {
        let t = texts(&run("hilfe"));
        assert_eq!(t[0], "VERFÜGBARE BEFEHLE:");
        assert_eq!(t.len(), 1 + HELP_LINES.len() + 1);
        assert_eq!(t[1], "  HILFE            DIESE LISTE");
        let l = texts(&run("lenz"));
        assert_eq!(l.len(), ASCII_LOGO.len() + 1);
        assert!(l.last().unwrap().starts_with("LENZ//84"));
    }

    #[test]
    fn datum_uhr_whoami() {
        assert_eq!(texts(&run("datum")), vec!["FREITAG, 04.09.2026"]);
        assert_eq!(texts(&run("uhr")), vec!["07:05:03"]);
        assert_eq!(texts(&run("whoami")), vec!["ANNA"]);
        assert_eq!(
            texts(&run("hallo")),
            vec!["HALLO, ANNA. SCHÖN, DASS DU DA BIST."]
        );
    }

    #[test]
    fn ls_suche_spiele() {
        assert_eq!(
            texts(&run("ls")),
            vec!["2 BIBLIOTHEKEN:", "  <DIR>  FILME", "  <DIR>  SERIEN"]
        );
        assert!(matches!(run("suche")[0], TermStep::Error(_)));
        let s = run("suche abc");
        assert_eq!(s.last(), Some(&TermStep::Go("/suche?q=abc".into())));
        let p = run("spiele test");
        assert_eq!(texts(&p)[1], "GEFUNDEN: TESTFILM (1999)");
        assert_eq!(p.last(), Some(&TermStep::Go("/titel/t1".into())));
        let none = execute("spiele x", &Fake { hits: vec![] }, &mut Rng::new(1));
        assert!(matches!(none.last(), Some(TermStep::Error(e)) if e.contains("KEINE KASSETTE")));
        assert_eq!(
            run("zufall").last(),
            Some(&TermStep::Go("/titel/t1".into()))
        );
    }

    #[test]
    fn design_und_prefs() {
        let t = texts(&run("design"));
        assert_eq!(
            t,
            vec!["AKTUELL: RETRO", "VERFÜGBAR: FLIX | PLUS | PRIME | RETRO"]
        );
        assert_eq!(
            texts(&run("design retro")),
            vec!["DIESES DESIGN IST BEREITS AKTIV."]
        );
        assert!(matches!(run("design gibtsnicht")[0], TermStep::Error(_)));
        let s = run("design plus");
        assert!(s.contains(&TermStep::Sleep(500)));
        assert_eq!(
            s.last(),
            Some(&TermStep::Action(TermAction::SetTheme("plus".into())))
        );

        assert_eq!(
            texts(&run("sound")),
            vec!["SOUND: AUS. NUTZUNG: SOUND AN|AUS"]
        );
        assert_eq!(texts(&run("crt")), vec!["CRT: AN. NUTZUNG: CRT AN|AUS"]);
        let on = run("sound an");
        assert_eq!(
            on[0],
            TermStep::Action(TermAction::SetPref {
                name: "sound".into(),
                on: true
            })
        );
        assert_eq!(on[1], TermStep::Action(TermAction::UnlockSound));
        assert_eq!(texts(&on), vec!["SOUND: AN"]);
        let off = run("crt aus");
        assert_eq!(off.len(), 2);
    }

    #[test]
    fn schliessen_und_eier() {
        assert_eq!(run("exit"), vec![TermStep::Close]);
        assert_eq!(run("cls"), vec![TermStep::Clear]);
        assert_eq!(
            texts(&run("42")),
            vec!["DIE ANTWORT. NUR WIE LAUTET DIE FRAGE?"]
        );
        assert_eq!(texts(&run("sudo make")), vec!["NETTER VERSUCH."]);
        assert_eq!(
            texts(&run("foo")),
            vec!["BEFEHL NICHT GEFUNDEN: foo. TIPPE 'HILFE'."]
        );
        assert_eq!(
            run("regen").last(),
            Some(&TermStep::Action(TermAction::Regen))
        );
        assert_eq!(
            run("reboot").last(),
            Some(&TermStep::Action(TermAction::Reboot))
        );
        assert_eq!(
            run("overdrive").last(),
            Some(&TermStep::Action(TermAction::Overdrive))
        );
        assert!(run("   ").is_empty());
    }

    #[test]
    fn eingabezeile() {
        let s = run_line("  datum ", &host(), &mut Rng::new(1));
        assert_eq!(
            s[0],
            TermStep::Print {
                text: "LENZ84:\\> datum".into(),
                style: LineStyle::Cmd
            }
        );
        assert!(run_line("", &host(), &mut Rng::new(1)).is_empty());
    }

    #[test]
    fn hack_ablauf_und_dauer() {
        let s = run("hack");
        let sleep: u32 = s
            .iter()
            .map(|x| if let TermStep::Sleep(n) = x { *n } else { 0 })
            .sum();
        assert_eq!(sleep, 500 + 14 * 60 + 11 * 110 + 8 * 45);
        let t = texts(&s);
        assert_eq!(t[0], "VERBINDE MIT MAINFRAME ...");
        assert!(t[1].starts_with("0x") && t[1].len() == "0x0000  00 00 00 00 00 00 00 00".len());
        assert!(t.contains(&">> ZUGRIFF GEWÄHRT <<".to_string()));
        assert!(s.contains(&TermStep::ReplaceLast(
            "[########################] 100%".into()
        )));
        // deterministisch
        assert_eq!(run("hack"), s);
    }

    #[test]
    fn hilfsformate() {
        assert_eq!(progress_bar(0.0, 24), format!("[{}]   0%", ".".repeat(24)));
        assert_eq!(progress_bar(50.0, 4), "[##..]  50%");
        assert_eq!(progress_bar(150.0, 4), "[####] 150%");
    }

    #[test]
    fn verlauf() {
        let mut h = History::new();
        h.push("a");
        h.push("a");
        h.push("b");
        assert_eq!(h.up("entwurf"), "b");
        assert_eq!(h.up("x"), "a");
        assert_eq!(h.up("x"), "a", "am Anfang bleibt der erste");
        assert_eq!(h.down(), "b");
        assert_eq!(h.down(), "entwurf");
        for i in 0..60 {
            h.push(&format!("c{i}"));
        }
        assert_eq!(h.items.len(), MAX_HISTORY);
        assert_eq!(h.up(""), "c59");
    }
}
