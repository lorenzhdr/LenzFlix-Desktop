//! Routen der App und der Router (Verlauf). Rein in Rust, ohne Oberfläche, damit testbar.

use lfx_core::library::{LibSort, LibraryKind};

/// Ansicht einer Bibliotheksseite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LibraryView {
    /// Zeilen (Standard).
    #[default]
    Rows,
    /// Raster.
    Grid,
}

/// Eine Seite der App.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    AddServer,
    Profiles {
        server: String,
    },
    Login {
        server: String,
        user: Option<String>,
    },
    Home,
    Detail {
        id: String,
    },
    Player {
        id: String,
    },
    Search,
    Library {
        kind: LibraryKind,
        genre: Option<String>,
        view: LibraryView,
        sort: LibSort,
    },
    MyList,
    NewPopular,
    /// W2-library: Sammlung, Ordner oder Bibliothek (Boxsets, Heimvideos, Mischinhalt, Wiedergabelisten).
    Collection {
        id: String,
        name: String,
    },
    TvMode,
    Settings,
    Admin,
    About,
}

impl Route {
    /// Kurzname für Automation und Protokoll (keine Ids, keine Adressen).
    pub fn name(&self) -> &'static str {
        match self {
            Self::AddServer => "add-server",
            Self::Profiles { .. } => "profiles",
            Self::Login { .. } => "login",
            Self::Home => "home",
            Self::Detail { .. } => "detail",
            Self::Player { .. } => "player",
            Self::Search => "search",
            Self::Library { .. } => "library",
            Self::MyList => "mylist",
            Self::NewPopular => "newpopular",
            Self::Collection { .. } => "collection", // W2-library
            Self::TvMode => "tvmode",
            Self::Settings => "settings",
            Self::Admin => "admin",
            Self::About => "about",
        }
    }

    /// Titel der „Folgt“-Seite für noch nicht gebaute Routen; `None` = die Route hat eine echte Seite.
    pub fn soon_title(&self) -> Option<&'static str> {
        match self {
            Self::Detail { .. } => Some("Details"),
            Self::Search => Some("Suche"),
            Self::Library {
                kind: LibraryKind::Series,
                ..
            } => Some("Serien"),
            Self::Library {
                kind: LibraryKind::Movies,
                ..
            } => Some("Filme"),
            Self::MyList => Some("Meine Liste"),
            Self::NewPopular => Some("Neu und beliebt"),
            Self::TvMode => Some("TV"),
            Self::Admin => Some("Verwaltung"),
            _ => None,
        }
    }

    /// Route aus einem Kurznamen (Automation, Navigationsleiste); nur Routen ohne Parameter.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "add-server" => Self::AddServer,
            "home" => Self::Home,
            "search" => Self::Search,
            "mylist" => Self::MyList,
            "newpopular" => Self::NewPopular,
            "tvmode" => Self::TvMode,
            "settings" => Self::Settings,
            "admin" => Self::Admin,
            "about" => Self::About,
            "series" => Self::Library {
                kind: LibraryKind::Series,
                genre: None,
                view: LibraryView::Rows,
                sort: LibSort::Az,
            },
            "movies" => Self::Library {
                kind: LibraryKind::Movies,
                genre: None,
                view: LibraryView::Rows,
                sort: LibSort::Az,
            },
            _ => return None,
        })
    }
}

/// Verlauf der Routen. `Home` ist die Wurzel nach der Anmeldung; Anmeldeseiten ersetzen den Verlauf.
#[derive(Debug)]
pub struct Router {
    stack: Vec<Route>,
}

impl Default for Router {
    fn default() -> Self {
        Self::new(Route::AddServer)
    }
}

impl Router {
    /// Neuer Router mit Startroute.
    pub fn new(start: Route) -> Self {
        Self { stack: vec![start] }
    }

    /// Aktuelle Route.
    pub fn current(&self) -> &Route {
        self.stack.last().expect("Router ist nie leer")
    }

    /// Tiefe des Verlaufs (1 = nur die aktuelle Route).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Geht zu einer Route; gleiche Route wie die aktuelle wird nicht doppelt gestapelt.
    pub fn push(&mut self, route: Route) {
        if self.current() != &route {
            self.stack.push(route);
        }
    }

    /// Ersetzt die aktuelle Route.
    pub fn replace(&mut self, route: Route) {
        self.stack.pop();
        self.stack.push(route);
    }

    /// Verwirft den Verlauf und setzt eine neue Wurzel (Anmeldung, Abmeldung).
    pub fn reset(&mut self, route: Route) {
        self.stack.clear();
        self.stack.push(route);
    }

    /// Zurück; `None`, wenn nur noch die Wurzel da ist. Liefert die neue aktuelle Route.
    pub fn back(&mut self) -> Option<&Route> {
        if self.stack.len() > 1 {
            self.stack.pop();
            Some(self.current())
        } else {
            None
        }
    }

    /// Ob die aktuelle Route eine angemeldete Sitzung voraussetzt.
    pub fn needs_session(&self) -> bool {
        !matches!(
            self.current(),
            Route::AddServer | Route::Profiles { .. } | Route::Login { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_back() {
        let mut r = Router::new(Route::Home);
        r.push(Route::Settings);
        r.push(Route::About);
        assert_eq!(r.depth(), 3);
        assert_eq!(r.back(), Some(&Route::Settings));
        assert_eq!(r.back(), Some(&Route::Home));
        assert_eq!(r.back(), None);
        assert_eq!(r.current(), &Route::Home);
    }

    #[test]
    fn same_route_is_not_stacked() {
        let mut r = Router::new(Route::Home);
        r.push(Route::Home);
        assert_eq!(r.depth(), 1);
    }

    #[test]
    fn replace_and_reset() {
        let mut r = Router::default();
        assert_eq!(r.current(), &Route::AddServer);
        r.push(Route::Profiles { server: "s".into() });
        r.replace(Route::Login {
            server: "s".into(),
            user: None,
        });
        assert_eq!(r.depth(), 2);
        r.reset(Route::Home);
        assert_eq!(r.depth(), 1);
        assert!(!r.needs_session() || r.current() == &Route::Home);
    }

    #[test]
    fn session_requirement() {
        let mut r = Router::default();
        assert!(!r.needs_session());
        r.reset(Route::Home);
        assert!(r.needs_session());
        r.push(Route::Player { id: "x".into() });
        assert!(r.needs_session());
    }

    #[test]
    fn names_round_trip() {
        for n in [
            "home",
            "search",
            "mylist",
            "newpopular",
            "tvmode",
            "settings",
            "admin",
            "about",
            "series",
            "movies",
            "add-server",
        ] {
            let r = Route::from_name(n).unwrap();
            if n != "series" && n != "movies" {
                assert_eq!(r.name(), if n == "add-server" { "add-server" } else { n });
            }
        }
        assert!(Route::from_name("gibt-es-nicht").is_none());
    }

    #[test]
    fn soon_pages() {
        assert_eq!(Route::Search.soon_title(), Some("Suche"));
        assert_eq!(Route::Home.soon_title(), None);
        assert_eq!(Route::Settings.soon_title(), None);
        assert_eq!(
            Route::from_name("movies").unwrap().soon_title(),
            Some("Filme")
        );
    }
}
