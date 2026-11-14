//! Where the terminal server's directory is, shared by `brindle-remote` and
//! Brindle's client so both resolve the same one.
//!
//! On Linux it is `$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle`, else
//! `~/.cache/brindle/run`. The server creates it (mode 0700); a client only
//! needs to know where `core.sock` is.
//!
//! On Windows it is `$BRINDLE_SERVER_DIR`, else `%LOCALAPPDATA%\brindle\run`
//! (`run-elevated` for an elevated process, so the two never share a server).
//! The directory holds lock files and logs; the core and the holders listen
//! on named pipes whose names come from the directory and the user's SID, and
//! the helper runs from a per-build copy. The Windows functions here only
//! compute names and paths from their arguments, so they are tested on every
//! platform.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

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

/// Where the server directory is on Windows, from the variables' values
/// (empty counts as unset). `BRINDLE_SERVER_DIR` is used as given, elevated
/// or not; otherwise the directory is under `%LOCALAPPDATA%\brindle`, and an
/// elevated process gets its own, `run-elevated`. Relative paths are made
/// absolute.
pub fn resolve_windows(
    server_dir: Option<OsString>,
    local_app_data: Option<OsString>,
    elevated: bool,
) -> io::Result<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    let dir = if let Some(dir) = set(server_dir) {
        PathBuf::from(dir)
    } else if let Some(base) = set(local_app_data) {
        PathBuf::from(base)
            .join("brindle")
            .join(if elevated { "run-elevated" } else { "run" })
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no server directory: BRINDLE_SERVER_DIR and LOCALAPPDATA are both unset",
        ));
    };
    std::path::absolute(dir)
}

/// FNV-1a (64-bit) over the directory's encoded bytes, then the SID's, so
/// every Brindle version names a server's pipes the same way. Another
/// directory or another user gets other names.
fn pipe_hash(dir: &Path, user_sid: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let bytes = dir.as_os_str().as_encoded_bytes().iter();
    for byte in bytes.chain(user_sid.as_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The core's named pipe. Callers pass the canonical server directory.
pub fn core_pipe(dir: &Path, user_sid: &str) -> String {
    format!(
        r"\\.\pipe\brindle-server-{:016x}-core",
        pipe_hash(dir, user_sid)
    )
}

/// The named pipe of holder number `n` of build `build`.
pub fn holder_pipe(dir: &Path, user_sid: &str, build: &str, n: u32) -> String {
    format!(
        r"\\.\pipe\brindle-server-{:016x}-holder-{build}-{n}",
        pipe_hash(dir, user_sid)
    )
}

/// A pipe a holder gives one session's pseudoconsole host, `n` counting
/// that holder's pipes: each session has one for input and one for output.
pub fn pty_pipe(holder_pid: u32, n: u64) -> String {
    format!(r"\\.\pipe\brindle-pty-{holder_pid}-{n}")
}

/// Where the per-build helper copies live: `<server_dir>\bin` when
/// `BRINDLE_SERVER_DIR` was given, else `<local_app_data>\brindle\server`.
pub fn helper_copies(server_dir: Option<&Path>, local_app_data: &Path) -> PathBuf {
    match server_dir {
        Some(dir) => dir.join("bin"),
        None => local_app_data.join("brindle").join("server"),
    }
}

/// The copy of the helper for one build, in `copies` (see [`helper_copies`]).
pub fn helper_copy(copies: &Path, build: &str) -> PathBuf {
    copies.join(build).join("brindle-remote.exe")
}

/// Splits a holder's lock file name, `holder-<build>-<n>.lock`, into the
/// build and the holder's number. The build is everything between the prefix
/// and the last `-`, so it may contain dashes itself.
pub fn parse_holder_lock(name: &str) -> Option<(String, u32)> {
    let stem = name.strip_prefix("holder-")?.strip_suffix(".lock")?;
    let (build, n) = stem.rsplit_once('-')?;
    if build.is_empty() || n.is_empty() || !n.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((build.to_string(), n.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    // Unix paths: on Windows, `std::path::absolute` adds the current drive.
    #[cfg(unix)]
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
    }

    #[test]
    fn nothing_set_is_an_error_and_relative_paths_become_absolute() {
        assert!(resolve(None, None, None).is_err());
        let relative = resolve(os("rel/dir"), None, None).unwrap();
        assert!(relative.is_absolute() && relative.ends_with("rel/dir"));
    }

    // Windows-style values are only compared as strings or built with `join`,
    // so these hold on any host; `resolve_windows` is checked through
    // `ends_with`, since `std::path::absolute` differs per platform.
    #[test]
    fn windows_directory_follows_the_variables() {
        let base = || os(r"C:\Users\u\AppData\Local");
        let plain = resolve_windows(None, base(), false).unwrap();
        assert!(plain.ends_with(Path::new("brindle").join("run")));
        let elevated = resolve_windows(os(""), base(), true).unwrap();
        assert!(elevated.ends_with(Path::new("brindle").join("run-elevated")));
        // BRINDLE_SERVER_DIR is used as given, even when elevated.
        let named = resolve_windows(os("rel/dir"), base(), true).unwrap();
        assert!(named.is_absolute() && named.ends_with("rel/dir"));
        let named = resolve_windows(os("srv"), None, true).unwrap();
        assert!(named.ends_with("srv"));
    }

    #[test]
    fn windows_directory_with_nothing_set_names_both_variables() {
        for local in [None, os("")] {
            let message = resolve_windows(None, local, false).unwrap_err().to_string();
            assert!(message.contains("BRINDLE_SERVER_DIR") && message.contains("LOCALAPPDATA"));
        }
    }

    #[test]
    fn pipe_names_are_pinned() {
        // A changed hash would strand running servers, so these values must
        // not change with the code.
        let dir = Path::new(r"C:\Users\u\AppData\Local\brindle\run");
        let sid = "S-1-5-21-1-2-3-1001";
        assert_eq!(pipe_hash(dir, sid), 0x87e5_2c06_456c_d5bd);
        assert_eq!(
            core_pipe(dir, sid),
            r"\\.\pipe\brindle-server-87e52c06456cd5bd-core"
        );
        assert_eq!(
            holder_pipe(dir, sid, "0.3.1-abc", 2),
            r"\\.\pipe\brindle-server-87e52c06456cd5bd-holder-0.3.1-abc-2"
        );
        assert_eq!(pty_pipe(4242, 7), r"\\.\pipe\brindle-pty-4242-7");
    }

    #[test]
    fn pipe_names_depend_on_the_directory_and_the_user() {
        let dir = Path::new(r"C:\a");
        let name = core_pipe(dir, "S-1-5-1");
        assert_eq!(name, core_pipe(dir, "S-1-5-1"));
        assert_ne!(name, core_pipe(Path::new(r"C:\b"), "S-1-5-1"));
        assert_ne!(name, core_pipe(dir, "S-1-5-2"));
        let hash = name.trim_start_matches(r"\\.\pipe\brindle-server-");
        assert_eq!(hash.find('-'), Some(16));
    }

    #[test]
    fn helper_copies_live_beside_the_server_or_under_local_app_data() {
        let local = Path::new("local");
        let dir = Path::new("srv");
        assert_eq!(helper_copies(Some(dir), local), dir.join("bin"));
        assert_eq!(
            helper_copies(None, local),
            local.join("brindle").join("server")
        );
        let copies = helper_copies(Some(dir), local);
        assert_eq!(
            helper_copy(&copies, "b1"),
            dir.join("bin").join("b1").join("brindle-remote.exe")
        );
    }

    #[test]
    fn holder_lock_names_parse_back() {
        assert_eq!(
            parse_holder_lock("holder-b1-0.lock"),
            Some(("b1".to_string(), 0))
        );
        assert_eq!(
            parse_holder_lock("holder-0.3.1-abc-12.lock"),
            Some(("0.3.1-abc".to_string(), 12))
        );
        for bad in [
            "holder-b1-0.sock",
            "holder-b1.lock",
            "holder--0.lock",
            "holder-b1-.lock",
            "holder-b1-x.lock",
            "holder-b1-+1.lock",
            "holder-b1-99999999999.lock",
            "core.lock",
            "xholder-b1-0.lock",
        ] {
            assert_eq!(parse_holder_lock(bad), None, "{bad}");
        }
    }
}
