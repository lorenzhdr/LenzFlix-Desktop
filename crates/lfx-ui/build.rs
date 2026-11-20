//! Kompiliert die Slint-Oberfläche (`ui/lib.slint`).
//! Bilder (Icons) und Schriften werden ins Binary eingebettet, damit zur Laufzeit keine Pfade
//! der Build-Maschine nötig sind (Standard wäre der absolute Pfad).

use slint_build::{CompilerConfiguration, EmbedResourcesKind};

fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../../assets/icons");
    println!("cargo:rerun-if-changed=../../assets/fonts");
    let config = CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .embed_resources(EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/lib.slint", config)
        .expect("Slint-Kompilierung fehlgeschlagen");
}
