//! Deciding whether a helper should replace the running core.
//!
//! Shared by Brindle's local upgrade offer and the relay on a remote host:
//! - a higher semver is newer and a lower one older;
//! - with equal versions (dogfooding builds keep the version), the helper is
//!   newer when its build id differs and its file is newer than the running
//!   core's binary.

use std::time::SystemTime;

/// A server binary's release, build id and file time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    pub build: String,
    pub mtime: Option<SystemTime>,
}

/// Whether `helper` should replace `core`.
pub fn is_newer(helper: &Build, core: &Build) -> bool {
    match (semver(&helper.version), semver(&core.version)) {
        (Some(offered), Some(running)) if offered != running => offered > running,
        (Some(_), Some(_)) => {
            helper.build != core.build
                && matches!((helper.mtime, core.mtime), (Some(offered), Some(running)) if offered > running)
        }
        _ => false,
    }
}

/// `MAJOR.MINOR.PATCH`, ignoring any pre-release or build suffix.
pub fn semver(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn build(version: &str, build: &str, secs: Option<u64>) -> Build {
        Build {
            version: version.into(),
            build: build.into(),
            mtime: secs.map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
        }
    }

    #[test]
    fn a_higher_version_is_newer_and_a_lower_one_older() {
        let core = build("0.3.0", "aaa", Some(100));
        assert!(is_newer(&build("0.4.0", "bbb", Some(50)), &core));
        assert!(is_newer(&build("0.3.1", "aaa", None), &core));
        assert!(is_newer(&build("1.0.0", "bbb", Some(100)), &build("0.10.9", "aaa", Some(100))));
        assert!(!is_newer(&build("0.2.9", "bbb", Some(200)), &core));
        assert!(!is_newer(&build("0.3.0", "bbb", Some(200)), &build("0.3.1", "aaa", Some(100))));
    }

    #[test]
    fn an_equal_version_needs_another_build_and_a_later_file() {
        let core = build("0.3.0", "aaa", Some(100));
        assert!(is_newer(&build("0.3.0", "bbb", Some(101)), &core));
        // The same build, however new its file.
        assert!(!is_newer(&build("0.3.0", "aaa", Some(500)), &core));
        // Another build, but an older file.
        assert!(!is_newer(&build("0.3.0", "bbb", Some(99)), &core));
        assert!(!is_newer(&build("0.3.0", "bbb", Some(100)), &core));
        // Either time unknown.
        assert!(!is_newer(&build("0.3.0", "bbb", None), &core));
        assert!(!is_newer(&build("0.3.0", "bbb", Some(101)), &build("0.3.0", "aaa", None)));
        // Pre-release suffixes compare by their numbers.
        assert!(is_newer(&build("0.3.0-rc.1", "bbb", Some(101)), &core));
    }

    #[test]
    fn unparsable_versions_are_never_newer() {
        let core = build("0.3.0", "aaa", Some(100));
        assert!(!is_newer(&build("dev", "bbb", Some(200)), &core));
        assert!(!is_newer(&build("0.4", "bbb", Some(200)), &core));
        assert!(!is_newer(&build("0.4.0", "bbb", Some(200)), &build("", "aaa", Some(1))));
        assert_eq!(semver(" 1.2.3\n"), Some((1, 2, 3)));
        assert_eq!(semver("1.2.3.4"), None);
    }
}
