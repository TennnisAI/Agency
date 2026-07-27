//! Semantic-version parsing shared by the DB version stamp and the update check.
//!
//! Deliberately minimal: Agency only ever compares its own release versions, so
//! this handles `major.minor.patch` with an optional pre-release/build suffix
//! and nothing else. Unparseable components read as 0 rather than erroring —
//! a garbled version should degrade to "looks different", never crash a launch.

/// Parse `major.minor.patch`, ignoring any `-pre` / `+build` suffix. Missing or
/// non-numeric components read as 0; each is clamped to 999 so `encode` stays
/// monotonic.
pub fn parse(version: &str) -> (i64, i64, i64) {
    let core = version.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.').map(|p| p.parse::<i64>().unwrap_or(0).clamp(0, 999));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// Encode a version as a single monotonic integer, for `PRAGMA user_version`.
pub fn encode(version: &str) -> i64 {
    let (major, minor, patch) = parse(version);
    major * 1_000_000 + minor * 1_000 + patch
}

/// True when `candidate` is a strictly newer release than `current`.
///
/// Suffixes are ignored, so `0.2.0-beta.1` and `0.2.0` compare equal — Agency
/// never ships both, and treating a pre-release as an update to the release of
/// the same number would loop testers between builds.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    encode(candidate) > encode(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_prefixed_versions() {
        assert_eq!(parse("0.1.0"), (0, 1, 0));
        assert_eq!(parse("v0.1.0"), (0, 1, 0), "release tags carry a v prefix");
        assert_eq!(parse(" 1.2.3 "), (1, 2, 3));
        assert_eq!(parse("2.0"), (2, 0, 0), "missing components read as 0");
    }

    #[test]
    fn ignores_prerelease_and_build_suffixes() {
        assert_eq!(parse("0.2.0-beta.1"), (0, 2, 0));
        assert_eq!(parse("0.2.0+build7"), (0, 2, 0));
        assert_eq!(encode("0.2.0-beta.1"), encode("0.2.0"));
    }

    #[test]
    fn garbage_degrades_to_zero() {
        assert_eq!(parse(""), (0, 0, 0));
        assert_eq!(parse("not.a.version"), (0, 0, 0));
        assert_eq!(encode(""), 0);
    }

    #[test]
    fn encode_is_monotonic_across_components() {
        assert!(encode("0.1.1") > encode("0.1.0"));
        assert!(encode("0.2.0") > encode("0.1.99"));
        assert!(encode("1.0.0") > encode("0.999.999"));
        assert!(encode("0.10.0") > encode("0.9.9"), "numeric, not lexical, ordering");
    }

    #[test]
    fn is_newer_only_for_strict_upgrades() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.1.0"), "compares a release tag to a package version");
        assert!(!is_newer("0.1.0", "0.1.0"), "same version is not an update");
        assert!(!is_newer("0.1.0", "0.2.0"), "never offers a downgrade");
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"), "a pre-release is not newer than its release");
    }
}
