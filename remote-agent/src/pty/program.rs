//! What a Linux session runs and where: the login shell, the home
//! directory, `~` in the requested directory, the start directory and the
//! default `TERM`.
//!
//! Pure, so it is tested on every platform. The Linux `unix` gathers the
//! request's and this process's variables, reads the passwd entry only when
//! they give nothing (it can block on NSS), and passes probes that ask
//! whether a path it derived exists.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The shell when there is none in the environment or the passwd entry.
const FALLBACK_SHELL: &str = "/bin/sh";

/// The `TERM` a session gets when its environment names none.
const DEFAULT_TERM: &str = "xterm-256color";

/// The login shell the environment names: the request's `SHELL`, else this
/// process's. An empty one counts as unset.
pub fn shell_from_env(request: Option<&str>, own: Option<&OsStr>) -> Option<OsString> {
    request
        .filter(|shell| !shell.is_empty())
        .map(OsString::from)
        .or_else(|| own.filter(|shell| !shell.is_empty()).map(OsString::from))
}

/// The login shell when the environment names none: the passwd entry's
/// `shell` when it is set and `exists`, else `/bin/sh`. An empty one isn't
/// asked about.
pub fn passwd_shell(shell: Option<OsString>, exists: impl Fn(&Path) -> bool) -> OsString {
    shell
        .filter(|shell| !shell.is_empty() && exists(Path::new(shell)))
        .unwrap_or_else(|| OsString::from(FALLBACK_SHELL))
}

/// The home directory the environment names: the request's `HOME`, else
/// this process's. An empty one counts as unset.
pub fn home_from_env(request: Option<&str>, own: Option<&OsStr>) -> Option<PathBuf> {
    request
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| own.filter(|home| !home.is_empty()).map(PathBuf::from))
}

/// The requested directory with a leading `~` or `~/` replaced by `home`.
/// Without a home, or for `~user`, it stays as written.
pub fn expand_home(cwd: &str, home: Option<&Path>) -> PathBuf {
    if cwd == "~" {
        return home.map_or_else(|| PathBuf::from(cwd), Path::to_path_buf);
    }
    if let Some(rest) = cwd.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(rest);
    }
    PathBuf::from(cwd)
}

/// Where the session starts: `wanted` when it is a directory here, else
/// `home` when that is one, else `None` (this process's own directory). A
/// directory from another host, or since deleted, starts it at home.
pub fn start_dir(
    wanted: &Path,
    home: Option<&Path>,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if is_dir(wanted) {
        return Some(wanted.to_path_buf());
    }
    home.filter(|home| is_dir(home)).map(Path::to_path_buf)
}

/// The `TERM` to add to a session's environment: the default, unless the
/// environment is `exact` or names one itself.
pub fn default_term(exact: bool, env: &BTreeMap<String, String>) -> Option<&'static str> {
    (!exact && !env.contains_key("TERM")).then_some(DEFAULT_TERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(text: &str) -> Option<&OsStr> {
        Some(OsStr::new(text))
    }

    #[test]
    fn the_shell_is_the_requests_then_this_processs() {
        assert_eq!(
            shell_from_env(Some("/bin/zsh"), os("/bin/bash")),
            Some(OsString::from("/bin/zsh"))
        );
        assert_eq!(
            shell_from_env(None, os("/bin/bash")),
            Some(OsString::from("/bin/bash"))
        );
        assert_eq!(shell_from_env(None, None), None);
    }

    #[test]
    fn an_empty_shell_counts_as_unset() {
        assert_eq!(
            shell_from_env(Some(""), os("/bin/bash")),
            Some(OsString::from("/bin/bash"))
        );
        assert_eq!(shell_from_env(Some(""), os("")), None);
        assert_eq!(shell_from_env(None, os("")), None);
    }

    #[test]
    fn the_passwd_shell_is_used_only_if_it_exists_else_bin_sh() {
        let exists = |path: &Path| path == Path::new("/usr/bin/fish");
        assert_eq!(
            passwd_shell(Some(OsString::from("/usr/bin/fish")), exists),
            OsString::from("/usr/bin/fish")
        );
        for missing in [Some("/usr/bin/nologin-here"), Some(""), None] {
            assert_eq!(
                passwd_shell(missing.map(OsString::from), exists),
                OsString::from("/bin/sh"),
                "{missing:?}"
            );
        }
        // An empty shell isn't asked about.
        let asked = |path: &Path| {
            assert!(!path.as_os_str().is_empty(), "asked about an empty path");
            true
        };
        assert_eq!(
            passwd_shell(Some(OsString::new()), asked),
            OsString::from("/bin/sh")
        );
    }

    #[test]
    fn home_is_the_requests_then_this_processs() {
        assert_eq!(
            home_from_env(Some("/home/remote"), os("/home/local")),
            Some(PathBuf::from("/home/remote"))
        );
        assert_eq!(
            home_from_env(None, os("/home/local")),
            Some(PathBuf::from("/home/local"))
        );
        // Empty counts as unset.
        assert_eq!(
            home_from_env(Some(""), os("/home/local")),
            Some(PathBuf::from("/home/local"))
        );
        assert_eq!(home_from_env(Some(""), os("")), None);
        assert_eq!(home_from_env(None, None), None);
    }

    #[test]
    fn a_tilde_expands_to_home() {
        let home = Path::new("/home/remote");
        assert_eq!(expand_home("~", Some(home)), home.to_path_buf());
        assert_eq!(expand_home("~/src tree", Some(home)), home.join("src tree"));
        assert_eq!(expand_home("/srv/app", Some(home)), PathBuf::from("/srv/app"));
        // Only a whole first component.
        assert_eq!(expand_home("~user/x", Some(home)), PathBuf::from("~user/x"));
        // With no home, the path stays as written.
        assert_eq!(expand_home("~", None), PathBuf::from("~"));
        assert_eq!(expand_home("~/src", None), PathBuf::from("~/src"));
    }

    #[test]
    fn a_missing_start_directory_falls_back_to_home_then_to_none() {
        let (wanted, home) = (Path::new("/srv/app"), Path::new("/home/me"));
        let all = |_: &Path| true;
        assert_eq!(start_dir(wanted, Some(home), all), Some(wanted.to_path_buf()));
        let only_home = |path: &Path| path == home;
        assert_eq!(
            start_dir(wanted, Some(home), only_home),
            Some(home.to_path_buf())
        );
        let none = |_: &Path| false;
        assert_eq!(start_dir(wanted, Some(home), none), None);
        assert_eq!(start_dir(wanted, None, only_home), None);
    }

    #[test]
    fn term_defaults_only_for_a_non_exact_environment_without_one() {
        let empty = BTreeMap::new();
        let with_term: BTreeMap<String, String> = [("TERM".to_string(), "dumb".to_string())].into();
        assert_eq!(default_term(false, &empty), Some("xterm-256color"));
        assert_eq!(default_term(false, &with_term), None);
        assert_eq!(default_term(true, &empty), None);
        assert_eq!(default_term(true, &with_term), None);
    }
}
