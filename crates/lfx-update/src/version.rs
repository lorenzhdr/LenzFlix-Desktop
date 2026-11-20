//! SemVer-Vergleich. Vorabversionen (`1.0.0-beta.1`) sind kleiner als die Release-Version.

use crate::error::UpdateError;
use semver::Version;

/// Parst eine Version; ein führendes `v` (Tag-Schreibweise) wird toleriert.
pub fn parse_version(s: &str) -> Result<Version, UpdateError> {
    let t = s.trim().trim_start_matches('v');
    Version::parse(t).map_err(|_| UpdateError::Version(s.to_string()))
}

/// Ist `candidate` echt neuer als `current`? Gleich oder älter ergibt `false` (kein Downgrade).
pub fn is_newer(current: &str, candidate: &str) -> Result<bool, UpdateError> {
    Ok(parse_version(candidate)? > parse_version(current)?)
}

/// Ist die Version eine Vorabversion (hat SemVer-Pre-Release-Teil)?
pub fn is_prerelease(v: &str) -> bool {
    parse_version(v).map(|v| !v.pre.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neuer_und_gleich_und_aelter() {
        assert!(is_newer("0.1.0", "0.1.1").unwrap());
        assert!(is_newer("0.9.0", "0.10.0").unwrap());
        assert!(!is_newer("0.1.0", "0.1.0").unwrap());
        assert!(!is_newer("0.2.0", "0.1.9").unwrap());
    }

    #[test]
    fn vorabversion_ist_kleiner_als_release() {
        assert!(is_newer("1.0.0-beta.1", "1.0.0").unwrap());
        assert!(!is_newer("1.0.0", "1.0.0-beta.2").unwrap());
        assert!(is_newer("1.0.0-beta.1", "1.0.0-beta.2").unwrap());
        assert!(is_prerelease("1.0.0-rc.1"));
        assert!(!is_prerelease("1.0.0"));
    }

    #[test]
    fn tag_praefix_und_fehler() {
        assert!(is_newer("v0.1.0", "v0.2.0").unwrap());
        assert!(is_newer("0.1.0", "kaputt").is_err());
    }
}
