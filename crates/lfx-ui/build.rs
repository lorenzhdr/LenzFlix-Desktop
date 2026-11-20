//! Kompiliert die Slint-Oberfläche (`ui/lib.slint`).

fn main() {
    println!("cargo:rerun-if-changed=ui");
    slint_build::compile("ui/lib.slint").expect("Slint-Kompilierung fehlgeschlagen");
}
