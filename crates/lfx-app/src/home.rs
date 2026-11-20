//! Startseite: Zeilen und Billboard laden, Bilder nachladen, Fokus und Aktivierung.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use lfx_api::{BaseItem, Client, ItemKind};
use lfx_core::billboard::{self, PoolOpts};
use lfx_core::images::{CardKind, card_images, display_title};
use lfx_core::rows::home::{HomeInput, build_home_defs};
use lfx_core::rows::{RowKind, load_row, logic};
use lfx_images::{
    ApiFetcher, DecodedImage, DiskCache, ImageLoader, ImageRequest, LoaderConfig, Priority,
};
use lfx_nav::Target;
use lfx_ui::{CardAspect, CardData, HeroData, RowData};
use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use tokio::task::JoinSet;

use crate::app::{App, NAV_LABELS, post};
use crate::focus::{self, HomeGeo};
use crate::route::Route;

/// Eine geladene Zeile samt Modell der Karten.
pub struct HomeRow {
    pub id: String,
    pub title: String,
    pub kind: CardKind,
    pub items: Vec<BaseItem>,
    pub model: Rc<VecModel<CardData>>,
}

/// Zustand der Startseite.
#[derive(Default)]
pub struct HomeState {
    pub rows: Vec<HomeRow>,
    pub pool: Vec<BaseItem>,
    pub hero_idx: usize,
    /// Zählt Ladevorgänge; veraltete Antworten werden verworfen.
    pub generation: u64,
    pub loaded: bool,
    pub loading: bool,
}

/// Ergebnis des Ladens (Send, ohne Oberflächenobjekte).
pub struct HomeData {
    pub rows: Vec<LoadedRow>,
    pub pool: Vec<BaseItem>,
}

pub struct LoadedRow {
    pub id: String,
    pub title: String,
    pub kind: RowKind,
    pub items: Vec<BaseItem>,
}

fn card_kind(k: RowKind) -> CardKind {
    match k {
        RowKind::Resume => CardKind::Resume,
        // Top 10 zeigt hier Querformat-Bilder (Rangzahl als Badge); Poster-Zeilen folgen mit der Bibliothek.
        RowKind::Landscape | RowKind::Top10 => CardKind::Landscape,
    }
}

/// Einfacher Zufall (xorshift) für die Billboard-Mischung.
fn rng_from_clock() -> impl FnMut() -> f64 {
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

async fn fetch_home(client: &Client) -> Result<HomeData, String> {
    let views = client.views().await;
    let genres = client
        .genres(None, &[ItemKind::Movie, ItemKind::Series])
        .await;
    let lookups_ok = views.is_ok() && genres.is_ok();
    let views = views.map(|v| v.items).unwrap_or_default();
    let genres = genres.map(|v| v.items).unwrap_or_default();
    let seed = lfx_core::dayseed::day_key_from_epoch_ms(App::now_ms() as i64, 0);
    let defs = build_home_defs(&HomeInput {
        views: &views,
        genres: &genres,
        lookups_ok,
        seed: &seed,
        genre_count: 6,
        brand: "LENZFLIX",
        charts_configured: false,
        charts: None,
    });
    let mut set = JoinSet::new();
    for (i, def) in defs.rows.iter().cloned().enumerate() {
        let c = client.clone();
        set.spawn(async move { (i, def.clone(), load_row(&c, &def).await) });
    }
    let mut slots: Vec<Option<LoadedRow>> = (0..defs.rows.len()).map(|_| None).collect();
    let mut errors = 0usize;
    while let Some(res) = set.join_next().await {
        let Ok((i, def, result)) = res else { continue };
        match result {
            Ok(r) if !r.items.is_empty() => {
                slots[i] = Some(LoadedRow {
                    id: def.id.clone(),
                    title: r.title.or(def.title.clone()).unwrap_or_default(),
                    kind: def.kind,
                    items: r.items,
                });
            }
            Ok(_) => {}
            Err(_) => errors += 1,
        }
    }
    let rows: Vec<LoadedRow> = slots.into_iter().flatten().collect();
    let (latest_call, random_call) = billboard::home_pool_calls();
    let (latest, random) = tokio::join!(latest_call.execute(client), random_call.execute(client));
    let (latest, random) = (latest.unwrap_or_default(), random.unwrap_or_default());
    let mut rng = rng_from_clock();
    let mut pool = billboard::build_pool(&latest, &random, &PoolOpts::home(None), &mut rng);
    if pool.is_empty() {
        // Ohne Logo-und-Hintergrund-Titel nimmt das Billboard den ersten Titel mit Hintergrund.
        pool = latest
            .iter()
            .chain(random.iter())
            .filter(|i| !i.backdrop_image_tags.is_empty() || !i.image_tags.is_empty())
            .take(5)
            .cloned()
            .collect();
    }
    if rows.is_empty() && pool.is_empty() && errors > 0 {
        return Err(
            "Die Startseite konnte nicht geladen werden. Ist der Server erreichbar?".into(),
        );
    }
    Ok(HomeData { rows, pool })
}

fn to_buffer(img: &DecodedImage) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(img.width, img.height);
    let dst = buf.make_mut_bytes();
    let n = dst.len().min(img.pixels.len());
    dst[..n].copy_from_slice(&img.pixels[..n]);
    buf
}

fn card_for(item: &BaseItem, kind: CardKind, rank: Option<usize>, now_ms: f64) -> CardData {
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
        badge: logic::badge_for(item, None, now_ms).unwrap_or("").into(),
        trend_rank: rank.map_or(0, |r| r as i32),
    }
}

impl App {
    pub fn images(&self) -> Option<ImageLoader> {
        self.st.borrow().images.clone()
    }

    /// Legt den Bild-Lader für die aktuelle Sitzung an.
    pub fn ensure_images(&self, client: &Client) {
        let disk = DiskCache::in_app_dirs(&self.dirs, 512 * 1024 * 1024)
            .ok()
            .map(Arc::new);
        let loader = ImageLoader::with_handle(
            self.rt.clone(),
            Arc::new(ApiFetcher::new(client.clone())),
            disk,
            LoaderConfig::default(),
        );
        self.st.borrow_mut().images = Some(loader);
    }

    pub fn reset_home(&self) {
        self.st.borrow_mut().home = HomeState::default();
        let ui = self.win();
        ui.set_rows(ModelRc::new(VecModel::<RowData>::default()));
        ui.set_hero(HeroData::default());
        self.st.borrow_mut().images = None;
    }

    /// Beim Betreten der Startseite: (neu) laden und Fokus setzen.
    pub fn enter_home(self: &Rc<Self>) {
        self.load_home();
        self.set_default_home_focus();
    }

    fn set_default_home_focus(self: &Rc<Self>) {
        let mut st = self.st.borrow_mut();
        if st.graph.focused().is_some() {
            return;
        }
        if !st.graph.focus_zone(focus::Z_HERO) {
            st.graph.focus_zone(focus::row_zone(0));
        }
        drop(st);
        self.show_focus();
    }

    /// Lädt Zeilen und Billboard. Vorhandene Zeilen bleiben bis zur Antwort stehen.
    pub fn load_home(self: &Rc<Self>) {
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        if self.st.borrow().images.is_none() {
            self.ensure_images(&client);
        }
        let epoch = {
            let mut st = self.st.borrow_mut();
            st.home.generation += 1;
            st.home.loading = true;
            st.home.generation
        };
        let ui = self.win();
        ui.set_home_loading(self.st.borrow().home.rows.is_empty());
        ui.set_home_error("".into());
        self.rt.spawn(async move {
            let data = fetch_home(&client).await;
            post(move |app| app.home_loaded(epoch, data));
        });
    }

    fn home_loaded(self: &Rc<Self>, epoch: u64, data: Result<HomeData, String>) {
        if self.st.borrow().home.generation != epoch {
            return;
        }
        let ui = self.win();
        self.st.borrow_mut().home.loading = false;
        ui.set_home_loading(false);
        let data = match data {
            Ok(d) => d,
            Err(text) => {
                tracing::warn!("Startseite: {text}");
                ui.set_home_error(text.into());
                return;
            }
        };
        let now = App::now_ms();
        let mut rows = Vec::new();
        for r in data.rows {
            let kind = card_kind(r.kind);
            let cards: Vec<CardData> = r
                .items
                .iter()
                .enumerate()
                .map(|(i, it)| card_for(it, kind, (r.kind == RowKind::Top10).then_some(i + 1), now))
                .collect();
            rows.push(HomeRow {
                id: r.id,
                title: r.title,
                kind,
                items: r.items,
                model: Rc::new(VecModel::from(cards)),
            });
        }
        let row_data: Vec<RowData> = rows
            .iter()
            .map(|r| RowData {
                id: r.id.as_str().into(),
                title: r.title.as_str().into(),
                aspect: CardAspect::Wide,
                items: ModelRc::from(r.model.clone()),
            })
            .collect();
        ui.set_rows(ModelRc::new(VecModel::from(row_data)));
        {
            let mut st = self.st.borrow_mut();
            st.home.rows = rows;
            st.home.pool = data.pool;
            st.home.hero_idx = 0;
            st.home.loaded = true;
        }
        self.show_hero();
        self.rebuild_home_graph();
        self.set_default_home_focus();
        self.load_card_images(epoch);
        self.start_hero_rotation();
        tracing::info!(
            rows = self.st.borrow().home.rows.len(),
            "Startseite geladen"
        );
    }

    /// Baut die Fokuszonen neu (nach Laden und nach Größenänderung), der Fokus bleibt wenn möglich.
    pub fn rebuild_home_graph(self: &Rc<Self>) {
        let (geo, has_hero, lens) = {
            let st = self.st.borrow();
            let Some(geo) = st.geo else { return };
            let lens: Vec<usize> = st.home.rows.iter().map(|r| r.items.len()).collect();
            (geo, !st.home.pool.is_empty(), lens)
        };
        let mut st = self.st.borrow_mut();
        let old = st.graph.focused();
        let mut g = focus::home_graph(&geo, NAV_LABELS.len(), has_hero, &lens);
        if let Some(t) = old {
            let _ = g.set_focus(t);
        }
        st.graph = g;
    }

    pub fn on_home_geometry(self: &Rc<Self>, g: lfx_ui::HomeGeometry) {
        let geo = HomeGeo::from(g);
        let changed = self.st.borrow().geo != Some(geo);
        if !changed {
            return;
        }
        self.st.borrow_mut().geo = Some(geo);
        if matches!(self.st.borrow().router.current(), Route::Home) {
            self.rebuild_home_graph();
            self.set_default_home_focus();
            self.show_focus();
        }
    }

    // ---------------------------------------------------------------- Billboard

    fn hero_item(&self) -> Option<BaseItem> {
        let st = self.st.borrow();
        st.home.pool.get(st.home.hero_idx).cloned()
    }

    fn show_hero(self: &Rc<Self>) {
        let ui = self.win();
        let Some(item) = self.hero_item() else {
            ui.set_hero(HeroData::default());
            return;
        };
        let meta = billboard::meta_parts(&item)
            .into_iter()
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join(" · ");
        ui.set_hero(HeroData {
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
        let imgs = billboard::hero_images(&item, 2.0, 1.0);
        let id = item.id.clone();
        if let Some(r) = imgs.backdrop {
            self.fetch_image(&r, Priority::Visible, move |app, img| {
                app.set_hero_image(&id, img, false);
            });
        }
        if let Some(r) = imgs.logo {
            let id = item.id.clone();
            self.fetch_image(&r, Priority::Visible, move |app, img| {
                app.set_hero_image(&id, img, true);
            });
        }
    }

    fn set_hero_image(&self, item_id: &str, img: Image, logo: bool) {
        // Nur setzen, wenn der Titel noch gezeigt wird (das Billboard rotiert).
        if self.hero_item().is_none_or(|h| h.id != item_id) {
            return;
        }
        let ui = self.win();
        let mut h = ui.get_hero();
        if logo {
            h.logo = img;
            h.has_logo = true;
        } else {
            h.backdrop = img;
            h.has_backdrop = true;
        }
        ui.set_hero(h);
    }

    fn start_hero_rotation(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.timers.hero.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(billboard::ROTATE_MS),
            move || {
                let Some(app) = weak.upgrade() else { return };
                let focus_in_hero = app
                    .st
                    .borrow()
                    .graph
                    .focused()
                    .is_some_and(|t| t.zone == focus::Z_HERO);
                let on_home = matches!(app.st.borrow().router.current(), Route::Home);
                let len = app.st.borrow().home.pool.len();
                if !on_home || focus_in_hero || !billboard::rotation_enabled(len, false) {
                    return;
                }
                {
                    let mut st = app.st.borrow_mut();
                    st.home.hero_idx = billboard::next_index(st.home.hero_idx, len);
                }
                app.show_hero();
            },
        );
    }

    // ---------------------------------------------------------------- Bilder

    /// Lädt ein Bild und ruft `apply` im UI-Thread auf.
    pub fn fetch_image(
        &self,
        r: &lfx_core::images::ImageRef,
        prio: Priority,
        apply: impl FnOnce(&Rc<App>, Image) + Send + 'static,
    ) {
        let Some(loader) = self.images() else { return };
        let mut req = ImageRequest::new(&r.pick.id, r.pick.kind)
            .tag(&r.pick.tag)
            .quality(r.quality);
        req.max_width = Some(r.max_width);
        self.rt.spawn(async move {
            match loader.request(req, prio).await {
                Ok(img) => {
                    let buf = to_buffer(&img);
                    post(move |app| apply(app, Image::from_rgba8(buf)));
                }
                Err(e) => tracing::debug!("Bild nicht geladen: {e}"),
            }
        });
    }

    fn load_card_images(self: &Rc<Self>, epoch: u64) {
        let jobs: Vec<(usize, usize, lfx_core::images::ImageRef)> = {
            let st = self.st.borrow();
            st.home
                .rows
                .iter()
                .enumerate()
                .flat_map(|(r, row)| {
                    row.items.iter().enumerate().filter_map(move |(i, it)| {
                        card_images(it, row.kind, false, 2.0, 1.0)
                            .main
                            .map(|m| (r, i, m))
                    })
                })
                .collect()
        };
        for (r, i, m) in jobs {
            self.fetch_image(&m, Priority::Visible, move |app, img| {
                if app.st.borrow().home.generation != epoch {
                    return;
                }
                let model = app.st.borrow().home.rows.get(r).map(|x| x.model.clone());
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

    // ---------------------------------------------------------------- Aktivierung

    pub fn home_activate(self: &Rc<Self>, t: Target) {
        if t.zone == focus::Z_NAV {
            self.nav_selected(t.index as i32);
        } else if t.zone == focus::Z_HERO {
            match (t.index, self.hero_item()) {
                (0, Some(item)) => self.open_item(&item),
                (_, Some(item)) => self.on_card_activate(&item.id), // W2-detail: Mehr Infos = Modal
                _ => {}
            }
        } else if let Some(r) = focus::zone_row(t.zone) {
            let item = self
                .st
                .borrow()
                .home
                .rows
                .get(r)
                .and_then(|row| row.items.get(t.index).cloned());
            if let Some(item) = item {
                self.on_card_activate(&item.id); // W2-detail: Kartenklick = Detail-Modal (Web: Info-Fenster)
            }
        }
    }

    /// Filme und Folgen werden abgespielt, alles andere öffnet die Detailseite (folgt).
    pub fn open_item(self: &Rc<Self>, item: &BaseItem) {
        match item.kind {
            ItemKind::Movie | ItemKind::Episode => self.play_item(item),
            _ => self.go(Route::Detail {
                id: item.id.clone(),
            }),
        }
    }

    pub fn nav_selected(self: &Rc<Self>, i: i32) {
        let name = match i {
            0 => {
                self.go_reset(Route::Home);
                return;
            }
            1 => "series",
            2 => "movies",
            3 => "newpopular",
            4 => "mylist",
            5 => "search",
            6 => "tvmode",
            _ => "settings",
        };
        if let Some(r) = Route::from_name(name) {
            self.go(r);
        }
    }

    pub fn card_clicked(self: &Rc<Self>, row: i32, idx: i32) {
        let t = Target {
            zone: focus::row_zone(row.max(0) as usize),
            index: idx.max(0) as usize,
        };
        let _ = self.st.borrow_mut().graph.set_focus(t);
        self.show_focus();
        self.home_activate(t);
    }

    /// Mausklick auf Billboard-Knöpfe.
    pub fn hero_clicked(self: &Rc<Self>, info: bool) {
        let t = Target {
            zone: focus::Z_HERO,
            index: usize::from(info),
        };
        let _ = self.st.borrow_mut().graph.set_focus(t);
        self.show_focus();
        self.home_activate(t);
    }
}
