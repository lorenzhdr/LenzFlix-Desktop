//! `lenzflix`: startet LENZFLIX Desktop.

use std::process::ExitCode;

fn main() -> ExitCode {
    let opts = match lfx_app::cli::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Fehler: {e}");
            eprintln!(
                "Aufruf: lenzflix [--home <Ordner>] [--server <Adresse>] [--design flix|plus|prime|retro] [--keyring]"
            );
            return ExitCode::from(2);
        }
    };
    match lfx_app::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::from(1)
        }
    }
}
