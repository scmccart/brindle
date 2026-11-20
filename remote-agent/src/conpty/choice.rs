//! Which ConPTY a session gets, and where the packaged one loads from: the
//! session's `BRINDLE_CONPTY=inbox` asks for the one built into Windows;
//! otherwise the packaged one is used from beside the exe when the files
//! there are the embedded ones, else from its extraction under
//! `%LOCALAPPDATA%`.
//!
//! Pure, so it is tested on every platform. `api` keeps the reads, the
//! writes, the loading and the log lines, and passes the probe that
//! compares a directory's files with the embedded ones. [`cached_ok`] keeps
//! a ConPTY that loaded, and not a failure, and [`missing`] names an export
//! the DLL lacks.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What the session's environment sets `BRINDLE_CONPTY` to for the ConPTY
/// built into Windows.
pub const INBOX: &str = "inbox";

/// Whether a session's environment asks for the ConPTY built into Windows:
/// `BRINDLE_CONPTY`, in any case, set to exactly [`INBOX`].
pub fn wants_inbox(env: &BTreeMap<String, String>) -> bool {
    env.iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("BRINDLE_CONPTY") && value == INBOX)
}

/// Where the packaged ConPTY of `version` is extracted to:
/// `%LOCALAPPDATA%\brindle\conpty\<version>`, or `None` without a local app
/// data directory (an empty variable counts as unset). An elevated process
/// has its own, `conpty-elevated\<version>`, so the files an elevated
/// holder loads are never the ones an unelevated process extracted.
pub fn extract_dir(
    local_app_data: Option<&OsStr>,
    version: &str,
    elevated: bool,
) -> Option<PathBuf> {
    let base = local_app_data.filter(|base| !base.is_empty())?;
    Some(
        Path::new(base)
            .join("brindle")
            .join(if elevated { "conpty-elevated" } else { "conpty" })
            .join(version),
    )
}

/// Where the packaged ConPTY loads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shipped {
    /// Beside the exe, whose files are the embedded ones.
    Beside(PathBuf),
    /// From its extraction, which holds the embedded files.
    Extracted(PathBuf),
    /// From its extraction, once the embedded files are written there: none
    /// were, or they are stale.
    ToExtract(PathBuf),
    /// Nowhere: the exe's files aren't the embedded ones, and there is no
    /// directory to extract to.
    Nowhere,
}

/// Where the packaged ConPTY loads from, given the exe's directory, the
/// extraction directory and a probe that says whether a directory holds the
/// embedded files. The exe's directory is probed first, and the extraction
/// only when that one doesn't hold them.
pub fn shipped(
    exe_dir: Option<&Path>,
    extract_dir: Option<&Path>,
    holds_embedded: impl Fn(&Path) -> bool,
) -> Shipped {
    if let Some(dir) = exe_dir.filter(|dir| holds_embedded(dir)) {
        return Shipped::Beside(dir.to_path_buf());
    }
    let Some(dir) = extract_dir else {
        return Shipped::Nowhere;
    };
    if holds_embedded(dir) {
        Shipped::Extracted(dir.to_path_buf())
    } else {
        Shipped::ToExtract(dir.to_path_buf())
    }
}

/// The value `cell` holds, or else `load`'s, which `cell` keeps only when
/// it loaded: a failure is the caller's to report, and the next call tries
/// again (`railway R6`: a failure isn't cached).
pub fn cached_ok<T>(
    cell: &OnceLock<T>,
    load: impl FnOnce() -> Result<T, String>,
) -> Result<&T, String> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = load()?;
    // Another thread's value may have got there first; either is good.
    Ok(cell.get_or_init(|| value))
}

/// The error for a `conpty.dll` that lacks the export `name`.
pub fn missing(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("conpty.dll lacks {name}"))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[test]
    fn a_failure_is_not_cached() {
        let cell = OnceLock::new();
        let calls = Cell::new(0);
        let failing = || {
            calls.set(calls.get() + 1);
            Err::<u32, _>("not loadable yet".to_string())
        };
        assert_eq!(cached_ok(&cell, failing), Err("not loadable yet".to_string()));
        assert_eq!(cached_ok(&cell, failing), Err("not loadable yet".to_string()));
        assert_eq!(calls.get(), 2, "each call tried again");
        // A later success is kept.
        assert_eq!(cached_ok(&cell, || Ok(7)), Ok(&7));
        assert_eq!(cell.get(), Some(&7));
    }

    #[test]
    fn a_success_is_cached() {
        let cell = OnceLock::new();
        let calls = Cell::new(0);
        let load = || {
            calls.set(calls.get() + 1);
            Ok::<u32, String>(calls.get())
        };
        assert_eq!(cached_ok(&cell, load), Ok(&1));
        assert_eq!(cached_ok(&cell, load), Ok(&1));
        assert_eq!(calls.get(), 1, "loaded once");
        // Nor does a failure replace it: the cell answers first.
        assert_eq!(cached_ok(&cell, || Err("broken".to_string())), Ok(&1));
    }

    #[test]
    fn missing_names_the_export() {
        let error = missing("ConptyCreatePseudoConsole");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "conpty.dll lacks ConptyCreatePseudoConsole");
    }

    fn env(list: &[(&str, &str)]) -> BTreeMap<String, String> {
        list.iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn only_inbox_asks_for_the_builtin_host() {
        assert!(!wants_inbox(&env(&[])));
        // The name in any case, the value exactly `inbox`.
        for name in ["BRINDLE_CONPTY", "brindle_conpty", "Brindle_ConPty"] {
            assert!(wants_inbox(&env(&[(name, "inbox")])), "{name}");
        }
        for value in ["Inbox", "INBOX", "", "packaged", " inbox"] {
            assert!(
                !wants_inbox(&env(&[("BRINDLE_CONPTY", value)])),
                "{value:?}"
            );
        }
        assert!(!wants_inbox(&env(&[("CONPTY", "inbox")])));
        assert_eq!(INBOX, "inbox");
    }

    #[test]
    fn the_extraction_is_under_local_app_data_by_version() {
        let base = r"C:\Users\me\AppData\Local";
        assert_eq!(
            extract_dir(Some(OsStr::new(base)), "1.25.0", false),
            Some(
                PathBuf::from(base)
                    .join("brindle")
                    .join("conpty")
                    .join("1.25.0")
            )
        );
    }

    /// An elevated process extracts to a directory of its own, beside the
    /// unelevated one and by version too.
    #[test]
    fn an_elevated_process_extracts_to_a_directory_of_its_own() {
        let base = r"C:\Users\me\AppData\Local";
        let elevated = extract_dir(Some(OsStr::new(base)), "1.25.0", true);
        assert_eq!(
            elevated,
            Some(
                PathBuf::from(base)
                    .join("brindle")
                    .join("conpty-elevated")
                    .join("1.25.0")
            )
        );
        assert_ne!(
            elevated,
            extract_dir(Some(OsStr::new(base)), "1.25.0", false)
        );
        // Neither is inside the other.
        let unelevated = PathBuf::from(base).join("brindle").join("conpty");
        assert!(!elevated.unwrap().starts_with(unelevated));
    }

    #[test]
    fn an_empty_local_app_data_is_unset() {
        for elevated in [false, true] {
            assert_eq!(extract_dir(Some(OsStr::new("")), "1.25.0", elevated), None);
            assert_eq!(extract_dir(None, "1.25.0", elevated), None);
        }
    }

    /// A probe that holds the embedded files in `matching` and records what
    /// it was asked about.
    fn probe<'a>(
        matching: &'a [&'a Path],
        asked: &'a RefCell<Vec<PathBuf>>,
    ) -> impl Fn(&Path) -> bool + 'a {
        move |dir| {
            asked.borrow_mut().push(dir.to_path_buf());
            matching.contains(&dir)
        }
    }

    #[test]
    fn matching_files_beside_the_exe_win() {
        let (exe, extract) = (Path::new("exe"), Path::new("extract"));
        let asked = RefCell::new(Vec::new());
        assert_eq!(
            shipped(Some(exe), Some(extract), probe(&[exe, extract], &asked)),
            Shipped::Beside(exe.to_path_buf())
        );
        // The extraction isn't looked at.
        assert_eq!(*asked.borrow(), [exe]);
        // With no extraction directory, too.
        assert_eq!(
            shipped(Some(exe), None, probe(&[exe], &asked)),
            Shipped::Beside(exe.to_path_buf())
        );
        // An extraction that matches is used when the exe's files don't.
        assert_eq!(
            shipped(Some(exe), Some(extract), probe(&[extract], &asked)),
            Shipped::Extracted(extract.to_path_buf())
        );
        assert_eq!(
            shipped(None, Some(extract), probe(&[extract], &asked)),
            Shipped::Extracted(extract.to_path_buf())
        );
    }

    #[test]
    fn a_stale_or_missing_extraction_is_to_extract() {
        let (exe, extract) = (Path::new("exe"), Path::new("extract"));
        let asked = RefCell::new(Vec::new());
        // A different DLL beside the exe, and none extracted yet.
        assert_eq!(
            shipped(Some(exe), Some(extract), probe(&[], &asked)),
            Shipped::ToExtract(extract.to_path_buf())
        );
        assert_eq!(*asked.borrow(), [exe, extract]);
        assert_eq!(
            shipped(None, Some(extract), probe(&[], &asked)),
            Shipped::ToExtract(extract.to_path_buf())
        );
    }

    #[test]
    fn no_extract_dir_is_nowhere() {
        let exe = Path::new("exe");
        let asked = RefCell::new(Vec::new());
        assert_eq!(shipped(None, None, probe(&[], &asked)), Shipped::Nowhere);
        assert_eq!(
            shipped(Some(exe), None, probe(&[], &asked)),
            Shipped::Nowhere
        );
    }
}
