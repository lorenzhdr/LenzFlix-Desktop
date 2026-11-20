//! Offline-Bibliothek: fertige Downloads auflisten, Belegung, Löschen, Element-JSON für die Anzeige,
//! lokale Wiedergabe-Anforderung (Dateipfad statt URL) und „gesehen, dann automatisch löschen“.

use crate::error::{OfflineError, Result};
use crate::fetch::sha256_file;
use crate::layout::OfflineLayout;
use crate::store::{DownloadStore, OfflineOutbox};
use crate::types::{DownloadRecord, DownloadState, LocalUserData};
use lfx_api::TICKS_PER_SECOND;
use lfx_player::{ExternalSubtitle, PlayRequest};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Art der Outbox-Meldungen des Offline-Modus.
pub const OUTBOX_KIND: &str = "offline.userdata";

/// Ein lokaler Untertitel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineSubtitle {
    /// Dateipfad.
    pub path: PathBuf,
    /// Sprache.
    pub language: Option<String>,
    /// Anzeigetitel.
    pub title: Option<String>,
    /// Format.
    pub codec: Option<String>,
    /// Jellyfin-Stream-Index.
    pub index: i32,
    /// Standardspur?
    pub is_default: bool,
}

/// Ein fertiger Offline-Titel mit allen lokalen Pfaden.
#[derive(Debug, Clone, PartialEq)]
pub struct OfflineEntry {
    /// Datensatz.
    pub record: DownloadRecord,
    /// Mediendatei.
    pub media_path: PathBuf,
    /// Bilder nach Art (`primary`, `backdrop`, `logo`).
    pub images: HashMap<String, PathBuf>,
    /// Untertitel.
    pub subtitles: Vec<OfflineSubtitle>,
}

/// Belegung des Speichers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Fertige Titel.
    pub completed: usize,
    /// Unfertige Downloads (Warteschlange, Pause, Fehler, laufend).
    pub incomplete: usize,
    /// Belegte Bytes aller Dateien fertiger Titel (Medien, Bilder, Untertitel, JSON).
    pub completed_bytes: u64,
    /// Bytes der unfertigen `.part`-Dateien.
    pub partial_bytes: u64,
}

/// Lokale Wiedergabe-Anforderung.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPlayback {
    /// Der Offline-Titel.
    pub entry: OfflineEntry,
    /// Startposition in Ticks (lokaler Stand, sonst Stand zum Zeitpunkt des Downloads; 0 bei „gesehen“).
    pub start_ticks: i64,
}

impl LocalPlayback {
    /// Auftrag für den Player: Dateipfad statt URL, keine Header, Untertitel als lokale Dateien.
    /// Fortschritt meldet der Aufrufer über [`crate::OutboxSink`] statt über den Server.
    pub fn to_play_request(&self) -> PlayRequest {
        let rec = &self.entry.record;
        let mut req = PlayRequest::new(
            rec.item_id.clone(),
            rec.media_source_id.clone(),
            self.entry.media_path.to_string_lossy().into_owned(),
        );
        req.start_seconds = self.start_ticks.max(0) as f64 / TICKS_PER_SECOND as f64;
        req.external_subtitles = self
            .entry
            .subtitles
            .iter()
            .map(|s| ExternalSubtitle {
                url: s.path.to_string_lossy().into_owned(),
                lang: s.language.clone(),
                title: s.title.clone(),
            })
            .collect();
        req
    }
}

/// Offline-Bibliothek eines Kontos.
#[derive(Clone)]
pub struct OfflineLibrary {
    store: Arc<dyn DownloadStore>,
    layout: OfflineLayout,
    server_id: String,
    user_id: String,
    auto_delete: Arc<AtomicBool>,
}

impl OfflineLibrary {
    /// Bibliothek über Ablage und Speicherort.
    pub fn new(
        store: Arc<dyn DownloadStore>,
        layout: OfflineLayout,
        server_id: String,
        user_id: String,
    ) -> Self {
        Self {
            store,
            layout,
            server_id,
            user_id,
            auto_delete: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Nutzt einen gemeinsamen Schalter (der Manager reicht ihn an alle Bibliotheken weiter).
    pub(crate) fn with_auto_delete_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.auto_delete = flag;
        self
    }

    /// „Gesehen, dann automatisch löschen“ ein-/ausschalten (Standard aus; Speichern ist Sache der App).
    pub fn set_auto_delete(&self, on: bool) {
        self.auto_delete.store(on, Ordering::SeqCst);
    }

    /// Ist das automatische Löschen an?
    pub fn auto_delete(&self) -> bool {
        self.auto_delete.load(Ordering::SeqCst)
    }

    fn dir(&self, rec: &DownloadRecord) -> Result<PathBuf> {
        self.layout
            .item_dir(&rec.server_id, &rec.user_id, &rec.item_id)
    }

    fn record_of(&self, item_id: &str) -> Result<Option<DownloadRecord>> {
        Ok(self
            .store
            .list(&self.server_id, &self.user_id)?
            .into_iter()
            .find(|r| r.item_id == item_id))
    }

    pub(crate) fn record(&self, item_id: &str) -> Result<DownloadRecord> {
        self.record_of(item_id)?
            .ok_or(OfflineError::UnknownDownload)
    }

    pub(crate) fn save(&self, rec: &DownloadRecord) -> Result<()> {
        self.store.put(rec)
    }

    fn entry_of(&self, rec: DownloadRecord) -> Result<OfflineEntry> {
        let dir = self.dir(&rec)?;
        let media_path = dir.join(format!("media.{}", rec.extension));
        let mut images = HashMap::new();
        let mut subtitles = Vec::new();
        for f in &rec.files {
            match f.kind.as_str() {
                "primary" | "backdrop" | "logo" => {
                    images.insert(f.kind.clone(), dir.join(&f.rel));
                }
                "subtitle" => subtitles.push(OfflineSubtitle {
                    path: dir.join(&f.rel),
                    language: f.language.clone(),
                    title: f.title.clone(),
                    codec: f.codec.clone(),
                    index: f.index.unwrap_or(0),
                    is_default: f.is_default,
                }),
                _ => {}
            }
        }
        Ok(OfflineEntry {
            record: rec,
            media_path,
            images,
            subtitles,
        })
    }

    /// Alle fertigen Titel des Kontos.
    pub fn entries(&self) -> Result<Vec<OfflineEntry>> {
        self.store
            .list(&self.server_id, &self.user_id)?
            .into_iter()
            .filter(|r| r.state == DownloadState::Completed)
            .map(|r| self.entry_of(r))
            .collect()
    }

    /// Ein fertiger Titel nach Element-Id.
    pub fn entry(&self, item_id: &str) -> Result<Option<OfflineEntry>> {
        self.record_of(item_id)?
            .filter(|r| r.state == DownloadState::Completed)
            .map(|r| self.entry_of(r))
            .transpose()
    }

    /// Ist der Titel fertig geladen?
    pub fn is_available(&self, item_id: &str) -> bool {
        self.entry(item_id)
            .ok()
            .flatten()
            .is_some_and(|e| e.media_path.is_file())
    }

    /// Gespeichertes Element-JSON (so, wie der Server es geliefert hat, ohne Serverpfade).
    pub fn item_json(&self, item_id: &str) -> Result<Value> {
        self.json_file(item_id, "item.json")
    }

    /// Gespeichertes Serien-JSON einer Folge.
    pub fn series_json(&self, item_id: &str) -> Result<Value> {
        self.json_file(item_id, "series.json")
    }

    /// Gespeichertes Staffel-JSON einer Folge.
    pub fn season_json(&self, item_id: &str) -> Result<Value> {
        self.json_file(item_id, "season.json")
    }

    fn json_file(&self, item_id: &str, name: &str) -> Result<Value> {
        let rec = self.record(item_id)?;
        let bytes = std::fs::read(self.dir(&rec)?.join(name))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Belegung des Speichers.
    pub fn usage(&self) -> Result<Usage> {
        let mut u = Usage::default();
        for rec in self.store.list(&self.server_id, &self.user_id)? {
            let dir = self.dir(&rec)?;
            let size_of = |p: PathBuf| std::fs::metadata(p).map_or(0, |m| m.len());
            if rec.state == DownloadState::Completed {
                u.completed += 1;
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    u.completed_bytes += rd
                        .flatten()
                        .filter_map(|e| e.metadata().ok())
                        .filter(|m| m.is_file())
                        .map(|m| m.len())
                        .sum::<u64>();
                }
            } else {
                u.incomplete += 1;
                u.partial_bytes += size_of(dir.join("media.part"));
            }
        }
        Ok(u)
    }

    /// Löscht einen fertigen Titel samt Dateien. Unfertige Downloads löscht
    /// [`crate::DownloadManager::cancel`]. Nicht gesendete Fortschrittsmeldungen bleiben in der Outbox
    /// und werden trotzdem abgeglichen.
    pub fn delete(&self, item_id: &str) -> Result<()> {
        let rec = self.record(item_id)?;
        if rec.state != DownloadState::Completed {
            return Err(OfflineError::InvalidState);
        }
        self.remove(&rec)
    }

    fn remove(&self, rec: &DownloadRecord) -> Result<()> {
        let dir = self.dir(rec)?;
        if let Err(e) = std::fs::remove_dir_all(&dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(e.into());
        }
        self.store.delete(&rec.id)?;
        Ok(())
    }

    /// Löscht alle fertigen Folgen einer Serie. Gibt die Anzahl zurück.
    pub fn delete_series(&self, series_id: &str) -> Result<usize> {
        let mut n = 0;
        for rec in self.store.list(&self.server_id, &self.user_id)? {
            if rec.state == DownloadState::Completed && rec.series_id.as_deref() == Some(series_id)
            {
                self.remove(&rec)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Löscht fertige Titel, die lokal als gesehen gelten und deren Fortschritt abgeglichen ist
    /// (keine ausstehende Outbox-Meldung zu diesem Titel). Nur wirksam, wenn das automatische Löschen an ist.
    /// Gibt die Element-Ids der gelöschten Titel zurück.
    pub fn apply_auto_delete(&self, outbox: &dyn OfflineOutbox) -> Result<Vec<String>> {
        if !self.auto_delete() {
            return Ok(Vec::new());
        }
        let pending = outbox.pending(&self.server_id, &self.user_id, OUTBOX_KIND)?;
        let unsynced: Vec<&str> = pending
            .iter()
            .filter_map(|p| p.payload.get("item_id").and_then(Value::as_str))
            .collect();
        let mut deleted = Vec::new();
        for rec in self.store.list(&self.server_id, &self.user_id)? {
            let played = rec.local.as_ref().is_some_and(|l| l.played);
            if rec.state == DownloadState::Completed
                && played
                && !unsynced.contains(&rec.item_id.as_str())
            {
                self.remove(&rec)?;
                deleted.push(rec.item_id);
            }
        }
        Ok(deleted)
    }

    /// Prüft die Mediendatei gegen gespeicherte Größe und Prüfsumme.
    pub async fn verify(&self, item_id: &str) -> Result<bool> {
        let entry = self.entry(item_id)?.ok_or(OfflineError::UnknownDownload)?;
        let Ok(meta) = tokio::fs::metadata(&entry.media_path).await else {
            return Ok(false);
        };
        if entry.record.bytes_total.is_some_and(|t| t != meta.len()) {
            return Ok(false);
        }
        match &entry.record.sha256 {
            Some(expected) => Ok(sha256_file(&entry.media_path)
                .await?
                .eq_ignore_ascii_case(expected)),
            None => Ok(true),
        }
    }

    /// Lokale Benutzerdaten eines Titels.
    pub fn local_user_data(&self, item_id: &str) -> Result<Option<LocalUserData>> {
        Ok(self.record(item_id)?.local)
    }

    /// Wiedergabe-Anforderung für einen fertigen Titel.
    pub fn playback(&self, item_id: &str) -> Result<LocalPlayback> {
        let entry = self.entry(item_id)?.ok_or(OfflineError::UnknownDownload)?;
        if !entry.media_path.is_file() {
            return Err(OfflineError::UnknownDownload);
        }
        let start_ticks = match &entry.record.local {
            Some(l) => l.position_ticks,
            None => self
                .item_json(item_id)
                .ok()
                .and_then(|v| v.get("UserData")?.get("PlaybackPositionTicks")?.as_i64())
                .unwrap_or(0),
        };
        Ok(LocalPlayback {
            entry,
            start_ticks: start_ticks.max(0),
        })
    }
}
