//! Release-Hilfen: Minisign-Signaturen, SHA-256-Summen und `latest.json` für den Auto-Updater.
//!
//! Schema von `latest.json`: der Typ `lfx_update::Manifest` (ein gemeinsamer Vertrag mit dem Update-Client):
//! ```json
//! { "version": "0.1.0", "notes": "...", "pub_date": "2026-11-20T12:00:00Z",
//!   "assets": { "windows-aarch64": { "url": "...", "size": 123, "sha256": "<hex>",
//!                                    "signature": "<Inhalt der .minisig-Datei>" } } }
//! ```
//! Plattform-Schlüssel (`windows-x86_64`, `windows-aarch64`, `macos-aarch64`) kommen aus dem Ziel-Triple des
//! Dateinamens über `lfx_update::platform_for_target`. Ohne Signaturen (kein Schlüssel in der Pipeline) schreibt
//! `manifest` nur `SHA256SUMS.txt` und keine `latest.json`, weil der Update-Client unsignierte Manifeste ablehnt.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lfx_update::{Asset, Manifest};
use sha2::{Digest, Sha256};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Release-Dateien (Installer) in einem Ordner: `(Dateiname, Ziel-Triple)`.
fn release_files(dir: &Path) -> Res<Vec<(String, &'static str)>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if let Some(t) = target_of(&name) {
            out.push((name, t));
        }
    }
    out.sort();
    Ok(out)
}

/// Ziel-Triple aus dem Dateinamen (`LENZFLIX_<v>_<arch>-setup.exe` bzw. `.dmg`).
pub fn target_of(name: &str) -> Option<&'static str> {
    if !name.starts_with("LENZFLIX_") {
        return None;
    }
    if name.ends_with("_arm64-setup.exe") {
        Some("aarch64-pc-windows-msvc")
    } else if name.ends_with("_x64-setup.exe") {
        Some("x86_64-pc-windows-msvc")
    } else if name.ends_with("_arm64.dmg") {
        Some("aarch64-apple-darwin")
    } else {
        None
    }
}

fn sha256_hex(path: &Path) -> Res<String> {
    let mut h = Sha256::new();
    h.update(fs::read(path)?);
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// `manifest --dir <ordner> --tag <vX.Y.Z> --base-url <url> [--notes-file <datei>] [--pub-date <iso>]`
/// schreibt `SHA256SUMS.txt` und `latest.json` in den Ordner.
pub fn manifest(args: &[String]) -> Res<()> {
    let dir = PathBuf::from(opt(args, "--dir").ok_or("--dir fehlt")?);
    let tag = opt(args, "--tag").ok_or("--tag fehlt")?;
    let base = opt(args, "--base-url").ok_or("--base-url fehlt")?;
    let notes = match opt(args, "--notes-file") {
        Some(f) => fs::read_to_string(f)?,
        None => String::new(),
    };
    let pub_date = opt(args, "--pub-date").unwrap_or_default();
    let doc = build_manifest(&dir, &tag, &base, &notes, &pub_date)?;
    if doc.assets.values().any(|a| a.signature.trim().is_empty()) {
        eprintln!(
            "WARNUNG: Es fehlen Signaturen (.minisig). latest.json wird NICHT geschrieben,              der Update-Client lehnt unsignierte Manifeste ab. SHA256SUMS.txt liegt in {}",
            dir.display()
        );
        return Ok(());
    }
    let json = serde_json::to_string_pretty(&doc)?
        + "
";
    // Gegenprobe: genau das liest der Update-Client später.
    Manifest::parse(json.as_bytes())?;
    fs::write(dir.join("latest.json"), json)?;
    eprintln!(
        "latest.json und SHA256SUMS.txt in {} geschrieben",
        dir.display()
    );
    Ok(())
}

/// Baut das Manifest aus den Release-Dateien im Ordner und schreibt `SHA256SUMS.txt`.
/// Fehlende Signaturen bleiben leer (der Aufrufer entscheidet).
pub fn build_manifest(
    dir: &Path,
    tag: &str,
    base_url: &str,
    notes: &str,
    pub_date: &str,
) -> Res<Manifest> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let files = release_files(dir)?;
    if files.is_empty() {
        return Err("Keine Release-Dateien im Ordner".into());
    }
    let mut sums = String::new();
    let mut assets = BTreeMap::new();
    for (name, target) in &files {
        let path = dir.join(name);
        let sha = sha256_hex(&path)?;
        sums.push_str(&format!(
            "{sha}  {name}
"
        ));
        let sig_path = dir.join(format!("{name}.minisig"));
        let signature = fs::read_to_string(&sig_path).unwrap_or_default();
        let key = lfx_update::platform_for_target(target)
            .ok_or_else(|| format!("kein Plattform-Schlüssel für {target}"))?;
        let asset = Asset {
            url: format!("{}/{tag}/{name}", base_url.trim_end_matches('/')),
            size: fs::metadata(&path)?.len(),
            sha256: sha,
            signature,
        };
        if assets.insert(key.to_owned(), asset).is_some() {
            return Err(format!("mehrere Pakete für {key}").into());
        }
    }
    fs::write(dir.join("SHA256SUMS.txt"), sums)?;
    Ok(Manifest {
        version: version.to_owned(),
        notes: notes.trim().to_owned(),
        pub_date: (!pub_date.is_empty()).then(|| pub_date.to_owned()),
        assets,
    })
}

/// `sign --dir <ordner>`: signiert alle Release-Dateien mit dem Schlüssel aus `MINISIGN_SECRET_KEY`
/// (Inhalt der Schlüsseldatei) und `MINISIGN_PASSWORD`. Ohne Schlüssel wird übersprungen.
pub fn sign(args: &[String]) -> Res<()> {
    let dir = PathBuf::from(opt(args, "--dir").ok_or("--dir fehlt")?);
    let Some(key) = std::env::var("MINISIGN_SECRET_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
    else {
        eprintln!("MINISIGN_SECRET_KEY nicht gesetzt: Signieren übersprungen");
        return Ok(());
    };
    let password = std::env::var("MINISIGN_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty());
    for (name, _) in release_files(&dir)? {
        let sig = sign_file(&dir.join(&name), &key, password.clone())?;
        fs::write(dir.join(format!("{name}.minisig")), sig)?;
        eprintln!("signiert: {name}");
    }
    Ok(())
}

/// Signatur-Datei (Text) für eine Datei erzeugen.
pub fn sign_file(path: &Path, secret_key: &str, password: Option<String>) -> Res<String> {
    let sk = minisign::SecretKeyBox::from_string(secret_key)?.into_secret_key(password)?;
    let data = fs::File::open(path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let sig = minisign::sign(
        None,
        &sk,
        data,
        Some(&format!("file:{}", name.unwrap_or_default())),
        Some("LENZFLIX Desktop"),
    )?;
    Ok(sig.into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_detection() {
        assert_eq!(
            target_of("LENZFLIX_0.1.0_arm64-setup.exe"),
            Some("aarch64-pc-windows-msvc")
        );
        assert_eq!(
            target_of("LENZFLIX_0.1.0_x64-setup.exe"),
            Some("x86_64-pc-windows-msvc")
        );
        assert_eq!(
            target_of("LENZFLIX_0.1.0_arm64.dmg"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(target_of("latest.json"), None);
        assert_eq!(target_of("LENZFLIX_0.1.0_arm64-setup.exe.minisig"), None);
    }

    #[test]
    fn sign_verify_and_manifest() {
        let dir = std::env::temp_dir().join(format!("lfx-xtask-rel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("LENZFLIX_0.1.0_x64-setup.exe");
        fs::write(&file, b"Testinhalt").unwrap();

        let kp = minisign::KeyPair::generate_encrypted_keypair(Some("testpw".into())).unwrap();
        let key_text = kp.sk.to_box(None).unwrap().into_string();
        let sig_text = sign_file(&file, &key_text, Some("testpw".into())).unwrap();
        fs::write(dir.join("LENZFLIX_0.1.0_x64-setup.exe.minisig"), &sig_text).unwrap();
        let sig_box = minisign::SignatureBox::from_string(&sig_text).unwrap();
        minisign::verify(
            &kp.pk,
            &sig_box,
            std::io::Cursor::new(b"Testinhalt"),
            true,
            false,
            false,
        )
        .unwrap();

        let doc =
            build_manifest(&dir, "v0.1.0", "https://example.invalid/dl/", "Notizen", "").unwrap();
        assert_eq!(doc.version, "0.1.0");
        let a = &doc.assets["windows-x86_64"];
        assert_eq!(a.size, 10);
        assert_eq!(
            a.url,
            "https://example.invalid/dl/v0.1.0/LENZFLIX_0.1.0_x64-setup.exe"
        );
        assert_eq!(a.sha256.len(), 64);
        assert!(a.signature.contains("untrusted comment"));

        // Vertrag mit dem Update-Client: das serialisierte Manifest parst dort und trägt eine Signatur,
        // die dessen Prüfung (minisign-verify) mit dem öffentlichen Schlüssel besteht.
        let json = serde_json::to_string_pretty(&doc).unwrap();
        let parsed = lfx_update::Manifest::parse(json.as_bytes()).unwrap();
        let asset = parsed.asset_for("windows-x86_64").unwrap();
        assert_eq!(asset.sha256, a.sha256);
        let pk_b64 = kp.pk.to_base64();
        lfx_update::verify::verify_signature(&pk_b64, b"Testinhalt", &asset.signature).unwrap();
        assert!(parsed.asset_for("macos-aarch64").is_err());
        assert!(!json.contains("\"target\""));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ohne_signatur_kein_latest_json() {
        let dir = std::env::temp_dir().join(format!("lfx-xtask-unsigned-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LENZFLIX_0.1.0_arm64-setup.exe"), b"Testinhalt").unwrap();
        let args: Vec<String> = [
            "--dir",
            &dir.to_string_lossy(),
            "--tag",
            "v0.1.0",
            "--base-url",
            "https://example.invalid/dl",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        manifest(&args).unwrap();
        assert!(dir.join("SHA256SUMS.txt").is_file());
        assert!(!dir.join("latest.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
