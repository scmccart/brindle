//! The server directory: where the core's and holders' sockets, locks and
//! logs live.
//!
//! It is `$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle`, else
//! `~/.cache/brindle/run`, created with mode 0700. An existing directory is
//! used only when it belongs to this user and no one else can enter it.

use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// Where the server directory is, from the variables' values (empty counts
/// as unset). Relative paths are made absolute, since the server changes
/// its working directory to `/`.
pub fn resolve(
    server_dir: Option<OsString>,
    runtime_dir: Option<OsString>,
    home: Option<PathBuf>,
) -> io::Result<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    let dir = if let Some(dir) = set(server_dir) {
        PathBuf::from(dir)
    } else if let Some(runtime) = set(runtime_dir) {
        PathBuf::from(runtime).join("brindle")
    } else if let Some(home) = home.filter(|home| !home.as_os_str().is_empty()) {
        home.join(".cache/brindle/run")
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no server directory: BRINDLE_SERVER_DIR, XDG_RUNTIME_DIR and HOME are all unset",
        ));
    };
    std::path::absolute(dir)
}

/// The server directory for this process's environment, created if needed.
pub fn from_env() -> io::Result<PathBuf> {
    let dir = resolve(
        std::env::var_os("BRINDLE_SERVER_DIR"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        crate::pty::home_dir(&Default::default()),
    )?;
    ensure(&dir, own_uid())?;
    Ok(dir)
}

pub fn own_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// Creates `dir` (and missing parents) with mode 0700, or checks that the
/// existing one is a real directory owned by `uid` with no group or other
/// permissions.
pub fn ensure(dir: &Path, uid: u32) -> io::Result<()> {
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
pub(crate) mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A fresh, empty directory under the system temp dir, removed on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("brindle-unit-{tag}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    #[test]
    fn the_variables_are_used_in_order() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            resolve(os("/srv/b"), os("/run/user/1"), home.clone()).unwrap(),
            PathBuf::from("/srv/b")
        );
        assert_eq!(
            resolve(None, os("/run/user/1"), home.clone()).unwrap(),
            PathBuf::from("/run/user/1/brindle")
        );
        assert_eq!(
            resolve(os(""), os(""), home.clone()).unwrap(),
            PathBuf::from("/home/u/.cache/brindle/run")
        );
        assert!(resolve(None, None, None).is_err());
        let relative = resolve(os("rel/dir"), None, None).unwrap();
        assert!(relative.is_absolute() && relative.ends_with("rel/dir"));
    }

    #[test]
    fn a_new_directory_is_user_only() {
        let temp = TempDir::new("dir-new");
        let dir = temp.0.join("a/brindle");
        ensure(&dir, own_uid()).unwrap();
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // Existing and correct is fine.
        ensure(&dir, own_uid()).unwrap();
    }

    #[test]
    fn wider_modes_other_owners_and_files_are_refused() {
        let temp = TempDir::new("dir-bad");
        let dir = temp.0.join("open");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o750)).unwrap();
        let error = ensure(&dir, own_uid()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("0750"), "{error}");

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let error = ensure(&dir, own_uid() + 1).unwrap_err();
        assert!(error.to_string().contains("belongs to uid"), "{error}");

        let file = temp.0.join("file");
        fs::write(&file, b"").unwrap();
        assert!(ensure(&file, own_uid()).is_err());

        let link = temp.0.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let error = ensure(&link, own_uid()).unwrap_err();
        assert!(error.to_string().contains("not a directory"), "{error}");
    }
}
