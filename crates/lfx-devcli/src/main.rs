//! `lfx-dev`: Anmeldung und Live-Prüflauf für die Entwicklung von LENZFLIX Desktop.

use clap::{Parser, Subcommand};
use lfx_devcli::{Ctx, commands, g4};
use lfx_platform::{AppDirs, os_secret_store};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "lfx-dev",
    version,
    about = "Entwickler-Werkzeug für LENZFLIX Desktop"
)]
struct Cli {
    /// Alle App-Ordner unter diesem Ordner anlegen (sonst OS-Standard bzw. `LFX_HOME`).
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Anmelden per Quick Connect (Code wird angezeigt, Bestätigung im Jellyfin-Web).
    Login {
        /// Adresse des Servers.
        #[arg(long)]
        server: String,
    },
    /// Zeigt das angemeldete Konto (fragt den Server).
    Whoami,
    /// Listet gespeicherte Server und Benutzer.
    Servers,
    /// Meldet das aktive Konto ab und löscht das Token.
    Logout,
    /// Sucht Server im lokalen Netz.
    Discover {
        /// Suchdauer in Sekunden.
        #[arg(long, default_value_t = 3)]
        seconds: u64,
    },
    /// Live-Prüflauf (Gate G4); Exit-Code ungleich 0 bei FAIL.
    G4,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let dirs = match cli.home {
        Some(root) => Some(AppDirs::from_root(root)),
        None => AppDirs::resolve(),
    };
    let Some(dirs) = dirs else {
        eprintln!("Fehler: kein Konfigurationsordner ermittelbar");
        return ExitCode::from(2);
    };
    let secrets = match os_secret_store() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Fehler: {e}");
            return ExitCode::from(2);
        }
    };
    let ctx = Ctx::new(dirs, secrets);
    let mut out = std::io::stdout();
    let result = match cli.command {
        Command::Login { server } => commands::login(&ctx, &server, &mut out)
            .await
            .map(|()| true),
        Command::Whoami => commands::whoami(&ctx, &mut out).await.map(|()| true),
        Command::Servers => commands::servers(&ctx, &mut out).map(|()| true),
        Command::Logout => commands::logout(&ctx, &mut out).await.map(|()| true),
        Command::Discover { seconds } => commands::discover(seconds, &mut out).await.map(|()| true),
        Command::G4 => g4::run(&ctx, &mut out).await.map(|s| s.passed()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("Fehler: {e:#}");
            ExitCode::from(1)
        }
    }
}
