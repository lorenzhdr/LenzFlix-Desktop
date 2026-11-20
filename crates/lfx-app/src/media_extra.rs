//! W4-media: Programmführer und Live-TV-Laden, Foto-Betrachter, Bücher (Herunterladen und mit dem System öffnen).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use lfx_api::{BaseItem, ItemKind, ProgramsQuery, TimerInfo};
use lfx_core::images::{ImageRef, own_image};
use lfx_images::Priority;
use lfx_nav::NavAction;
use lfx_tv_schedule::time::{civil_from_days, date_parse_iso, format_clock, offset_ms};
use lfx_ui::{GuideCell, GuideHour, GuideRow, ToastKind};
use slint::{Image, ModelRc, Timer, TimerMode, VecModel};

use crate::app::{App, current};
use crate::media::{Act, Loaded, View, Z_BODY};
use crate::media_model::{
    GUIDE_HOURS, GUIDE_PX_PER_MIN, ZOOM_MAX, ZOOM_MIN, book_file_name, clamp_pan, guide_cell,
    guide_now_x, guide_window_start, zoom_step,
};
use lfx_api::ImageType;

const HOUR_MS: i64 = 3_600_000;

// ---------------------------------------------------------------- Zeit

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let r = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        r / 3600,
        r % 3600 / 60,
        r % 60
    )
}

fn parse_ms(s: &str) -> Option<i64> {
    date_parse_iso(s).map(|v| v as i64)
}

/// „TT.MM. HH:MM“ in der Planzone (MEZ/MESZ).
pub fn when_text(iso: &str) -> String {
    let Some(t) = parse_ms(iso) else {
        return String::new();
    };
    let w = t + offset_ms(t);
    let (_, m, d) = civil_from_days(w.div_euclid(86_400_000));
    format!("{d:02}.{m:02}. {}", format_clock(t))
}

// ---------------------------------------------------------------- Programmführer

#[derive(Clone, Debug)]
pub struct GProg {
    pub item: BaseItem,
    pub start: i64,
    pub end: i64,
    pub x: f64,
    pub w: f64,
}

#[derive(Clone, Default)]
pub struct GuideData {
    pub channels: Vec<BaseItem>,
    pub rows: Vec<Vec<GProg>>,
    pub win_start: i64,
    pub timers: Vec<TimerInfo>,
}

/// Lädt die Live-TV-Reiter: 0 Kanäle, 1 Programm, 2 Aufnahmen, 3 Timer.
pub async fn load_live(client: &lfx_api::Client, tab: usize) -> Result<Loaded, String> {
    let e = |e: lfx_api::ApiError| e.to_string();
    Ok(match tab {
        0 => Loaded::Cards {
            head: None,
            items: client.live_channels(true, 100, 0).await.map_err(e)?.items,
            w: 300.0,
            h: 169.0,
            empty: "Keine Sender gefunden. Live-TV braucht einen Server mit Tuner oder Senderliste.",
        },
        1 => {
            let channels = client.live_channels(false, 40, 0).await.map_err(e)?.items;
            let win_start = guide_window_start(now_ms());
            let q = ProgramsQuery {
                channel_ids: channels.iter().map(|c| c.id.clone()).collect(),
                min_end_date: Some(iso_from_ms(win_start)),
                max_start_date: Some(iso_from_ms(win_start + GUIDE_HOURS * HOUR_MS)),
                limit: Some(1200),
                ..ProgramsQuery::default()
            };
            let programs = client.live_programs(&q).await.map_err(e)?.items;
            let timers = client.timers().await.map(|t| t.items).unwrap_or_default();
            let mut rows: Vec<Vec<GProg>> = vec![Vec::new(); channels.len()];
            for p in programs {
                let Some(ci) = channels
                    .iter()
                    .position(|c| Some(&c.id) == p.channel_id.as_ref())
                else {
                    continue;
                };
                let (Some(s), Some(en)) = (
                    p.start_date.as_deref().and_then(parse_ms),
                    p.end_date.as_deref().and_then(parse_ms),
                ) else {
                    continue;
                };
                if let Some((x, w)) = guide_cell(win_start, s, en) {
                    rows[ci].push(GProg {
                        item: p,
                        start: s,
                        end: en,
                        x,
                        w,
                    });
                }
            }
            for r in &mut rows {
                r.sort_by_key(|g| g.start);
            }
            Loaded::Guide(GuideData {
                channels,
                rows,
                win_start,
                timers,
            })
        }
        2 => Loaded::Cards {
            head: None,
            items: client.recordings(100, 0).await.map_err(e)?.items,
            w: 300.0,
            h: 169.0,
            empty: "Keine Aufnahmen vorhanden.",
        },
        _ => Loaded::Timers(client.timers().await.map_err(e)?.items),
    })
}

impl App {
    pub(crate) fn guide_loaded(self: &Rc<Self>, g: GuideData) {
        let now = now_ms();
        let rows: Vec<GuideRow> = g
            .channels
            .iter()
            .zip(&g.rows)
            .map(|(c, progs)| GuideRow {
                channel: c.name.as_str().into(),
                number: c.channel_number.clone().unwrap_or_default().into(),
                cells: ModelRc::new(VecModel::from(
                    progs
                        .iter()
                        .map(|p| GuideCell {
                            title: p.item.name.as_str().into(),
                            time: format!("{}–{}", format_clock(p.start), format_clock(p.end))
                                .into(),
                            x: p.x as f32,
                            w: p.w as f32,
                            live: p.start <= now && now < p.end,
                            rec: g
                                .timers
                                .iter()
                                .any(|t| t.program_id.as_deref() == Some(p.item.id.as_str())),
                        })
                        .collect::<Vec<_>>(),
                )),
            })
            .collect();
        let hours: Vec<GuideHour> = (0..GUIDE_HOURS)
            .map(|k| GuideHour {
                label: format_clock(g.win_start + k * HOUR_MS).into(),
                x: (k * 60) as f32 * GUIDE_PX_PER_MIN as f32,
            })
            .collect();
        let airing_col = g
            .rows
            .first()
            .and_then(|r| r.iter().position(|p| p.start <= now && now < p.end))
            .map_or(-1, |c| c as i32);
        let now_x = guide_now_x(g.win_start, now) as f32;
        {
            let mut st = self.st.borrow_mut();
            st.media.view = View::Guide;
            st.media.col = airing_col;
            st.media.items = Vec::new();
            st.media.timers = Vec::new();
            st.media.guide = g;
        }
        let empty = rows.is_empty();
        self.with_media(|ui| {
            ui.set_loading(false);
            ui.set_view(2);
            ui.set_guide(ModelRc::new(VecModel::from(rows)));
            ui.set_guide_hours(ModelRc::new(VecModel::from(hours)));
            ui.set_guide_now_x(now_x);
            ui.set_guide_width(GUIDE_HOURS as f32 * 60.0 * GUIDE_PX_PER_MIN as f32);
            ui.set_empty(if empty {
                "Kein Programm vorhanden. Live-TV braucht einen Server mit Tuner oder Senderliste."
                    .into()
            } else {
                "".into()
            });
        });
    }

    /// Enter im Programmführer: Sender ansehen (Senderspalte) oder Menü zur Sendung.
    pub fn guide_activate(self: &Rc<Self>) {
        let (row, col) = {
            let st = self.st.borrow();
            (st.media.index, st.media.col)
        };
        let (channel, prog, timer) = {
            let st = self.st.borrow();
            let g = &st.media.guide;
            let channel = g.channels.get(row).cloned();
            let prog = (col >= 0)
                .then(|| g.rows.get(row).and_then(|r| r.get(col as usize)).cloned())
                .flatten();
            let timer = prog.as_ref().and_then(|p| {
                g.timers
                    .iter()
                    .find(|t| t.program_id.as_deref() == Some(p.item.id.as_str()))
                    .map(|t| t.id.clone())
            });
            (channel, prog, timer)
        };
        let Some(channel) = channel else { return };
        match prog {
            None => self.run_act(Act::PlayItem(Box::new(channel))),
            Some(p) => {
                let mut e = vec![(
                    "Sender ansehen".to_owned(),
                    Act::PlayItem(Box::new(channel)),
                )];
                match timer {
                    Some(t) => e.push(("Aufnahme abbrechen".into(), Act::CancelTimer(t))),
                    None => e.push(("Aufnehmen".into(), Act::Record(p.item.id.clone()))),
                }
                self.media_open_menu(&p.item.name, e);
            }
        }
    }

    // ------------------------------------------------------------ Bücher

    pub(crate) fn book_loaded(self: &Rc<Self>, b: BaseItem) {
        let ext = crate::media_model::book_extension(b.container.as_deref()).to_uppercase();
        let author = if b.artists.is_empty() {
            b.album_artist.clone().unwrap_or_default()
        } else {
            b.artists.join(", ")
        };
        let meta = [author, ext]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let text = b
            .overview
            .clone()
            .unwrap_or_else(|| "Keine Beschreibung vorhanden.".into());
        {
            let mut st = self.st.borrow_mut();
            st.media.view = View::Book;
            st.media.items = Vec::new();
            st.media.head = Some(b.clone());
        }
        self.with_media(|g| {
            g.set_loading(false);
            g.set_view(3);
            g.set_book_meta(meta.into());
            g.set_book_text(text.into());
            g.set_book_action(
                if b.can_download {
                    "Herunterladen und öffnen"
                } else {
                    "Nicht zum Download freigegeben"
                }
                .into(),
            );
        });
        self.media_head_public(&b);
    }

    fn media_head_public(self: &Rc<Self>, b: &BaseItem) {
        let epoch = self.st.borrow().media.epoch;
        if let Some(pick) = crate::media::cover_pick(b) {
            self.fetch_image(
                &ImageRef {
                    pick,
                    max_width: 500,
                    quality: 85,
                },
                Priority::Visible,
                move |app, img| {
                    if app.st.borrow().media.epoch != epoch {
                        return;
                    }
                    app.with_media(|g| {
                        g.set_head_image(img);
                        g.set_head_has_image(true);
                    });
                },
            );
        }
    }

    /// Lädt das Buch in den Cache-Ordner und öffnet es mit dem Systemprogramm.
    pub fn book_open(self: &Rc<Self>) {
        let Some(book) = self.st.borrow().media.head.clone() else {
            return;
        };
        if book.kind != ItemKind::Book && book.kind != ItemKind::AudioBook {
            return;
        }
        if !book.can_download {
            self.toast(
                "Dieses Buch ist nicht zum Download freigegeben.",
                ToastKind::Info,
            );
            return;
        }
        if std::mem::replace(&mut self.st.borrow_mut().media.book_busy, true) {
            return;
        }
        let Some(client) = self.media_client() else {
            return;
        };
        let dir = self.dirs.cache.join("books");
        let name = book_file_name(&book.id, book.container.as_deref());
        self.toast("Das Buch wird geladen …", ToastKind::Info);
        self.rt.spawn(async move {
            let res: Result<PathBuf, String> = async {
                let url = client.download_url(&book.id).map_err(|e| e.to_string())?;
                let bytes = client.fetch_bytes(&url).await.map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(&name);
                std::fs::write(&path, &bytes[..]).map_err(|e| e.to_string())?;
                Ok(path)
            }
            .await;
            crate::app::post(move |app| {
                app.st.borrow_mut().media.book_busy = false;
                match res {
                    Ok(path) => app.book_downloaded(path),
                    Err(e) => {
                        tracing::warn!("Buch nicht geladen: {e}");
                        app.toast("Das Buch konnte nicht geladen werden.", ToastKind::Error);
                    }
                }
            });
        });
    }

    fn book_downloaded(self: &Rc<Self>, path: PathBuf) {
        let record_only =
            cfg!(feature = "dev-automation") || std::env::var_os("LFX_NO_OPEN").is_some();
        if record_only {
            self.st.borrow_mut().media.opened_files.push(path);
            self.toast("Buch geladen.", ToastKind::Success);
            return;
        }
        match open_external(&path) {
            Ok(()) => self.toast(
                "Das Buch wird mit dem Standardprogramm geöffnet.",
                ToastKind::Success,
            ),
            Err(e) => {
                tracing::warn!("Öffnen fehlgeschlagen: {e}");
                self.toast(
                    "Das Buch ist geladen, ließ sich aber nicht öffnen.",
                    ToastKind::Error,
                );
            }
        }
    }

    // ------------------------------------------------------------ Foto-Betrachter

    pub fn viewer_open(self: &Rc<Self>, item_index: usize) {
        let items = self.st.borrow().media.items.clone();
        let Some(cur) = items.get(item_index) else {
            return;
        };
        let photos: Vec<BaseItem> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Photo)
            .cloned()
            .collect();
        let index = photos.iter().position(|p| p.id == cur.id).unwrap_or(0);
        self.st.borrow_mut().media.viewer = Some(Viewer {
            photos,
            index,
            zoom: 1.0,
            pan: (0.0, 0.0),
            slide: false,
            serial: 0,
            timer: Timer::default(),
        });
        self.with_media(|g| g.set_viewer_open(true));
        self.viewer_load();
    }

    pub fn viewer_close(self: &Rc<Self>) {
        let v = self.st.borrow_mut().media.viewer.take();
        if let Some(v) = v {
            v.timer.stop();
            self.with_media(|g| {
                g.set_viewer_open(false);
                g.set_viewer_has_image(false);
                g.set_viewer_slide(false);
            });
        }
    }

    fn viewer_load(self: &Rc<Self>) {
        let (photo, serial) = {
            let mut st = self.st.borrow_mut();
            let Some(v) = st.media.viewer.as_mut() else {
                return;
            };
            v.serial += 1;
            v.zoom = 1.0;
            v.pan = (0.0, 0.0);
            (v.photos.get(v.index).cloned(), v.serial)
        };
        let Some(photo) = photo else { return };
        self.viewer_sync();
        self.with_media(|g| g.set_viewer_has_image(false));
        if let Some(pick) = own_image(&photo, ImageType::Primary) {
            self.fetch_image(
                &ImageRef {
                    pick,
                    max_width: 2560,
                    quality: 85,
                },
                Priority::Visible,
                move |app, img: Image| {
                    let ok = app
                        .st
                        .borrow()
                        .media
                        .viewer
                        .as_ref()
                        .is_some_and(|v| v.serial == serial);
                    if ok {
                        app.with_media(|g| {
                            g.set_viewer_image(img);
                            g.set_viewer_has_image(true);
                        });
                    }
                },
            );
        }
    }

    fn viewer_sync(self: &Rc<Self>) {
        let st = self.st.borrow();
        let Some(v) = st.media.viewer.as_ref() else {
            return;
        };
        let title = v
            .photos
            .get(v.index)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let counter = format!("{} / {}", v.index + 1, v.photos.len());
        let (zoom, pan, slide) = (v.zoom, v.pan, v.slide);
        drop(st);
        self.with_media(|g| {
            g.set_viewer_title(title.into());
            g.set_viewer_counter(counter.into());
            g.set_viewer_zoom(zoom);
            g.set_viewer_pan_x(pan.0);
            g.set_viewer_pan_y(pan.1);
            g.set_viewer_slide(slide);
        });
    }

    pub fn viewer_step(self: &Rc<Self>, delta: i32) {
        {
            let mut st = self.st.borrow_mut();
            let Some(v) = st.media.viewer.as_mut() else {
                return;
            };
            let n = v.photos.len() as i32;
            if n == 0 {
                return;
            }
            v.index = (v.index as i32 + delta).rem_euclid(n) as usize;
        }
        self.viewer_load();
    }

    pub fn viewer_zoom(self: &Rc<Self>, steps: i32) {
        {
            let mut st = self.st.borrow_mut();
            let Some(v) = st.media.viewer.as_mut() else {
                return;
            };
            v.zoom = if steps == -20 {
                ZOOM_MIN
            } else {
                zoom_step(v.zoom, steps)
            };
            v.zoom = v.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
            v.pan = (clamp_pan(v.zoom, v.pan.0), clamp_pan(v.zoom, v.pan.1));
        }
        self.viewer_sync();
    }

    fn viewer_pan(self: &Rc<Self>, dx: f32, dy: f32) {
        {
            let mut st = self.st.borrow_mut();
            let Some(v) = st.media.viewer.as_mut() else {
                return;
            };
            v.pan = (
                clamp_pan(v.zoom, v.pan.0 + dx / v.zoom),
                clamp_pan(v.zoom, v.pan.1 + dy / v.zoom),
            );
        }
        self.viewer_sync();
    }

    pub fn viewer_toggle_slide(self: &Rc<Self>) {
        let ms: u64 = std::env::var("LFX_SLIDE_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4000);
        {
            let mut st = self.st.borrow_mut();
            let Some(v) = st.media.viewer.as_mut() else {
                return;
            };
            v.slide = !v.slide;
            if v.slide {
                v.timer
                    .start(TimerMode::Repeated, Duration::from_millis(ms), || {
                        if let Some(app) = current() {
                            app.viewer_step(1);
                        }
                    });
            } else {
                v.timer.stop();
            }
        }
        self.viewer_sync();
    }

    pub fn viewer_press(self: &Rc<Self>, i: i32) {
        match i {
            0 => self.viewer_step(-1),
            1 => self.viewer_step(1),
            2 => self.viewer_zoom(1),
            3 => self.viewer_zoom(-1),
            4 => self.viewer_toggle_slide(),
            _ => self.viewer_close(),
        }
    }

    pub fn viewer_action(self: &Rc<Self>, a: NavAction) -> bool {
        let zoomed = self
            .st
            .borrow()
            .media
            .viewer
            .as_ref()
            .is_some_and(|v| v.zoom > 1.0);
        match a {
            NavAction::Back => self.viewer_close(),
            NavAction::Left if zoomed => self.viewer_pan(0.15, 0.0),
            NavAction::Right if zoomed => self.viewer_pan(-0.15, 0.0),
            NavAction::Left | NavAction::Previous => self.viewer_step(-1),
            NavAction::Right | NavAction::Next => self.viewer_step(1),
            NavAction::Up if zoomed => self.viewer_pan(0.0, 0.15),
            NavAction::Down if zoomed => self.viewer_pan(0.0, -0.15),
            NavAction::Up | NavAction::PageUp => self.viewer_zoom(1),
            NavAction::Down | NavAction::PageDown => self.viewer_zoom(-1),
            NavAction::Activate | NavAction::PlayPause => self.viewer_toggle_slide(),
            NavAction::Menu | NavAction::Info => self.viewer_zoom(-20),
            _ => {}
        }
        let _ = Z_BODY;
        true
    }
}

/// Zustand des Foto-Betrachters.
pub struct Viewer {
    pub photos: Vec<BaseItem>,
    pub index: usize,
    pub zoom: f32,
    pub pan: (f32, f32),
    pub slide: bool,
    pub serial: u64,
    pub timer: Timer,
}

/// Öffnet eine Datei mit dem Standardprogramm des Systems (kein Dateiname vom Server, nur der eigene Pfad).
pub fn open_external(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let mut cmd = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut cmd = Command::new("xdg-open");
    cmd.arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}
