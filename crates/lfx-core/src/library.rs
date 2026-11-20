//! Bibliotheksseiten „Serien“/„Filme“ und „Meine Liste“ (Port von `features/library.js`, `library/*`,
//! `features/mylist.js`, `shared/routes.js`): Routen, Sortierungen, Zeilen, Raster-Paging, Cache-Schlüssel.

use crate::cache::home_key;
use crate::call::{ApiCall, Endpoint};
use crate::collate::de_compare;
use crate::dayseed::{GenreInput, pick_genres};
use crate::rows::home::{ChartLists, MIN_CHART_ROW, chart_title, top_new_title};
use crate::rows::logic::base_query;
use crate::rows::{RowDef, RowKind, RowSource, items_call};
use crate::util::{decode_query_component, encode_uri_component as enc};
use lfx_api::{BaseItem, ItemFilter, ItemKind, ItemsQuery, SortBy, SortOrder};

// ---------------------------------------------------------------- Art, Sortierung

/// Serien- oder Filmseite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryKind {
    /// `lfx=series`.
    Series,
    /// `lfx=movies`.
    Movies,
}

impl LibraryKind {
    /// Kennung in der Route.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Series => "series",
            Self::Movies => "movies",
        }
    }
    /// Aus Kennung.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "series" => Some(Self::Series),
            "movies" => Some(Self::Movies),
            _ => None,
        }
    }
    /// Element-Art (`LIB_TYPES`).
    pub fn item_kind(self) -> ItemKind {
        match self {
            Self::Series => ItemKind::Series,
            Self::Movies => ItemKind::Movie,
        }
    }
    /// Art für „Weiterschauen“ (`RESUME_TYPES`): Serien zeigen Folgen.
    pub fn resume_kind(self) -> ItemKind {
        match self {
            Self::Series => ItemKind::Episode,
            Self::Movies => ItemKind::Movie,
        }
    }
    /// Seitentitel.
    pub fn label(self) -> &'static str {
        match self {
            Self::Series => "Serien",
            Self::Movies => "Filme",
        }
    }
    /// Sortierung „zuletzt hinzugefügt“: bei Serien zählt der letzte neue Inhalt.
    pub fn new_sort_key(self) -> &'static str {
        match self {
            Self::Series => "DateLastContentAdded",
            Self::Movies => "DateCreated",
        }
    }
}

/// Sortierung des Rasters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibSort {
    /// A–Z (Standard).
    Az,
    /// Z–A.
    Za,
    /// Neu hinzugefügt.
    New,
    /// Erscheinungsjahr (neueste zuerst).
    Year,
    /// Bewertung (beste zuerst). **Desktop-Erweiterung**, im Web nicht vorhanden.
    Rating,
}

impl LibSort {
    /// Sortierungen des Webs in Anzeigereihenfolge.
    pub const WEB: [LibSort; 4] = [Self::Az, Self::Za, Self::New, Self::Year];
    /// Alle, mit Desktop-Erweiterung.
    pub const ALL: [LibSort; 5] = [Self::Az, Self::Za, Self::New, Self::Year, Self::Rating];

    /// Kennung in der Route.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Az => "az",
            Self::Za => "za",
            Self::New => "new",
            Self::Year => "year",
            Self::Rating => "rating",
        }
    }
    /// Aus Kennung.
    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.slug() == s)
    }
    /// Anzeigetext.
    pub fn label(self) -> &'static str {
        match self {
            Self::Az => "A–Z",
            Self::Za => "Z–A",
            Self::New => "Neu hinzugefügt",
            Self::Year => "Erscheinungsjahr",
            Self::Rating => "Bewertung",
        }
    }
}

/// `sortParams`: (`SortBy`, `SortOrder`) als Parameterwerte. Bei reinem Serien-Query zählt „neu“ als
/// letzter Zuwachs (`DateLastContentAdded`).
pub fn sort_params(sort: LibSort, series_only: bool) -> (&'static str, &'static str) {
    match sort {
        LibSort::Za => ("SortName", "Descending"),
        LibSort::New => (
            if series_only {
                "DateLastContentAdded"
            } else {
                "DateCreated"
            },
            "Descending",
        ),
        LibSort::Year => ("ProductionYear,SortName", "Descending,Ascending"),
        LibSort::Rating => ("CommunityRating,SortName", "Descending,Ascending"),
        LibSort::Az => ("SortName", "Ascending"),
    }
}

fn apply_sort(q: ItemsQuery, sort: LibSort, series_only: bool) -> ItemsQuery {
    let (by, order) = sort_params(sort, series_only);
    q.raw("sortBy", by).raw("sortOrder", order)
}

// ---------------------------------------------------------------- Routen

/// Ansicht der Bibliotheksseite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Billboard + Zeilen.
    Rows,
    /// Raster.
    Grid,
}

/// Zustand der Serien-/Filme-Seite (`#/home?lfx=series|movies&genre=&view=grid&sort=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoute {
    /// Serien oder Filme.
    pub kind: LibraryKind,
    /// Genre-Id.
    pub genre: Option<String>,
    /// Zeilen oder Raster.
    pub view: ViewMode,
    /// Sortierung (nur im Raster wirksam).
    pub sort: Option<LibSort>,
}

impl LibraryRoute {
    /// Startzustand einer Art.
    pub fn new(kind: LibraryKind) -> Self {
        Self {
            kind,
            genre: None,
            view: ViewMode::Rows,
            sort: None,
        }
    }

    /// `libraryHref`: leere Werte entfallen, `view` nur `grid`.
    pub fn href(&self) -> String {
        let mut s = format!("#/home?lfx={}", self.kind.slug());
        if let Some(g) = self.genre.as_deref().filter(|g| !g.is_empty()) {
            s.push_str(&format!("&genre={}", enc(g)));
        }
        if self.view == ViewMode::Grid {
            s.push_str("&view=grid");
        }
        if let Some(v) = self.sort {
            s.push_str(&format!("&sort={}", v.slug()));
        }
        s
    }
}

/// Typfilter von „Meine Liste“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MyListType {
    /// Nur Filme.
    Movies,
    /// Nur Serien.
    Series,
}

impl MyListType {
    /// Kennung.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Series => "series",
        }
    }
    /// Anzeigetext des Filters.
    pub fn label(self) -> &'static str {
        match self {
            Self::Movies => "Filme",
            Self::Series => "Serien",
        }
    }
    fn item_kind(self) -> ItemKind {
        match self {
            Self::Movies => ItemKind::Movie,
            Self::Series => ItemKind::Series,
        }
    }
}

/// Sortierungen von „Meine Liste“ (ohne Z–A).
pub const MYLIST_SORTS: [LibSort; 3] = [LibSort::Az, LibSort::New, LibSort::Year];

/// Zustand der Seite „Meine Liste“ (`#/home?lfx=mylist&type=&sort=`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MyListRoute {
    /// Typfilter (`None` = alle).
    pub kind: Option<MyListType>,
    /// Sortierung (`None` = A–Z).
    pub sort: Option<LibSort>,
}

impl MyListRoute {
    /// Wirksame Sortierung (Standard A–Z; unzulässige wie Z–A fallen auf A–Z).
    pub fn effective_sort(&self) -> LibSort {
        self.sort
            .filter(|s| MYLIST_SORTS.contains(s))
            .unwrap_or(LibSort::Az)
    }

    /// `myListHref`.
    pub fn href(&self) -> String {
        let mut s = String::from("#/home?lfx=mylist");
        if let Some(t) = self.kind {
            s.push_str(&format!("&type={}", t.slug()));
        }
        if let Some(v) = self.sort {
            s.push_str(&format!("&sort={}", v.slug()));
        }
        s
    }
}

/// Erkannte Route der Bibliotheks-Features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfxRoute {
    /// Serien-/Filme-Seite.
    Library(LibraryRoute),
    /// Meine Liste.
    MyList(MyListRoute),
}

/// `parseLfxRoute` für die Bibliotheksseiten: Eingabe ist der Hash (`#/home?lfx=…`) oder `/home?lfx=…`.
/// Unbekannte Sortierungen/Typen/Ansichten fallen weg wie im Web; andere `lfx`-Seiten = `None`.
pub fn parse_route(hash: &str) -> Option<LfxRoute> {
    let h = hash.strip_prefix('#').unwrap_or(hash);
    let (path, query) = h.split_once('?').unwrap_or((h, ""));
    if path != "/home" {
        return None;
    }
    let get = |name: &str| -> Option<String> {
        query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (decode_query_component(k) == name).then(|| decode_query_component(v))
        })
    };
    let sort = get("sort").and_then(|s| LibSort::from_slug(&s));
    match get("lfx")?.as_str() {
        k @ ("series" | "movies") => Some(LfxRoute::Library(LibraryRoute {
            kind: LibraryKind::from_slug(k)?,
            genre: get("genre").filter(|g| !g.is_empty()),
            view: if get("view").as_deref() == Some("grid") {
                ViewMode::Grid
            } else {
                ViewMode::Rows
            },
            sort,
        })),
        "mylist" => Some(LfxRoute::MyList(MyListRoute {
            kind: match get("type").as_deref() {
                Some("movies") => Some(MyListType::Movies),
                Some("series") => Some(MyListType::Series),
                _ => None,
            },
            sort,
        })),
        _ => None,
    }
}

// ---------------------------------------------------------------- Genres

/// Genre der Seite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenreRef {
    /// Genre-Id.
    pub id: String,
    /// Name.
    pub name: String,
}

/// Abfrage der Genres einer Art (`getLibraryGenres`).
pub fn genres_call(kind: LibraryKind) -> ApiCall {
    ApiCall::new(
        Endpoint::Genres,
        ItemsQuery::new()
            .include_types(&[kind.item_kind()])
            .recursive(true)
            .sort_by(&[SortBy::SortName]),
    )
}

/// Genre-Liste aus der Antwort: nur mit Id und Name, nach deutscher Sortierung.
pub fn genres_from(items: &[BaseItem]) -> Vec<GenreRef> {
    let mut v: Vec<GenreRef> = items
        .iter()
        .filter(|g| !g.id.is_empty() && !g.name.is_empty())
        .map(|g| GenreRef {
            id: g.id.clone(),
            name: g.name.clone(),
        })
        .collect();
    v.sort_by(|a, b| de_compare(&a.name, &b.name));
    v
}

// ---------------------------------------------------------------- Zeilen

const MIN_ROW: usize = 8;
const DECADES: [i32; 5] = [2020, 2010, 2000, 1990, 1980];

/// Präfix der Zeilen-Ids und Pool-Schlüssel (`lib:<kind>:<genre|all>:`).
pub fn id_prefix(kind: LibraryKind, genre: Option<&str>) -> String {
    format!(
        "lib:{}:{}:",
        kind.slug(),
        genre.filter(|g| !g.is_empty()).unwrap_or("all")
    )
}

/// Cache-Schlüssel des Hero-Pools (`libraryPoolKey`). Das Billboard cacht selbst darunter: als Abruf den
/// **ungecachten** Pool laden (gleicher Schlüssel in zwei Ebenen = Deadlock im Web).
pub fn pool_key(user_id: &str, kind: LibraryKind, genre: Option<&str>) -> String {
    home_key(
        user_id,
        &["lib", kind.slug(), genre.unwrap_or("all"), "pool"],
    )
}

/// Cache-Schlüssel der Genre-Liste.
pub fn genres_key(user_id: &str, kind: LibraryKind) -> String {
    home_key(user_id, &["lib", kind.slug(), "genres"])
}

/// Cache-Schlüssel der ersten Rasterseite je Sortierung.
pub fn grid_cache_key(
    user_id: &str,
    kind: LibraryKind,
    genre: Option<&str>,
    sort: LibSort,
) -> String {
    home_key(
        user_id,
        &[
            "lib",
            kind.slug(),
            genre.unwrap_or("all"),
            "grid",
            sort.slug(),
        ],
    )
}

fn row(id: String, title: Option<String>, kind: RowKind, source: RowSource) -> RowDef {
    RowDef {
        id,
        title,
        kind,
        source,
        no_cache: false,
    }
}

fn q_row(id: String, title: &str, call: ApiCall, min_items: usize) -> RowDef {
    row(
        id,
        Some(title.to_owned()),
        RowKind::Landscape,
        RowSource::Query {
            call,
            min_items,
            type_filter: None,
        },
    )
}

/// Eingabe für [`build_library_rows`].
#[derive(Debug, Clone)]
pub struct LibraryInput<'a> {
    /// Serien oder Filme.
    pub kind: LibraryKind,
    /// Gewähltes Genre (Id).
    pub genre: Option<&'a str>,
    /// Genres der Art (für Namen und Tagesauswahl).
    pub genres: &'a [GenreRef],
    /// Tages-Seed (`dayKey`).
    pub day_key: &'a str,
    /// Markenname.
    pub brand: &'a str,
    /// Charts-Dienst konfiguriert (Ersatzzeile cacht dann nicht).
    pub charts_configured: bool,
    /// Aufgelöste Charts.
    pub charts: Option<&'a ChartLists>,
}

/// Zeilen der Seite. Ohne Genre: Weiterschauen, (Serien: Als Nächstes), Top 10, Neu hinzugefügt, „Weil du …“,
/// 6 Tages-Genres verteilt um Hochbewertet/Zufällige Auswahl. Mit Genre: Weiterschauen (nur Treffer), Neu,
/// Hochbewertet, Zufällig, Jahrzehnte.
pub fn build_library_rows(input: &LibraryInput<'_>) -> Vec<RowDef> {
    let kind = input.kind;
    let ty = kind.item_kind();
    let pre = id_prefix(kind, input.genre);
    let genre_name = input.genre.map(|g| {
        input
            .genres
            .iter()
            .find(|x| x.id == g)
            .map_or_else(|| "Genre".to_owned(), |x| x.name.clone())
    });
    let gp = |q: ItemsQuery| match input.genre {
        Some(g) => q.raw("genreIds", g),
        None => q,
    };
    let items = |extra: &dyn Fn(ItemsQuery) -> ItemsQuery| items_call(&[ty], |q| extra(gp(q)));

    let resume_call = ApiCall::new(
        Endpoint::Resume,
        base_query()
            .media_types(&["Video"])
            .include_types(&[kind.resume_kind()])
            .limit(20),
    );
    let resume = row(
        format!("{pre}resume"),
        Some("Weiterschauen".into()),
        RowKind::Resume,
        match (input.genre, &genre_name) {
            (Some(g), Some(n)) => RowSource::ResumeGenre {
                call: resume_call,
                type_filter: kind.resume_kind(),
                genre_id: g.to_owned(),
                genre_name: n.clone(),
            },
            _ => RowSource::Query {
                call: resume_call,
                min_items: 0,
                type_filter: Some(kind.resume_kind()),
            },
        },
    );
    let latest = q_row(
        format!("{pre}latest"),
        "Neu hinzugefügt",
        if input.genre.is_some() {
            items(&|q| {
                q.raw("sortBy", kind.new_sort_key())
                    .order(SortOrder::Descending)
                    .limit(20)
            })
        } else {
            ApiCall::new(
                Endpoint::Latest,
                base_query()
                    .include_types(&[ty])
                    .raw("groupItems", "true")
                    .limit(20),
            )
        },
        0,
    );
    let toprated = q_row(
        format!("{pre}toprated"),
        "Hochbewertet",
        items(&|q| {
            q.sort_by(&[SortBy::CommunityRating])
                .order(SortOrder::Descending)
                .min_community_rating(7.5)
                .limit(20)
        }),
        0,
    );
    let random = q_row(
        format!("{pre}random"),
        "Zufällige Auswahl",
        items(&|q| q.sort_by(&[SortBy::Random]).limit(20)),
        0,
    );

    if let Some(name) = &genre_name {
        let mut defs = vec![resume, latest, toprated, random];
        for start in DECADES {
            let years: Vec<i32> = (start..start + 10).collect();
            defs.push(q_row(
                format!("{pre}decade-{start}"),
                &format!("{name} der {start}er"),
                items(&|q| q.years(&years).sort_by(&[SortBy::Random]).limit(20)),
                MIN_ROW,
            ));
        }
        return defs;
    }

    let mut defs = vec![resume];
    if kind == LibraryKind::Series {
        defs.push(q_row(
            format!("{pre}nextup"),
            "Als Nächstes",
            ApiCall::new(
                Endpoint::NextUp,
                base_query()
                    .limit(20)
                    .raw("enableResumable", "false")
                    .raw("enableRewatching", "false"),
            ),
            0,
        ));
    }
    defs.push(top10_row(input, &pre));
    defs.push(latest);
    defs.push(row(
        format!("{pre}because"),
        None,
        RowKind::Landscape,
        RowSource::Because {
            played: items_call(&[kind.resume_kind()], |q| {
                q.filters(&[ItemFilter::IsPlayed])
                    .sort_by(&[SortBy::DatePlayed])
                    .order(SortOrder::Descending)
                    .limit(1)
            }),
            similar_limit: 20,
            type_filter: Some(ty),
        },
    ));

    let names: Vec<GenreInput> = input
        .genres
        .iter()
        .map(|g| GenreInput::named(g.name.clone()))
        .collect();
    let seed = format!("{}:{}", input.day_key, kind.slug());
    let picked = pick_genres(&names, &seed, 6, crate::dayseed::MIN_GENRE_TITLES);
    let genre_rows: Vec<RowDef> = picked
        .iter()
        .filter_map(|n| input.genres.iter().find(|g| &g.name == n))
        .map(|g| {
            q_row(
                format!("{pre}genre-{}", g.id),
                &g.name,
                items_call(&[ty], |q| {
                    q.raw("genreIds", g.id.as_str())
                        .sort_by(&[SortBy::Random])
                        .limit(20)
                }),
                MIN_ROW,
            )
        })
        .collect();
    let mut g = genre_rows.into_iter();
    defs.extend(g.by_ref().take(2));
    defs.push(toprated);
    defs.extend(g.by_ref().take(2));
    defs.push(random);
    defs.extend(g);
    defs
}

/// Top-10-Zeile der Seite: echte Charts, wenn mindestens 5 Titel der eigenen Art; sonst Ersatz „Neu auf …“
/// (nicht gecacht, wenn ein Charts-Dienst konfiguriert ist).
fn top10_row(input: &LibraryInput<'_>, pre: &str) -> RowDef {
    let (key, list) = match input.kind {
        LibraryKind::Series => ("series", input.charts.map(|c| &c.series)),
        LibraryKind::Movies => ("movies", input.charts.map(|c| &c.movies)),
    };
    let own_kind = input.kind.item_kind();
    let own: Vec<BaseItem> = list
        .map(|l| l.iter().filter(|i| i.kind == own_kind).cloned().collect())
        .unwrap_or_default();
    let id = format!("{pre}top10");
    if own.len() >= MIN_CHART_ROW {
        return row(
            id,
            Some(chart_title(key, input.brand)),
            RowKind::Top10,
            RowSource::Fixed(own),
        );
    }
    let mut r = row(
        id,
        Some(top_new_title(input.brand)),
        RowKind::Top10,
        RowSource::TopNew {
            call: items_call(&[own_kind], |q| {
                q.sort_by(&[SortBy::DateCreated])
                    .order(SortOrder::Descending)
                    .limit(10)
            }),
        },
    );
    r.no_cache = input.charts_configured;
    r
}

// ---------------------------------------------------------------- Raster

/// Standard-Seitengröße des Rasters.
pub const GRID_PAGE_SIZE: usize = 40;

/// Abfrage einer Rasterseite (`createGrid.loadMore`): Basis-Parameter, rekursiv, Art, optional Genre,
/// Sortierung, Gesamtzahl, `StartIndex`/`Limit`.
pub fn grid_page_call(
    kind: LibraryKind,
    genre: Option<&str>,
    sort: LibSort,
    start_index: usize,
    page_size: usize,
) -> ApiCall {
    let q = base_query()
        .recursive(true)
        .include_types(&[kind.item_kind()]);
    let q = match genre {
        Some(g) => q.raw("genreIds", g),
        None => q,
    };
    ApiCall::new(
        Endpoint::Items,
        apply_sort(q, sort, kind == LibraryKind::Series)
            .total_count(true)
            .start_index(start_index as u32)
            .limit(page_size as u32),
    )
}

/// „Meine Liste“-Seite: Favoriten, Art (Standard Film und Serie), Sortierung, Seite.
pub fn mylist_page_call(route: &MyListRoute, start_index: usize, page_size: usize) -> ApiCall {
    let types: Vec<ItemKind> = route.kind.map_or_else(
        || vec![ItemKind::Movie, ItemKind::Series],
        |t| vec![t.item_kind()],
    );
    let series_only = route.kind == Some(MyListType::Series);
    ApiCall::new(
        Endpoint::Items,
        apply_sort(
            base_query()
                .recursive(true)
                .include_types(&types)
                .filters(&[ItemFilter::IsFavorite]),
            route.effective_sort(),
            series_only,
        )
        .total_count(true)
        .start_index(start_index as u32)
        .limit(page_size as u32),
    )
}

/// Zustand des Nachladens (`createGrid`): zählt geladene Titel und erkennt das Ende.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridPager {
    /// Bisher geladene Titel (zugleich nächster `StartIndex`).
    pub count: usize,
    /// Gesamtzahl laut Server.
    pub total: Option<usize>,
    /// Alles geladen.
    pub done: bool,
    /// Ein Abruf läuft.
    pub loading: bool,
    /// Letzter Abruf scheiterte (Knopf „Erneut versuchen“).
    pub failed: bool,
    /// Generation; veraltete Antworten werden verworfen.
    pub seq: u64,
}

impl GridPager {
    /// Neuer Pager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Neuaufbau (Sortierung/Abfrage geändert): beginnt von vorn, macht ältere Antworten ungültig.
    pub fn reset(&mut self) {
        *self = Self {
            seq: self.seq + 1,
            ..Self::default()
        };
    }

    /// Darf eine weitere Seite geladen werden?
    pub fn can_load(&self) -> bool {
        !self.loading && !self.done && !self.failed
    }

    /// Abruf beginnt; gibt die Generation zurück.
    pub fn begin(&mut self) -> u64 {
        self.loading = true;
        self.seq
    }

    /// Seite eingetroffen. Verwirft Antworten alter Generationen (`false`). Ende, wenn die Seite kürzer als
    /// `page_size` ist oder `count >= total`.
    pub fn apply(&mut self, seq: u64, got: usize, total: Option<usize>, page_size: usize) -> bool {
        if seq != self.seq {
            return false;
        }
        self.loading = false;
        self.total = total;
        self.count += got;
        self.done = got < page_size || total.is_some_and(|t| self.count >= t);
        true
    }

    /// Abruf gescheitert.
    pub fn fail(&mut self, seq: u64) {
        if seq == self.seq {
            self.loading = false;
            self.failed = true;
        }
    }

    /// „Erneut versuchen“.
    pub fn retry(&mut self) {
        self.failed = false;
    }

    /// `n` Karten wurden außerhalb entfernt (z. B. Favorit weg): Versatz und Gesamtzahl nachführen.
    pub fn drop_items(&mut self, n: usize) {
        self.count = self.count.saturating_sub(n);
        self.total = self.total.map(|t| t.saturating_sub(n));
    }

    /// Anzuzeigende Zahl (`onCount`): Gesamtzahl, sonst geladene.
    pub fn display_count(&self) -> usize {
        self.total.unwrap_or(self.count)
    }
}

/// Zähltext der Seite: „12 Titel“, in retro großgeschrieben (nur „Meine Liste“).
pub fn count_text(n: usize, retro: bool) -> String {
    if retro {
        format!("{n} TITEL")
    } else {
        format!("{n} Titel")
    }
}

/// Leer-Text von „Meine Liste“: (Überschrift, Hinweis sichtbar, Linktext, Ziel).
pub fn mylist_empty_text(route: &MyListRoute, retro: bool) -> (String, bool, String, String) {
    match route.kind {
        Some(t) => {
            let k = t.label();
            (
                if retro {
                    format!("KEINE {} GEMERKT", k.to_uppercase())
                } else {
                    format!("Keine {k} in deiner Liste")
                },
                false,
                if retro {
                    "ALLE ANZEIGEN".into()
                } else {
                    "Alle anzeigen".into()
                },
                MyListRoute {
                    kind: None,
                    sort: route.sort.filter(|s| *s != LibSort::Az),
                }
                .href(),
            )
        }
        None => (
            if retro {
                "KEINE BÄNDER GEMERKT".into()
            } else {
                "Deine Liste ist leer".into()
            },
            true,
            if retro {
                "ZURÜCK ZUM STARTMENÜ".into()
            } else {
                "Zur Startseite".into()
            },
            "#/home".into(),
        ),
    }
}

/// Tab-Rückkehr: erst nach so langer Abwesenheit neu laden (`HIDDEN_RELOAD_MS`).
pub const MYLIST_HIDDEN_RELOAD_MS: u64 = 60_000;
/// `update()` innerhalb dieser Zeit nach dem Laden ist ein Router-Echo (`STALE_MS`).
pub const MYLIST_STALE_MS: u64 = 2_000;

#[cfg(test)]
mod tests {
    use super::*;

    fn genre(id: &str, name: &str) -> GenreRef {
        GenreRef {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn routes_roundtrip_like_web() {
        assert_eq!(
            LibraryRoute::new(LibraryKind::Series).href(),
            "#/home?lfx=series"
        );
        let r = LibraryRoute {
            kind: LibraryKind::Movies,
            genre: Some("a b".into()),
            view: ViewMode::Grid,
            sort: Some(LibSort::Za),
        };
        assert_eq!(r.href(), "#/home?lfx=movies&genre=a%20b&view=grid&sort=za");
        assert_eq!(parse_route(&r.href()), Some(LfxRoute::Library(r)));
        let m = MyListRoute {
            kind: Some(MyListType::Series),
            sort: Some(LibSort::New),
        };
        assert_eq!(m.href(), "#/home?lfx=mylist&type=series&sort=new");
        assert_eq!(parse_route(&m.href()), Some(LfxRoute::MyList(m)));
        assert_eq!(parse_route("#/home?lfx=title&id=1"), None);
        assert_eq!(parse_route("#/details?lfx=series"), None);
        let Some(LfxRoute::Library(l)) = parse_route("#/home?lfx=series&sort=bogus&view=x") else {
            panic!()
        };
        assert_eq!((l.sort, l.view), (None, ViewMode::Rows));
    }

    #[test]
    fn mylist_rejects_za() {
        let r = MyListRoute {
            kind: None,
            sort: Some(LibSort::Za),
        };
        assert_eq!(r.effective_sort(), LibSort::Az);
    }

    #[test]
    fn sort_params_match_web() {
        assert_eq!(sort_params(LibSort::Az, false), ("SortName", "Ascending"));
        assert_eq!(sort_params(LibSort::Za, false), ("SortName", "Descending"));
        assert_eq!(
            sort_params(LibSort::New, true),
            ("DateLastContentAdded", "Descending")
        );
        assert_eq!(
            sort_params(LibSort::New, false),
            ("DateCreated", "Descending")
        );
        assert_eq!(
            sort_params(LibSort::Year, false),
            ("ProductionYear,SortName", "Descending,Ascending")
        );
        assert_eq!(
            LibSort::WEB.map(LibSort::label),
            ["A–Z", "Z–A", "Neu hinzugefügt", "Erscheinungsjahr"]
        );
    }

    #[test]
    fn grid_call_and_pager() {
        let c = grid_page_call(LibraryKind::Series, Some("g1"), LibSort::New, 80, 40);
        assert_eq!(c.param("IncludeItemTypes"), Some("Series"));
        assert_eq!(c.param("GenreIds"), Some("g1"));
        assert_eq!(c.param("SortBy"), Some("DateLastContentAdded"));
        assert_eq!(
            (c.param("StartIndex"), c.param("Limit")),
            (Some("80"), Some("40"))
        );
        assert_eq!(c.param("EnableTotalRecordCount"), Some("true"));
        assert_eq!(c.param("Recursive"), Some("true"));

        let mut p = GridPager::new();
        let s = p.begin();
        assert!(p.apply(s, 40, Some(95), 40) && !p.done);
        let s = p.begin();
        assert!(p.apply(s, 40, Some(95), 40) && !p.done);
        let s = p.begin();
        p.apply(s, 15, Some(95), 40);
        assert!(p.done && p.count == 95 && !p.can_load());
        p.reset();
        assert!(p.can_load() && p.count == 0);
        let old = p.seq - 1;
        assert!(!p.apply(old, 40, None, 40), "alte Generation verworfen");
        let s = p.begin();
        p.fail(s);
        assert!(!p.can_load());
        p.retry();
        assert!(p.can_load());
        p.count = 10;
        p.total = Some(12);
        p.drop_items(3);
        assert_eq!((p.count, p.display_count()), (7, 9));
    }

    #[test]
    fn done_when_total_reached_exactly() {
        let mut p = GridPager::new();
        let s = p.begin();
        p.apply(s, 40, Some(40), 40);
        assert!(p.done);
    }

    #[test]
    fn mylist_call_filters_favorites() {
        let c = mylist_page_call(&MyListRoute::default(), 0, 40);
        assert_eq!(c.param("Filters"), Some("IsFavorite"));
        assert_eq!(c.param("IncludeItemTypes"), Some("Movie,Series"));
        let s = mylist_page_call(
            &MyListRoute {
                kind: Some(MyListType::Series),
                sort: Some(LibSort::New),
            },
            0,
            40,
        );
        assert_eq!(s.param("SortBy"), Some("DateLastContentAdded"));
        assert_eq!(
            mylist_empty_text(&MyListRoute::default(), false).0,
            "Deine Liste ist leer"
        );
        let t = mylist_empty_text(
            &MyListRoute {
                kind: Some(MyListType::Movies),
                sort: None,
            },
            true,
        );
        assert_eq!(
            (t.0.as_str(), t.3.as_str()),
            ("KEINE FILME GEMERKT", "#/home?lfx=mylist")
        );
        assert_eq!(count_text(3, true), "3 TITEL");
    }

    fn genres() -> Vec<GenreRef> {
        (0..10)
            .map(|i| genre(&format!("id{i}"), &format!("Genre {i}")))
            .collect()
    }

    fn input<'a>(kind: LibraryKind, genre: Option<&'a str>, g: &'a [GenreRef]) -> LibraryInput<'a> {
        LibraryInput {
            kind,
            genre,
            genres: g,
            day_key: "2026-11-20",
            brand: "LENZFLIX",
            charts_configured: false,
            charts: None,
        }
    }

    fn ids(v: &[RowDef]) -> Vec<String> {
        v.iter().map(|r| r.id.clone()).collect()
    }

    #[test]
    fn series_rows_layout() {
        let g = genres();
        let rows = build_library_rows(&input(LibraryKind::Series, None, &g));
        let id = ids(&rows);
        assert!(id.iter().all(|i| i.starts_with("lib:series:all:")));
        let tail: Vec<&str> = id.iter().map(|s| s.rsplit(':').next().unwrap()).collect();
        assert_eq!(
            &tail[..5],
            ["resume", "nextup", "top10", "latest", "because"]
        );
        assert_eq!(
            tail.iter()
                .filter(|t| t.starts_with("genre-") || t.contains("id"))
                .count(),
            6
        );
        assert_eq!(tail[7], "toprated", "2 Genres, dann Hochbewertet");
        assert_eq!(tail[10], "random");
        let RowSource::Query { type_filter, .. } = &rows[0].source else {
            panic!()
        };
        assert_eq!(*type_filter, Some(ItemKind::Episode));
        let RowSource::Query { call, .. } = &rows[3].source else {
            panic!()
        };
        assert_eq!(call.endpoint, Endpoint::Latest);
        assert_eq!(call.param("IncludeItemTypes"), Some("Series"));
    }

    #[test]
    fn movie_rows_have_no_nextup() {
        let g = genres();
        let rows = build_library_rows(&input(LibraryKind::Movies, None, &g));
        assert!(!ids(&rows).iter().any(|i| i.ends_with("nextup")));
    }

    #[test]
    fn genre_page_rows() {
        let g = genres();
        let rows = build_library_rows(&input(LibraryKind::Series, Some("id3"), &g));
        let id = ids(&rows);
        assert_eq!(id.len(), 4 + 5);
        assert!(id[0].starts_with("lib:series:id3:resume"));
        assert!(matches!(rows[0].source, RowSource::ResumeGenre { .. }));
        let RowSource::Query { call, .. } = &rows[1].source else {
            panic!()
        };
        assert_eq!(
            (call.param("SortBy"), call.param("GenreIds")),
            (Some("DateLastContentAdded"), Some("id3"))
        );
        let RowSource::Query {
            call, min_items, ..
        } = &rows[4].source
        else {
            panic!()
        };
        assert_eq!(*min_items, 8);
        assert_eq!(
            call.param("Years"),
            Some("2020,2021,2022,2023,2024,2025,2026,2027,2028,2029")
        );
        assert_eq!(rows[4].title.as_deref(), Some("Genre 3 der 2020er"));
    }

    #[test]
    fn top10_charts_or_fallback() {
        let g = genres();
        let mk = |n: usize, kind| -> Vec<BaseItem> {
            (0..n)
                .map(|i| BaseItem {
                    id: format!("c{i}"),
                    kind,
                    ..Default::default()
                })
                .collect()
        };
        let charts = ChartLists {
            series: mk(6, ItemKind::Series),
            movies: mk(6, ItemKind::Series), // falsche Art: zählt nicht
            all: vec![],
        };
        let mut i = input(LibraryKind::Series, None, &g);
        i.charts = Some(&charts);
        let r = top10_row(&i, "p:");
        assert!(matches!(r.source, RowSource::Fixed(_)) && !r.no_cache);
        let mut m = input(LibraryKind::Movies, None, &g);
        m.charts = Some(&charts);
        m.charts_configured = true;
        let r = top10_row(&m, "p:");
        assert!(matches!(r.source, RowSource::TopNew { .. }) && r.no_cache);
        assert_eq!(r.title.as_deref(), Some("Top 10: Neu auf LENZFLIX"));
    }

    #[test]
    fn keys_and_genre_list() {
        assert_eq!(
            pool_key("u", LibraryKind::Movies, None),
            "home:u:lib:movies:all:pool"
        );
        assert_eq!(
            grid_cache_key("u", LibraryKind::Series, Some("g"), LibSort::Az),
            "home:u:lib:series:g:grid:az"
        );
        let raw = vec![
            BaseItem {
                id: "2".into(),
                name: "Zeit".into(),
                ..Default::default()
            },
            BaseItem {
                id: "1".into(),
                name: "Ärger".into(),
                ..Default::default()
            },
            BaseItem {
                id: "".into(),
                name: "ohne".into(),
                ..Default::default()
            },
        ];
        let l = genres_from(&raw);
        assert_eq!(
            l.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["Ärger", "Zeit"]
        );
        assert_eq!(
            genres_call(LibraryKind::Movies).param("IncludeItemTypes"),
            Some("Movie")
        );
    }
}
