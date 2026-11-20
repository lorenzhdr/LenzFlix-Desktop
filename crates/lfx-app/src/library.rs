//! Bibliotheksseiten (W2-library): Serien/Filme (Zeilen und Raster), Meine Liste und Sammlungen.
//!
//! „Neu und beliebt“ steht in `library_np.rs`, die Suche in `library_search.rs`. Alle Seiten teilen sich die Slint-Seite
//! `LibraryPage` und den Zustand `LibraryState` (Global). Rust hält Zustand, Laden und Fokus; die Oberfläche meldet
//! Ereignisse über [`App::lib_event`].

use std::collections::HashSet;
use std::rc::Rc;

use lfx_api::BaseItem;
use lfx_core::images::{CardKind, display_title};
use lfx_core::library::{GenreRef, GridPager, LibSort, LibraryKind, MyListRoute, MyListType};
use lfx_core::rows::logic;
use lfx_images::DecodedImage;
use lfx_nav::{Target, ZoneId};
use lfx_ui::{CardAspect, CardData, LibGeometry};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::route::{LibraryView, Route};

// Fokuszonen (wie in library.slint).
pub const Z_NAV: ZoneId = ZoneId(0);
/// Ziele der Kopfleiste (wie auf der Startseite): 5 Links, Suche, TV, Datensparmodus, Profil.
pub const NAV_TARGETS: usize = 9;
pub const Z_TOOL: ZoneId = ZoneId(1);
pub const Z_HERO: ZoneId = ZoneId(2);
pub const Z_ALPHA: ZoneId = ZoneId(3);
pub const Z_MENU: ZoneId = ZoneId(4);
pub const Z_BODY: ZoneId = ZoneId(5);
pub const Z_FIELD: ZoneId = ZoneId(6);
pub const Z_KBD: ZoneId = ZoneId(7);
pub const Z_KBD2: ZoneId = ZoneId(30);
pub const Z_ROW0: u32 = 10;
pub const Z_CHIP0: u32 = 40;

/// Wie viele Titel ein Genre-Menü in drei Spalten statt einer zeigt.
pub const MENU_COLS_THRESHOLD: usize = 12;

/// Art der gezeigten Seite (Reihenfolge = Slint-`mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    Rows,
    #[default]
    Grid,
    MyList,
    Collection,
    NewPop,
    Search,
}

impl Mode {
    pub fn index(self) -> i32 {
        match self {
            Mode::Rows => 0,
            Mode::Grid => 1,
            Mode::MyList => 2,
            Mode::Collection => 3,
            Mode::NewPop => 4,
            Mode::Search => 5,
        }
    }
}

/// Was ein Werkzeug der Kopfleiste auslöst.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolAct {
    MyType(Option<MyListType>),
    MySort(LibSort),
    GenreMenu,
    SortMenu,
    View(LibraryView),
    NpTab(usize),
}

#[derive(Clone, Debug)]
pub struct Tool {
    pub label: String,
    /// 0 Chip, 1 Sortierung, 2 Ansicht Zeilen, 3 Ansicht Raster, 4 Menü.
    pub kind: i32,
    pub active: bool,
    pub act: ToolAct,
    pub right: bool,
}

/// Quelle der Rasterseiten.
#[derive(Clone, Debug)]
pub enum GridSrc {
    Lib {
        kind: LibraryKind,
        genre: Option<String>,
        sort: LibSort,
    },
    MyList(MyListRoute),
    Collection {
        id: String,
        sort: LibSort,
        letter: Option<char>,
    },
}

/// Eine Zeile samt Kartenmodell.
pub struct LibRow {
    pub id: String,
    pub title: String,
    pub kind: CardKind,
    pub aspect: CardAspect,
    pub items: Vec<BaseItem>,
    pub model: Rc<VecModel<CardData>>,
}

/// Geöffnetes Auswahlmenü.
#[derive(Clone, Debug)]
pub struct Menu {
    pub genre: bool,
    pub items: Vec<String>,
    pub cols: usize,
    pub saved: Option<Target>,
}

/// Maße der Seite (aus [`LibGeometry`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LibGeo {
    pub width: f32,
    pub height: f32,
    pub nav_h: f32,
    pub head_h: f32,
    pub cols: usize,
    pub gutter: f32,
    pub card_w: f32,
    pub card_h: f32,
    pub gap: f32,
    pub pitch_y: f32,
    pub hero_h: f32,
    pub row_h: f32,
    pub entry_h: f32,
    pub btn_y: f32,
    pub btn_h: f32,
}

impl From<LibGeometry> for LibGeo {
    fn from(g: LibGeometry) -> Self {
        Self {
            width: g.width,
            height: g.height,
            nav_h: g.nav_h,
            head_h: g.head_h,
            cols: (g.cols.max(1)) as usize,
            gutter: g.gutter,
            card_w: g.card_w,
            card_h: g.card_h,
            gap: g.gap,
            pitch_y: g.pitch_y,
            hero_h: g.hero_h,
            row_h: g.row_h,
            entry_h: g.entry_h,
            btn_y: g.btn_y,
            btn_h: g.btn_h,
        }
    }
}

impl Default for LibGeo {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 780.0,
            nav_h: 52.0,
            head_h: 56.0,
            cols: 5,
            gutter: 50.0,
            card_w: 220.0,
            card_h: 124.0,
            gap: 12.0,
            pitch_y: 148.0,
            hero_h: 380.0,
            row_h: 200.0,
            entry_h: 570.0,
            btn_y: 380.0,
            btn_h: 40.0,
        }
    }
}

/// Zustand der Bibliotheksseiten (nur UI-Thread).
#[derive(Default)]
pub struct LibState {
    pub mode: Mode,
    /// Zählt Seitenaufrufe; veraltete Antworten werden verworfen.
    pub epoch: u64,
    pub route_kind: Option<LibraryKind>,
    pub genre: Option<String>,
    pub genres: Vec<GenreRef>,
    pub sort: Option<LibSort>,
    pub tools: Vec<Tool>,
    pub n_left: usize,
    pub items: Vec<BaseItem>,
    pub model: Option<Rc<VecModel<CardData>>>,
    pub pager: GridPager,
    pub src: Option<GridSrc>,
    pub requested: HashSet<usize>,
    pub vis: (usize, usize),
    pub rows: Vec<LibRow>,
    pub pool: Vec<BaseItem>,
    pub hero_idx: usize,
    pub mylist: MyListRoute,
    pub coll_id: String,
    pub coll_name: String,
    pub letter: Option<char>,
    pub alpha: Vec<String>,
    pub menu: Option<Menu>,
    pub geo: Option<LibGeo>,
    pub error: String,
    pub empty_action: EmptyAction,
    pub np: crate::library_np::NpState,
    pub search: crate::library_search::SearchState,
    pub hero_timer: slint::Timer,
}

/// Aktion der Schaltfläche im Leerzustand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EmptyAction {
    #[default]
    None,
    /// Alle Titel (Typfilter von Meine Liste aufheben).
    ShowAll,
    /// Zur Startseite.
    Home,
    /// Alle Anfangsbuchstaben.
    ClearLetter,
}

// ---------------------------------------------------------------- Hilfen

pub fn to_buffer(img: &DecodedImage) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(img.width, img.height);
    let dst = buf.make_mut_bytes();
    let n = dst.len().min(img.pixels.len());
    dst[..n].copy_from_slice(&img.pixels[..n]);
    buf
}

/// Einfacher Zufall (xorshift) für die Billboard-Mischung.
pub fn rng_from_clock() -> impl FnMut() -> f64 {
    let mut s = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9E37_79B9, |d| d.as_nanos() as u64))
        | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Kartendaten ohne Bild.
pub fn card_data(
    item: &BaseItem,
    kind: CardKind,
    rank: Option<usize>,
    new_series: Option<&HashSet<String>>,
    now_ms: f64,
) -> CardData {
    let progress = if kind == CardKind::Resume {
        (logic::progress_percent(item) / 100.0) as f32
    } else {
        0.0
    };
    CardData {
        title: SharedString::from(display_title(item)),
        subtitle: item
            .production_year
            .map(|y| y.to_string())
            .unwrap_or_default()
            .into(),
        image: Image::default(),
        has_image: false,
        progress,
        badge: logic::badge_for(item, new_series, now_ms)
            .unwrap_or("")
            .into(),
        trend_rank: rank.map_or(0, |r| r as i32),
    }
}

pub fn nav_index_for(route: &Route) -> i32 {
    match route {
        Route::Library {
            kind: LibraryKind::Series,
            ..
        } => 1,
        Route::Library {
            kind: LibraryKind::Movies,
            ..
        } => 2,
        Route::NewPopular => 3,
        Route::MyList => 4,
        Route::Search => 5,
        _ => -1,
    }
}

pub fn tool_width(label: &str, kind: i32) -> f32 {
    match kind {
        2 | 3 => 38.0,
        4 => label.chars().count() as f32 * 8.0 + 44.0,
        _ => label.chars().count() as f32 * 8.0 + 32.0,
    }
}

pub fn is_library_route(route: &Route) -> bool {
    matches!(
        route,
        Route::Library { .. }
            | Route::MyList
            | Route::Collection { .. }
            | Route::NewPopular
            | Route::Search
    )
}
