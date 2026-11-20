//! Bibliotheksseiten, Teil 2: Laden, Raster, Menü, Fokus und Ereignisse (Zustand und Typen in `library.rs`).

use std::rc::Rc;

use lfx_api::{BaseItem, Client, ItemKind};
use lfx_core::billboard::{self, PoolOpts};
use lfx_core::call::{ApiCall, Endpoint};
use lfx_core::images::{CardKind, card_images, display_title};
use lfx_core::library::{
    GRID_PAGE_SIZE, GenreRef, LibSort, LibraryInput, LibraryKind, MYLIST_SORTS, MyListType,
    build_library_rows, count_text, genres_call, genres_from, grid_page_call, mylist_empty_text,
    mylist_page_call, sort_params,
};
use lfx_core::rows::logic::base_query;
use lfx_core::rows::{RowKind, load_row};
use lfx_images::{ImageRequest, Priority};
use lfx_nav::{FocusGraph, InputContext, Rect, Target, Zone, ZoneId};
use lfx_ui::{
    CardAspect, CardData, HeroData, LibChip, LibGeometry, LibTool, LibraryState, NpEntry, RowData,
};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use tokio::task::JoinSet;

use crate::app::{App, NAV_LABELS, post};
use crate::library::{
    EmptyAction, GridSrc, LibGeo, LibRow, LibState, MENU_COLS_THRESHOLD, Menu, Mode, Tool, ToolAct,
    Z_ALPHA, Z_BODY, Z_FIELD, Z_HERO, Z_MENU, Z_NAV, Z_ROW0, Z_TOOL, card_data, is_library_route,
    nav_index_for, rng_from_clock, to_buffer, tool_width,
};
use crate::route::{LibraryView, Route};

/// Daten der Zeilenansicht (Send).
struct RowsData {
    genres: Vec<GenreRef>,
    rows: Vec<(String, String, RowKind, Vec<BaseItem>)>,
    pool: Vec<BaseItem>,
}

async fn fetch_rows(
    client: &Client,
    kind: LibraryKind,
    genre: Option<String>,
) -> Result<RowsData, String> {
    let genres = genres_call(kind)
        .execute(client)
        .await
        .map(|g| genres_from(&g))
        .unwrap_or_default();
    let seed = lfx_core::dayseed::day_key_from_epoch_ms(App::now_ms() as i64, 0);
    let defs = build_library_rows(&LibraryInput {
        kind,
        genre: genre.as_deref(),
        genres: &genres,
        day_key: &seed,
        brand: "LENZFLIX",
        charts_configured: false,
        charts: None,
    });
    let mut set = JoinSet::new();
    for (i, def) in defs.into_iter().enumerate() {
        let c = client.clone();
        set.spawn(async move {
            let r = load_row(&c, &def).await;
            (i, def, r)
        });
    }
    type Slot = Option<(String, String, RowKind, Vec<BaseItem>)>;
    let mut slots: Vec<Slot> = Vec::new();
    let mut errors = 0usize;
    while let Some(res) = set.join_next().await {
        let Ok((i, def, result)) = res else { continue };
        if slots.len() <= i {
            slots.resize_with(i + 1, || None);
        }
        match result {
            Ok(r) if !r.items.is_empty() => {
                slots[i] = Some((
                    def.id.clone(),
                    r.title.or(def.title.clone()).unwrap_or_default(),
                    def.kind,
                    r.items,
                ));
            }
            Ok(_) => {}
            Err(_) => errors += 1,
        }
    }
    let rows: Vec<_> = slots.into_iter().flatten().collect();
    let (latest_call, random_call) =
        billboard::library_pool_calls(kind.item_kind(), genre.as_deref());
    let (latest, random) = tokio::join!(latest_call.execute(client), random_call.execute(client));
    let (latest, random) = (latest.unwrap_or_default(), random.unwrap_or_default());
    let mut rng = rng_from_clock();
    let mut pool = billboard::build_pool(&latest, &random, &PoolOpts::library(), &mut rng);
    if pool.is_empty() {
        pool = latest
            .iter()
            .chain(random.iter())
            .filter(|i| !i.backdrop_image_tags.is_empty())
            .take(8)
            .cloned()
            .collect();
    }
    if rows.is_empty() && pool.is_empty() && errors > 0 {
        return Err("Die Seite konnte nicht geladen werden. Ist der Server erreichbar?".into());
    }
    Ok(RowsData { genres, rows, pool })
}

fn grid_call(src: &GridSrc, start: usize) -> ApiCall {
    match src {
        GridSrc::Lib { kind, genre, sort } => {
            grid_page_call(*kind, genre.as_deref(), *sort, start, GRID_PAGE_SIZE)
        }
        GridSrc::MyList(r) => mylist_page_call(r, start, GRID_PAGE_SIZE),
        GridSrc::Collection { id, sort, letter } => {
            let (by, order) = sort_params(*sort, false);
            let mut q = base_query()
                .parent_id(id)
                .raw("sortBy", by)
                .raw("sortOrder", order)
                .total_count(true)
                .start_index(start as u32)
                .limit(GRID_PAGE_SIZE as u32);
            match letter {
                Some('#') => q = q.raw("nameLessThan", "A"),
                Some(c) => q = q.name_starts_with(&c.to_string()),
                None => {}
            }
            ApiCall::new(Endpoint::Items, q)
        }
    }
}

/// Sammlungstypen, die später kommen (hier nur ein Hinweis).
fn collection_soon(ctype: &str) -> bool {
    matches!(
        ctype,
        "music" | "books" | "photos" | "livetv" | "musicvideos" | "audiobooks"
    )
}

const LOAD_ERROR: &str = "Die Titel konnten nicht geladen werden. Ist der Server erreichbar?";

impl App {
    pub fn with_lib<R>(&self, f: impl FnOnce(&LibraryState) -> R) -> R {
        let ui = self.win();
        let g = ui.global::<LibraryState>();
        f(&g)
    }

    pub fn lib_geo(&self) -> LibGeo {
        self.st.borrow().lib.geo.unwrap_or_default()
    }

    fn lib_retro(&self) -> bool {
        self.st.borrow().design == crate::cli::DesignId::Retro
    }

    // ------------------------------------------------------------ Einstieg

    /// Baut die Seite für eine Bibliotheksroute auf und startet das Laden. Gibt den Eingabekontext zurück.
    pub fn render_library_route(self: &Rc<Self>, route: &Route) -> InputContext {
        let ui = self.win();
        ui.set_page(lfx_ui::Page::Library);
        self.st.borrow_mut().graph = FocusGraph::new();
        let (mode, title) = match route {
            Route::Library {
                kind,
                view: LibraryView::Rows,
                ..
            } => (Mode::Rows, kind.label().to_owned()),
            Route::Library { kind, .. } => (Mode::Grid, kind.label().to_owned()),
            Route::MyList => (Mode::MyList, "Meine Liste".to_owned()),
            Route::Collection { name, .. } => (Mode::Collection, name.clone()),
            Route::NewPopular => (Mode::NewPop, String::new()),
            _ => (Mode::Search, String::new()),
        };
        let epoch = {
            let mut st = self.st.borrow_mut();
            let keep_geo = st.lib.geo;
            let keep_mylist = std::mem::take(&mut st.lib.mylist);
            let keep_search = std::mem::take(&mut st.lib.search);
            let epoch = st.lib.epoch + 1;
            st.lib = LibState {
                mode,
                epoch,
                geo: keep_geo,
                mylist: keep_mylist,
                search: keep_search,
                vis: (0, 48),
                ..LibState::default()
            };
            epoch
        };
        self.with_lib(|g| {
            g.set_mode(mode.index());
            g.set_title(title.as_str().into());
            g.set_count_text("".into());
            g.set_nav_labels(ModelRc::new(VecModel::from(
                NAV_LABELS
                    .iter()
                    .map(|s| SharedString::from(*s))
                    .collect::<Vec<_>>(),
            )));
            g.set_nav_current(nav_index_for(route));
            g.set_items(ModelRc::new(VecModel::<CardData>::default()));
            g.set_rows(ModelRc::new(VecModel::<RowData>::default()));
            g.set_entries(ModelRc::new(VecModel::<NpEntry>::default()));
            g.set_hero(HeroData::default());
            g.set_chips(ModelRc::new(VecModel::<LibChip>::default()));
            g.set_alpha(ModelRc::new(VecModel::<SharedString>::default()));
            g.set_tools_l(ModelRc::new(VecModel::<LibTool>::default()));
            g.set_tools_r(ModelRc::new(VecModel::<LibTool>::default()));
            g.set_loading(true);
            g.set_loading_more(false);
            g.set_error("".into());
            g.set_empty_title("".into());
            g.set_empty_hint("".into());
            g.set_empty_action("".into());
            g.set_menu_open(false);
            g.set_kbd_open(false);
            g.set_note("".into());
            g.set_focus_zone(-1);
            g.set_focus_index(0);
            g.set_alpha_current(-1);
        });
        if self.st.borrow().images.is_none()
            && let Some(a) = self.session.active()
        {
            self.ensure_images(&a.client);
        }
        let mut ctx = InputContext::Browse;
        match route {
            Route::Library {
                kind,
                genre,
                view,
                sort,
            } => {
                {
                    let mut st = self.st.borrow_mut();
                    st.lib.route_kind = Some(*kind);
                    st.lib.genre = genre.clone();
                    st.lib.sort = Some(*sort);
                }
                self.lib_build_tools(route);
                self.lib_load_genres(epoch, *kind);
                match view {
                    LibraryView::Rows => self.lib_load_rows(epoch, *kind, genre.clone()),
                    LibraryView::Grid => {
                        self.st.borrow_mut().lib.src = Some(GridSrc::Lib {
                            kind: *kind,
                            genre: genre.clone(),
                            sort: *sort,
                        });
                        self.lib_more();
                    }
                }
            }
            Route::MyList => {
                let r = self.st.borrow().lib.mylist.clone();
                self.st.borrow_mut().lib.src = Some(GridSrc::MyList(r));
                self.lib_build_tools(route);
                self.lib_more();
            }
            Route::Collection { id, name } => {
                {
                    let mut st = self.st.borrow_mut();
                    st.lib.coll_id = id.clone();
                    st.lib.coll_name = name.clone();
                    st.lib.sort = Some(LibSort::Az);
                    st.lib.alpha = ["Alle".to_owned(), "#".to_owned()]
                        .into_iter()
                        .chain(('A'..='Z').map(|c| c.to_string()))
                        .collect();
                }
                self.lib_open_collection(epoch, id.clone());
            }
            Route::NewPopular => self.np_enter(epoch),
            _ => ctx = self.search_enter(epoch),
        }
        self.lib_rebuild_graph();
        self.lib_default_focus();
        ctx
    }

    // ------------------------------------------------------------ Werkzeuge

    pub fn lib_build_tools(self: &Rc<Self>, route: &Route) {
        let mut tools: Vec<Tool> = Vec::new();
        let genres = self.st.borrow().lib.genres.clone();
        match route {
            Route::Library {
                kind,
                genre,
                view,
                sort,
            } => {
                let found = genre
                    .as_ref()
                    .and_then(|g| genres.iter().find(|x| &x.id == g));
                tools.push(Tool {
                    label: found.map_or_else(|| "Genres".to_owned(), |g| g.name.clone()),
                    kind: 4,
                    active: genre.is_some(),
                    act: ToolAct::GenreMenu,
                    right: false,
                });
                if *view == LibraryView::Grid {
                    tools.push(Tool {
                        label: sort.label().to_owned(),
                        kind: 4,
                        active: false,
                        act: ToolAct::SortMenu,
                        right: true,
                    });
                }
                tools.push(Tool {
                    label: "Zeilen".into(),
                    kind: 2,
                    active: *view == LibraryView::Rows,
                    act: ToolAct::View(LibraryView::Rows),
                    right: true,
                });
                tools.push(Tool {
                    label: "Raster".into(),
                    kind: 3,
                    active: *view == LibraryView::Grid,
                    act: ToolAct::View(LibraryView::Grid),
                    right: true,
                });
                // Titel mit Genre als Brotkrume
                if genre.is_some() {
                    let name = found.map_or("…", |x| x.name.as_str());
                    let t = format!("{} › {name}", kind.label());
                    self.with_lib(|l| l.set_title(t.as_str().into()));
                }
            }
            Route::MyList => {
                let r = self.st.borrow().lib.mylist.clone();
                for (label, t) in [
                    ("Alle", None),
                    ("Filme", Some(MyListType::Movies)),
                    ("Serien", Some(MyListType::Series)),
                ] {
                    tools.push(Tool {
                        label: label.to_owned(),
                        kind: 0,
                        active: r.kind == t,
                        act: ToolAct::MyType(t),
                        right: false,
                    });
                }
                for s in MYLIST_SORTS {
                    tools.push(Tool {
                        label: s.label().to_owned(),
                        kind: 1,
                        active: r.effective_sort() == s,
                        act: ToolAct::MySort(s),
                        right: true,
                    });
                }
            }
            Route::Collection { .. } => {
                let s = self.st.borrow().lib.sort.unwrap_or(LibSort::Az);
                tools.push(Tool {
                    label: s.label().to_owned(),
                    kind: 4,
                    active: false,
                    act: ToolAct::SortMenu,
                    right: true,
                });
            }
            _ => {}
        }
        self.lib_set_tools(tools);
    }

    /// Setzt die Werkzeuge (links zuerst, dann rechts; der Fokus-Index zählt in dieser Reihenfolge).
    pub fn lib_set_tools(self: &Rc<Self>, tools: Vec<Tool>) {
        let left: Vec<Tool> = tools.iter().filter(|t| !t.right).cloned().collect();
        let right: Vec<Tool> = tools.iter().filter(|t| t.right).cloned().collect();
        let n_left = left.len();
        let all: Vec<Tool> = left.into_iter().chain(right).collect();
        let conv = |t: &Tool| LibTool {
            label: t.label.as_str().into(),
            kind: t.kind,
            active: t.active,
        };
        let l: Vec<LibTool> = all[..n_left].iter().map(conv).collect();
        let r: Vec<LibTool> = all[n_left..].iter().map(conv).collect();
        self.with_lib(|g| {
            g.set_tools_l(ModelRc::new(VecModel::from(l)));
            g.set_tools_r(ModelRc::new(VecModel::from(r)));
        });
        let mut st = self.st.borrow_mut();
        st.lib.tools = all;
        st.lib.n_left = n_left;
    }

    fn lib_title_len(&self) -> usize {
        let st = self.st.borrow();
        match st.router.current() {
            Route::Library { kind, genre, .. } => {
                kind.label().chars().count() + genre.as_ref().map_or(0, |_| 12)
            }
            Route::MyList => 11,
            Route::Collection { name, .. } => name.chars().count(),
            _ => 0,
        }
    }

    fn lib_tool_rects(&self) -> Vec<Rect> {
        let geo = self.lib_geo();
        let title_w = self.lib_title_len() as f32 * 15.0 + 16.0;
        let st = self.st.borrow();
        let y = geo.nav_h + 11.0;
        let mut out = Vec::new();
        let mut x = geo.gutter + title_w;
        for t in st.lib.tools.iter().take(st.lib.n_left) {
            let w = tool_width(&t.label, t.kind);
            out.push(Rect::new(x, y, w, 34.0));
            x += w + 10.0;
        }
        let right: Vec<&Tool> = st.lib.tools.iter().skip(st.lib.n_left).collect();
        let total: f32 = right
            .iter()
            .map(|t| tool_width(&t.label, t.kind) + 10.0)
            .sum();
        let mut x = geo.width - geo.gutter - total + 10.0;
        for t in right {
            let w = tool_width(&t.label, t.kind);
            out.push(Rect::new(x, y, w, 34.0));
            x += w + 10.0;
        }
        out
    }

    // ------------------------------------------------------------ Genres, Zeilen

    fn lib_load_genres(self: &Rc<Self>, epoch: u64, kind: LibraryKind) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            if let Ok(items) = genres_call(kind).execute(&client).await {
                let g = genres_from(&items);
                post(move |app| app.lib_genres_loaded(epoch, g));
            }
        });
    }

    fn lib_genres_loaded(self: &Rc<Self>, epoch: u64, genres: Vec<GenreRef>) {
        if self.st.borrow().lib.epoch != epoch {
            return;
        }
        self.st.borrow_mut().lib.genres = genres;
        let route = self.st.borrow().router.current().clone();
        if matches!(route, Route::Library { .. }) {
            self.lib_build_tools(&route);
            self.lib_rebuild_graph();
        }
    }

    fn lib_load_rows(self: &Rc<Self>, epoch: u64, kind: LibraryKind, genre: Option<String>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let data = fetch_rows(&client, kind, genre).await;
            post(move |app| app.lib_rows_loaded(epoch, data));
        });
    }

    fn lib_rows_loaded(self: &Rc<Self>, epoch: u64, data: Result<RowsData, String>) {
        if self.st.borrow().lib.epoch != epoch {
            return;
        }
        self.with_lib(|g| g.set_loading(false));
        let data = match data {
            Ok(d) => d,
            Err(text) => {
                self.with_lib(|g| g.set_error(text.as_str().into()));
                self.st.borrow_mut().lib.error = text;
                self.lib_rebuild_graph();
                self.lib_default_focus();
                return;
            }
        };
        let now = App::now_ms();
        let rows: Vec<LibRow> = data
            .rows
            .into_iter()
            .map(|(id, title, kind, items)| {
                let ck = match kind {
                    RowKind::Resume => CardKind::Resume,
                    _ => CardKind::Landscape,
                };
                let cards: Vec<CardData> = items
                    .iter()
                    .enumerate()
                    .map(|(i, it)| {
                        card_data(it, ck, (kind == RowKind::Top10).then_some(i + 1), None, now)
                    })
                    .collect();
                LibRow {
                    id,
                    title,
                    kind: ck,
                    aspect: CardAspect::Wide,
                    items,
                    model: Rc::new(VecModel::from(cards)),
                }
            })
            .collect();
        let row_data: Vec<RowData> = rows
            .iter()
            .map(|r| RowData {
                id: r.id.as_str().into(),
                title: r.title.as_str().into(),
                aspect: r.aspect,
                items: ModelRc::from(r.model.clone()),
            })
            .collect();
        self.with_lib(|g| g.set_rows(ModelRc::new(VecModel::from(row_data))));
        {
            let mut st = self.st.borrow_mut();
            st.lib.genres = data.genres;
            st.lib.rows = rows;
            st.lib.pool = data.pool;
            st.lib.hero_idx = 0;
        }
        let route = self.st.borrow().router.current().clone();
        self.lib_build_tools(&route);
        self.lib_show_hero();
        self.lib_load_row_images(epoch);
        self.lib_start_hero_rotation();
        self.lib_rebuild_graph();
        self.lib_default_focus();
        tracing::info!(
            rows = self.st.borrow().lib.rows.len(),
            "Bibliothek (Zeilen) geladen"
        );
    }

    /// Bilder aller Zeilenkarten (Zeilen sind kurz) laden.
    pub fn lib_load_row_images(self: &Rc<Self>, epoch: u64) {
        let jobs: Vec<(usize, usize, lfx_core::images::ImageRef)> = {
            let st = self.st.borrow();
            st.lib
                .rows
                .iter()
                .enumerate()
                .flat_map(|(r, row)| {
                    let k = row.kind;
                    row.items.iter().enumerate().filter_map(move |(i, it)| {
                        card_images(it, k, false, 2.0, 1.0).main.map(|m| (r, i, m))
                    })
                })
                .collect()
        };
        for (r, i, m) in jobs {
            self.lib_fetch_image(&m, Priority::Visible, move |app, img| {
                if app.st.borrow().lib.epoch != epoch {
                    return;
                }
                let model = app.st.borrow().lib.rows.get(r).map(|x| x.model.clone());
                if let Some(model) = model
                    && let Some(mut c) = model.row_data(i)
                {
                    c.image = img;
                    c.has_image = true;
                    model.set_row_data(i, c);
                }
            });
        }
    }

    // ------------------------------------------------------------ Billboard

    fn lib_hero_item(&self) -> Option<BaseItem> {
        let st = self.st.borrow();
        st.lib.pool.get(st.lib.hero_idx).cloned()
    }

    fn lib_show_hero(self: &Rc<Self>) {
        let Some(item) = self.lib_hero_item() else {
            self.with_lib(|g| g.set_hero(HeroData::default()));
            return;
        };
        let meta = billboard::meta_parts(&item)
            .into_iter()
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join(" · ");
        self.with_lib(|g| {
            g.set_hero(HeroData {
                title: display_title(&item).into(),
                meta: meta.into(),
                genres: billboard::genre_line(&item).into(),
                overview: "".into(),
                backdrop: Image::default(),
                has_backdrop: false,
                logo: Image::default(),
                has_logo: false,
                has_item: true,
            });
        });
        let epoch = self.st.borrow().lib.epoch;
        let imgs = billboard::hero_images(&item, 2.0, 1.0);
        let id = item.id.clone();
        if let Some(r) = imgs.backdrop {
            self.lib_fetch_image(&r, Priority::Visible, move |app, img| {
                app.lib_set_hero_image(epoch, &id, img, false);
            });
        }
        if let Some(r) = imgs.logo {
            let id = item.id.clone();
            self.lib_fetch_image(&r, Priority::Visible, move |app, img| {
                app.lib_set_hero_image(epoch, &id, img, true);
            });
        }
    }

    fn lib_set_hero_image(&self, epoch: u64, item_id: &str, img: Image, logo: bool) {
        if self.st.borrow().lib.epoch != epoch
            || self.lib_hero_item().is_none_or(|h| h.id != item_id)
        {
            return;
        }
        self.with_lib(|g| {
            let mut h = g.get_hero();
            if logo {
                h.logo = img;
                h.has_logo = true;
            } else {
                h.backdrop = img;
                h.has_backdrop = true;
            }
            g.set_hero(h);
        });
    }

    fn lib_start_hero_rotation(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.st.borrow().lib.hero_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(billboard::ROTATE_MS),
            move || {
                let Some(app) = weak.upgrade() else { return };
                let (rows_mode, on_hero, len) = {
                    let st = app.st.borrow();
                    (
                        st.lib.mode == Mode::Rows
                            && matches!(st.router.current(), Route::Library { .. }),
                        st.graph.focused().is_some_and(|t| t.zone == Z_HERO),
                        st.lib.pool.len(),
                    )
                };
                if !rows_mode || on_hero || !billboard::rotation_enabled(len, false) {
                    return;
                }
                {
                    let mut st = app.st.borrow_mut();
                    st.lib.hero_idx = billboard::next_index(st.lib.hero_idx, len);
                }
                app.lib_show_hero();
            },
        );
    }

    // ------------------------------------------------------------ Bilder

    /// Lädt ein Bild und ruft `apply` im UI-Thread auf.
    pub fn lib_fetch_image(
        &self,
        r: &lfx_core::images::ImageRef,
        prio: Priority,
        apply: impl FnOnce(&Rc<App>, Image) + Send + 'static,
    ) {
        let Some(loader) = self.st.borrow().images.clone() else {
            return;
        };
        let mut req = ImageRequest::new(&r.pick.id, r.pick.kind)
            .tag(&r.pick.tag)
            .quality(r.quality);
        req.max_width = Some(r.max_width);
        self.rt.spawn(async move {
            if let Ok(img) = loader.request(req, prio).await {
                let buf = to_buffer(&img);
                post(move |app| apply(app, Image::from_rgba8(buf)));
            }
        });
    }

    /// Lädt die Bilder der sichtbaren Rasterkarten (plus eine Reihe Puffer), jede Karte nur einmal.
    pub fn lib_load_grid_images(self: &Rc<Self>) {
        let (epoch, jobs) = {
            let mut st = self.st.borrow_mut();
            let cols = st.lib.geo.map_or(5, |g| g.cols);
            let (a, b) = st.lib.vis;
            let end = (b + cols).min(st.lib.items.len());
            let mut jobs = Vec::new();
            for idx in a.min(end)..end {
                if st.lib.requested.insert(idx)
                    && let Some(m) =
                        card_images(&st.lib.items[idx], CardKind::Landscape, false, 2.0, 1.0).main
                {
                    jobs.push((idx, m));
                }
            }
            (st.lib.epoch, jobs)
        };
        for (idx, m) in jobs {
            self.lib_fetch_image(&m, Priority::Visible, move |app, img| {
                if app.st.borrow().lib.epoch != epoch {
                    return;
                }
                let model = app.st.borrow().lib.model.clone();
                if let Some(model) = model
                    && let Some(mut c) = model.row_data(idx)
                {
                    c.image = img;
                    c.has_image = true;
                    model.set_row_data(idx, c);
                }
            });
        }
    }

    // ------------------------------------------------------------ Raster

    /// Lädt die nächste Seite des Rasters (Endlos-Scroll).
    pub fn lib_more(self: &Rc<Self>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        let (epoch, seq, call, first) = {
            let mut st = self.st.borrow_mut();
            if !matches!(st.lib.mode, Mode::Grid | Mode::MyList | Mode::Collection)
                || !st.lib.pager.can_load()
            {
                return;
            }
            let Some(src) = st.lib.src.clone() else {
                return;
            };
            let start = st.lib.pager.count;
            let seq = st.lib.pager.begin();
            (st.lib.epoch, seq, grid_call(&src, start), start == 0)
        };
        self.with_lib(|g| {
            if first {
                g.set_loading(true);
            } else {
                g.set_loading_more(true);
            }
        });
        self.rt.spawn(async move {
            let res = call
                .execute_page(&client)
                .await
                .map_err(|_| LOAD_ERROR.to_owned());
            post(move |app| app.lib_page_loaded(epoch, seq, res));
        });
    }

    fn lib_page_loaded(
        self: &Rc<Self>,
        epoch: u64,
        seq: u64,
        res: Result<(Vec<BaseItem>, Option<usize>), String>,
    ) {
        if self.st.borrow().lib.epoch != epoch {
            return;
        }
        self.with_lib(|g| {
            g.set_loading(false);
            g.set_loading_more(false);
        });
        let retro = self.lib_retro();
        match res {
            Ok((items, total)) => {
                let now = App::now_ms();
                let got = items.len();
                let applied = self
                    .st
                    .borrow_mut()
                    .lib
                    .pager
                    .apply(seq, got, total, GRID_PAGE_SIZE);
                if !applied {
                    return;
                }
                let cards: Vec<CardData> = items
                    .iter()
                    .map(|it| card_data(it, CardKind::Landscape, None, None, now))
                    .collect();
                let model = {
                    let mut st = self.st.borrow_mut();
                    st.lib.items.extend(items);
                    st.lib.error.clear();
                    st.lib
                        .model
                        .get_or_insert_with(|| Rc::new(VecModel::default()))
                        .clone()
                };
                let first_page = model.row_count() == 0;
                for c in cards {
                    model.push(c);
                }
                if first_page {
                    self.with_lib(|g| g.set_items(ModelRc::from(model.clone())));
                }
                let (count, mode, empty) = {
                    let st = self.st.borrow();
                    (
                        st.lib.pager.display_count(),
                        st.lib.mode,
                        st.lib.items.is_empty(),
                    )
                };
                let text = if empty {
                    String::new()
                } else {
                    count_text(count, retro && mode == Mode::MyList)
                };
                self.with_lib(|g| {
                    g.set_error("".into());
                    g.set_count_text(text.as_str().into());
                });
                if empty {
                    self.lib_set_empty();
                } else {
                    self.with_lib(|g| {
                        g.set_empty_title("".into());
                        g.set_empty_hint("".into());
                        g.set_empty_action("".into());
                    });
                    self.st.borrow_mut().lib.empty_action = EmptyAction::None;
                }
                self.lib_load_grid_images();
                self.lib_rebuild_graph();
                self.lib_default_focus();
            }
            Err(text) => {
                self.st.borrow_mut().lib.pager.fail(seq);
                let empty = self.st.borrow().lib.items.is_empty();
                if empty {
                    self.st.borrow_mut().lib.error = text.clone();
                    self.with_lib(|g| g.set_error(text.as_str().into()));
                    self.lib_rebuild_graph();
                    self.lib_default_focus();
                } else {
                    self.toast(&text, lfx_ui::ToastKind::Error);
                    self.st.borrow_mut().lib.pager.retry();
                }
            }
        }
    }

    /// Leerzustand je Seite.
    fn lib_set_empty(self: &Rc<Self>) {
        let retro = self.lib_retro();
        let (mode, r, letter) = {
            let st = self.st.borrow();
            (st.lib.mode, st.lib.mylist.clone(), st.lib.letter)
        };
        let (title, hint, action, act) = match mode {
            Mode::MyList => {
                let (t, show_hint, link, _) = mylist_empty_text(&r, retro);
                (
                    t,
                    if show_hint {
                        "Mit dem Plus an einem Titel merkst du ihn dir hier vor.".to_owned()
                    } else {
                        String::new()
                    },
                    link,
                    if r.kind.is_some() {
                        EmptyAction::ShowAll
                    } else {
                        EmptyAction::Home
                    },
                )
            }
            Mode::Collection if letter.is_some() => (
                "Keine Titel mit diesem Anfangsbuchstaben".to_owned(),
                String::new(),
                "Alle anzeigen".to_owned(),
                EmptyAction::ClearLetter,
            ),
            _ => (
                "Hier gibt es noch nichts".to_owned(),
                String::new(),
                String::new(),
                EmptyAction::None,
            ),
        };
        self.st.borrow_mut().lib.empty_action = act;
        self.with_lib(|g| {
            g.set_empty_title(title.as_str().into());
            g.set_empty_hint(hint.as_str().into());
            g.set_empty_action(action.as_str().into());
        });
    }

    /// Raster neu beginnen (Filter oder Sortierung geändert).
    pub fn lib_restart_grid(self: &Rc<Self>, src: GridSrc) {
        {
            let mut st = self.st.borrow_mut();
            st.lib.epoch += 1;
            st.lib.pager.reset();
            st.lib.items.clear();
            st.lib.requested.clear();
            st.lib.model = None;
            st.lib.vis = (0, 48);
            st.lib.error.clear();
            st.lib.empty_action = EmptyAction::None;
            st.lib.src = Some(src);
        }
        self.with_lib(|g| {
            g.set_items(ModelRc::new(VecModel::<CardData>::default()));
            g.set_count_text("".into());
            g.set_error("".into());
            g.set_empty_title("".into());
            g.set_empty_hint("".into());
            g.set_empty_action("".into());
        });
        self.lib_more();
    }

    // ------------------------------------------------------------ Sammlungen

    fn lib_open_collection(self: &Rc<Self>, epoch: u64, id: String) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let q = lfx_api::ItemsQuery::new().ids(&[id.as_str()]);
            let ctype = client
                .items(&q)
                .await
                .ok()
                .and_then(|r| r.items.into_iter().next())
                .and_then(|i| i.collection_type)
                .unwrap_or_default();
            post(move |app| app.lib_collection_ready(epoch, &ctype));
        });
    }

    fn lib_collection_ready(self: &Rc<Self>, epoch: u64, ctype: &str) {
        if self.st.borrow().lib.epoch != epoch {
            return;
        }
        if collection_soon(ctype) {
            self.with_lib(|g| {
                g.set_loading(false);
                g.set_note("Diese Bibliothek folgt in einer späteren Version.".into());
            });
            return;
        }
        let route = self.st.borrow().router.current().clone();
        self.lib_build_tools(&route);
        let alpha: Vec<SharedString> = self
            .st
            .borrow()
            .lib
            .alpha
            .iter()
            .map(|s| SharedString::from(s.as_str()))
            .collect();
        self.with_lib(|g| {
            g.set_alpha(ModelRc::new(VecModel::from(alpha)));
            g.set_alpha_current(0);
        });
        let id = self.st.borrow().lib.coll_id.clone();
        self.st.borrow_mut().lib.src = Some(GridSrc::Collection {
            id,
            sort: LibSort::Az,
            letter: None,
        });
        self.lib_more();
        self.lib_rebuild_graph();
    }

    // ------------------------------------------------------------ Menü

    fn lib_open_menu(self: &Rc<Self>, genre: bool, tool_index: usize) {
        let (items, current) = if genre {
            let st = self.st.borrow();
            let mut v = vec!["Alle Genres".to_owned()];
            v.extend(st.lib.genres.iter().map(|g| g.name.clone()));
            let cur = st
                .lib
                .genre
                .as_ref()
                .and_then(|id| st.lib.genres.iter().position(|g| &g.id == id))
                .map_or(0, |p| p + 1);
            (v, cur as i32)
        } else {
            let cur = self.st.borrow().lib.sort.unwrap_or(LibSort::Az);
            let p = LibSort::ALL.iter().position(|s| *s == cur).unwrap_or(0);
            (
                LibSort::ALL.iter().map(|s| s.label().to_owned()).collect(),
                p as i32,
            )
        };
        let cols = if items.len() > MENU_COLS_THRESHOLD {
            3
        } else {
            1
        };
        let rects = self.lib_tool_rects();
        let x = rects.get(tool_index).map_or(0.0, |r| r.x);
        let saved = self.st.borrow().graph.focused();
        let geo = self.lib_geo();
        let n = items.len();
        self.with_lib(|g| {
            g.set_menu_items(ModelRc::new(VecModel::from(
                items
                    .iter()
                    .map(|s| SharedString::from(s.as_str()))
                    .collect::<Vec<_>>(),
            )));
            g.set_menu_current(current);
            g.set_menu_cols(cols as i32);
            g.set_menu_x(x);
            g.set_menu_open(true);
        });
        // Fokusfalle: nur das Menü.
        let mut graph = FocusGraph::new();
        let rect = Rect::new(
            x,
            geo.nav_h + geo.head_h + 4.0,
            cols as f32 * 200.0,
            n.div_ceil(cols) as f32 * 34.0,
        );
        graph.register_zone(Zone::grid(Z_MENU, rect, n, cols));
        let _ = graph.set_focus(Target {
            zone: Z_MENU,
            index: current.max(0) as usize,
        });
        {
            let mut st = self.st.borrow_mut();
            st.graph = graph;
            st.lib.menu = Some(Menu {
                genre,
                items,
                cols,
                saved,
            });
        }
        self.lib_show_focus();
    }

    pub fn lib_close_menu(self: &Rc<Self>) {
        let saved = self.st.borrow_mut().lib.menu.take().and_then(|m| m.saved);
        self.with_lib(|g| g.set_menu_open(false));
        self.lib_rebuild_graph();
        if let Some(t) = saved {
            let _ = self.st.borrow_mut().graph.set_focus(t);
        }
        self.lib_show_focus();
    }

    fn lib_menu_pick(self: &Rc<Self>, i: usize) {
        let Some(menu) = self.st.borrow().lib.menu.clone() else {
            return;
        };
        let route = self.st.borrow().router.current().clone();
        self.st.borrow_mut().lib.menu = None;
        self.with_lib(|g| g.set_menu_open(false));
        if menu.genre {
            if let Route::Library {
                kind, view, sort, ..
            } = route
            {
                let genre = if i == 0 {
                    None
                } else {
                    self.st.borrow().lib.genres.get(i - 1).map(|g| g.id.clone())
                };
                self.go_replace(Route::Library {
                    kind,
                    genre,
                    view,
                    sort,
                });
            }
        } else if let Some(s) = LibSort::ALL.get(i).copied() {
            match route {
                Route::Library {
                    kind, genre, view, ..
                } => self.go_replace(Route::Library {
                    kind,
                    genre,
                    view,
                    sort: s,
                }),
                Route::Collection { .. } => {
                    let (id, letter) = {
                        let mut st = self.st.borrow_mut();
                        st.lib.sort = Some(s);
                        (st.lib.coll_id.clone(), st.lib.letter)
                    };
                    self.lib_restart_grid(GridSrc::Collection {
                        id,
                        sort: s,
                        letter,
                    });
                    self.lib_build_tools(&route);
                    self.lib_rebuild_graph();
                    self.lib_default_focus();
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------ Fokus

    /// Baut die Fokuszonen der aktuellen Seite neu; der Fokus bleibt, wenn das Ziel noch existiert.
    pub fn lib_rebuild_graph(self: &Rc<Self>) {
        if self.st.borrow().lib.menu.is_some() {
            return;
        }
        let old = self.st.borrow().graph.focused();
        let mut g = self.lib_build_graph();
        if let Some(t) = old {
            let _ = g.set_focus(t);
        }
        self.st.borrow_mut().graph = g;
        self.lib_show_focus();
    }

    fn lib_row_zones(&self, g: &mut FocusGraph, geo: &LibGeo, top: f32) {
        let st = self.st.borrow();
        for (r, row) in st.lib.rows.iter().enumerate() {
            let y = top + r as f32 * geo.row_h + 36.0;
            let items: Vec<Rect> = (0..row.items.len())
                .map(|i| {
                    Rect::new(
                        geo.gutter + i as f32 * (geo.card_w + geo.gap),
                        y,
                        geo.card_w,
                        geo.card_h,
                    )
                })
                .collect();
            g.register_zone(
                Zone::horizontal(
                    ZoneId(Z_ROW0 + r as u32),
                    Rect::new(
                        geo.gutter,
                        y,
                        (items.len() as f32 * (geo.card_w + geo.gap)).max(1.0),
                        geo.card_h,
                    ),
                    items.len(),
                )
                .with_items(items)
                .remembering(true),
            );
        }
    }

    fn lib_build_graph(&self) -> FocusGraph {
        let geo = self.lib_geo();
        let tool_rects = self.lib_tool_rects();
        let mut g = FocusGraph::new();
        let nav: Vec<Rect> = (0..NAV_LABELS.len())
            .map(|i| Rect::new(geo.gutter + i as f32 * 130.0, 6.0, 120.0, geo.nav_h - 12.0))
            .collect();
        g.register_zone(
            Zone::horizontal(Z_NAV, Rect::new(0.0, 0.0, geo.width, geo.nav_h), nav.len())
                .with_items(nav)
                .remembering(true),
        );
        let mode = self.st.borrow().lib.mode;
        if mode == Mode::Search {
            return self.search_graph(g, &geo);
        }
        let y0 = geo.nav_h + geo.head_h;
        if !tool_rects.is_empty() {
            g.register_zone(
                Zone::horizontal(
                    Z_TOOL,
                    Rect::new(geo.gutter, geo.nav_h, geo.width - 2.0 * geo.gutter, 56.0),
                    tool_rects.len(),
                )
                .with_items(tool_rects)
                .remembering(true),
            );
        }
        {
            let st = self.st.borrow();
            if mode == Mode::Collection && !st.lib.alpha.is_empty() {
                let items: Vec<Rect> = (0..st.lib.alpha.len())
                    .map(|i| {
                        Rect::new(
                            geo.gutter + i as f32 * 32.0,
                            geo.nav_h + 56.0 + 5.0,
                            30.0,
                            30.0,
                        )
                    })
                    .collect();
                g.register_zone(
                    Zone::horizontal(
                        Z_ALPHA,
                        Rect::new(
                            geo.gutter,
                            geo.nav_h + 56.0,
                            geo.width - 2.0 * geo.gutter,
                            40.0,
                        ),
                        items.len(),
                    )
                    .with_items(items)
                    .remembering(true),
                );
            }
        }
        match mode {
            Mode::Rows => {
                let (has_hero, has_err, no_rows) = {
                    let st = self.st.borrow();
                    (
                        !st.lib.pool.is_empty(),
                        !st.lib.error.is_empty(),
                        st.lib.rows.is_empty(),
                    )
                };
                if has_hero {
                    let play = Rect::new(geo.gutter, geo.btn_y, 170.0, geo.btn_h);
                    let info = Rect::new(geo.gutter + 184.0, geo.btn_y, 190.0, geo.btn_h);
                    g.register_zone(
                        Zone::horizontal(
                            Z_HERO,
                            Rect::new(geo.gutter, geo.btn_y, 374.0, geo.btn_h),
                            2,
                        )
                        .with_items(vec![play, info]),
                    );
                }
                self.lib_row_zones(&mut g, &geo, y0 + if has_hero { geo.hero_h } else { 12.0 });
                if no_rows && has_err {
                    g.register_zone(Zone::horizontal(
                        Z_BODY,
                        Rect::new(geo.gutter, y0 + 60.0, 300.0, 44.0),
                        1,
                    ));
                }
            }
            Mode::NewPop => {
                let n = self.st.borrow().lib.np.entries.len();
                let media_w = (geo.width - 2.0 * geo.gutter - 112.0).min(720.0);
                let items: Vec<Rect> = (0..n)
                    .map(|i| {
                        Rect::new(
                            geo.gutter + 112.0,
                            y0 + 4.0 + i as f32 * geo.entry_h,
                            media_w,
                            media_w * 9.0 / 16.0,
                        )
                    })
                    .collect();
                g.register_zone(
                    Zone::vertical(
                        Z_BODY,
                        Rect::new(
                            geo.gutter + 112.0,
                            y0,
                            media_w,
                            (n as f32 * geo.entry_h).max(1.0),
                        ),
                        n,
                    )
                    .with_items(items)
                    .remembering(true),
                );
                self.lib_row_zones(&mut g, &geo, y0 + n as f32 * geo.entry_h + 16.0);
            }
            _ => {
                let st = self.st.borrow();
                let n = st.lib.items.len();
                if n == 0 {
                    if !st.lib.error.is_empty() || st.lib.empty_action != EmptyAction::None {
                        g.register_zone(Zone::horizontal(
                            Z_BODY,
                            Rect::new(geo.gutter, y0 + 60.0, 300.0, 44.0),
                            1,
                        ));
                    }
                } else {
                    let items: Vec<Rect> = (0..n)
                        .map(|i| {
                            Rect::new(
                                geo.gutter + (i % geo.cols) as f32 * (geo.card_w + geo.gap),
                                y0 + 10.0 + (i / geo.cols) as f32 * geo.pitch_y,
                                geo.card_w,
                                geo.card_h,
                            )
                        })
                        .collect();
                    g.register_zone(
                        Zone::grid(
                            Z_BODY,
                            Rect::new(
                                geo.gutter,
                                y0,
                                geo.width - 2.0 * geo.gutter,
                                n.div_ceil(geo.cols) as f32 * geo.pitch_y,
                            ),
                            n,
                            geo.cols,
                        )
                        .with_items(items)
                        .remembering(true),
                    );
                }
            }
        }
        g
    }

    /// Setzt den Fokus, wenn noch keiner besteht: erster passender Inhalt, sonst die Werkzeuge.
    pub fn lib_default_focus(self: &Rc<Self>) {
        if self.st.borrow().lib.menu.is_some() {
            return;
        }
        let mode = self.st.borrow().lib.mode;
        // Solange geladen wird, bleibt der Fokus leer (kein Ring auf Werkzeugen).
        if self.with_lib(|g| g.get_loading()) && mode != Mode::Search {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            if st.graph.focused().is_none() {
                let order: Vec<ZoneId> = match mode {
                    Mode::Rows => vec![Z_HERO, ZoneId(Z_ROW0), Z_BODY, Z_TOOL],
                    Mode::Search => vec![Z_FIELD],
                    Mode::NewPop => vec![Z_BODY, ZoneId(Z_ROW0), Z_TOOL],
                    _ => vec![Z_BODY, Z_TOOL],
                };
                for z in order {
                    if st.graph.focus_zone(z) {
                        break;
                    }
                }
                if st.graph.focused().is_none() {
                    let _ = st.graph.focus_zone(Z_NAV);
                }
            }
        }
        self.lib_show_focus();
    }

    /// Schreibt den Fokus in `LibraryState`.
    pub fn lib_show_focus(&self) {
        let f = self.st.borrow().graph.focused();
        let (z, i) = f.map_or((-1, 0), |t| (t.zone.0 as i32, t.index as i32));
        self.with_lib(|g| {
            g.set_focus_zone(z);
            g.set_focus_index(i);
        });
    }

    /// Fokus wurde bewegt. Gibt `true` zurück, wenn die Route dem Bibliotheksbereich gehört.
    pub fn library_focus_moved(self: &Rc<Self>, route: &Route, to: Target) -> bool {
        if !is_library_route(route) {
            return false;
        }
        let text = matches!(route, Route::Search) && to.zone == Z_FIELD && to.index == 0;
        self.st.borrow_mut().ctx = if text {
            InputContext::TextInput
        } else {
            InputContext::Browse
        };
        if text {
            self.lib_focus_field_later();
        } else {
            self.win().invoke_focus_root();
        }
        let near_end = {
            let st = self.st.borrow();
            let cols = st.lib.geo.map_or(5, |g| g.cols);
            to.zone == Z_BODY
                && matches!(st.lib.mode, Mode::Grid | Mode::MyList | Mode::Collection)
                && to.index + 2 * cols >= st.lib.items.len()
        };
        if near_end {
            self.lib_more();
        }
        true
    }

    pub fn lib_focus_field_later(self: &Rc<Self>) {
        let weak = self.ui.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
            if let Some(ui) = weak.upgrade() {
                let g = ui.global::<LibraryState>();
                g.set_field_tick(g.get_field_tick() + 1);
            }
        });
    }

    /// Zurück: Menü schließen statt die Seite zu verlassen. `true` = behandelt.
    pub fn library_back(self: &Rc<Self>, route: &Route) -> bool {
        if !is_library_route(route) {
            return false;
        }
        if self.st.borrow().lib.menu.is_some() {
            self.lib_close_menu();
            return true;
        }
        if matches!(route, Route::Search) {
            return self.search_back();
        }
        false
    }

    // ------------------------------------------------------------ Aktivierung

    pub fn library_activate(self: &Rc<Self>, t: Target) {
        let mode = self.st.borrow().lib.mode;
        if t.zone == Z_NAV {
            self.nav_selected(t.index as i32);
        } else if t.zone == Z_TOOL {
            self.lib_tool_activated(t.index);
        } else if t.zone == Z_ALPHA {
            self.lib_alpha(t.index);
        } else if t.zone == Z_MENU {
            self.lib_menu_pick(t.index);
        } else if t.zone == Z_HERO {
            match (t.index, self.lib_hero_item()) {
                (0, Some(item)) => self.play_item(&item),
                (_, Some(item)) => self.lib_open_detail(&item),
                _ => {}
            }
        } else if mode == Mode::Search {
            self.search_activate(t);
        } else if t.zone == Z_BODY {
            self.lib_body_activate(t.index);
        } else if let Some(r) = t.zone.0.checked_sub(Z_ROW0).map(|r| r as usize) {
            let item = self
                .st
                .borrow()
                .lib
                .rows
                .get(r)
                .and_then(|row| row.items.get(t.index).cloned());
            if let Some(item) = item {
                self.lib_open_item(&item);
            }
        }
    }

    fn lib_body_activate(self: &Rc<Self>, idx: usize) {
        let (mode, empty, has_err) = {
            let st = self.st.borrow();
            (
                st.lib.mode,
                st.lib.items.is_empty(),
                !st.lib.error.is_empty(),
            )
        };
        if mode == Mode::NewPop {
            self.np_entry_activated(idx);
            return;
        }
        if empty {
            if has_err {
                self.lib_retry();
            } else {
                self.lib_empty_action();
            }
            return;
        }
        let item = self.st.borrow().lib.items.get(idx).cloned();
        if let Some(item) = item {
            self.lib_open_item(&item);
        }
    }

    /// Öffnet einen Titel: Ordner/Sammlungen in der Sammlungsansicht als Unterseite, sonst die Detailseite.
    pub fn lib_open_item(self: &Rc<Self>, item: &BaseItem) {
        let in_collection = self.st.borrow().lib.mode == Mode::Collection;
        if in_collection
            && matches!(
                item.kind,
                ItemKind::BoxSet
                    | ItemKind::Folder
                    | ItemKind::CollectionFolder
                    | ItemKind::Playlist
            )
        {
            self.go(Route::Collection {
                id: item.id.clone(),
                name: item.name.clone(),
            });
        } else {
            self.lib_open_detail(item);
        }
    }

    pub fn lib_open_detail(self: &Rc<Self>, item: &BaseItem) {
        self.go(Route::Detail {
            id: item.id.clone(),
        });
    }

    fn lib_retry(self: &Rc<Self>) {
        let (mode, src) = {
            let st = self.st.borrow();
            (st.lib.mode, st.lib.src.clone())
        };
        match mode {
            Mode::Rows | Mode::NewPop | Mode::Search => {
                let route = self.st.borrow().router.current().clone();
                let ctx = self.render_library_route(&route);
                self.st.borrow_mut().ctx = ctx;
                self.win().invoke_focus_root();
            }
            _ => {
                if let Some(src) = src {
                    self.lib_restart_grid(src);
                }
            }
        }
    }

    fn lib_empty_action(self: &Rc<Self>) {
        let act = self.st.borrow().lib.empty_action;
        match act {
            EmptyAction::Home => self.go_reset(Route::Home),
            EmptyAction::ShowAll => {
                self.st.borrow_mut().lib.mylist.kind = None;
                self.lib_mylist_changed();
            }
            EmptyAction::ClearLetter => self.lib_alpha(0),
            EmptyAction::None => {}
        }
    }

    fn lib_tool_activated(self: &Rc<Self>, idx: usize) {
        let Some(tool) = self.st.borrow().lib.tools.get(idx).cloned() else {
            return;
        };
        let route = self.st.borrow().router.current().clone();
        match tool.act {
            ToolAct::GenreMenu => self.lib_open_menu(true, idx),
            ToolAct::SortMenu => self.lib_open_menu(false, idx),
            ToolAct::View(v) => {
                if let Route::Library {
                    kind, genre, sort, ..
                } = route
                {
                    self.go_replace(Route::Library {
                        kind,
                        genre,
                        view: v,
                        sort,
                    });
                }
            }
            ToolAct::MyType(t) => {
                self.st.borrow_mut().lib.mylist.kind = t;
                self.lib_mylist_changed();
            }
            ToolAct::MySort(s) => {
                self.st.borrow_mut().lib.mylist.sort = Some(s);
                self.lib_mylist_changed();
            }
            ToolAct::NpTab(i) => self.np_tab(i),
        }
    }

    fn lib_mylist_changed(self: &Rc<Self>) {
        let r = self.st.borrow().lib.mylist.clone();
        self.lib_restart_grid(GridSrc::MyList(r));
        self.lib_build_tools(&Route::MyList);
        self.lib_rebuild_graph();
        self.lib_default_focus();
    }

    fn lib_alpha(self: &Rc<Self>, i: usize) {
        let letter = {
            let st = self.st.borrow();
            st.lib
                .alpha
                .get(i)
                .filter(|_| i != 0)
                .and_then(|s| s.chars().next())
        };
        let (id, sort) = {
            let mut st = self.st.borrow_mut();
            st.lib.letter = letter;
            (st.lib.coll_id.clone(), st.lib.sort.unwrap_or(LibSort::Az))
        };
        self.with_lib(|g| g.set_alpha_current(i as i32));
        self.lib_restart_grid(GridSrc::Collection { id, sort, letter });
        self.lib_rebuild_graph();
    }

    // ------------------------------------------------------------ Ereignisse der Oberfläche

    /// Alle Ereignisse der Seite (Maus, Rad, Layout).
    pub fn lib_event(self: &Rc<Self>, kind: &str, a: i32, b: i32) {
        let (a_us, b_us) = (a.max(0) as usize, b.max(0) as usize);
        let focus = |app: &Rc<App>, zone: ZoneId, index: usize| {
            let _ = app.st.borrow_mut().graph.set_focus(Target { zone, index });
            app.lib_show_focus();
        };
        match kind {
            "nav" => self.nav_selected(a),
            "tool" => {
                focus(self, Z_TOOL, a_us);
                self.library_activate(Target {
                    zone: Z_TOOL,
                    index: a_us,
                });
            }
            "card" => {
                focus(self, Z_BODY, a_us);
                self.library_activate(Target {
                    zone: Z_BODY,
                    index: a_us,
                });
            }
            "rowcard" => {
                let z = ZoneId(Z_ROW0 + a_us as u32);
                focus(self, z, b_us);
                self.library_activate(Target {
                    zone: z,
                    index: b_us,
                });
            }
            "hero" => {
                focus(self, Z_HERO, a_us);
                self.library_activate(Target {
                    zone: Z_HERO,
                    index: a_us,
                });
            }
            "alpha" => {
                focus(self, Z_ALPHA, a_us);
                self.lib_alpha(a_us);
            }
            "menu" => self.lib_menu_pick(a_us),
            "menu-close" => self.lib_close_menu(),
            "retry" => self.lib_retry(),
            "empty-action" => self.lib_empty_action(),
            "more" => self.lib_more(),
            "visible" => {
                self.st.borrow_mut().lib.vis = (a_us, b_us);
                self.lib_load_grid_images();
            }
            "np-play" | "np-info" | "np-entry" => self.np_event(kind, a_us),
            "chip" | "key" | "kbd-toggle" | "query" | "field" => {
                self.search_event(kind, a_us, b_us);
            }
            _ => {}
        }
    }

    pub fn lib_on_geometry(self: &Rc<Self>, g: LibGeometry) {
        let geo = LibGeo::from(g);
        let changed = self.st.borrow().lib.geo != Some(geo);
        if !changed {
            return;
        }
        self.st.borrow_mut().lib.geo = Some(geo);
        if is_library_route(self.st.borrow().router.current()) {
            self.lib_rebuild_graph();
        }
    }

    /// Zustand für die Automation.
    pub fn lib_state_json(&self) -> serde_json::Value {
        let st = self.st.borrow();
        let l = &st.lib;
        let ui = self.win();
        let g = ui.global::<LibraryState>();
        serde_json::json!({
            "mode": l.mode.index(),
            "title": g.get_title().to_string(),
            "count_text": g.get_count_text().to_string(),
            "items": l.items.len(),
            "first_titles": l.items.iter().take(3).map(|i| i.name.clone()).collect::<Vec<_>>(),
            "genres_of_items": l.items.iter().take(60).flat_map(|i| i.genres.clone()).collect::<std::collections::BTreeSet<_>>(),
            "total": l.pager.total,
            "pager_done": l.pager.done,
            "rows": l.rows.iter().map(|r| serde_json::json!({"title": r.title, "count": r.items.len()})).collect::<Vec<_>>(),
            "pool": l.pool.len(),
            "hero": {"title": g.get_hero().title.to_string(), "has_item": g.get_hero().has_item, "backdrop": g.get_hero().has_backdrop},
            "tools": l.tools.iter().map(|t| serde_json::json!({"label": t.label, "active": t.active})).collect::<Vec<_>>(),
            "genres": l.genres.len(),
            "menu_open": l.menu.is_some(),
            "loading": g.get_loading(),
            "error": g.get_error().to_string(),
            "empty_title": g.get_empty_title().to_string(),
            "note": g.get_note().to_string(),
            "images_requested": l.requested.len(),
            "images_set": l.model.as_ref().map_or(0, |m| (0..m.row_count()).filter(|i| m.row_data(*i).is_some_and(|c| c.has_image)).count()),
            "np_entries": l.np.entries.len(),
            "search": self.search_state_json(),
            "focus_zone": g.get_focus_zone(),
            "focus_index": g.get_focus_index(),
            "geo_cols": l.geo.map(|g| g.cols),
        })
    }

    /// Automationsbefehle der Bibliotheksseiten; `None` = Befehl unbekannt.
    pub fn lib_automation(
        self: &Rc<Self>,
        cmd: &str,
        msg: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, String>> {
        let ok = || Some(Ok(serde_json::json!({})));
        match cmd {
            "library" => {
                // {"kind":"series|movies","genre":"<Genre-Id>","view":"grid|rows","sort":"az"}
                let Some(kind) = msg["kind"].as_str().and_then(LibraryKind::from_slug) else {
                    return Some(Err("kind fehlt (series|movies)".into()));
                };
                let view = if msg["view"].as_str() == Some("grid") {
                    LibraryView::Grid
                } else {
                    LibraryView::Rows
                };
                let sort = msg["sort"]
                    .as_str()
                    .and_then(LibSort::from_slug)
                    .unwrap_or(LibSort::Az);
                let genre = msg["genre"].as_str().map(str::to_owned);
                self.go(Route::Library {
                    kind,
                    genre,
                    view,
                    sort,
                });
                ok()
            }
            "collection" => {
                let id = msg["id"].as_str().unwrap_or("").to_owned();
                let name = msg["name"].as_str().unwrap_or("Sammlung").to_owned();
                self.go(Route::Collection { id, name });
                ok()
            }
            "mylist" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.lib.mylist.kind = match msg["type"].as_str() {
                        Some("movies") => Some(MyListType::Movies),
                        Some("series") => Some(MyListType::Series),
                        _ => None,
                    };
                    st.lib.mylist.sort = msg["sort"].as_str().and_then(LibSort::from_slug);
                }
                self.go(Route::MyList);
                ok()
            }
            "search_text" => {
                let t = msg["text"].as_str().unwrap_or("").to_owned();
                if !matches!(self.st.borrow().router.current(), Route::Search) {
                    self.go(Route::Search);
                }
                self.search_set_text(&t);
                ok()
            }
            "lib_scroll" => {
                // Raster „durchblättern“: setzt die sichtbare Spanne und lädt nach.
                let n = msg["rows"].as_u64().unwrap_or(0) as usize;
                let cols = self.lib_geo().cols;
                self.st.borrow_mut().lib.vis = (n * cols, n * cols + 48);
                self.lib_load_grid_images();
                self.lib_more();
                ok()
            }
            "lib_menu" => {
                let genre = msg["which"].as_str() == Some("genre");
                let want = if genre {
                    ToolAct::GenreMenu
                } else {
                    ToolAct::SortMenu
                };
                let idx = self
                    .st
                    .borrow()
                    .lib
                    .tools
                    .iter()
                    .position(|t| t.act == want);
                match idx {
                    Some(i) => {
                        self.lib_open_menu(genre, i);
                        ok()
                    }
                    None => Some(Err("Menü nicht vorhanden".into())),
                }
            }
            "lib_shot" => {
                // Nur die Oberfläche (kein Desktop-Abzug): unabhängig von anderen Fenstern.
                let Some(path) = msg["path"].as_str() else {
                    return Some(Err("path fehlt".into()));
                };
                let snap = match self.win().window().take_snapshot() {
                    Ok(s) => s,
                    Err(e) => return Some(Err(format!("Schnappschuss nicht möglich: {e}"))),
                };
                match crate::capture::write_png(
                    std::path::Path::new(path),
                    snap.width(),
                    snap.height(),
                    snap.as_bytes(),
                ) {
                    Ok(()) => Some(Ok(
                        serde_json::json!({"width": snap.width(), "height": snap.height()}),
                    )),
                    Err(e) => Some(Err(e)),
                }
            }
            "np_tab" => {
                self.np_tab(msg["index"].as_u64().unwrap_or(0) as usize);
                ok()
            }
            _ => None,
        }
    }
}
