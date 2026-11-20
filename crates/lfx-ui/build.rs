//! Kompiliert die Slint-Oberfläche (`ui/lib.slint`).
//! Bilder (Icons) und Schriften werden ins Binary eingebettet, damit zur Laufzeit keine Pfade
//! der Build-Maschine nötig sind (Standard wäre der absolute Pfad).
//!
//! Der Token-Generator (`tools/tokens/extract.mjs`) schreibt `pure function` ohne `public`; Slint
//! meldet den Zugriff von außen als (künftigen) Fehler. Bis der Generator `pure public function`
//! ausgibt, patcht dieses Skript eine Kopie nach `OUT_DIR/lfx-tokens.slint`, die per Include-Pfad
//! gefunden wird. `ui/theme/tokens.slint` bleibt unverändert.

use slint_build::{CompilerConfiguration, EmbedResourcesKind};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../../assets/icons");
    println!("cargo:rerun-if-changed=../../assets/fonts");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let tokens = fs::read_to_string("ui/theme/tokens.slint").expect("tokens.slint lesen");
    let patched = tokens.replace("    pure function ", "    pure public function ");
    fs::write(out.join("lfx-tokens.slint"), patched).expect("lfx-tokens.slint schreiben");
    let config = CompilerConfiguration::new()
        .with_include_paths(vec![out])
        .embed_resources(EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/lib.slint", config)
        .expect("Slint-Kompilierung fehlgeschlagen");
}
