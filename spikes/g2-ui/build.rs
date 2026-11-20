//! Kompiliert die Spike-Oberfläche; Theme (`lib.slint`) kommt aus `crates/lfx-ui/ui`.

fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../../crates/lfx-ui/ui");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![std::path::PathBuf::from("../../crates/lfx-ui/ui")]);
    slint_build::compile_with_config("ui/main.slint", config)
        .expect("Slint-Kompilierung fehlgeschlagen");
}
