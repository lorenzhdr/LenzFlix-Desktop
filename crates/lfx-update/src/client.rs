//! Update-Client: Versionsprüfung, Download mit Fortschritt, Verifikation.

use crate::error::UpdateError;
use crate::manifest::{Asset, MAX_ASSET_BYTES, Manifest, current_platform};
use crate::settings::Channel;
use crate::verify::{UPDATE_PUBLIC_KEY, hex_lower, is_placeholder, verify_signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Obergrenze für Manifest und Release-Liste.
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;

/// Konfiguration; die Standardwerte zeigen auf die GitHub-Releases des Projekts.
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// Manifest des neuesten stabilen Releases.
    pub stable_manifest_url: String,
    /// Release-Liste (Vorab-Kanal).
    pub releases_api_url: String,
    /// Name des Manifest-Assets in einem Release.
    pub manifest_asset_name: String,
    /// Öffentlicher Minisign-Schlüssel (Base64-Zeile).
    pub public_key: String,
    /// Plattform-Schlüssel; `None` = automatisch.
    pub platform: Option<String>,
    /// Nur für Tests: `http` auf Loopback erlauben.
    pub allow_loopback_http: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            stable_manifest_url:
                "https://github.com/lorenzhdr/LenzFlix-Desktop/releases/latest/download/latest.json"
                    .into(),
            releases_api_url:
                "https://api.github.com/repos/lorenzhdr/LenzFlix-Desktop/releases?per_page=10"
                    .into(),
            manifest_asset_name: "latest.json".into(),
            public_key: UPDATE_PUBLIC_KEY.into(),
            platform: None,
            allow_loopback_http: false,
        }
    }
}

/// Ein verfügbares Update für diese Plattform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Neue Version.
    pub version: String,
    /// Änderungshinweise.
    pub notes: String,
    /// Veröffentlichungsdatum (Anzeige).
    pub pub_date: Option<String>,
    /// Paket für diese Plattform.
    pub asset: Asset,
}

#[derive(Deserialize)]
struct Release {
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

/// Der Updater.
pub struct Updater {
    cfg: UpdateConfig,
    http: reqwest::Client,
}

impl Updater {
    /// Neuer Updater; `app_version` geht nur in den User-Agent.
    pub fn new(cfg: UpdateConfig, app_version: &str) -> Result<Self, UpdateError> {
        let http = reqwest::Client::builder()
            .user_agent(format!("LENZFLIX-Desktop/{app_version}"))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self { cfg, http })
    }

    fn platform(&self) -> Result<String, UpdateError> {
        match (&self.cfg.platform, current_platform()) {
            (Some(p), _) => Ok(p.clone()),
            (None, Some(p)) => Ok(p.to_string()),
            (None, None) => Err(UpdateError::NoAssetForPlatform("unbekannt".into())),
        }
    }

    fn check_url(&self, url: &str) -> Result<(), UpdateError> {
        url_allowed(url, self.cfg.allow_loopback_http)
    }

    async fn get_limited(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        self.check_url(url)?;
        let mut resp = self
            .http
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(UpdateError::Status(resp.status().as_u16()));
        }
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if buf.len() + chunk.len() > MAX_JSON_BYTES {
                return Err(UpdateError::Manifest("Antwort zu groß".into()));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }

    /// Ermittelt die Manifest-URL des Kanals.
    async fn manifest_url(&self, channel: Channel) -> Result<String, UpdateError> {
        match channel {
            Channel::Stable => Ok(self.cfg.stable_manifest_url.clone()),
            Channel::Prerelease => {
                let body = self.get_limited(&self.cfg.releases_api_url).await?;
                let releases: Vec<Release> = serde_json::from_slice(&body)
                    .map_err(|e| UpdateError::Manifest(e.to_string()))?;
                // Die API liefert das neueste Release zuerst (Entwürfe sind für Anonyme unsichtbar).
                releases
                    .into_iter()
                    .filter(|r| !r.draft)
                    .find_map(|r| {
                        r.assets
                            .into_iter()
                            .find(|a| a.name == self.cfg.manifest_asset_name)
                    })
                    .map(|a| a.browser_download_url)
                    .ok_or_else(|| UpdateError::Manifest("kein Release mit Manifest".into()))
            }
        }
    }

    /// Prüft auf ein neueres Update. `None` = aktuell. Auf dem stabilen Kanal werden
    /// Vorabversionen ignoriert; es gibt nie ein Downgrade.
    pub async fn check(
        &self,
        current_version: &str,
        channel: Channel,
    ) -> Result<Option<UpdateInfo>, UpdateError> {
        let url = self.manifest_url(channel).await?;
        let body = self.get_limited(&url).await?;
        let m = Manifest::parse(&body)?;
        if channel == Channel::Stable && crate::version::is_prerelease(&m.version) {
            return Ok(None);
        }
        if !crate::version::is_newer(current_version, &m.version)? {
            return Ok(None);
        }
        let asset = m.asset_for(&self.platform()?)?.clone();
        self.check_url(&asset.url)?;
        Ok(Some(UpdateInfo {
            version: m.version,
            notes: m.notes,
            pub_date: m.pub_date,
            asset,
        }))
    }

    /// Lädt das Paket nach `dest_dir`, prüft Größe, SHA-256 UND Minisign-Signatur und liefert den
    /// Pfad der fertigen Datei. Bei jedem Fehler wird die Teildatei gelöscht.
    /// `progress(geladen, gesamt)` wird nach jedem Datenblock aufgerufen.
    pub async fn download(
        &self,
        info: &UpdateInfo,
        dest_dir: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<PathBuf, UpdateError> {
        if is_placeholder(&self.cfg.public_key) {
            return Err(UpdateError::KeyNotConfigured);
        }
        tokio::fs::create_dir_all(dest_dir).await?;
        let file_name = file_name_from_url(&info.asset.url);
        let final_path = dest_dir.join(&file_name);
        let part_path = dest_dir.join(format!("{file_name}.part"));
        let result = self
            .download_inner(&info.asset, &part_path, &mut progress)
            .await;
        match result {
            Ok(()) => {
                tokio::fs::rename(&part_path, &final_path).await?;
                Ok(final_path)
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                tracing::warn!("Update-Download verworfen: {e}");
                Err(e)
            }
        }
    }

    async fn download_inner(
        &self,
        asset: &Asset,
        part: &Path,
        progress: &mut impl FnMut(u64, u64),
    ) -> Result<(), UpdateError> {
        self.check_url(&asset.url)?;
        if asset.size == 0 || asset.size > MAX_ASSET_BYTES {
            return Err(UpdateError::SizeMismatch);
        }
        let mut resp = self.http.get(&asset.url).send().await?;
        if !resp.status().is_success() {
            return Err(UpdateError::Status(resp.status().as_u16()));
        }
        if resp.content_length().is_some_and(|l| l != asset.size) {
            return Err(UpdateError::SizeMismatch);
        }
        let mut file = tokio::fs::File::create(part).await?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(30), resp.chunk())
                .await
                .map_err(|_| UpdateError::Http("Zeitüberschreitung beim Download".into()))??;
            let Some(chunk) = chunk else { break };
            total += chunk.len() as u64;
            if total > asset.size {
                return Err(UpdateError::SizeMismatch);
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            progress(total, asset.size);
        }
        file.flush().await?;
        drop(file);
        if total != asset.size {
            return Err(UpdateError::SizeMismatch);
        }
        if !hex_lower(&hasher.finalize()).eq_ignore_ascii_case(&asset.sha256) {
            return Err(UpdateError::HashMismatch);
        }
        let data = tokio::fs::read(part).await?;
        verify_signature(&self.cfg.public_key, &data, &asset.signature)
    }
}

/// Erlaubt `https`; `http` nur für Loopback und nur wenn ausdrücklich erlaubt (Tests).
fn url_allowed(url: &str, allow_loopback_http: bool) -> Result<(), UpdateError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if allow_loopback_http
        && ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
            .iter()
            .any(|p| url.starts_with(p))
    {
        return Ok(());
    }
    Err(UpdateError::InsecureUrl)
}

/// Dateiname aus der URL; nur harmlose Zeichen, nie Pfadanteile.
fn file_name_from_url(url: &str) -> String {
    let last = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");
    let clean: String = last
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let clean = clean.trim_start_matches('.').to_string();
    if clean.is_empty() {
        "lenzflix-update.bin".into()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::{sha256_hex, testkeys::TestKeys};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimaler HTTP-Testserver: Pfad -> Inhalt, optional Umleitung.
    async fn serve(routes: HashMap<String, Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut n = 0;
                    while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        let r = s.read(&mut buf[n..]).await.unwrap_or(0);
                        if r == 0 {
                            return;
                        }
                        n += r;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let resp = match routes.get(&path) {
                        Some(b) if b.starts_with(b"REDIRECT:") => format!(
                            "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            String::from_utf8_lossy(&b[9..])
                        )
                        .into_bytes(),
                        Some(b) => {
                            let mut r = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                b.len()
                            )
                            .into_bytes();
                            r.extend_from_slice(b);
                            r
                        }
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = s.write_all(&resp).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    struct Fixture {
        base: String,
        keys: TestKeys,
        payload: Vec<u8>,
    }

    /// Baut Server mit Paket, Manifest (Version `ver`) und optional manipuliertem Paket.
    async fn fixture(ver: &str, tamper: bool, pre_api: bool) -> Fixture {
        let keys = TestKeys::new(7);
        let payload: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let mut served = payload.clone();
        if tamper {
            served[1234] ^= 0xff;
        }
        // Basis-URL ist erst nach dem Start bekannt; Manifest verweist daher relativ per Zweitstart.
        let mut routes = HashMap::new();
        routes.insert("/pkg/setup.exe".to_string(), served);
        let base0 = serve(routes.clone()).await;
        let manifest = |base: &str| {
            serde_json::json!({
                "version": ver, "notes": "Testnotiz",
                "assets": { "windows-x86_64": {
                    "url": format!("{base}/pkg/setup.exe"), "size": payload.len(),
                    "sha256": sha256_hex(&payload), "signature": keys.sign(&payload)
                }}
            })
            .to_string()
            .into_bytes()
        };
        // Zweiter Server kennt Manifest, das auf den ersten (Paket) zeigt.
        routes.insert("/latest.json".to_string(), manifest(&base0));
        routes.insert(
            "/latest-redirect.json".to_string(),
            b"REDIRECT:/latest.json".to_vec(),
        );
        if pre_api {
            routes.insert("/releases".to_string(), b"[]".to_vec());
        }
        let base = serve(routes).await;
        Fixture {
            base,
            keys,
            payload,
        }
    }

    fn updater(f: &Fixture, manifest_path: &str) -> Updater {
        let cfg = UpdateConfig {
            stable_manifest_url: format!("{}{manifest_path}", f.base),
            releases_api_url: format!("{}/releases", f.base),
            public_key: f.keys.public_b64(),
            platform: Some("windows-x86_64".into()),
            allow_loopback_http: true,
            ..UpdateConfig::default()
        };
        Updater::new(cfg, "0.1.0").unwrap()
    }

    #[tokio::test]
    async fn check_findet_neuere_version_auch_ueber_umleitung() {
        let f = fixture("0.2.0", false, false).await;
        let up = updater(&f, "/latest-redirect.json");
        let info = up.check("0.1.0", Channel::Stable).await.unwrap().unwrap();
        assert_eq!(info.version, "0.2.0");
        assert_eq!(info.notes, "Testnotiz");
        assert!(up.check("0.2.0", Channel::Stable).await.unwrap().is_none());
        assert!(up.check("0.3.0", Channel::Stable).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn stabiler_kanal_ignoriert_vorabversion() {
        let f = fixture("0.3.0-beta.1", false, false).await;
        let up = updater(&f, "/latest.json");
        assert!(up.check("0.1.0", Channel::Stable).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn vorab_kanal_ohne_release_ist_fehler() {
        let f = fixture("0.3.0-beta.1", false, true).await;
        let up = updater(&f, "/latest.json");
        assert!(up.check("0.1.0", Channel::Prerelease).await.is_err());
    }

    #[tokio::test]
    async fn download_mit_fortschritt_und_verifikation() {
        let f = fixture("0.2.0", false, false).await;
        let up = updater(&f, "/latest.json");
        let info = up.check("0.1.0", Channel::Stable).await.unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut last = (0, 0);
        let mut calls = 0;
        let path = up
            .download(&info, dir.path(), |d, t| {
                last = (d, t);
                calls += 1;
            })
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), f.payload);
        assert_eq!(last, (f.payload.len() as u64, f.payload.len() as u64));
        assert!(calls >= 1);
        assert!(!dir.path().join("setup.exe.part").exists());
    }

    #[tokio::test]
    async fn manipuliertes_paket_wird_verworfen() {
        let f = fixture("0.2.0", true, false).await;
        let up = updater(&f, "/latest.json");
        let info = up.check("0.1.0", Channel::Stable).await.unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let e = up.download(&info, dir.path(), |_, _| {}).await.unwrap_err();
        assert!(matches!(e, UpdateError::HashMismatch));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn gueltiger_hash_aber_falsche_signatur_wird_verworfen() {
        let f = fixture("0.2.0", false, false).await;
        let mut up = updater(&f, "/latest.json");
        let info = up.check("0.1.0", Channel::Stable).await.unwrap().unwrap();
        up.cfg.public_key = TestKeys::new(9).public_b64(); // anderer Schlüssel
        let dir = tempfile::tempdir().unwrap();
        let e = up.download(&info, dir.path(), |_, _| {}).await.unwrap_err();
        assert!(matches!(e, UpdateError::Signature(_)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn platzhalterschluessel_blockiert_download() {
        let f = fixture("0.2.0", false, false).await;
        let mut up = updater(&f, "/latest.json");
        let info = up.check("0.1.0", Channel::Stable).await.unwrap().unwrap();
        up.cfg.public_key = UPDATE_PUBLIC_KEY.into();
        let dir = tempfile::tempdir().unwrap();
        let e = up.download(&info, dir.path(), |_, _| {}).await.unwrap_err();
        assert!(matches!(e, UpdateError::KeyNotConfigured));
    }

    #[tokio::test]
    async fn http_ausserhalb_loopback_ist_verboten() {
        assert!(url_allowed("http://example.invalid/x", true).is_err());
        assert!(url_allowed("http://127.0.0.1:1/x", false).is_err());
        assert!(url_allowed("https://example.invalid/x", false).is_ok());
    }

    #[test]
    fn dateiname_ist_bereinigt() {
        assert_eq!(
            file_name_from_url("https://h/a/b/setup.exe?x=1"),
            "setup.exe"
        );
        assert_eq!(
            file_name_from_url("https://h/..%2F..%2Fx"),
            "2F..2Fx".trim_start_matches('.')
        );
        assert_eq!(file_name_from_url("https://h/"), "lenzflix-update.bin");
    }
}
