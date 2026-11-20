//! W5-tray: bettet unter Windows Symbol und Versionsangaben in `lenzflix.exe` ein (Taskleiste, Alt+Tab,
//! Startmenü). Auf anderen Zielen passiert nichts (macOS bekommt das Symbol über das App-Bündel).
//! Geprüft wird das Zielsystem, nicht das Build-System, damit auch Quer-Builds stimmen.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/app/lenzflix.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app/lenzflix.ico")
        .set("ProductName", "LENZFLIX")
        .set("FileDescription", "LENZFLIX Desktop")
        .set("InternalName", "lenzflix")
        .set("OriginalFilename", "lenzflix.exe")
        .set("LegalCopyright", "LENZFLIX Desktop, MIT-Lizenz")
        .set("ProductVersion", &version)
        .set("FileVersion", &version);
    if let Err(e) = res.compile() {
        // Ohne Ressourcen-Compiler (rc.exe) läuft die App trotzdem, nur ohne Exe-Symbol.
        println!("cargo:warning=Windows-Ressourcen nicht eingebettet: {e}");
    }
}
