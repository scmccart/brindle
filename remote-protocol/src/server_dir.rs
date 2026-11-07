//! Where the terminal server's directory is, shared by `brindle-remote` and
//! Brindle's client so both resolve the same one.
//!
//! It is `$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle`, else
//! `~/.cache/brindle/run`. The server creates it (mode 0700); a client only
//! needs to know where `core.sock` is.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

/// The core's socket, in the server directory.
pub const CORE_SOCKET: &str = "core.sock";
/// The core's log, in the server directory.
pub const CORE_LOG: &str = "core.log";

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
