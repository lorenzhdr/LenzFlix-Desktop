//! Startseite: Zeilen und Billboard laden, Bilder nachladen, Fokus und Aktivierung.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use lfx_api::{BaseItem, Client, ItemKind};
use lfx_core::billboard::{self, PoolOpts};
use lfx_core::images::{CardKind, card_images, display_title};
use lfx_core::rows::home::{ChartLists, HomeInput, build_home_defs};
use lfx_core::rows::{RowKind, load_row, logic};
use lfx_core::trend::{TrendInfo, trend_badge};
use lfx_images::{
    ApiFetcher, DecodedImage, DiskCache, ImageLoader, ImageRequest, LoaderConfig, Priority,
};
use lfx_nav::Target;
use lfx_ui::{HCard, HRow, HeroData};
use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use tokio::task::JoinSet;

use crate::app::{App, post};
use crate::focus;
use crate::home_focus::{self, RowShape};
use crate::home_ui::HomeExtra;
use crate::route::Route;

/// Eine geladene Zeile samt Modell der Karten.
pub struct HomeRow {
    pub id: String,
    pub title: String,
    pub kind: CardKind,
    pub items: Vec<BaseItem>,
    pub model: Rc<VecModel<HCard>>,
    /// Bild der Karte schon angefordert?
    pub requested: Vec<bool>,
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
    /// Zusatzzustand (Kopfleiste, Menüs, Trailer, Datensparmodus).
    pub x: HomeExtra,
}

/// Ergebnis des Ladens (Send, ohne Oberflächenobjekte).
pub struct HomeData {
    pub rows: Vec<LoadedRow>,
    pub pool: Vec<BaseItem>,
    pub trend: TrendInfo,
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
        RowKind::Landscape => CardKind::Landscape,
        RowKind::Top10 => CardKind::Top10,
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

/// Charts des Zusatzdienstes (Adresse aus dem Branding-CSS des Servers), Titel aufgelöst. `None` = nicht
/// konfiguriert oder nicht erreichbar.
async fn fetch_charts(client: &Client) -> Option<(ChartLists, TrendInfo)> {
    let branding = client.branding().await.ok()?;
    let charts_client = lfx_api::charts::ChartsClient::from_branding(client, &branding)?.ok()?;
    let charts = charts_client.charts().await.ok()?;
    let ids = |list: &[lfx_api::charts::ChartEntry]| -> Vec<String> {
        list.iter().take(10).map(|e| e.id.clone()).collect()
    };
    let (m, s, a) = (ids(&charts.movies), ids(&charts.series), ids(&charts.all));
    let mut all_ids: Vec<&str> = m.iter().chain(&s).chain(&a).map(String::as_str).collect();
    all_ids.sort_unstable();
    all_ids.dedup();
    let items = client
        .items(&logic::base_query().ids(&all_ids))
        .await
        .ok()?
        .items;
    let resolve = |list: &[String]| -> Vec<BaseItem> {
        list.iter()
            .filter_map(|id| items.iter().find(|i| &i.id == id).cloned())
            .collect()
    };
    let lists = ChartLists {
        movies: resolve(&m),
        series: resolve(&s),
        all: resolve(&a),
    };
    Some((lists, TrendInfo::from_lists(&m, &s)))
}

async fn fetch_home(client: &Client) -> Result<HomeData, String> {
    let charts = fetch_charts(client).await;
    let (chart_lists, trend) = match charts {
        Some((l, t)) => (Some(l), t),
        None => (None, TrendInfo::default()),
    };
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
        charts_configured: chart_lists.is_some(),
        charts: chart_lists.as_ref(),
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
    Ok(HomeData { rows, pool, trend })
}

fn to_buffer(img: &DecodedImage) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(img.width, img.height);
    let dst = buf.make_mut_bytes();
    let n = dst.len().min(img.pixels.len());
    dst[..n].copy_from_slice(&img.pixels[..n]);
    buf
}

/// Ambient-Farbe aus einem entschlüsselten Bild (Stichprobe wie das Web: etwa 64 Spalten).
fn ambient_of(img: &DecodedImage) -> [u8; 3] {
    let (w, h) = (img.width as usize, img.height as usize);
    let (sx, sy) = ((w / 64).max(1), (h / 36).max(1));
    let mut px = Vec::with_capacity(64 * 36 * 4);
    for y in (0..h).step_by(sy) {
        for x in (0..w).step_by(sx) {
            let o = (y * w + x) * 4;
            if let Some(p) = img.pixels.get(o..o + 4) {
                px.extend_from_slice(p);
            }
        }
    }
    lfx_core::color::ambient_from_pixels(&px)
}

fn card_for(
    item: &BaseItem,
    kind: CardKind,
    rank: Option<usize>,
    now_ms: f64,
    trend: &TrendInfo,
) -> HCard {
    let progress = if kind == CardKind::Resume {
        (logic::progress_percent(item) / 100.0) as f32
    } else {
        0.0
    };
    // Genau eine Marke: „Neu“ vor Trend (wie `cardBadge`).
    let badge = logic::badge_for(item, None, now_ms)
        .or_else(|| trend_badge(item, trend))
        .unwrap_or("");
    HCard {
        title: SharedString::from(display_title(item)),
        image: Image::default(),
        has_image: false,
        progress,
        badge: badge.into(),
        rank: rank.map_or(0, |r| r as i32),
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

    pub fn reset_home(self: &Rc<Self>) {
        self.stop_trailer();
        {
            let mut st = self.st.borrow_mut();
            let keep_layout = st.home.x.layout.clone();
            st.home = HomeState::default();
            st.home.x.layout = keep_layout;
        }
        let ui = self.win();
        ui.set_home_rows(ModelRc::new(VecModel::<HRow>::default()));
        ui.set_hero(HeroData::default());
        self.st.borrow_mut().images = None;
        self.push_home_ui();
    }

    /// Beim Betreten der Startseite: (neu) laden und Fokus setzen.
    pub fn enter_home(self: &Rc<Self>) {
        self.load_home_settings();
        self.push_home_ui();
        self.load_home();
        self.set_default_home_focus();
        // Ein offenes Menü gehört zur vorigen Besuchszeit.
        self.home_close_menu();
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

    pub fn set_default_home_focus_pub(self: &Rc<Self>) {
        self.set_default_home_focus();
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
        let trend = data.trend.clone();
        let mut rows = Vec::new();
        for r in data.rows {
            let kind = card_kind(r.kind);
            let cards: Vec<HCard> = r
                .items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    card_for(
                        it,
                        kind,
                        (r.kind == RowKind::Top10).then_some(i + 1),
                        now,
                        &trend,
                    )
                })
                .collect();
            rows.push(HomeRow {
                id: r.id,
                title: r.title,
                kind,
                requested: vec![false; r.items.len()],
                items: r.items,
                model: Rc::new(VecModel::from(cards)),
            });
        }
        let shapes: Vec<RowShape> = rows
            .iter()
            .map(|r| RowShape {
                len: r.items.len(),
                top10: r.kind == CardKind::Top10,
            })
            .collect();
        let tall = home_focus::tall_before(&shapes);
        let row_data: Vec<HRow> = rows
            .iter()
            .zip(&tall)
            .map(|(r, tb)| HRow {
                id: r.id.as_str().into(),
                title: r.title.as_str().into(),
                top10: r.kind == CardKind::Top10,
                tall_before: *tb,
                items: ModelRc::from(r.model.clone()),
            })
            .collect();
        ui.set_home_rows(ModelRc::new(VecModel::from(row_data)));
        {
            let mut st = self.st.borrow_mut();
            st.home.rows = rows;
            st.home.x.pool_full = data.pool;
            st.home.x.trend = data.trend;
            st.home.hero_idx = 0;
            st.home.loaded = true;
        }
        self.apply_pool_cap();
        self.rebuild_home_graph();
        self.set_default_home_focus();
        self.load_initial_card_images();
        self.start_hero_rotation();
        tracing::info!(
            rows = self.st.borrow().home.rows.len(),
            "Startseite geladen"
        );
    }

    /// Kürzt den Pool nach dem Datensparteil `hero` und zeigt den Titel neu.
    pub fn apply_pool_cap(self: &Rc<Self>) {
        let cap = self.effects().hero_pool_cap.is_some();
        {
            let mut st = self.st.borrow_mut();
            let full = st.home.x.pool_full.clone();
            let keep = st.home.pool.get(st.home.hero_idx).map(|i| i.id.clone());
            st.home.pool = if cap {
                billboard::saver_pool(&full, keep.as_deref())
            } else {
                full
            };
            let len = st.home.pool.len();
            st.home.hero_idx = match keep {
                Some(k) => st.home.pool.iter().position(|i| i.id == k).unwrap_or(0),
                None => 0,
            }
            .min(len.saturating_sub(1));
        }
        self.show_hero();
    }

    /// Baut die Fokuszonen neu (nach Laden und nach Größenänderung), der Fokus bleibt wenn möglich.
    pub fn rebuild_home_graph(self: &Rc<Self>) {
        let (layout, has_hero, shapes) = {
            let st = self.st.borrow();
            let Some(l) = st.home.x.layout.clone() else {
                return;
            };
            let shapes: Vec<RowShape> = st
                .home
                .rows
                .iter()
                .map(|r| RowShape {
                    len: r.items.len(),
                    top10: r.kind == CardKind::Top10,
                })
                .collect();
            (l, !st.home.pool.is_empty(), shapes)
        };
        let mut st = self.st.borrow_mut();
        let old = st.graph.focused();
        let mut g = home_focus::home_graph(&layout, has_hero, &shapes);
        if let Some(t) = old {
            let _ = g.set_focus(t);
        }
        st.graph = g;
    }

    /// Alte Rückmeldung der ersten Startseite (ersetzt durch `on_home_layout`).
    pub fn on_home_geometry(self: &Rc<Self>, _g: lfx_ui::HomeGeometry) {}

    // ---------------------------------------------------------------- Billboard

    fn hero_item(&self) -> Option<BaseItem> {
        let st = self.st.borrow();
        st.home.pool.get(st.home.hero_idx).cloned()
    }

    pub fn show_hero(self: &Rc<Self>) {
        let ui = self.win();
        let Some(item) = self.hero_item() else {
            ui.set_hero(HeroData::default());
            self.push_home_ui();
            return;
        };
        let meta = billboard::meta_parts(&item)
            .into_iter()
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join("  \u{2022}  ");
        let old = ui.get_hero();
        ui.set_hero(HeroData {
            title: display_title(&item).into(),
            meta: meta.into(),
            genres: billboard::genre_line(&item).into(),
            overview: item.overview.clone().unwrap_or_default().into(),
            // Das alte Bild bleibt stehen, bis das neue da ist (kein Aufblitzen); das Logo gehört zum Titel.
            backdrop: old.backdrop,
            has_backdrop: old.has_backdrop,
            logo: Image::default(),
            has_logo: false,
            has_item: true,
        });
        self.push_home_ui();
        let (_, dpr) = self.shell.window_info();
        let scale = self.image_scale();
        let imgs = billboard::hero_images(&item, dpr.max(1.0), scale);
        let id = item.id.clone();
        if let Some(r) = imgs.backdrop {
            self.fetch_image_amb(&r, Priority::Visible, move |app, img, amb| {
                app.set_hero_image(&id, img, false, Some(amb));
            });
        }
        if let Some(r) = imgs.logo {
            let id = item.id.clone();
            self.fetch_image(&r, Priority::Visible, move |app, img| {
                app.set_hero_image(&id, img, true, None);
            });
        }
        self.schedule_trailer();
    }

    fn set_hero_image(&self, item_id: &str, img: Image, logo: bool, ambient: Option<[u8; 3]>) {
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
        if let Some(a) = ambient {
            self.set_ambient(a);
        }
    }

    /// Springt zu einem Titel des Pools (Positionspunkte).
    pub fn hero_goto(self: &Rc<Self>, i: usize) {
        {
            let mut st = self.st.borrow_mut();
            if i >= st.home.pool.len() || i == st.home.hero_idx {
                return;
            }
            st.home.hero_idx = i;
        }
        self.show_hero();
        self.start_hero_rotation();
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
                let trailer = app.st.borrow().home.x.trailer.active;
                let menu = app.st.borrow().home.x.menu != 0;
                if !on_home
                    || focus_in_hero
                    || trailer
                    || menu
                    || !billboard::rotation_enabled(len, false)
                {
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
        self.fetch_image_amb(r, prio, move |app, img, _| apply(app, img));
    }

    /// Wie [`Self::fetch_image`], liefert zusätzlich die Ambient-Farbe des Bildes.
    pub fn fetch_image_amb(
        &self,
        r: &lfx_core::images::ImageRef,
        prio: Priority,
        apply: impl FnOnce(&Rc<App>, Image, [u8; 3]) + Send + 'static,
    ) {
        let Some(loader) = self.images() else { return };
        let mut req = ImageRequest::new(&r.pick.id, r.pick.kind)
            .tag(&r.pick.tag)
            .quality(r.quality);
        req.max_width = Some(r.max_width);
        self.rt.spawn(async move {
            match loader.request(req, prio).await {
                Ok(img) => {
                    let amb = ambient_of(&img);
                    let buf = to_buffer(&img);
                    post(move |app| apply(app, Image::from_rgba8(buf), amb));
                }
                Err(e) => tracing::debug!("Bild nicht geladen: {e}"),
            }
        });
    }

    /// Fordert das Bild einer Karte an (einmal).
    fn request_card_image(self: &Rc<Self>, r: usize, i: usize, prio: Priority) {
        let epoch = self.st.borrow().home.generation;
        let (_, dpr) = self.shell.window_info();
        let scale = self.image_scale();
        let job = {
            let mut st = self.st.borrow_mut();
            let Some(row) = st.home.rows.get_mut(r) else {
                return;
            };
            if i >= row.items.len() || row.requested[i] {
                return;
            }
            row.requested[i] = true;
            card_images(&row.items[i], row.kind, false, dpr.max(1.0), scale).main
        };
        let Some(m) = job else { return };
        self.fetch_image(&m, prio, move |app, img| {
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

    /// Sichtbare Karten sofort (Datensparteil `lazy`: nur diese), der Rest im Hintergrund (Teil `prefetch`).
    fn load_initial_card_images(self: &Rc<Self>) {
        let n = self
            .st
            .borrow()
            .home
            .x
            .layout
            .as_ref()
            .map_or(5, |l| (l.width / l.pitch.max(1.0)).ceil() as usize + 2);
        let rows = self.st.borrow().home.rows.len();
        for r in 0..rows {
            for i in 0..n {
                self.request_card_image(r, i, Priority::Visible);
            }
        }
        self.prefetch_rest();
    }

    /// Hintergrund-Vorladen der übrigen Karten (nur ohne Datensparteil `prefetch`).
    pub fn prefetch_rest(self: &Rc<Self>) {
        if !self.effects().prefetch {
            return;
        }
        let lens: Vec<usize> = self
            .st
            .borrow()
            .home
            .rows
            .iter()
            .map(|r| r.items.len())
            .collect();
        for (r, len) in lens.into_iter().enumerate() {
            for i in 0..len {
                self.request_card_image(r, i, Priority::Prefetch);
            }
        }
    }

    /// Eine Zeile scrollte: die dann sichtbaren Karten laden (`arg` = Zeile * 1000 + erste Karte).
    pub fn row_view(self: &Rc<Self>, arg: i32) {
        let (r, first) = ((arg / 1000) as usize, (arg % 1000) as usize);
        let n = self
            .st
            .borrow()
            .home
            .x
            .layout
            .as_ref()
            .map_or(5, |l| (l.width / l.pitch.max(1.0)).ceil() as usize + 3);
        for i in first..first + n {
            self.request_card_image(r, i, Priority::Visible);
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

    /// „Abspielen“ im Billboard: Filme und Folgen werden direkt abgespielt, alles andere öffnet das Detail-Modal.
    pub fn open_item(self: &Rc<Self>, item: &BaseItem) {
        match item.kind {
            ItemKind::Movie | ItemKind::Episode => self.play_item(item),
            _ => self.on_card_activate(&item.id), // W2-detail
        }
    }

    /// Ziele der Kopfleiste: 0..=4 Seiten, 5 Suche, 6 TV, 7 Datensparmodus, 8 Profilmenü.
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
            7 => {
                self.home_open_menu(2);
                return;
            }
            8 => {
                self.home_open_menu(1);
                return;
            }
            _ => return,
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
