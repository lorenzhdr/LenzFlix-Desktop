//! Windows erkennt Programmnamen mit „update“ als Installer und verlangt ohne Manifest erhöhte Rechte
//! (Fehler 740 beim Test-Start). Für die Test-Binaries wird deshalb ein `asInvoker`-Manifest eingebettet.

use std::{env, fs, path::PathBuf};

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let tenv = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" && tenv == "msvc" {
        let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        let path = out.join("asinvoker.manifest");
        fs::write(&path, MANIFEST).expect("Manifest schreiben");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", path.display());
    }
}
