//! Startet den Mock-Server von Hand: `cargo run -p lfx-mock-jellyfin` (freier Port auf 127.0.0.1).

use lfx_mock_jellyfin::{DEMO_PASSWORD, DEMO_USER, MockConfig, start_with};

#[tokio::main]
async fn main() {
    let (addr, handle) = start_with(MockConfig::default()).await;
    println!("Mock-Server läuft auf http://{addr}");
    println!("Testkonto: {DEMO_USER} / {DEMO_PASSWORD}");
    let _ = tokio::signal::ctrl_c().await;
    handle.shutdown().await;
}
