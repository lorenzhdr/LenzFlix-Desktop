//! W3-retro: Terminal-Überlagerung (Web: `fx/terminal.js`, `terminal-commands.js`).
//!
//! Die Eingabezeile hält Rust selbst (alle Tasten laufen über das Hauptfenster). Befehle werden in einem
//! Hintergrundthread ausgeführt (`lfx_retro_fx::terminal::run_line`, Daten per Server-Abfrage) und als Schritte
//! zurückgespielt; Schließen verwirft die Reste.

use std::rc::Rc;
use std::time::Duration;

use lfx_api::{Client, ItemKind, ItemsQuery, SortBy};
use lfx_retro_fx::rng::Rng;
use lfx_retro_fx::synth::FxSound;
use lfx_retro_fx::terminal::{
    DateParts, LineStyle, PROMPT, TermAction, TermHost, TermStep, TitleHit, run_line, tab_complete,
};
use lfx_ui::RetroFx;
use slint::ComponentHandle;

use crate::app::{App, post};
use crate::cli::DesignId;
use crate::retro::TERM_MAX_LINES;
use crate::route::Route;

/// Datenquelle der Befehle (läuft im Hintergrundthread, blockiert auf der Laufzeit).
struct Host {
    user: Option<String>,
    client: Option<Client>,
    rt: tokio::runtime::Handle,
    design: String,
    crt: bool,
    sound: bool,
}

impl Host {
    fn query(&self, q: ItemsQuery) -> Vec<lfx_api::BaseItem> {
        let Some(c) = self.client.clone() else {
            return Vec::new();
        };
        self.rt
            .block_on(async move { c.items(&q).await.map(|r| r.items).unwrap_or_default() })
    }
}

impl TermHost for Host {
    fn user_name(&self) -> Option<String> {
        self.user.clone()
    }
    fn libraries(&self) -> Vec<String> {
        let Some(c) = self.client.clone() else {
            return Vec::new();
        };
        self.rt.block_on(async move {
            c.views()
                .await
                .map(|r| r.items.into_iter().map(|i| i.name).collect())
                .unwrap_or_default()
        })
    }
    fn search_titles(&self, query: &str) -> Vec<TitleHit> {
        self.query(
            ItemsQuery::new()
                .search_term(query)
                .include_types(&[ItemKind::Movie, ItemKind::Series])
                .recursive(true)
                .limit(10),
        )
        .into_iter()
        .map(|i| TitleHit {
            id: i.id,
            name: i.name,
            year: i.production_year,
        })
        .collect()
    }
    fn random_title(&self) -> Option<TitleHit> {
        self.query(
            ItemsQuery::new()
                .include_types(&[ItemKind::Movie])
                .recursive(true)
                .sort_by(&[SortBy::Random])
                .limit(1),
        )
        .into_iter()
        .next()
        .map(|i| TitleHit {
            id: i.id,
            name: i.name,
            year: i.production_year,
        })
    }
    fn themes(&self) -> Vec<String> {
        ["flix", "plus", "prime", "retro"]
            .map(String::from)
            .to_vec()
    }
    fn theme_name(&self, id: &str) -> String {
        match id {
            "flix" => "LENZFLIX",
            "plus" => "LENZ+",
            "prime" => "LENZPRIME",
            "retro" => "LENZ//84",
            other => other,
        }
        .to_owned()
    }
    fn current_theme(&self) -> String {
        self.design.clone()
    }
    fn pref(&self, name: &str) -> bool {
        match name {
            "crt" => self.crt,
            "sound" => self.sound,
            _ => false,
        }
    }
    fn now(&self) -> DateParts {
        use chrono::{Datelike, Timelike};
        let n = chrono::Local::now();
        DateParts {
            weekday: n.weekday().num_days_from_sunday() as u8,
            day: n.day() as u8,
            month: n.month() as u8,
            year: n.year(),
            hour: n.hour() as u8,
            minute: n.minute() as u8,
            second: n.second() as u8,
        }
    }
    fn search_href(&self, query: &str) -> String {
        format!("search:{query}")
    }
    fn title_href(&self, id: &str) -> String {
        format!("title:{id}")
    }
}

fn style_kind(s: LineStyle) -> i32 {
    match s {
        LineStyle::Normal => 0,
        LineStyle::Cmd => 1,
        LineStyle::Err => 2,
        LineStyle::Hint => 3,
        LineStyle::Ok => 4,
    }
}

impl App {
    pub fn retro_term_open(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if !st.retro.fx_active || st.retro.term_open || st.retro.boot.is_some() {
                return;
            }
            st.retro.term_open = true;
            st.retro.term_input.clear();
            if st.retro.term_lines.is_empty() {
                st.retro
                    .term_lines
                    .push(("LENZ//84 TERMINAL. HILFE ZEIGT DIE BEFEHLE.".into(), 3));
            }
        }
        self.retro_rain_stop();
        self.retro_play(FxSound::Open);
        self.retro_push_lines();
        let win = self.win();
        let g = win.global::<RetroFx>();
        g.set_term_prompt(PROMPT.into());
        g.set_term_input("".into());
        g.set_term_caret(true);
        g.set_term_open(true);
    }

    pub fn retro_term_close(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if !st.retro.term_open {
                return;
            }
            st.retro.term_open = false;
            st.retro.term_pending.clear();
            st.retro.term_epoch += 1;
            st.retro.term_timer.stop();
        }
        self.retro_play(FxSound::Close);
        self.win().global::<RetroFx>().set_term_open(false);
    }

    fn retro_term_sync_input(&self) {
        let s = self.st.borrow().retro.term_input.clone();
        self.win().global::<RetroFx>().set_term_input(s.into());
    }

    /// Eine Taste bei offenem Terminal.
    pub fn retro_term_key(self: &Rc<Self>, text: &str) {
        use slint::platform::Key as K;
        let is = |k: K| {
            let c: char = k.into();
            text.chars().count() == 1 && text.starts_with(c)
        };
        if is(K::Escape) {
            self.retro_term_close();
        } else if is(K::Return) || text == "\n" || text == "\r" {
            self.retro_term_submit();
        } else if is(K::Backspace) {
            self.st.borrow_mut().retro.term_input.pop();
            self.retro_term_sync_input();
        } else if is(K::Tab) {
            let cur = self.st.borrow().retro.term_input.clone();
            if let Some(c) = tab_complete(&cur) {
                self.st.borrow_mut().retro.term_input = c;
                self.retro_term_sync_input();
            }
        } else if is(K::UpArrow) {
            let cur = self.st.borrow().retro.term_input.clone();
            let n = self.st.borrow_mut().retro.history.up(&cur);
            self.st.borrow_mut().retro.term_input = n;
            self.retro_term_sync_input();
        } else if is(K::DownArrow) {
            let n = self.st.borrow_mut().retro.history.down();
            self.st.borrow_mut().retro.term_input = n;
            self.retro_term_sync_input();
        } else if matches!(text, "`" | "^" | "°") {
            // Die Taste, die das Terminal öffnet, wird nicht eingefügt.
        } else if text.chars().all(|c| !c.is_control()) && !text.is_empty() {
            let mut st = self.st.borrow_mut();
            if st.retro.term_input.chars().count() < 120 {
                // Das Zeichen links vom Rand des Web-Terminals: Backquote selbst wird nicht eingefügt.
                st.retro.term_input.push_str(&text.to_uppercase());
            }
            drop(st);
            self.retro_play(FxSound::Type);
            self.retro_term_sync_input();
        }
    }

    /// Befehlszeile abschicken.
    pub fn retro_term_submit(self: &Rc<Self>) {
        let line = std::mem::take(&mut self.st.borrow_mut().retro.term_input);
        self.retro_term_sync_input();
        let line = line.trim().to_owned();
        if line.is_empty() {
            return;
        }
        self.st.borrow_mut().retro.history.push(&line);
        let (epoch, host, seed) = {
            let st = self.st.borrow();
            let active = self.session.active();
            (
                st.retro.term_epoch,
                Host {
                    user: active.as_ref().map(|a| a.user_name.clone()),
                    client: active.map(|a| a.client),
                    rt: self.rt.clone(),
                    design: st.design.slug().to_owned(),
                    crt: st.retro.crt,
                    sound: st.retro.sound,
                },
                st.retro.seed ^ (self.retro_term_counter()),
            )
        };
        std::thread::spawn(move || {
            let mut rng = Rng::new(seed);
            let steps = run_line(&line, &host, &mut rng);
            post(move |app| app.retro_term_steps(epoch, steps));
        });
    }

    fn retro_term_counter(&self) -> u32 {
        self.st.borrow().retro.term_lines.len() as u32
    }

    /// Schritte (von einem Hintergrundthread) einreihen und abspielen.
    pub fn retro_term_steps(self: &Rc<Self>, epoch: u64, steps: Vec<TermStep>) {
        {
            let mut st = self.st.borrow_mut();
            if st.retro.term_epoch != epoch || !st.retro.term_open {
                return;
            }
            st.retro.term_pending.extend(steps);
        }
        self.retro_term_play();
    }

    /// Spielt Schritte ab, bis ein Warteschritt kommt.
    pub fn retro_term_play(self: &Rc<Self>) {
        loop {
            let step = self.st.borrow_mut().retro.term_pending.pop_front();
            let Some(step) = step else { break };
            match step {
                TermStep::Print { text, style } => self.retro_term_line(text, style_kind(style)),
                TermStep::Error(t) => {
                    self.retro_play(FxSound::Error);
                    self.retro_term_line(t, 2);
                }
                TermStep::ReplaceLast(t) => {
                    let mut st = self.st.borrow_mut();
                    if let Some(l) = st.retro.term_lines.last_mut() {
                        l.0 = t;
                    }
                    drop(st);
                    self.retro_push_lines();
                }
                TermStep::Clear => {
                    self.st.borrow_mut().retro.term_lines.clear();
                    self.retro_push_lines();
                }
                TermStep::Close => {
                    self.retro_term_close();
                    break;
                }
                TermStep::Go(href) => {
                    // Wie im Web: kurz warten, Terminal schließen, dann navigieren.
                    let weak = Rc::downgrade(self);
                    self.retro_term_close();
                    slint::Timer::single_shot(Duration::from_millis(250), move || {
                        if let Some(app) = weak.upgrade() {
                            app.retro_go(&href);
                        }
                    });
                    break;
                }
                TermStep::Sleep(ms) => {
                    let weak = Rc::downgrade(self);
                    let epoch = self.st.borrow().retro.term_epoch;
                    self.st.borrow().retro.term_timer.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(u64::from(ms)),
                        move || {
                            if let Some(app) = weak.upgrade()
                                && app.st.borrow().retro.term_epoch == epoch
                            {
                                app.retro_term_play();
                            }
                        },
                    );
                    break;
                }
                TermStep::Action(a) => self.retro_term_action(a),
            }
        }
    }

    fn retro_term_line(self: &Rc<Self>, text: String, kind: i32) {
        {
            let mut st = self.st.borrow_mut();
            st.retro.term_lines.push((text, kind));
            let n = st.retro.term_lines.len();
            if n > TERM_MAX_LINES {
                st.retro.term_lines.drain(..n - TERM_MAX_LINES);
            }
        }
        self.retro_push_lines();
    }

    fn retro_term_action(self: &Rc<Self>, a: TermAction) {
        match a {
            TermAction::SetTheme(id) => {
                if let Some(d) = DesignId::parse(&id) {
                    self.choose_design(d);
                }
            }
            TermAction::SetPref { name, on } => self.retro_set_pref(&name, on),
            TermAction::UnlockSound => self.retro_set_pref("sound", true),
            TermAction::Regen => self.retro_rain_start(),
            TermAction::Overdrive => self.retro_overdrive(),
            TermAction::Reboot => {
                self.retro_term_close();
                self.retro_boot_start(true);
            }
        }
    }

    /// App-interne Adresse aus den Befehlen (`search:<Text>`, `title:<Id>`).
    fn retro_go(self: &Rc<Self>, href: &str) {
        if let Some(q) = href.strip_prefix("search:") {
            self.go(Route::Search);
            self.search_set_text(q);
        } else if let Some(id) = href.strip_prefix("title:") {
            self.go(Route::Detail { id: id.to_owned() });
        }
    }
}
