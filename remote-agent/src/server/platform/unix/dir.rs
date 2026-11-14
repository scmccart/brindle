//! The server directory on Linux: `$BRINDLE_SERVER_DIR`, else
//! `$XDG_RUNTIME_DIR/brindle`, else `~/.cache/brindle/run`, created with
//! mode 0700. An existing directory is used only when it belongs to this
//! user and no one else can enter it.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// Shared with Brindle's client, so both find the same directory.
use brindle_remote_protocol::server_dir::resolve;

/// Where the server directory for this process's environment is, without
/// creating it.
pub fn resolve_from_env() -> io::Result<PathBuf> {
    resolve(
        std::env::var_os("BRINDLE_SERVER_DIR"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        crate::pty::home_dir(&Default::default()),
    )
}

pub(super) fn own_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// Creates `dir` (and missing parents) for this user only, or checks that
/// the existing one is.
pub fn ensure(dir: &Path) -> io::Result<()> {
    ensure_for(dir, own_uid())
}

/// Creates `dir` (and missing parents) with mode 0700, or checks that the
/// existing one is a real directory owned by `uid` with no group or other
/// permissions.
fn ensure_for(dir: &Path, uid: u32) -> io::Result<()> {
    match DirBuilder::new().recursive(true).mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "can't create the server directory {}: {error}",
                    dir.display()
                ),
            ));
        }
    }
    let metadata = fs::symlink_metadata(dir)?;
    let refuse = |problem: String| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing the server directory {}: {problem}", dir.display()),
        ))
    };
    if !metadata.is_dir() {
        return refuse("it is not a directory".into());
    }
    if metadata.uid() != uid {
        return refuse(format!("it belongs to uid {}, not {uid}", metadata.uid()));
    }
    let mode = metadata.mode() & 0o7777;
    if mode & 0o077 != 0 {
        return refuse(format!("its mode is {mode:04o}; it must be 0700"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::server::dir::tests::TempDir;

    #[test]
    fn a_new_directory_is_user_only() {
        let temp = TempDir::new("dir-new");
        let dir = temp.0.join("a/brindle");
        ensure(&dir).unwrap();
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // Existing and correct is fine.
        ensure(&dir).unwrap();
    }

    #[test]
    fn wider_modes_other_owners_and_files_are_refused() {
        let temp = TempDir::new("dir-bad");
        let dir = temp.0.join("open");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o750)).unwrap();
        let error = ensure(&dir).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("0750"), "{error}");

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let error = ensure_for(&dir, own_uid() + 1).unwrap_err();
        assert!(error.to_string().contains("belongs to uid"), "{error}");

        let file = temp.0.join("file");
        fs::write(&file, b"").unwrap();
        assert!(ensure(&file).is_err());

        let link = temp.0.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let error = ensure(&link).unwrap_err();
        assert!(error.to_string().contains("not a directory"), "{error}");
    }
}
