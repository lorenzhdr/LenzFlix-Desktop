//! Server hinzufügen, „Wer schaut?“ und Anmeldung (Passwort und Quick Connect).

use std::rc::Rc;

use lfx_nav::{FocusGraph, Target, ZoneId};
use lfx_session::{QuickConnectState, UserChoice};
use lfx_ui::AvatarData;
use slint::{Color, Image, ModelRc, VecModel};

use crate::app::{App, post};
use crate::focus;
use crate::route::Route;

const PALETTE: [u32; 6] = [0x3a6ea5, 0x9b3d3d, 0x4b8f5a, 0x7a5ea8, 0xb0782c, 0x2e8c8c];

fn avatar_for(u: &UserChoice) -> AvatarData {
    let hash = u
        .name
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let c = PALETTE[hash as usize % PALETTE.len()];
    AvatarData {
        name: u.name.as_str().into(),
        initial: u
            .name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into(),
        color: Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8),
        image: Image::default(),
        has_image: false,
    }
}

impl App {
    // ---------------------------------------------------------------- Server

    pub fn submit_server(self: &Rc<Self>, url: String) {
        let url = url.trim().to_owned();
        let ui = self.win();
        if url.is_empty() {
            ui.set_add_error("Bitte eine Serveradresse eingeben.".into());
            return;
        }
        ui.set_add_error("".into());
        ui.set_add_busy(true);
        let session = self.session.clone();
        self.rt.spawn(async move {
            let result = session.add_server(&url).await;
            post(move |app| {
                let ui = app.win();
                ui.set_add_busy(false);
                match result {
                    Ok(entry) => {
                        tracing::info!("Server hinzugefügt");
                        app.show_profiles(entry.id);
                    }
                    Err(e) => ui.set_add_error(e.to_string().into()),
                }
            });
        });
    }

    // ---------------------------------------------------------------- Profile

    /// Zeigt „Wer schaut?“ für einen Server (lädt die Benutzerliste).
    pub fn show_profiles(self: &Rc<Self>, server_id: String) {
        {
            let mut st = self.st.borrow_mut();
            st.server_id = Some(server_id.clone());
            st.users.clear();
        }
        let name = self
            .session
            .servers()
            .into_iter()
            .find(|s| s.id == server_id)
            .map(|s| s.name)
            .unwrap_or_default();
        let ui = self.win();
        ui.set_server_name(name.as_str().into());
        ui.set_avatars(ModelRc::new(VecModel::<AvatarData>::default()));
        ui.set_profile_busy(true);
        self.go_reset(Route::Profiles {
            server: server_id.clone(),
        });
        let session = self.session.clone();
        self.rt.spawn(async move {
            let users = session.users_for(&server_id).await;
            post(move |app| {
                let ui = app.win();
                ui.set_profile_busy(false);
                match users {
                    Ok(users) => app.profiles_loaded(users),
                    Err(e) => ui.set_profile_error(e.to_string().into()),
                }
            });
        });
    }

    fn profiles_loaded(self: &Rc<Self>, users: Vec<UserChoice>) {
        let avatars: Vec<AvatarData> = users.iter().map(avatar_for).collect();
        self.st.borrow_mut().users = users;
        self.win()
            .set_avatars(ModelRc::new(VecModel::from(avatars)));
        if matches!(self.st.borrow().router.current(), Route::Profiles { .. }) {
            self.render_profiles_graph();
            self.show_focus();
        }
    }

    pub fn render_profiles_graph(self: &Rc<Self>) {
        let n = self.st.borrow().users.len();
        let mut g: FocusGraph = focus::profiles_graph(n);
        if n > 0 {
            g.focus_zone(ZoneId(0));
        } else {
            g.focus_zone(ZoneId(1));
        }
        self.st.borrow_mut().graph = g;
    }

    pub fn profile_slot_activated(self: &Rc<Self>, slot: usize) {
        let (users, server) = {
            let st = self.st.borrow();
            (st.users.clone(), st.server_id.clone())
        };
        let Some(server) = server else { return };
        let n = users.len();
        if slot < n {
            let user = users[slot].clone();
            if user.signed_in {
                self.switch_user(server, user.id);
            } else {
                self.show_login(server, Some(user.name));
            }
        } else if slot == n {
            self.show_login(server, None);
        } else {
            self.go_reset(Route::AddServer);
        }
    }

    fn switch_user(self: &Rc<Self>, server: String, user_id: String) {
        let ui = self.win();
        ui.set_profile_busy(true);
        ui.set_profile_error("".into());
        let session = self.session.clone();
        self.rt.spawn(async move {
            let r = session.switch_user(&server, &user_id).await;
            post(move |app| {
                let ui = app.win();
                ui.set_profile_busy(false);
                match r {
                    Ok(_) => app.login_ok(),
                    Err(e) => ui.set_profile_error(e.to_string().into()),
                }
            });
        });
    }

    // ---------------------------------------------------------------- Anmeldung

    pub fn show_login(self: &Rc<Self>, server: String, user: Option<String>) {
        let ui = self.win();
        ui.set_login_user(user.clone().unwrap_or_default().as_str().into());
        ui.set_login_password("".into());
        self.go(Route::Login { server, user });
    }

    pub fn submit_login(self: &Rc<Self>, user: String, password: String) {
        let ui = self.win();
        let Some(server) = self.st.borrow().server_id.clone() else {
            return;
        };
        if user.trim().is_empty() {
            ui.set_login_error("Bitte einen Benutzernamen eingeben.".into());
            return;
        }
        ui.set_login_error("".into());
        ui.set_login_busy(true);
        let session = self.session.clone();
        self.rt.spawn(async move {
            let r = session
                .login_password(&server, user.trim(), &password)
                .await;
            post(move |app| {
                let ui = app.win();
                ui.set_login_busy(false);
                match r {
                    Ok(_) => app.login_ok(),
                    Err(e) => ui.set_login_error(e.to_string().into()),
                }
            });
        });
    }

    pub fn start_quick_connect(self: &Rc<Self>) {
        let Some(server) = self.st.borrow().server_id.clone() else {
            return;
        };
        let ui = self.win();
        ui.set_login_error("".into());
        ui.set_login_busy(true);
        ui.set_qc_hint("Bestätige den Code in einer angemeldeten Sitzung.".into());
        let session = self.session.clone();
        let task = self.rt.spawn(async move {
            let mut handle = match session.login_quick_connect(&server) {
                Ok(h) => h,
                Err(e) => {
                    let text = e.to_string();
                    post(move |app| {
                        let ui = app.win();
                        ui.set_login_busy(false);
                        ui.set_login_error(text.into());
                    });
                    return;
                }
            };
            while let Some(state) = handle.next().await {
                match state {
                    QuickConnectState::Code(c) => post(move |app| app.win().set_qc_code(c.into())),
                    QuickConnectState::Waiting => {}
                    QuickConnectState::Authorized => {
                        post(|app| app.login_ok());
                        return;
                    }
                    QuickConnectState::Failed(text) => {
                        post(move |app| {
                            let ui = app.win();
                            ui.set_login_busy(false);
                            ui.set_qc_code("".into());
                            ui.set_login_error(text.into());
                        });
                        return;
                    }
                }
            }
        });
        self.st.borrow_mut().qc_task = Some(task.abort_handle());
    }

    /// Anmeldung gelungen: Einstellungen laden, Startseite zeigen.
    pub fn login_ok(self: &Rc<Self>) {
        let Some(a) = self.session.active() else {
            return;
        };
        tracing::info!("angemeldet");
        let ui = self.win();
        ui.set_login_password("".into());
        ui.set_login_busy(false);
        self.reset_home();
        self.on_session_ready(a.server_id.clone(), a.user_id.clone());
        self.st.borrow_mut().server_id = Some(a.server_id);
        self.go_reset(Route::Home);
    }

    /// Hilfsfunktion für Tests: Fokus auf Slot setzen.
    pub fn focus_slot(self: &Rc<Self>, zone: u32, index: usize) {
        let _ = self.st.borrow_mut().graph.set_focus(Target {
            zone: ZoneId(zone),
            index,
        });
        self.show_focus();
    }
}
