//! Plattencache für kodierte Bild-Bytes. Dateiname = Hash des Schlüssels, Verwaltung
//! (Größe, letzte Nutzung) über das Trait [`DiskIndex`]. Die Standard-Implementierung
//! [`FileIndex`] speichert eine JSON-Datei; `lfx-store` kann später eine Datenbank anbinden.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lfx_platform::AppDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Name der Indexdatei der Standard-Implementierung.
pub const INDEX_FILE: &str = "index.json";
/// Unterordner im App-Cache.
pub const CACHE_SUBDIR: &str = "images";
/// Änderungen zwischen zwei automatischen Index-Speicherungen.
const PERSIST_EVERY: u32 = 16;

/// Eintrag im Index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskEntry {
    /// Dateigröße in Bytes.
    pub size: u64,
    /// Nutzungszähler (größer = zuletzt benutzt); kein Zeitstempel, damit deterministisch.
    pub tick: u64,
}

/// Verwaltung der Cache-Einträge. Schlüssel ist der Dateiname (Hash).
pub trait DiskIndex: Send + Sync {
    /// Eintrag lesen.
    fn get(&self, name: &str) -> Option<DiskEntry>;
    /// Eintrag anlegen oder aktualisieren und als zuletzt benutzt markieren.
    fn upsert(&self, name: &str, size: u64);
    /// Als zuletzt benutzt markieren.
    fn touch(&self, name: &str);
    /// Eintrag entfernen.
    fn remove(&self, name: &str);
    /// Am längsten ungenutzter Eintrag.
    fn oldest(&self) -> Option<String>;
    /// Alle Namen (zum Abgleich mit dem Ordner).
    fn names(&self) -> Vec<String>;
    /// Summe aller Größen.
    fn total_bytes(&self) -> u64;
    /// Dauerhaft sichern (für Indizes ohne eigene Persistenz: nichts tun).
    fn persist(&self) -> io::Result<()>;
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default, Serialize, Deserialize)]
struct IndexData {
    tick: u64,
    entries: HashMap<String, DiskEntry>,
}

/// Index als JSON-Datei (atomar geschrieben). Eine kaputte Datei führt zu einem leeren Index;
/// der Abgleich in [`DiskCache::open`] baut ihn aus dem Ordner wieder auf.
pub struct FileIndex {
    path: PathBuf,
    data: Mutex<(IndexData, bool)>,
}

impl FileIndex {
    /// Lädt die Datei (fehlt sie oder ist sie kaputt, startet der Index leer).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let data = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<IndexData>(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            data: Mutex::new((data, false)),
        }
    }
}

impl DiskIndex for FileIndex {
    fn get(&self, name: &str) -> Option<DiskEntry> {
        lock(&self.data).0.entries.get(name).copied()
    }

    fn upsert(&self, name: &str, size: u64) {
        let mut g = lock(&self.data);
        g.0.tick += 1;
        let tick = g.0.tick;
        g.0.entries
            .insert(name.to_owned(), DiskEntry { size, tick });
        g.1 = true;
    }

    fn touch(&self, name: &str) {
        let mut g = lock(&self.data);
        g.0.tick += 1;
        let tick = g.0.tick;
        if let Some(e) = g.0.entries.get_mut(name) {
            e.tick = tick;
            g.1 = true;
        }
    }

    fn remove(&self, name: &str) {
        let mut g = lock(&self.data);
        if g.0.entries.remove(name).is_some() {
            g.1 = true;
        }
    }

    fn oldest(&self) -> Option<String> {
        lock(&self.data)
            .0
            .entries
            .iter()
            .min_by_key(|(_, e)| e.tick)
            .map(|(n, _)| n.clone())
    }

    fn names(&self) -> Vec<String> {
        lock(&self.data).0.entries.keys().cloned().collect()
    }

    fn total_bytes(&self) -> u64 {
        lock(&self.data).0.entries.values().map(|e| e.size).sum()
    }

    fn persist(&self) -> io::Result<()> {
        let mut g = lock(&self.data);
        if !g.1 {
            return Ok(());
        }
        let json = serde_json::to_vec(&g.0).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        g.1 = false;
        Ok(())
    }
}

/// Dateiname zu einem Schlüssel: die ersten 16 Bytes von SHA-256 als Hex (32 Zeichen).
pub fn file_name_for(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let mut out = String::with_capacity(32);
    for b in digest.iter().take(16) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn is_cache_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Plattencache mit Byte-Budget und LRU-Verdrängung.
pub struct DiskCache {
    dir: PathBuf,
    index: Arc<dyn DiskIndex>,
    budget: u64,
    pending: AtomicU32,
}

impl DiskCache {
    /// Öffnet den Cache in `dir` mit eigenem Index und gleicht Index und Ordner ab.
    pub fn open(
        dir: impl Into<PathBuf>,
        budget: u64,
        index: Arc<dyn DiskIndex>,
    ) -> io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let cache = Self {
            dir,
            index,
            budget,
            pending: AtomicU32::new(0),
        };
        cache.reconcile()?;
        Ok(cache)
    }

    /// Öffnet den Cache mit dem Standard-[`FileIndex`] (`<dir>/index.json`).
    pub fn open_default(dir: impl Into<PathBuf>, budget: u64) -> io::Result<Self> {
        let dir = dir.into();
        let index = Arc::new(FileIndex::load(dir.join(INDEX_FILE)));
        Self::open(dir, budget, index)
    }

    /// Standardort im App-Cache (`<cache>/images`).
    pub fn in_app_dirs(dirs: &AppDirs, budget: u64) -> io::Result<Self> {
        Self::open_default(dirs.cache.join(CACHE_SUBDIR), budget)
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Index und Ordner angleichen: fremde Dateien im Ordner werden aufgenommen,
    /// Einträge ohne Datei und liegengebliebene `.tmp`-Dateien verschwinden.
    fn reconcile(&self) -> io::Result<()> {
        let mut on_disk = HashMap::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name.ends_with(".tmp") && file_name != format!("{INDEX_FILE}.tmp") {
                let _ = std::fs::remove_file(entry.path());
            } else if is_cache_name(&file_name) {
                on_disk.insert(file_name, entry.metadata()?.len());
            }
        }
        for name in self.index.names() {
            if !on_disk.contains_key(&name) {
                self.index.remove(&name);
            }
        }
        for (name, size) in on_disk {
            if self.index.get(&name).is_none() {
                self.index.upsert(&name, size);
            }
        }
        self.evict();
        self.index.persist()
    }

    /// Bytes zu `key`, falls vorhanden (zählt als Nutzung).
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let name = file_name_for(key);
        self.index.get(&name)?;
        match std::fs::read(self.path_for(&name)) {
            Ok(bytes) => {
                self.index.touch(&name);
                self.note_change();
                Some(bytes)
            }
            Err(_) => {
                self.index.remove(&name);
                self.note_change();
                None
            }
        }
    }

    /// Ob `key` im Cache steht (ohne Lesen, ohne Nutzungsmarke).
    pub fn contains(&self, key: &str) -> bool {
        self.index.get(&file_name_for(key)).is_some()
    }

    /// Speichert `bytes` (atomar). `Ok(false)`, wenn die Datei größer als das ganze Budget ist.
    pub fn put(&self, key: &str, bytes: &[u8]) -> io::Result<bool> {
        if bytes.len() as u64 > self.budget {
            return Ok(false);
        }
        let name = file_name_for(key);
        let tmp = self.path_for(&format!("{name}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, self.path_for(&name))?;
        self.index.upsert(&name, bytes.len() as u64);
        self.evict();
        self.note_change();
        Ok(true)
    }

    /// Entfernt einen Eintrag.
    pub fn remove(&self, key: &str) {
        let name = file_name_for(key);
        let _ = std::fs::remove_file(self.path_for(&name));
        self.index.remove(&name);
        self.note_change();
    }

    /// Leert den Cache.
    pub fn clear(&self) {
        for name in self.index.names() {
            let _ = std::fs::remove_file(self.path_for(&name));
            self.index.remove(&name);
        }
        let _ = self.index.persist();
    }

    /// Summe der gespeicherten Bytes.
    pub fn total_bytes(&self) -> u64 {
        self.index.total_bytes()
    }

    /// Anzahl der Einträge.
    pub fn len(&self) -> usize {
        self.index.names().len()
    }

    /// Ob der Cache leer ist.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ordner des Caches.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Index sichern.
    pub fn flush(&self) -> io::Result<()> {
        self.pending.store(0, Ordering::Relaxed);
        self.index.persist()
    }

    /// Verdrängt die am längsten ungenutzten Einträge, bis das Budget eingehalten ist.
    fn evict(&self) {
        while self.index.total_bytes() > self.budget {
            let Some(name) = self.index.oldest() else {
                break;
            };
            let _ = std::fs::remove_file(self.path_for(&name));
            self.index.remove(&name);
        }
    }

    fn note_change(&self) {
        if self.pending.fetch_add(1, Ordering::Relaxed) + 1 >= PERSIST_EVERY {
            let _ = self.flush();
        }
    }
}

impl Drop for DiskCache {
    fn drop(&mut self) {
        let _ = self.index.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_stable_hex() {
        let a = file_name_for("abc|Primary||0x0|q0");
        assert_eq!(a.len(), 32);
        assert!(is_cache_name(&a));
        assert_eq!(a, file_name_for("abc|Primary||0x0|q0"));
        assert_ne!(a, file_name_for("abd|Primary||0x0|q0"));
        // Bekannter SHA-256 von "abc" beginnt mit ba7816bf8f01cfea414140de5dae2223.
        assert_eq!(file_name_for("abc"), "ba7816bf8f01cfea414140de5dae2223");
    }

    #[test]
    fn roundtrip_and_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let cache = DiskCache::open_default(tmp.path(), 1000).unwrap();
            assert!(cache.get("k1").is_none());
            assert!(cache.put("k1", b"hallo").unwrap());
            assert_eq!(cache.get("k1").unwrap(), b"hallo");
            assert!(cache.contains("k1"));
            assert_eq!(cache.total_bytes(), 5);
        }
        let cache = DiskCache::open_default(tmp.path(), 1000).unwrap();
        assert_eq!(cache.get("k1").unwrap(), b"hallo");
        assert_eq!(cache.len(), 1);
        cache.remove("k1");
        assert!(cache.get("k1").is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn evicts_least_recently_used() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskCache::open_default(tmp.path(), 10).unwrap();
        cache.put("a", &[1; 4]).unwrap();
        cache.put("b", &[2; 4]).unwrap();
        // a wird benutzt, damit b der älteste ist.
        assert!(cache.get("a").is_some());
        cache.put("c", &[3; 4]).unwrap();
        assert!(cache.contains("a"));
        assert!(!cache.contains("b"));
        assert!(cache.contains("c"));
        assert!(cache.total_bytes() <= 10);
        // Zu große Datei wird abgelehnt.
        assert!(!cache.put("big", &[0; 11]).unwrap());
    }

    #[test]
    fn reconcile_repairs_index() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let cache = DiskCache::open_default(tmp.path(), 1000).unwrap();
            cache.put("a", b"AAAA").unwrap();
            cache.put("b", b"BB").unwrap();
        }
        // Index kaputt machen, Datei von b löschen, Fremddatei und Rest-tmp anlegen.
        std::fs::write(tmp.path().join(INDEX_FILE), b"{kaputt").unwrap();
        std::fs::remove_file(tmp.path().join(file_name_for("b"))).unwrap();
        std::fs::write(tmp.path().join("rest.tmp"), b"x").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"x").unwrap();
        let cache = DiskCache::open_default(tmp.path(), 1000).unwrap();
        assert_eq!(cache.get("a").unwrap(), b"AAAA");
        assert!(cache.get("b").is_none());
        assert_eq!(cache.len(), 1);
        assert!(!tmp.path().join("rest.tmp").exists());
        assert!(tmp.path().join("notes.txt").exists());
    }

    #[test]
    fn file_index_keeps_order_across_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(INDEX_FILE);
        let idx = FileIndex::load(&path);
        idx.upsert("x", 1);
        idx.upsert("y", 2);
        idx.touch("x");
        idx.persist().unwrap();
        let again = FileIndex::load(&path);
        assert_eq!(again.oldest().as_deref(), Some("y"));
        assert_eq!(again.total_bytes(), 3);
        again.upsert("z", 1);
        assert_eq!(again.get("z").unwrap().tick, 4);
    }

    #[test]
    fn app_dirs_location() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(tmp.path());
        let cache = DiskCache::in_app_dirs(&dirs, 100).unwrap();
        assert!(cache.dir().ends_with(CACHE_SUBDIR));
        assert!(cache.dir().starts_with(&dirs.cache));
    }
}
