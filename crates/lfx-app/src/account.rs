//! W4-account: Benutzer-Einstellungen, Konten/Server und Verwaltung (nur Administratoren) als eine Hostseite.
//!
//! Rust hält Zustand, Fokus und Daten; die Oberfläche (`pages/account.slint`) zeigt nur die Zeilenliste
//! (`AccountState.rows`) und meldet Ereignisse. Die Seite hat zwei Fokuszonen: Seitenleiste (0) und Inhalt (1).
//! Netzwerk, Aktionen und Ladevorgänge stehen in `account_net.rs`, Zeilenaufbau in `account_rows.rs`.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use lfx_api::{
    ActivityLogEntry, DeviceInfo, LogFile, SessionInfo, SystemInfo, TaskInfo, UserConfiguration,
    UserDto, VirtualFolder,
};
use lfx_nav::{InputContext, NavAction};
use lfx_ui::{AcRow, AccountState};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::account_logic::{AdmSec, RowKind, SetSec};
use crate::app::App;
use crate::route::Route;

/// Unteransicht innerhalb eines Bereichs.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum View {
    #[default]
    Root,
    Server(String),
    Session(String),
    User(String),
    NewUser,
    Log(String),
}

/// Was eine Zeile bei Eingabe tut.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    None,
    Design,
    DataSaver,
    Local(&'static str),
    CfgToggle(crate::account_logic::CfgToggle),
    CfgChoice(crate::account_logic::CfgChoice),
    Field(&'static str),
    ChangePassword,
    UploadAvatar,
    RemoveAvatar,
    QcAuthorize,
    ServerOpen(String),
    ServerAdd,
    ServerDiscover,
    ServerAddFound(String),
    ServerWho(String),
    ServerRemove(String),
    UserSwitch,
    RemoveAccount,
    SignOut,
    SessionOpen(String),
    SessionMsg(String),
    SessionPause(String, bool),
    SessionStop(String),
    UserOpen(String),
    UserNew,
    UserCreate,
    Policy(String, usize),
    UserDelete(String),
    LibScanAll,
    LibScan(String),
    TaskToggle(String, bool),
    LogOpen(String),
    DeviceDelete(String),
    Restart,
    Shutdown,
    OpenBrowser,
    Refresh,
    Back,
}

impl Act {
    /// Text und Beschriftung der Bestätigung, falls die Aktion eine braucht.
    pub fn confirm(&self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::ServerRemove(_) => (
                "Diesen Server samt allen gespeicherten Anmeldungen und lokalen Daten von diesem Gerät entfernen?",
                "Entfernen",
            ),
            Self::RemoveAccount => (
                "Dieses Konto von diesem Gerät entfernen? Die Anmeldung und alle lokalen Daten dazu werden gelöscht.",
                "Entfernen",
            ),
            Self::SignOut => ("Von diesem Server abmelden?", "Abmelden"),
            Self::UserDelete(_) => ("Diesen Benutzer endgültig vom Server löschen?", "Löschen"),
            Self::DeviceDelete(_) => ("Dieses Gerät vom Server abmelden?", "Abmelden"),
            Self::Restart => (
                "Den Server neu starten? Laufende Wiedergaben werden unterbrochen.",
                "Neu starten",
            ),
            Self::Shutdown => (
                "Den Server herunterfahren? Er muss danach von Hand wieder gestartet werden.",
                "Herunterfahren",
            ),
            _ => return None,
        })
    }
}

/// Eine Zeile der Inhaltsliste.
#[derive(Clone, Debug)]
pub struct Row {
    pub kind: RowKind,
    pub label: String,
    pub value: String,
    pub hint: String,
    pub on: bool,
    pub danger: bool,
    pub enabled: bool,
    pub secret: bool,
    pub act: Act,
}

impl Row {
    fn new(kind: RowKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
            value: String::new(),
            hint: String::new(),
            on: false,
            danger: false,
            enabled: true,
            secret: false,
            act: Act::None,
        }
    }
    pub fn heading(l: impl Into<String>) -> Self {
        Self::new(RowKind::Heading, l)
    }
    pub fn note(l: impl Into<String>) -> Self {
        Self::new(RowKind::Note, l)
    }
    pub fn info(l: impl Into<String>, v: impl Into<String>) -> Self {
        let mut r = Self::new(RowKind::Info, l);
        r.value = v.into();
        r
    }
    pub fn toggle(l: impl Into<String>, on: bool, act: Act) -> Self {
        let mut r = Self::new(RowKind::Toggle, l);
        r.on = on;
        r.act = act;
        r
    }
    pub fn choice(l: impl Into<String>, v: impl Into<String>, act: Act) -> Self {
        let mut r = Self::new(RowKind::Choice, l);
        r.value = v.into();
        r.act = act;
        r
    }
    pub fn button(l: impl Into<String>, act: Act) -> Self {
        let mut r = Self::new(RowKind::Button, l);
        r.act = act;
        r
    }
    pub fn danger(l: impl Into<String>, act: Act) -> Self {
        let mut r = Self::button(l, act);
        r.danger = true;
        r
    }
    pub fn field(
        l: impl Into<String>,
        key: &'static str,
        value: String,
        placeholder: &str,
        secret: bool,
    ) -> Self {
        let mut r = Self::new(RowKind::Field, l);
        r.value = value;
        r.hint = placeholder.to_owned();
        r.secret = secret;
        r.act = Act::Field(key);
        r
    }
    pub fn item(
        l: impl Into<String>,
        hint: impl Into<String>,
        value: impl Into<String>,
        act: Act,
    ) -> Self {
        let mut r = Self::new(RowKind::Item, l);
        r.hint = hint.into();
        r.value = value.into();
        r.act = act;
        r
    }
    pub fn off(mut self) -> Self {
        self.enabled = false;
        self
    }
    pub fn focusable(&self) -> bool {
        self.kind.focusable() && self.enabled
    }
}

/// Bestätigungsdialog.
#[derive(Clone, Debug)]
pub struct Confirm {
    pub text: String,
    pub yes: String,
    pub act: Act,
}

/// Zustand der Seite (nur UI-Thread).
#[derive(Default)]
pub struct AccountCtl {
    pub admin_mode: bool,
    pub section: usize,
    pub view: View,
    /// 0 Seitenleiste, 1 Inhalt.
    pub zone: u8,
    pub focus: usize,
    pub rows: Vec<Row>,
    pub fields: HashMap<&'static str, String>,
    pub editing: Option<usize>,
    pub edit_backup: String,
    pub confirm: Option<Confirm>,
    pub confirm_focus: usize,
    /// Zählt Seitenaufrufe und Bereichswechsel; veraltete Antworten werden verworfen.
    pub epoch: u64,
    pub pending: HashSet<&'static str>,
    pub status: String,
    pub status_error: bool,
    pub timer: slint::Timer,
    pub opened_url: Option<String>,
    // Daten (None = noch nicht geladen)
    pub me: Option<UserDto>,
    pub cfg: Option<UserConfiguration>,
    pub qc_enabled: Option<bool>,
    pub discovered: Vec<lfx_api::DiscoveredServer>,
    pub discovering: bool,
    pub info: Option<SystemInfo>,
    pub sessions: Option<Vec<SessionInfo>>,
    pub users: Option<Vec<UserDto>>,
    pub folders: Option<Vec<VirtualFolder>>,
    pub tasks: Option<Vec<TaskInfo>>,
    pub activity: Option<Vec<ActivityLogEntry>>,
    pub logs: Option<Vec<LogFile>>,
    pub log_lines: Option<Vec<String>>,
    pub devices: Option<Vec<DeviceInfo>>,
}

impl AccountCtl {
    /// Setzt alles auf den Anfangszustand einer frisch geöffneten Seite.
    pub fn reset(&mut self, admin_mode: bool) {
        let epoch = self.epoch + 1;
        let timer = std::mem::take(&mut self.timer);
        *self = Self {
            admin_mode,
            epoch,
            timer,
            ..Self::default()
        };
    }

    pub fn first_focusable(&self) -> Option<usize> {
        self.rows.iter().position(Row::focusable)
    }

    fn next_focusable(&self, from: usize, dir: i32) -> Option<usize> {
        let mut i = from as i64 + i64::from(dir);
        while i >= 0 && (i as usize) < self.rows.len() {
            if self.rows[i as usize].focusable() {
                return Some(i as usize);
            }
            i += i64::from(dir);
        }
        None
    }
}

impl App {
    // ---------------------------------------------------------------- Eintritt und Austritt

    /// Baut die Seite für `Route::Settings` (false) bzw. `Route::Admin` (true).
    pub fn render_account(self: &Rc<Self>, admin: bool) {
        let is_admin = self.session.active().is_some_and(|a| a.is_admin);
        let mode = admin && is_admin;
        if admin && !is_admin {
            self.toast(
                "Die Verwaltung ist nur für Administratoren.",
                lfx_ui::ToastKind::Error,
            );
            self.st.borrow_mut().router.replace(Route::Settings);
        }
        self.st.borrow_mut().account.reset(mode);
        self.st.borrow_mut().graph = lfx_nav::FocusGraph::new();
        self.acc_start_timer();
        self.acc_ensure_data();
        self.acc_rebuild();
    }

    /// Räumt beim Verlassen der Seite auf.
    pub fn account_leave(self: &Rc<Self>) {
        let mut st = self.st.borrow_mut();
        st.account.timer.stop();
        st.account.epoch += 1;
        st.account.editing = None;
        st.account.confirm = None;
    }

    fn acc_start_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.st.borrow().account.timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(5),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.acc_tick();
                }
            },
        );
    }

    pub fn on_account_page(&self) -> bool {
        matches!(
            self.st.borrow().router.current(),
            Route::Settings | Route::Admin
        )
    }

    // ---------------------------------------------------------------- Bereiche

    pub fn acc_set_secs(&self) -> Vec<SetSec> {
        SetSec::list(self.session.active().is_some_and(|a| a.is_admin))
    }

    pub fn acc_section_names(&self) -> Vec<&'static str> {
        if self.st.borrow().account.admin_mode {
            AdmSec::ALL.iter().map(|s| s.label()).collect()
        } else {
            self.acc_set_secs().iter().map(|s| s.label()).collect()
        }
    }

    /// Aktueller Einstellungs-Bereich (nicht im Admin-Modus).
    pub fn acc_set_sec(&self) -> Option<SetSec> {
        let i = {
            let st = self.st.borrow();
            if st.account.admin_mode {
                return None;
            }
            st.account.section
        };
        self.acc_set_secs().get(i).copied()
    }

    /// Aktueller Verwaltungs-Bereich.
    pub fn acc_adm_sec(&self) -> Option<AdmSec> {
        let st = self.st.borrow();
        st.account
            .admin_mode
            .then(|| AdmSec::ALL.get(st.account.section).copied())
            .flatten()
    }

    /// Wählt einen Bereich (Seitenleiste), lädt bei Bedarf Daten.
    pub fn acc_select_section(self: &Rc<Self>, i: usize) {
        let n = self.acc_section_names().len();
        if n == 0 {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.account;
            a.section = i.min(n - 1);
            a.view = View::Root;
            a.focus = 0;
            a.editing = None;
            a.status.clear();
            a.status_error = false;
            a.epoch += 1;
            a.pending.clear();
            a.invalidate_section();
        }
        self.acc_ensure_data();
        self.acc_rebuild();
    }

    // ---------------------------------------------------------------- Anzeige

    /// Baut die Zeilen neu und schreibt alles in die Oberfläche.
    pub fn acc_rebuild(self: &Rc<Self>) {
        let rows = self.acc_build_rows();
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.account;
            a.rows = rows;
            if a.zone == 1 {
                if a.focus >= a.rows.len() || !a.rows[a.focus].focusable() {
                    match a
                        .next_focusable(a.focus.min(a.rows.len()), -1)
                        .or_else(|| a.first_focusable())
                    {
                        Some(f) => a.focus = f,
                        None => a.zone = 0,
                    }
                }
            } else if a.focus >= a.rows.len() {
                a.focus = 0;
            }
        }
        self.acc_push();
    }

    pub fn acc_push(self: &Rc<Self>) {
        if !self.on_account_page() {
            return;
        }
        let names = self.acc_section_names();
        let (title, subtitle, crumb) = self.acc_titles();
        let ui = self.win();
        let g = ui.global::<AccountState>();
        let st = self.st.borrow();
        let a = &st.account;
        g.set_title(title.into());
        g.set_subtitle(subtitle.into());
        g.set_crumb(crumb.into());
        g.set_sections(ModelRc::new(VecModel::from(
            names
                .iter()
                .map(|s| SharedString::from(*s))
                .collect::<Vec<_>>(),
        )));
        g.set_section_current(a.section as i32);
        let rows: Vec<AcRow> = a
            .rows
            .iter()
            .map(|r| AcRow {
                kind: r.kind as i32,
                label: r.label.as_str().into(),
                value: r.value.as_str().into(),
                hint: r.hint.as_str().into(),
                on: r.on,
                danger: r.danger,
                enabled: r.enabled,
                secret: r.secret,
            })
            .collect();
        g.set_rows(ModelRc::new(VecModel::from(rows)));
        g.set_focus_zone(i32::from(a.zone));
        g.set_focus_row(a.focus as i32);
        g.set_editing(a.editing.map_or(-1, |e| e as i32));
        g.set_loading(!a.pending.is_empty() || a.discovering);
        g.set_status(a.status.as_str().into());
        g.set_status_error(a.status_error);
        match &a.confirm {
            Some(c) => {
                g.set_confirm_open(true);
                g.set_confirm_text(c.text.as_str().into());
                g.set_confirm_yes(c.yes.as_str().into());
                g.set_confirm_focus(a.confirm_focus as i32);
            }
            None => g.set_confirm_open(false),
        }
        drop(st);
        self.acc_push_preview(&g);
    }

    fn acc_push_preview(&self, g: &AccountState<'_>) {
        let get = |k: &str, d: &str| -> String {
            self.local_get(k)
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| d.to_owned())
        };
        let size =
            crate::account_logic::preview_size(&get(crate::account_logic::KEY_SUB_SIZE, "m"));
        let (r, gr, b) =
            crate::account_logic::preview_rgb(&get(crate::account_logic::KEY_SUB_COLOR, "white"));
        g.set_preview_size(size);
        g.set_preview_color(slint::Color::from_rgb_u8(r, gr, b));
        g.set_preview_bg(crate::account_logic::preview_bg(&get(
            crate::account_logic::KEY_SUB_BG,
            "shadow",
        )));
    }

    fn acc_titles(&self) -> (String, String, String) {
        let user = self
            .session
            .active()
            .map(|a| a.user_name)
            .unwrap_or_default();
        let (admin, view, info_name) = {
            let st = self.st.borrow();
            (
                st.account.admin_mode,
                st.account.view.clone(),
                st.account
                    .info
                    .as_ref()
                    .map(|i| i.server_name.clone())
                    .unwrap_or_default(),
            )
        };
        let crumb = match &view {
            View::Root => String::new(),
            View::Server(id) => self
                .session
                .servers()
                .into_iter()
                .find(|s| &s.id == id)
                .map(|s| s.name)
                .unwrap_or_default(),
            View::Session(id) => {
                let st = self.st.borrow();
                st.account
                    .sessions
                    .as_ref()
                    .and_then(|v| v.iter().find(|s| &s.id == id))
                    .and_then(|s| s.user_name.clone())
                    .unwrap_or_default()
            }
            View::User(id) => {
                let st = self.st.borrow();
                st.account
                    .users
                    .as_ref()
                    .and_then(|v| v.iter().find(|u| &u.id == id))
                    .map(|u| u.name.clone())
                    .unwrap_or_default()
            }
            View::NewUser => "Neuer Benutzer".to_owned(),
            View::Log(name) => name.clone(),
        };
        if admin {
            ("Verwaltung".to_owned(), info_name, crumb)
        } else {
            ("Einstellungen".to_owned(), user, crumb)
        }
    }

    // ---------------------------------------------------------------- Eingabe

    /// Nimmt eine Navigationsaktion auf den Konto-Seiten entgegen. `true` = verbraucht.
    pub fn account_handle_action(self: &Rc<Self>, action: NavAction) -> bool {
        if !self.on_account_page() {
            return false;
        }
        // Bestätigungsdialog
        let confirm_open = self.st.borrow().account.confirm.is_some();
        if confirm_open {
            match action {
                NavAction::Left | NavAction::Up => self.st.borrow_mut().account.confirm_focus = 0,
                NavAction::Right | NavAction::Down => {
                    self.st.borrow_mut().account.confirm_focus = 1;
                }
                NavAction::Activate => {
                    let yes = self.st.borrow().account.confirm_focus == 1;
                    self.acc_confirm(yes);
                    return true;
                }
                NavAction::Back => {
                    self.acc_confirm(false);
                    return true;
                }
                _ => {}
            }
            self.acc_push();
            return true;
        }
        // Texteingabe
        let editing = self.st.borrow().account.editing.is_some();
        if editing {
            match action {
                NavAction::Activate => self.acc_finish_edit(true),
                NavAction::Back => self.acc_finish_edit(false),
                _ => {}
            }
            return true;
        }
        let zone = self.st.borrow().account.zone;
        match action {
            NavAction::Back => {
                self.acc_back();
                true
            }
            NavAction::Up | NavAction::Down | NavAction::PageUp | NavAction::PageDown => {
                let step = match action {
                    NavAction::Up => -1,
                    NavAction::Down => 1,
                    NavAction::PageUp => -6,
                    _ => 6,
                };
                if zone == 0 {
                    self.acc_move_section(step.clamp(-1, 1));
                } else {
                    self.acc_move_focus(step);
                }
                true
            }
            NavAction::Left => {
                if zone == 1 {
                    let kind = self.acc_focus_kind();
                    if matches!(kind, Some(RowKind::Choice | RowKind::Toggle)) {
                        self.acc_adjust(self.st.borrow().account.focus, -1);
                    } else {
                        self.st.borrow_mut().account.zone = 0;
                        self.acc_push();
                    }
                }
                true
            }
            NavAction::Right => {
                if zone == 0 {
                    self.acc_enter_content();
                } else {
                    let kind = self.acc_focus_kind();
                    if matches!(kind, Some(RowKind::Choice | RowKind::Toggle)) {
                        self.acc_adjust(self.st.borrow().account.focus, 1);
                    }
                }
                true
            }
            NavAction::Activate => {
                if zone == 0 {
                    self.acc_side_activate();
                } else {
                    let f = self.st.borrow().account.focus;
                    self.acc_row_activate(f);
                }
                true
            }
            _ => false,
        }
    }

    fn acc_focus_kind(&self) -> Option<RowKind> {
        let st = self.st.borrow();
        st.account.rows.get(st.account.focus).map(|r| r.kind)
    }

    fn acc_move_section(self: &Rc<Self>, dir: i32) {
        let n = self.acc_section_names().len() as i32;
        let cur = self.st.borrow().account.section as i32;
        let next = (cur + dir).clamp(0, n - 1);
        if next != cur {
            self.acc_select_section(next as usize);
        }
    }

    fn acc_move_focus(self: &Rc<Self>, step: i32) {
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.account;
            let dir = step.signum();
            let mut f = a.focus;
            for _ in 0..step.abs() {
                match a.next_focusable(f, dir) {
                    Some(n) => f = n,
                    None => break,
                }
            }
            a.focus = f;
        }
        self.acc_push();
    }

    fn acc_enter_content(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.account;
            if let Some(f) = a.first_focusable() {
                a.zone = 1;
                a.focus = f;
            }
        }
        self.acc_push();
    }

    /// Eingabe in der Seitenleiste: Inhalt betreten oder Aktion des Eintrags.
    fn acc_side_activate(self: &Rc<Self>) {
        if let Some(adm) = self.acc_adm_sec() {
            if adm == AdmSec::Back {
                self.acc_leave_admin();
            } else {
                self.acc_enter_content();
            }
            return;
        }
        match self.acc_set_sec() {
            Some(SetSec::Admin) => self.go(Route::Admin),
            Some(SetSec::About) => self.go(Route::About),
            Some(SetSec::SignOut) => self.acc_exec(Act::SignOut),
            Some(_) => self.acc_enter_content(),
            None => {}
        }
    }

    fn acc_leave_admin(self: &Rc<Self>) {
        let depth = self.st.borrow().router.depth();
        if depth > 1 {
            self.go_back();
        } else {
            self.go_replace(Route::Settings);
        }
    }

    /// Zurück: Unteransicht schließen, sonst Zone wechseln bzw. Seite verlassen.
    pub fn acc_back(self: &Rc<Self>) {
        let (view, zone) = {
            let st = self.st.borrow();
            (st.account.view.clone(), st.account.zone)
        };
        if view != View::Root {
            self.acc_pop_view();
        } else if zone == 1 {
            self.st.borrow_mut().account.zone = 0;
            self.acc_push();
        } else {
            let depth = self.st.borrow().router.depth();
            if depth > 1 {
                self.go_back();
            } else {
                self.go_reset(Route::Home);
            }
        }
    }

    pub fn acc_pop_view(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.account.view = View::Root;
            st.account.focus = 0;
            st.account.editing = None;
            st.account.log_lines = None;
        }
        self.acc_rebuild();
        // Fokus auf die erste bedienbare Zeile
        let mut st = self.st.borrow_mut();
        if let Some(f) = st.account.first_focusable() {
            st.account.focus = f;
        }
        drop(st);
        self.acc_push();
    }

    pub fn acc_open_view(self: &Rc<Self>, view: View) {
        {
            let mut st = self.st.borrow_mut();
            st.account.view = view;
            st.account.zone = 1;
            st.account.focus = 0;
            st.account.editing = None;
        }
        self.acc_ensure_data();
        self.acc_rebuild();
        let mut st = self.st.borrow_mut();
        if let Some(f) = st.account.first_focusable() {
            st.account.focus = f;
        }
        drop(st);
        self.acc_push();
    }

    // ---------------------------------------------------------------- Zeilen

    pub fn acc_row_activate(self: &Rc<Self>, i: usize) {
        let row = self.st.borrow().account.rows.get(i).cloned();
        let Some(row) = row else { return };
        if !row.focusable() {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            st.account.zone = 1;
            st.account.focus = i;
        }
        match row.kind {
            RowKind::Toggle | RowKind::Choice => self.acc_adjust(i, 1),
            RowKind::Field => self.acc_begin_edit(i),
            _ => {
                if let Some((text, yes)) = row.act.confirm() {
                    {
                        let mut st = self.st.borrow_mut();
                        st.account.confirm = Some(Confirm {
                            text: text.to_owned(),
                            yes: yes.to_owned(),
                            act: row.act.clone(),
                        });
                        st.account.confirm_focus = 0;
                    }
                    self.acc_push();
                } else {
                    self.acc_exec(row.act);
                }
            }
        }
    }

    /// Frage vor einer Aktion (auch aus der Seitenleiste).
    pub fn acc_ask(self: &Rc<Self>, act: Act) {
        if let Some((text, yes)) = act.confirm() {
            {
                let mut st = self.st.borrow_mut();
                st.account.confirm = Some(Confirm {
                    text: text.to_owned(),
                    yes: yes.to_owned(),
                    act,
                });
                st.account.confirm_focus = 0;
            }
            self.acc_push();
        } else {
            self.acc_exec(act);
        }
    }

    pub fn acc_confirm(self: &Rc<Self>, yes: bool) {
        let c = self.st.borrow_mut().account.confirm.take();
        self.acc_push();
        if yes && let Some(c) = c {
            self.acc_do(c.act);
        }
    }

    // ---------------------------------------------------------------- Texteingabe

    fn acc_begin_edit(self: &Rc<Self>, i: usize) {
        let (key, value) = {
            let st = self.st.borrow();
            let Some(Row {
                act: Act::Field(key),
                ..
            }) = st.account.rows.get(i)
            else {
                return;
            };
            (
                *key,
                st.account.fields.get(key).cloned().unwrap_or_default(),
            )
        };
        {
            let mut st = self.st.borrow_mut();
            st.account.editing = Some(i);
            st.account.edit_backup = value.clone();
            st.ctx = InputContext::TextInput;
        }
        let ui = self.win();
        let g = ui.global::<AccountState>();
        g.set_edit_text(value.into());
        self.acc_push();
        let weak = self.ui.clone();
        let _ = key;
        slint::Timer::single_shot(Duration::from_millis(30), move || {
            if let Some(ui) = weak.upgrade() {
                let g = ui.global::<AccountState>();
                g.set_edit_tick(g.get_edit_tick() + 1);
            }
        });
    }

    pub fn acc_finish_edit(self: &Rc<Self>, save: bool) {
        let ui = self.win();
        let text = ui.global::<AccountState>().get_edit_text().to_string();
        {
            let mut st = self.st.borrow_mut();
            let a = &mut st.account;
            if let Some(i) = a.editing.take()
                && let Some(Row {
                    act: Act::Field(key),
                    ..
                }) = a.rows.get(i)
            {
                let key = *key;
                if save {
                    a.fields.insert(key, text);
                } else {
                    let b = a.edit_backup.clone();
                    a.fields.insert(key, b);
                }
            }
            st.ctx = InputContext::Browse;
        }
        ui.invoke_focus_root();
        self.acc_rebuild();
        if save {
            // Weiter zur nächsten bedienbaren Zeile (Formulare ausfüllen)
            self.acc_move_focus(1);
        }
    }

    /// Setzt ein Feld direkt (Automation und Tests).
    pub fn acc_set_field(self: &Rc<Self>, key: &str, value: &str) -> bool {
        let known = [
            "pw_cur",
            "pw_new",
            "pw_rep",
            "avatar_path",
            "qc_code",
            "new_user",
            "new_pass",
            "msg_text",
        ];
        let Some(k) = known.iter().find(|k| **k == key) else {
            return false;
        };
        self.st
            .borrow_mut()
            .account
            .fields
            .insert(k, value.to_owned());
        self.acc_rebuild();
        true
    }

    // ---------------------------------------------------------------- Ereignisse der Oberfläche

    /// Ereignis aus der Slint-Seite.
    pub fn acc_event(self: &Rc<Self>, kind: &str, a: i32, b: i32) {
        match kind {
            "side" => {
                let i = a.max(0) as usize;
                self.acc_select_section(i);
                self.st.borrow_mut().account.zone = 0;
                let action_item = self.acc_adm_sec() == Some(AdmSec::Back)
                    || matches!(
                        self.acc_set_sec(),
                        Some(SetSec::Admin | SetSec::About | SetSec::SignOut)
                    );
                if action_item {
                    self.acc_side_activate();
                } else {
                    self.acc_push();
                }
            }
            "row" => self.acc_row_activate(a.max(0) as usize),
            "adjust" => self.acc_adjust(a.max(0) as usize, b),
            "accept" => self.acc_finish_edit(true),
            "confirm" => self.acc_confirm(a == 1),
            _ => {}
        }
    }
}

impl AccountCtl {
    /// Verwirft die Daten des aktuellen Bereichs, damit sie neu geladen werden.
    pub fn invalidate_section(&mut self) {
        self.log_lines = None;
        if self.admin_mode {
            match AdmSec::ALL.get(self.section) {
                Some(AdmSec::Overview) => {
                    self.info = None;
                    self.sessions = None;
                }
                Some(AdmSec::Users) => self.users = None,
                Some(AdmSec::Libraries) => self.folders = None,
                Some(AdmSec::Tasks) => self.tasks = None,
                Some(AdmSec::Activity) => self.activity = None,
                Some(AdmSec::Logs) => self.logs = None,
                Some(AdmSec::Devices) => self.devices = None,
                _ => {}
            }
        } else {
            self.qc_enabled = None;
            self.discovered.clear();
        }
    }
}
