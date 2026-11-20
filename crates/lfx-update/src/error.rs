//! Fehlertyp des Updaters. Meldungen enthalten nie Tokens; URLs werden nicht ausgegeben.

/// Alle Fehler beim Prüfen, Laden, Verifizieren und Installieren.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Netzwerk- oder HTTP-Fehler.
    #[error("Netzwerkfehler beim Update: {0}")]
    Http(String),
    /// Server antwortete mit einem Fehlerstatus.
    #[error("Update-Server antwortete mit Status {0}")]
    Status(u16),
    /// Manifest ist kein gültiges JSON oder verletzt das Schema.
    #[error("Update-Manifest ungültig: {0}")]
    Manifest(String),
    /// Versionsangabe ist keine gültige SemVer-Version.
    #[error("Ungültige Version: {0}")]
    Version(String),
    /// Für diese Plattform gibt es kein Paket im Manifest.
    #[error("Kein Update-Paket für diese Plattform ({0})")]
    NoAssetForPlatform(String),
    /// URL ist nicht erlaubt (nur https, Ausnahme: Loopback im Test).
    #[error("Unzulässige Update-URL")]
    InsecureUrl,
    /// Paket ist größer als erlaubt oder weicht von der Manifest-Größe ab.
    #[error("Paketgröße stimmt nicht mit dem Manifest überein")]
    SizeMismatch,
    /// SHA-256 stimmt nicht.
    #[error("Prüfsumme (SHA-256) stimmt nicht überein")]
    HashMismatch,
    /// Minisign-Signatur ungültig oder nicht prüfbar.
    #[error("Signaturprüfung fehlgeschlagen: {0}")]
    Signature(String),
    /// Es ist noch kein echter öffentlicher Schlüssel eingebaut.
    #[error("Kein Update-Schlüssel hinterlegt (Platzhalter)")]
    KeyNotConfigured,
    /// Dateifehler.
    #[error("Dateifehler: {0}")]
    Io(#[from] std::io::Error),
    /// Installation nicht möglich oder fehlgeschlagen.
    #[error("Installation nicht möglich: {0}")]
    Install(String),
}

impl From<reqwest::Error> for UpdateError {
    fn from(e: reqwest::Error) -> Self {
        // Ohne URL, damit keine Server-Adressen in Logs landen.
        Self::Http(e.without_url().to_string())
    }
}
