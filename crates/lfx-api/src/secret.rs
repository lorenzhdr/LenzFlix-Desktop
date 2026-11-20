//! Geheimnis-Typ: Debug und Display zeigen nie den Inhalt.

use serde::{Deserialize, Deserializer};
use std::fmt;

/// Kapselt Tokens und andere Geheimnisse. `Debug`/`Display` geben immer `***` aus,
/// der Klartext ist nur über [`Secret::expose`] erreichbar (bewusst, z. B. für den Schlüsselbund).
/// `Serialize` ist absichtlich nicht implementiert.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    /// Neues Geheimnis aus beliebigem Text.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Klartext. Nie loggen, nie in Fehlertexte oder URLs schreiben.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Leer (kein Token vorhanden)?
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}
