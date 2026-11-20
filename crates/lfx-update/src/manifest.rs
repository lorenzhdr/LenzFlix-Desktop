//! Update-Manifest `latest.json` (Release-Asset).
//!
//! Entscheidung: statische `latest.json` als Release-Asset statt Auswertung der Release-API allein.
//! Grund: Das Manifest trägt SHA-256 und Minisign-Signatur je Plattform, ist ein einziger kleiner
//! Abruf (Stabil: `releases/latest/download/latest.json`, ohne API-Ratenlimit) und für Menschen
//! lesbar. Nur für den Vorab-Kanal wird die Release-API gefragt, um das neueste (auch Vorab-)Release
//! zu finden.
//!
//! Schema (Beispiel mit Platzhalterwerten):
//! ```json
//! {
//!   "version": "0.2.0",
//!   "notes": "Änderungen",
//!   "pub_date": "2026-12-01T10:00:00Z",
//!   "assets": {
//!     "windows-x86_64": { "url": "https://…/x64-setup.exe", "size": 1234,
//!                          "sha256": "<hex>", "signature": "<Inhalt der .minisig-Datei>" },
//!     "windows-aarch64": { "…": "…" },
//!     "macos-aarch64":   { "url": "https://…/aarch64.dmg", "…": "…" }
//!   }
//! }
//! ```

use crate::error::UpdateError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Obergrenze für die Paketgröße (Schutz vor Endlos-Downloads): 1 GiB.
pub const MAX_ASSET_BYTES: u64 = 1024 * 1024 * 1024;

/// Ein Paket für eine Plattform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Download-URL (https).
    pub url: String,
    /// Größe in Bytes.
    pub size: u64,
    /// SHA-256 als Hex (Groß-/Kleinschreibung egal).
    pub sha256: String,
    /// Minisign-Signatur (kompletter Text der `.minisig`-Datei).
    pub signature: String,
}

/// Das Manifest eines Releases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// SemVer-Version des Releases.
    pub version: String,
    /// Änderungshinweise (Text).
    #[serde(default)]
    pub notes: String,
    /// Veröffentlichungsdatum (ISO 8601), nur Anzeige.
    #[serde(default)]
    pub pub_date: Option<String>,
    /// Pakete je Plattform-Schlüssel (`windows-x86_64`, `windows-aarch64`, `macos-aarch64`).
    pub assets: BTreeMap<String, Asset>,
}

impl Manifest {
    /// Parst und validiert (Version, Größen, Hash-Länge).
    pub fn parse(bytes: &[u8]) -> Result<Self, UpdateError> {
        let m: Manifest =
            serde_json::from_slice(bytes).map_err(|e| UpdateError::Manifest(e.to_string()))?;
        crate::version::parse_version(&m.version)?;
        for (k, a) in &m.assets {
            let hex_ok = a.sha256.len() == 64 && a.sha256.bytes().all(|b| b.is_ascii_hexdigit());
            if !hex_ok {
                return Err(UpdateError::Manifest(format!("sha256 von {k} ungültig")));
            }
            if a.size == 0 || a.size > MAX_ASSET_BYTES {
                return Err(UpdateError::Manifest(format!("Größe von {k} unzulässig")));
            }
            if a.signature.trim().is_empty() {
                return Err(UpdateError::Manifest(format!("Signatur von {k} fehlt")));
            }
        }
        Ok(m)
    }

    /// Paket für den Plattform-Schlüssel.
    pub fn asset_for(&self, platform: &str) -> Result<&Asset, UpdateError> {
        self.assets
            .get(platform)
            .ok_or_else(|| UpdateError::NoAssetForPlatform(platform.to_string()))
    }
}

/// Plattform-Schlüssel der laufenden Anwendung, `None` auf nicht unterstützten Systemen.
pub fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some("windows-aarch64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(hash: &str) -> String {
        format!(
            r#"{{"version":"0.2.0","notes":"x","assets":{{"windows-x86_64":
            {{"url":"https://example.invalid/a.exe","size":10,"sha256":"{hash}","signature":"sig"}}}}}}"#
        )
    }

    #[test]
    fn parst_gueltiges_manifest() {
        let m = Manifest::parse(sample(&"a".repeat(64)).as_bytes()).unwrap();
        assert_eq!(m.version, "0.2.0");
        assert!(m.asset_for("windows-x86_64").is_ok());
        assert!(matches!(
            m.asset_for("macos-aarch64"),
            Err(UpdateError::NoAssetForPlatform(_))
        ));
    }

    #[test]
    fn lehnt_kaputte_hashes_und_json_ab() {
        assert!(Manifest::parse(sample("zz").as_bytes()).is_err());
        assert!(Manifest::parse(b"{}").is_err());
        assert!(Manifest::parse(b"kein json").is_err());
    }
}
