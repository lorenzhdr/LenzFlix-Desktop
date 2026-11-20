//! „Neu und beliebt“: Liste „Neu bei …“ plus Zeilen „Am häufigsten geschaut“, „Top 10 Serien“, „Top 10 Filme“.
//!
//! Ohne Charts-Dienst gilt der Web-Rückfall: „Am häufigsten geschaut“ nach eigener Wiedergabezahl bzw. Bewertung,
//! die Top-10-Zeilen heißen dann „Neueste Serien“ und „Neueste Filme“ (die zehn neuesten).

use std::collections::HashSet;
use std::rc::Rc;

use lfx_api::{BaseItem, Client, ImageType, ItemKind};
use lfx_core::images::{
    CardKind, ImageRef, Slot, display_title, own_image, pick_landscape, pixel_width,
};
use lfx_core::newpopular::{
    LIST_LIMIT, Section, date_parts, has_play_counts, most_watched_call,
    most_watched_fallback_call, new_call, since_label, top_call, unique_titles,
};
use lfx_core::rows::logic::new_series_query;
use lfx_images::Priority;
use lfx_ui::{CardAspect, CardData, NpEntry, RowData};
use slint::{Model, ModelRc, VecModel};

use crate::app::{App, post};
use crate::library::{LibRow, Tool, ToolAct, card_data};

/// Zustand der Seite.
#[derive(Default)]
pub struct NpState {
    pub entries: Vec<BaseItem>,
    pub model: Option<Rc<VecModel<NpEntry>>>,
    pub tab: usize,
    /// Zeilenindex je Reiter 1..3 (`None` = Zeile entfällt).
    pub tab_rows: [Option<usize>; 3],
}

struct NpData {
    new: Vec<BaseItem>,
    watched: Vec<BaseItem>,
    top_series: Vec<BaseItem>,
    top_movies: Vec<BaseItem>,
    new_series: HashSet<String>,
}

async fn fetch_np(client: &Client, now_ms: f64) -> Result<NpData, String> {
    let (c_new, c_plays) = (new_call(), most_watched_call());
    let (c_series, c_movies) = (top_call(ItemKind::Series), top_call(ItemKind::Movie));
    let c_eps = new_series_query();
    let (new, plays, series, movies, eps) = tokio::join!(
        c_new.execute(client),
        c_plays.execute(client),
        c_series.execute(client),
        c_movies.execute(client),
        c_eps.execute(client),
    );
    if new.is_err() && plays.is_err() && series.is_err() && movies.is_err() {
        return Err("Die Seite konnte nicht geladen werden. Ist der Server erreichbar?".into());
    }
    let plays = plays.unwrap_or_default();
    let watched = if has_play_counts(&plays) {
        unique_titles(&plays, LIST_LIMIT as usize)
    } else {
        let by_rating = most_watched_fallback_call()
            .execute(client)
            .await
            .unwrap_or_default();
        unique_titles(&by_rating, LIST_LIMIT as usize)
    };
    Ok(NpData {
        new: unique_titles(&new.unwrap_or_default(), LIST_LIMIT as usize),
        watched,
        top_series: unique_titles(&series.unwrap_or_default(), 10),
        top_movies: unique_titles(&movies.unwrap_or_default(), 10),
        new_series: lfx_core::rows::logic::new_series_ids_from(&eps.unwrap_or_default(), now_ms),
    })
}

fn backdrop_ref(item: &BaseItem) -> Option<ImageRef> {
    let pick = own_image(item, ImageType::Backdrop).or_else(|| pick_landscape(item))?;
    Some(ImageRef {
        pick,
        max_width: pixel_width(Slot::NewPopularBackdrop, false, 2.0, 1.0),
        quality: Slot::NewPopularBackdrop.quality(),
    })
}

impl App {
    pub fn np_enter(self: &Rc<Self>, epoch: u64) {
        let labels: Vec<String> = Section::ALL.iter().map(|s| s.label("LENZFLIX")).collect();
        let tools: Vec<Tool> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| Tool {
                label: l.clone(),
                kind: 0,
                active: i == 0,
                act: ToolAct::NpTab(i),
                right: false,
            })
            .collect();
        self.lib_set_tools(tools);
        let Some(active) = self.session.active() else {
            return;
        };
        let client = active.client.clone();
        self.rt.spawn(async move {
            let now = App::now_ms();
            let data = fetch_np(&client, now).await;
            post(move |app| app.np_loaded(epoch, data, now));
        });
    }

    fn np_loaded(self: &Rc<Self>, epoch: u64, data: Result<NpData, String>, now: f64) {
        if self.st.borrow().lib.epoch != epoch {
            return;
        }
        self.with_lib(|g| g.set_loading(false));
        let data = match data {
            Ok(d) => d,
            Err(t) => {
                self.st.borrow_mut().lib.error = t.clone();
                self.with_lib(|g| g.set_error(t.as_str().into()));
                self.lib_rebuild_graph();
                self.lib_default_focus();
                return;
            }
        };
        // Mitteleuropäische Zeit (feste Stunde Versatz); nur für die Anzeige des Kalendertags.
        let offset_min = 60;
        let entries: Vec<NpEntry> = data
            .new
            .iter()
            .map(|it| {
                let (month, day) = date_parts(it.date_created.as_deref(), offset_min)
                    .map(|(m, d)| (m.to_owned(), d))
                    .unwrap_or_default();
                NpEntry {
                    month: month.as_str().into(),
                    day: day.as_str().into(),
                    title: it.name.as_str().into(),
                    since: since_label(it, Some(&data.new_series), now, offset_min)
                        .as_str()
                        .into(),
                    overview: it.overview.clone().unwrap_or_default().as_str().into(),
                    genres: it
                        .genres
                        .iter()
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" • ")
                        .as_str()
                        .into(),
                    image: slint::Image::default(),
                    has_image: false,
                }
            })
            .collect();
        let model = Rc::new(VecModel::from(entries));
        self.with_lib(|g| g.set_entries(ModelRc::from(model.clone())));

        // Zeilen
        let mut rows: Vec<LibRow> = Vec::new();
        let mut tab_rows = [None; 3];
        let defs: [(&str, Vec<BaseItem>, bool, usize); 3] = [
            ("Am häufigsten geschaut", data.watched, false, 0),
            ("Neueste Serien", data.top_series, true, 1),
            ("Neueste Filme", data.top_movies, true, 2),
        ];
        for (title, items, top10, tab) in defs {
            if items.is_empty() {
                continue;
            }
            let cards: Vec<CardData> = items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    card_data(
                        it,
                        CardKind::Landscape,
                        top10.then_some(i + 1),
                        Some(&data.new_series),
                        now,
                    )
                })
                .collect();
            tab_rows[tab] = Some(rows.len());
            rows.push(LibRow {
                id: format!("np:{tab}"),
                title: title.to_owned(),
                kind: CardKind::Landscape,
                aspect: CardAspect::Wide,
                items,
                model: Rc::new(VecModel::from(cards)),
            });
        }
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
            st.lib.np.entries = data.new;
            st.lib.np.model = Some(model);
            st.lib.np.tab_rows = tab_rows;
            st.lib.rows = rows;
        }
        self.np_load_images(epoch);
        self.lib_load_row_images(epoch);
        self.lib_rebuild_graph();
        self.lib_default_focus();
        tracing::info!(
            entries = self.st.borrow().lib.np.entries.len(),
            "Neu und beliebt geladen"
        );
    }

    fn np_load_images(self: &Rc<Self>, epoch: u64) {
        let jobs: Vec<(usize, ImageRef)> = self
            .st
            .borrow()
            .lib
            .np
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, it)| backdrop_ref(it).map(|r| (i, r)))
            .collect();
        for (i, r) in jobs {
            self.lib_fetch_image(&r, Priority::Visible, move |app, img| {
                if app.st.borrow().lib.epoch != epoch {
                    return;
                }
                let model = app.st.borrow().lib.np.model.clone();
                if let Some(m) = model
                    && let Some(mut e) = m.row_data(i)
                {
                    e.image = img;
                    e.has_image = true;
                    m.set_row_data(i, e);
                }
            });
        }
    }

    /// Reiter gewählt: aktiv markieren und zum Abschnitt scrollen.
    pub fn np_tab(self: &Rc<Self>, i: usize) {
        let row = if i == 0 {
            Some(-1)
        } else {
            self.st
                .borrow()
                .lib
                .np
                .tab_rows
                .get(i - 1)
                .copied()
                .flatten()
                .map(|r| r as i32)
        };
        let Some(row) = row else { return };
        self.st.borrow_mut().lib.np.tab = i;
        let tools: Vec<Tool> = self
            .st
            .borrow()
            .lib
            .tools
            .iter()
            .enumerate()
            .map(|(k, t)| Tool {
                active: k == i,
                ..t.clone()
            })
            .collect();
        self.lib_set_tools(tools);
        self.with_lib(|g| {
            g.set_scroll_row(row);
            g.set_scroll_tick(g.get_scroll_tick() + 1);
        });
    }

    pub fn np_entry_activated(self: &Rc<Self>, i: usize) {
        let item = self.st.borrow().lib.np.entries.get(i).cloned();
        if let Some(item) = item {
            self.lib_open_detail(&item);
        }
    }

    pub fn np_event(self: &Rc<Self>, kind: &str, i: usize) {
        let item = self.st.borrow().lib.np.entries.get(i).cloned();
        let Some(item) = item else { return };
        match kind {
            "np-play" => self.play_item(&item),
            _ => self.lib_open_detail(&item),
        }
        let _ = display_title(&item);
    }
}
