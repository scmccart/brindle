//! The server directory on Windows: `%BRINDLE_SERVER_DIR%`, else
//! `%LOCALAPPDATA%\brindle\run` (`run-elevated` for an elevated process),
//! created with a protected DACL for the user and SYSTEM, or for an
//! elevated process's `run-elevated` for Administrators and SYSTEM only,
//! held open while the role runs. It holds locks and logs; the pipes are
//! named from it and protected on their own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Shared with Brindle's client, so both find the same directory.
use brindle_remote_protocol::server_dir::{self, resolve_windows};
use brindle_remote_protocol::win::{self, Contents, ProtectedDir};

use super::super::ensure_dir::{self, AfterCreate, First, Found, Protect, Protection};

/// What a role keeps of its server directory while it runs: the elevated
/// directory, held so it stays the one that was checked; nothing for any
/// other.
#[derive(Debug)]
pub struct Held(#[allow(dead_code, reason = "held for its handles")] Option<ProtectedDir>);

/// Where the server directory for this process's environment is, without
/// creating it.
pub fn resolve_from_env() -> io::Result<PathBuf> {
    resolve_windows(
        std::env::var_os("BRINDLE_SERVER_DIR"),
        std::env::var_os("LOCALAPPDATA"),
        win::is_elevated()?,
    )
}

/// Whether this process is elevated, read once: an elevated role opens its
/// locks and logs with `win::open_elevated_file`.
pub(super) fn elevated_role() -> io::Result<bool> {
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if let Some(elevated) = ELEVATED.get() {
        return Ok(*elevated);
    }
    let elevated = win::is_elevated()?;
    Ok(*ELEVATED.get_or_init(|| elevated))
}

/// The directory used when `BRINDLE_SERVER_DIR` isn't set, for a process
/// that is `elevated` or not.
fn default_dir(elevated: bool) -> Option<PathBuf> {
    resolve_windows(None, std::env::var_os("LOCALAPPDATA"), elevated).ok()
}

/// What a core's start did beside its server directory, and what it holds
/// there for its life.
#[derive(Debug)]
pub struct Pruned {
    /// An elevated core's copies' directory (`server-elevated`).
    _held: Held,
    /// Lines for the core's log.
    pub notes: Vec<String>,
}

/// Removes the per-build helper copies of other builds, where nothing runs
/// from them any more (Windows runs every role from a copy, since a running
/// image can't be replaced); a copy still in use stays for a later core,
/// and a note says why.
///
/// An elevated core's copies are in `server-elevated`, which Brindle made
/// protected before it started the core there: it is checked, not changed,
/// and held, and nothing is pruned in it unless it is the protected
/// directory. An elevated core also removes what was set aside for not
/// being protected, beside it and among the ConPTY extractions, leaving a
/// link alone.
pub fn prune_copies(build: &str) -> Pruned {
    let mut notes = Vec::new();
    let given = std::env::var_os("BRINDLE_SERVER_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let none = |notes| Pruned {
        _held: Held(None),
        notes,
    };
    let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|dir| !dir.is_empty()) else {
        return none(notes);
    };
    let elevated = match win::is_elevated() {
        Ok(elevated) => elevated,
        Err(error) => {
            notes.push(format!("not pruning the helper copies: can't tell whether this process is elevated: {error}"));
            return none(notes);
        }
    };
    let local = PathBuf::from(local);
    let copies = server_dir::helper_copies(given.as_deref(), &local, elevated);
    let mut held = None;
    if elevated && given.is_none() {
        let brindle = local.join("brindle");
        for dir in [brindle.clone(), brindle.join("conpty-elevated")] {
            let removed = win::remove_untrusted(&dir);
            if removed.removed > 0 {
                notes.push(format!("removed {} directories set aside in {}", removed.removed, dir.display()));
            }
            for (path, why) in removed.kept {
                notes.push(format!("kept {}: {why}", path.display()));
            }
        }
        match win::hold_protected_dir(&copies) {
            Ok(Some(dir)) => held = Some(dir),
            Ok(None) => return none(notes),
            Err(error) => {
                notes.push(format!("not pruning the helper copies: {error}"));
                return none(notes);
            }
        }
    }
    let pruned = win::prune_helper_copies(&copies, build);
    if pruned.removed > 0 {
        notes.push(format!("removed {} other build(s)' helper copies", pruned.removed));
    }
    for (path, why) in &pruned.kept {
        notes.push(format!("kept the helper copy {}: {why}", path.display()));
    }
    Pruned {
        _held: Held(held),
        notes,
    }
}

/// Creates `dir` (and missing parents) for this user and SYSTEM only. An
/// existing directory is used as it is, except the default one, whose DACL
/// is set again: a directory someone named with `BRINDLE_SERVER_DIR` is
/// theirs to protect. An elevated process's default directory is made or
/// checked for Administrators and SYSTEM only and held
/// ([`ensure_dir::protection`]).
pub fn ensure(dir: &Path) -> io::Result<Held> {
    let elevated = win::is_elevated()?;
    match ensure_dir::protection(default_dir(elevated).as_deref() == Some(dir), elevated) {
        Protection::Elevated => win::protected_dir(dir, Contents::State)
            .map(|held| Held(Some(held)))
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("can't use the elevated server directory {}: {error}", dir.display()),
                )
            }),
        Protection::Private(protect) => {
            ensure_with(dir, protect == Protect::Existing).map(|()| Held(None))
        }
    }
}

/// [`ensure`], re-protecting an existing `dir` when `protect_existing`.
/// What it does with what it finds is [`ensure_dir`]'s.
fn ensure_with(dir: &Path, protect_existing: bool) -> io::Result<()> {
    let refuse = |problem: &str| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing the server directory {}: {problem}", dir.display()),
        ))
    };
    let failed = |what: &str, error: Option<io::Error>| {
        let error = error.unwrap_or_else(|| io::ErrorKind::Other.into());
        io::Error::new(
            error.kind(),
            format!("can't {what} the server directory {}: {error}", dir.display()),
        )
    };
    let protect = if protect_existing {
        Protect::Existing
    } else {
        Protect::AsFound
    };
    let (found, error) = look(dir);
    match ensure_dir::first(found, protect) {
        First::Refuse => refuse("it is not a directory"),
        First::Protect => win::protect_dir(dir).map_err(|error| failed("protect", Some(error))),
        First::Use => Ok(()),
        First::Create => {
            create(dir).map_err(|error| failed("create", Some(error)))?;
            let (found, error) = look(dir);
            match ensure_dir::after_create(found) {
                AfterCreate::Use => Ok(()),
                AfterCreate::Refuse => refuse("it is not a directory"),
                AfterCreate::FailCreate => Err(failed("create", error)),
            }
        }
        First::FailRead => Err(failed("read", error)),
    }
}

/// What is at `dir`, its link not followed, with the error that said so
/// when it is missing or can't be read.
fn look(dir: &Path) -> (Found, Option<io::Error>) {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.is_dir() => (Found::Dir, None),
        Ok(_) => (Found::NotDir, None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Found::Missing, Some(error)),
        Err(error) => (Found::Unreadable, Some(error)),
    }
}

/// Creates `dir` and each missing parent, all with the private DACL. One
/// that appears meanwhile (another role starting) is used.
fn create(dir: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut next = Some(dir);
    while let Some(path) = next.filter(|path| !path.as_os_str().is_empty() && !path.exists()) {
        missing.push(path);
        next = path.parent();
    }
    for path in missing.into_iter().rev() {
        match win::create_private_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::win::{Dacl, SYSTEM_SID, own_sid};

    use super::*;
    use crate::server::dir::tests::TempDir;

    fn private(path: &Path) -> bool {
        let dacl = Dacl::of_path(path).unwrap();
        dacl.allows_only(&[&own_sid().unwrap(), SYSTEM_SID])
    }

    #[test]
    fn a_new_directory_and_its_new_parents_are_user_only() {
        let temp = TempDir::new("dir-new");
        let dir = temp.0.join("a").join("brindle");
        ensure(&dir).unwrap();
        let dacl = Dacl::of_path(&dir).unwrap();
        assert!(dacl.protected, "{dacl:?}");
        assert_eq!(dacl.allowed.len(), 2, "{dacl:?}");
        assert!(private(&dir), "{dacl:?}");
        assert!(private(&temp.0.join("a")));
        // The parent that was there keeps its own DACL.
        assert!(!private(&temp.0));
        // Existing is fine, and so is a file created inside.
        ensure(&dir).unwrap();
        fs::write(dir.join("core.lock"), b"").unwrap();
    }

    #[test]
    fn an_existing_directory_is_protected_only_when_it_is_the_default() {
        let temp = TempDir::new("dir-existing");
        let dir = temp.0.join("named");
        fs::create_dir(&dir).unwrap();
        ensure(&dir).unwrap();
        assert!(!private(&dir));
        ensure_with(&dir, true).unwrap();
        assert!(private(&dir));
    }

    #[test]
    fn a_file_is_refused_and_a_failed_create_names_the_directory() {
        let temp = TempDir::new("dir-bad");
        let file = temp.0.join("file");
        fs::write(&file, b"").unwrap();
        let error = ensure(&file).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("not a directory"), "{error}");
        // A directory can't be made under a file.
        let under = file.join("dir");
        let error = ensure(&under).unwrap_err();
        let message = error.to_string();
        assert!(message.starts_with("can't ") && message.contains("file"), "{message}");
    }

    #[test]
    fn the_directory_comes_from_the_environment() {
        // The variables are whatever this test process has: with
        // BRINDLE_SERVER_DIR the path is that one, else it is the default.
        let dir = resolve_from_env().unwrap();
        assert!(dir.is_absolute());
        if std::env::var_os("BRINDLE_SERVER_DIR").is_none_or(|value| value.is_empty()) {
            assert_eq!(Some(dir), default_dir(win::is_elevated().unwrap()));
        }
    }
}
