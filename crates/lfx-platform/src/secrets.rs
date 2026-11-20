//! Geheimnisse (Zugriffstokens) im OS-Schlüsselbund.
//!
//! Konto-Name ist `<server-id>:<user-id>`, Dienstname `LENZFLIX`. Die App spricht nur den Trait
//! [`SecretStore`] an; Tests nehmen [`MemorySecretStore`]. [`KeyringSecretStore`] nutzt `keyring-core`
//! mit dem nativen Speicher der Plattform (Windows Credential Manager, macOS Keychain).

use lfx_api::Secret;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Dienstname im Schlüsselbund.
pub const SERVICE: &str = "LENZFLIX";

/// Fehler beim Zugriff auf den Schlüsselbund. Enthält nie den Klartext.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// Auf der Plattform gibt es keinen unterstützten Schlüsselbund.
    #[error("Kein Schlüsselbund auf dieser Plattform verfügbar")]
    Unsupported,
    /// Der Schlüsselbund hat den Zugriff abgelehnt oder ist nicht erreichbar.
    #[error("Schlüsselbund nicht erreichbar: {0}")]
    Access(String),
}

impl From<keyring_core::Error> for SecretError {
    fn from(err: keyring_core::Error) -> Self {
        Self::Access(err.to_string())
    }
}

/// Name des Eintrags: `<server-id>:<user-id>`.
pub fn account_name(server_id: &str, user_id: &str) -> String {
    format!("{server_id}:{user_id}")
}

/// Ablage für Tokens.
pub trait SecretStore: Send + Sync {
    /// Liest das Geheimnis. `Ok(None)`, wenn es keinen Eintrag gibt.
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError>;
    /// Schreibt oder ersetzt das Geheimnis.
    fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError>;
    /// Löscht den Eintrag; gibt es keinen, ist das kein Fehler.
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

/// Speicher nur im Arbeitsspeicher (Tests, Entwicklung ohne Schlüsselbund).
#[derive(Default)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<String, Secret>>,
}

impl MemorySecretStore {
    /// Leerer Speicher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Anzahl der Einträge.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Ist der Speicher leer?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Secret>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
        Ok(self.lock().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError> {
        self.lock().insert(account.to_owned(), secret.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.lock().remove(account);
        Ok(())
    }
}

/// Nativer Schlüsselbund über `keyring-core`.
pub struct KeyringSecretStore {
    store: Arc<keyring_core::CredentialStore>,
    service: String,
}

impl KeyringSecretStore {
    /// Mit einem beliebigen `keyring-core`-Speicher (Dienstname [`SERVICE`]).
    pub fn with_store(store: Arc<keyring_core::CredentialStore>) -> Self {
        Self::with_service(store, SERVICE)
    }

    /// Wie [`Self::with_store`] mit eigenem Dienstnamen (für Tests, damit echte Einträge unberührt bleiben).
    pub fn with_service(store: Arc<keyring_core::CredentialStore>, service: &str) -> Self {
        Self {
            store,
            service: service.to_owned(),
        }
    }

    /// Nativer Speicher der Plattform; `Unsupported` auf anderen Systemen.
    pub fn native() -> Result<Self, SecretError> {
        native_store().map(Self::with_store)
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, SecretError> {
        Ok(self.store.build(&self.service, account, None)?)
    }
}

#[cfg(windows)]
fn native_store() -> Result<Arc<keyring_core::CredentialStore>, SecretError> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "macos")]
fn native_store() -> Result<Arc<keyring_core::CredentialStore>, SecretError> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn native_store() -> Result<Arc<keyring_core::CredentialStore>, SecretError> {
    Err(SecretError::Unsupported)
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
        match self.entry(account)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError> {
        Ok(self.entry(account)?.set_password(secret.expose())?)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

/// Schlüsselbund der Plattform als Trait-Objekt.
pub fn os_secret_store() -> Result<Arc<dyn SecretStore>, SecretError> {
    Ok(Arc::new(KeyringSecretStore::native()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(store: &dyn SecretStore) {
        let acc = account_name("srv", "usr");
        assert!(store.get(&acc).unwrap().is_none());
        store.set(&acc, &Secret::new("geheim-1")).unwrap();
        assert_eq!(store.get(&acc).unwrap().unwrap().expose(), "geheim-1");
        store.set(&acc, &Secret::new("geheim-2")).unwrap();
        assert_eq!(store.get(&acc).unwrap().unwrap().expose(), "geheim-2");
        store.delete(&acc).unwrap();
        assert!(store.get(&acc).unwrap().is_none());
        store.delete(&acc).unwrap(); // zweites Löschen: kein Fehler
    }

    #[test]
    fn account_format() {
        assert_eq!(account_name("a", "b"), "a:b");
    }

    #[test]
    fn memory_store_round_trip() {
        let store = MemorySecretStore::new();
        round_trip(&store);
        assert!(store.is_empty());
    }

    #[test]
    fn accounts_are_separate() {
        let store = MemorySecretStore::new();
        store.set("a:1", &Secret::new("x")).unwrap();
        store.set("a:2", &Secret::new("y")).unwrap();
        assert_eq!(store.get("a:1").unwrap().unwrap().expose(), "x");
        assert_eq!(store.len(), 2);
    }

    /// Echter Schlüsselbund; verändert den Credential Manager bzw. die Keychain (eigener Testdienst).
    #[test]
    #[ignore = "braucht den echten Schlüsselbund des Betriebssystems"]
    fn native_keyring_round_trip() {
        let native = native_store().expect("nativer Speicher");
        let store = KeyringSecretStore::with_service(native, "LENZFLIX-test");
        round_trip(&store);
    }
}
