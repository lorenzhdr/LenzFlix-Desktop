//! Prüfung von SHA-256 und Minisign-Signatur.
//!
//! Der öffentliche Schlüssel ist als Konstante eingebaut. Das Schlüsselpaar erzeugt der Nutzer
//! einmalig mit `minisign -G`; der PRIVATE Schlüssel gehört ausschließlich als GitHub-Secret in die
//! Release-Pipeline und NIE ins Repo. Danach wird [`UPDATE_PUBLIC_KEY`] durch die Base64-Zeile aus der
//! `.pub`-Datei ersetzt (Zeile ohne den Kommentar, beginnt meist mit `RW`).

use crate::error::UpdateError;
use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

/// Platzhalter, bis der Nutzer sein Schlüsselpaar erzeugt hat. Solange er hier steht,
/// verweigert der Updater jede Installation ([`UpdateError::KeyNotConfigured`]).
pub const KEY_PLACEHOLDER: &str = "PLATZHALTER-MINISIGN-OEFFENTLICHER-SCHLUESSEL";

/// Eingebauter öffentlicher Minisign-Schlüssel (Base64-Zeile der `.pub`-Datei).
pub const UPDATE_PUBLIC_KEY: &str = KEY_PLACEHOLDER;

/// Ist noch der Platzhalter eingetragen?
pub fn is_placeholder(key: &str) -> bool {
    key == KEY_PLACEHOLDER
}

/// SHA-256 als Kleinbuchstaben-Hex.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex_lower(&h.finalize())
}

/// Hex-Darstellung (klein).
pub fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Prüft die Minisign-Signatur (`sig_text` = Inhalt der `.minisig`-Datei) über `data`.
/// Es werden nur vorgehashte Signaturen (Standardmodus von `minisign -S`) akzeptiert.
pub fn verify_signature(public_key: &str, data: &[u8], sig_text: &str) -> Result<(), UpdateError> {
    if is_placeholder(public_key) {
        return Err(UpdateError::KeyNotConfigured);
    }
    let pk = PublicKey::from_base64(public_key.trim())
        .map_err(|e| UpdateError::Signature(format!("Schlüssel: {e}")))?;
    let sig = Signature::decode(sig_text)
        .map_err(|e| UpdateError::Signature(format!("Signatur: {e}")))?;
    pk.verify(data, &sig, false)
        .map_err(|e| UpdateError::Signature(e.to_string()))
}

#[cfg(test)]
pub(crate) mod testkeys {
    //! NUR FÜR TESTS: Schlüsselpaar aus festem Seed, im Test erzeugt, nirgends sonst verwendet.
    //! Kein echter Schlüssel; der Seed ist absichtlich erkennbar ("TEST-ONLY").
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    use blake2::{Blake2b512, Digest};
    use ed25519_dalek::{Signer, SigningKey};

    pub struct TestKeys {
        sk: SigningKey,
        key_id: [u8; 8],
    }

    impl TestKeys {
        pub fn new(tag: u8) -> Self {
            let mut seed = *b"TEST-ONLY-not-a-real-key-0000000";
            seed[31] = tag;
            Self {
                sk: SigningKey::from_bytes(&seed),
                key_id: [tag, 1, 2, 3, 4, 5, 6, 7],
            }
        }

        /// Base64-Zeile wie in einer `.pub`-Datei.
        pub fn public_b64(&self) -> String {
            let mut raw = Vec::from(*b"Ed");
            raw.extend_from_slice(&self.key_id);
            raw.extend_from_slice(self.sk.verifying_key().as_bytes());
            B64.encode(raw)
        }

        /// Erzeugt den Text einer `.minisig`-Datei (vorgehashter Modus "ED").
        pub fn sign(&self, data: &[u8]) -> String {
            let digest = Blake2b512::digest(data);
            let sig = self.sk.sign(&digest).to_bytes();
            let mut raw = Vec::from(*b"ED");
            raw.extend_from_slice(&self.key_id);
            raw.extend_from_slice(&sig);
            let comment = "timestamp:1 file:test";
            let mut gmsg = sig.to_vec();
            gmsg.extend_from_slice(comment.as_bytes());
            let gsig = self.sk.sign(&gmsg).to_bytes();
            format!(
                "untrusted comment: TEST-ONLY signature\n{}\ntrusted comment: {comment}\n{}\n",
                B64.encode(raw),
                B64.encode(gsig)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkeys::TestKeys;
    use super::*;

    #[test]
    fn gueltige_signatur_wird_akzeptiert() {
        let k = TestKeys::new(1);
        let data = b"paketinhalt";
        let sig = k.sign(data);
        verify_signature(&k.public_b64(), data, &sig).unwrap();
    }

    #[test]
    fn manipulierte_daten_werden_erkannt() {
        let k = TestKeys::new(1);
        let sig = k.sign(b"original");
        let e = verify_signature(&k.public_b64(), b"manipuliert", &sig).unwrap_err();
        assert!(matches!(e, UpdateError::Signature(_)));
    }

    #[test]
    fn fremder_schluessel_und_kaputte_signatur_werden_abgelehnt() {
        let a = TestKeys::new(1);
        let b = TestKeys::new(2);
        let sig = a.sign(b"x");
        assert!(verify_signature(&b.public_b64(), b"x", &sig).is_err());
        assert!(verify_signature(&a.public_b64(), b"x", "Müll").is_err());
    }

    #[test]
    fn platzhalterschluessel_verweigert() {
        let k = TestKeys::new(1);
        let e = verify_signature(UPDATE_PUBLIC_KEY, b"x", &k.sign(b"x")).unwrap_err();
        assert!(matches!(e, UpdateError::KeyNotConfigured));
    }

    #[test]
    fn sha256_bekannter_wert() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
