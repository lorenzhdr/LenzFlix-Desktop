//! Build-Hilfsbefehle für LENZFLIX Desktop.
//!
//! Aufruf: `cargo run -p xtask -- <befehl>`.
//! Befehle: `fetch-mpv`, `mpv-info`.

mod mpv;

use std::process::ExitCode;

const USAGE: &str = "Verwendung: xtask <befehl>

  fetch-mpv [--target <triple>]   libmpv (LGPL) laden und nach vendor/mpv/<triple>/ entpacken
  mpv-info  [--target <triple>]   Exporte und API-Version der libmpv prüfen

Ziele: aarch64-pc-windows-msvc, x86_64-pc-windows-msvc, aarch64-apple-darwin";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("fetch-mpv") => mpv::fetch(&args[1..]),
        Some("mpv-info") => mpv::info(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}
