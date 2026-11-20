//! libmpv holen (gepinnt, SHA-256-geprüft) und prüfen.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const PINS: &str = include_str!("../mpv-pins.toml");

#[derive(Deserialize)]
struct Pins {
    targets: BTreeMap<String, Pin>,
}

#[derive(Deserialize)]
struct Pin {
    url: String,
    file: String,
    sha256: String,
    kind: String,
}

/// Symbole, die der Client mindestens braucht.
const REQUIRED_SYMBOLS: &[&str] = &[
    "mpv_create",
    "mpv_initialize",
    "mpv_client_api_version",
    "mpv_render_context_create",
    "mpv_render_context_render",
];

fn workspace_root() -> PathBuf {
    // Vom Arbeitsverzeichnis aufwärts die Cargo.toml mit [workspace] suchen;
    // Rückfall: das Elternverzeichnis von xtask.
    if let Ok(mut dir) = std::env::current_dir() {
        loop {
            if fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
                return dir;
            }
            if !dir.pop() {
                break;
            }
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask hat ein Elternverzeichnis")
        .to_path_buf()
}

fn host_target() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else {
        "aarch64-apple-darwin"
    }
}

fn parse_target(args: &[String]) -> Res<String> {
    let mut target = host_target().to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--target" => target = it.next().ok_or("--target braucht einen Wert")?.clone(),
            other => return Err(format!("Unbekanntes Argument: {other}").into()),
        }
    }
    Ok(target)
}

fn pin_for(target: &str) -> Res<Pin> {
    let pins: Pins = toml::from_str(PINS)?;
    pins.targets
        .into_iter()
        .find(|(k, _)| k == target)
        .map(|(_, v)| v)
        .ok_or_else(|| format!("Kein Pin für Ziel {target}").into())
}

fn vendor_dir(target: &str) -> PathBuf {
    workspace_root().join("vendor").join("mpv").join(target)
}

fn sha256_file(path: &Path) -> Res<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn download(url: &str, dest: &Path) -> Res<()> {
    eprintln!("Lade {url}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into();
    let mut resp = agent.get(url).call()?;
    let mut out = fs::File::create(dest)?;
    std::io::copy(&mut resp.body_mut().as_reader(), &mut out)?;
    Ok(())
}

pub fn fetch(args: &[String]) -> Res<()> {
    let target = parse_target(args)?;
    let pin = pin_for(&target)?;
    let out_dir = vendor_dir(&target);
    let cache = workspace_root().join("vendor").join("mpv").join(".cache");
    fs::create_dir_all(&cache)?;
    let archive = cache.join(&pin.file);

    let needs_download = !archive.exists() || sha256_file(&archive)? != pin.sha256;
    if needs_download {
        download(&pin.url, &archive)?;
    }
    let got = sha256_file(&archive)?;
    if got != pin.sha256 {
        let _ = fs::remove_file(&archive);
        return Err(format!(
            "SHA-256 stimmt nicht: erwartet {}, erhalten {got}",
            pin.sha256
        )
        .into());
    }
    eprintln!("SHA-256 ok: {got}");

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::create_dir_all(&out_dir)?;
    match pin.kind.as_str() {
        "7z" => sevenz_rust2::decompress_file(&archive, &out_dir)?,
        "tar.gz" => {
            let f = fs::File::open(&archive)?;
            tar::Archive::new(flate2::read::GzDecoder::new(f)).unpack(&out_dir)?;
        }
        other => return Err(format!("Unbekannte Archivart {other}").into()),
    }
    eprintln!("Entpackt nach {}", out_dir.display());
    list_files(&out_dir, &out_dir)?;
    Ok(())
}

fn list_files(base: &Path, dir: &Path) -> Res<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            list_files(base, &p)?;
        } else {
            let rel = p.strip_prefix(base).unwrap_or(&p);
            println!("{}  ({} B)", rel.display(), e.metadata()?.len());
        }
    }
    Ok(())
}

/// Sucht die libmpv-Bibliothek rekursiv im Vendor-Ordner.
fn find_lib(dir: &Path) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_lib(&p) {
                return Some(f);
            }
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str())
            && matches!(n, "libmpv-2.dll" | "mpv-2.dll" | "libmpv.dylib")
        {
            return Some(p);
        }
    }
    None
}

pub fn info(args: &[String]) -> Res<()> {
    let target = parse_target(args)?;
    let dir = vendor_dir(&target);
    let lib = find_lib(&dir).ok_or_else(|| {
        format!(
            "Keine libmpv in {} gefunden (erst `xtask fetch-mpv`)",
            dir.display()
        )
    })?;
    println!("Bibliothek: {}", lib.display());

    // Exporte per PE-Parser (funktioniert auch für fremde Architekturen).
    if lib.extension().is_some_and(|e| e == "dll") {
        let data = fs::read(&lib)?;
        let pe = goblin::pe::PE::parse(&data)?;
        let names: Vec<&str> = pe.exports.iter().filter_map(|e| e.name).collect();
        println!("Exporte gesamt: {}", names.len());
        for s in REQUIRED_SYMBOLS {
            println!("  {s}: {}", if names.contains(s) { "ja" } else { "FEHLT" });
        }
        let d3d: Vec<_> = names.iter().filter(|n| n.contains("d3d11")).collect();
        println!("  d3d11-Exporte: {d3d:?}");
    }

    // Laden nur, wenn die Architektur zum Host passt.
    if target == host_target() {
        let ver = load_version(&lib)?;
        println!(
            "mpv_client_api_version: {}.{} (0x{ver:08x})",
            ver >> 16,
            ver & 0xffff
        );
    } else {
        println!(
            "Laden übersprungen (Ziel {target} != Host {}).",
            host_target()
        );
    }
    Ok(())
}

fn load_version(lib: &Path) -> Res<u64> {
    #[cfg(windows)]
    let library: libloading::Library = unsafe {
        use libloading::os::windows::{LOAD_WITH_ALTERED_SEARCH_PATH, Library};
        Library::load_with_flags(lib, LOAD_WITH_ALTERED_SEARCH_PATH)?.into()
    };
    #[cfg(not(windows))]
    let library = unsafe { libloading::Library::new(lib)? };
    // SAFETY: Signatur laut client.h: unsigned long mpv_client_api_version(void).
    let ver = unsafe {
        let f: libloading::Symbol<unsafe extern "C" fn() -> std::ffi::c_ulong> =
            library.get(b"mpv_client_api_version\0")?;
        f()
    };
    Ok(u64::from(ver))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_parse_and_are_complete() {
        for t in [
            "aarch64-pc-windows-msvc",
            "x86_64-pc-windows-msvc",
            "aarch64-apple-darwin",
        ] {
            let p = pin_for(t).unwrap();
            assert_eq!(p.sha256.len(), 64);
            assert!(p.url.ends_with(&p.file));
        }
        assert!(pin_for("riscv").is_err());
    }

    #[test]
    fn target_argument() {
        let a = vec!["--target".to_string(), "x86_64-pc-windows-msvc".to_string()];
        assert_eq!(parse_target(&a).unwrap(), "x86_64-pc-windows-msvc");
        assert!(parse_target(&["--x".to_string()]).is_err());
    }
}
