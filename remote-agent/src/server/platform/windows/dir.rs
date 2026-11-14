//! The server directory on Windows: `%BRINDLE_SERVER_DIR%`, else
//! `%LOCALAPPDATA%\brindle\run` (`run-elevated` for an elevated process),
//! created with a protected DACL for the user and SYSTEM. It holds locks and
//! logs; the pipes are named from it and protected on their own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Shared with Brindle's client, so both find the same directory.
use brindle_remote_protocol::server_dir::resolve_windows;
use brindle_remote_protocol::win;

/// Where the server directory for this process's environment is, without
/// creating it.
pub fn resolve_from_env() -> io::Result<PathBuf> {
    resolve_windows(
        std::env::var_os("BRINDLE_SERVER_DIR"),
        std::env::var_os("LOCALAPPDATA"),
        win::is_elevated()?,
    )
}

/// The directory used when `BRINDLE_SERVER_DIR` isn't set.
fn default_dir() -> Option<PathBuf> {
    resolve_windows(None, std::env::var_os("LOCALAPPDATA"), win::is_elevated().ok()?).ok()
}

/// Creates `dir` (and missing parents) for this user and SYSTEM only. An
/// existing directory is used as it is, except the default one, whose DACL
/// is set again: a directory someone named with `BRINDLE_SERVER_DIR` is
/// theirs to protect.
pub fn ensure(dir: &Path) -> io::Result<()> {
    ensure_with(dir, default_dir().as_deref() == Some(dir))
}

/// [`ensure`], re-protecting an existing `dir` when `protect_existing`.
fn ensure_with(dir: &Path, protect_existing: bool) -> io::Result<()> {
    let refuse = |problem: &str| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing the server directory {}: {problem}", dir.display()),
        ))
    };
    let failed = |what: &str, error: io::Error| {
        io::Error::new(
            error.kind(),
            format!("can't {what} the server directory {}: {error}", dir.display()),
        )
    };
    match fs::symlink_metadata(dir) {
        // A link (a symlink or a junction) is not a directory here.
        Ok(metadata) if !metadata.is_dir() => refuse("it is not a directory"),
        Ok(_) if protect_existing => win::protect_dir(dir).map_err(|error| failed("protect", error)),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create(dir).map_err(|error| failed("create", error))?;
            // Whoever created it, it must be a directory now.
            match fs::symlink_metadata(dir) {
                Ok(metadata) if metadata.is_dir() => Ok(()),
                Ok(_) => refuse("it is not a directory"),
                Err(error) => Err(failed("create", error)),
            }
        }
        Err(error) => Err(failed("read", error)),
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
            assert_eq!(Some(dir), default_dir());
        }
    }
}
