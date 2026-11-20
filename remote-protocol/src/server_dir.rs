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
//! the helper runs from a per-build copy, an elevated process's from copies
//! of its own ([`helper_copies`]). A directory an elevated process doesn't
//! trust is set aside under a name of its own ([`untrusted_name`]). The
//! Windows functions here only compute names and paths from their
//! arguments, so they are tested on every platform. So are the rules the
//! platforms' seams apply to what they read:
//! which names are pipe names ([`check_pipe_name`]), which files show a
//! Linux holder ([`holder_socket_stem`]), and which helper copy is current,
//! how a lost race to place one ends and which builds a prune removes
//! ([`after_lost_rename`], [`pruned_builds`], [`after_image_removal`]).

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The core's socket, in the server directory.
pub const CORE_SOCKET: &str = "core.sock";
/// The core's log, in the server directory.
pub const CORE_LOG: &str = "core.log";

/// Which directory is the server's, from the variables' values (empty counts
/// as unset). The path comes back as given, so a relative variable gives a
/// relative path; [`resolve`] makes it absolute.
pub fn choose(
    server_dir: Option<OsString>,
    runtime_dir: Option<OsString>,
    home: Option<PathBuf>,
) -> io::Result<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    if let Some(dir) = set(server_dir) {
        Ok(PathBuf::from(dir))
    } else if let Some(runtime) = set(runtime_dir) {
        Ok(PathBuf::from(runtime).join("brindle"))
    } else if let Some(home) = home.filter(|home| !home.as_os_str().is_empty()) {
        Ok(home.join(".cache/brindle/run"))
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no server directory: BRINDLE_SERVER_DIR, XDG_RUNTIME_DIR and HOME are all unset",
        ))
    }
}

/// Where the server directory is: [`choose`]'s path made absolute, since the
/// server changes its working directory to `/`. Reads the working directory.
pub fn resolve(
    server_dir: Option<OsString>,
    runtime_dir: Option<OsString>,
    home: Option<PathBuf>,
) -> io::Result<PathBuf> {
    choose(server_dir, runtime_dir, home).and_then(std::path::absolute)
}

/// Which directory is the server's on Windows, from the variables' values
/// (empty counts as unset). `BRINDLE_SERVER_DIR` is used as given, elevated
/// or not; otherwise the directory is under `%LOCALAPPDATA%\brindle`, and an
/// elevated process gets its own, `run-elevated`. The path comes back as
/// given; [`resolve_windows`] makes it absolute.
pub fn choose_windows(
    server_dir: Option<OsString>,
    local_app_data: Option<OsString>,
    elevated: bool,
) -> io::Result<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    if let Some(dir) = set(server_dir) {
        Ok(PathBuf::from(dir))
    } else if let Some(base) = set(local_app_data) {
        Ok(PathBuf::from(base)
            .join("brindle")
            .join(if elevated { "run-elevated" } else { "run" }))
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no server directory: BRINDLE_SERVER_DIR and LOCALAPPDATA are both unset",
        ))
    }
}

/// Where the server directory is on Windows: [`choose_windows`]'s path made
/// absolute. Reads the working directory (and its drive).
pub fn resolve_windows(
    server_dir: Option<OsString>,
    local_app_data: Option<OsString>,
    elevated: bool,
) -> io::Result<PathBuf> {
    choose_windows(server_dir, local_app_data, elevated).and_then(std::path::absolute)
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

/// What every pipe name starts with.
pub const PIPE_PREFIX: &str = r"\\.\pipe\";

/// The most characters (UTF-16 units) a pipe name has, prefix included.
pub const MAX_NAME: usize = 256;

/// Why a path is not a pipe name a server can listen at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadPipeName {
    /// It doesn't start with [`PIPE_PREFIX`].
    NoPrefix,
    /// The name after the prefix is empty, or has a backslash or a NUL.
    BadName,
    /// It is longer than [`MAX_NAME`].
    TooLong,
}

impl fmt::Display for BadPipeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoPrefix => r"it doesn't start with \\.\pipe\",
            Self::BadName => "the name after the prefix is empty or has a backslash",
            Self::TooLong => "it is longer than 256 characters",
        })
    }
}

impl std::error::Error for BadPipeName {}

/// Whether `name` is a pipe name a server can listen at: [`PIPE_PREFIX`]
/// and a name without a backslash or a NUL, at most [`MAX_NAME`]
/// characters in all. It is text, read the same way on every platform.
pub fn check_pipe_name(name: &str) -> Result<(), BadPipeName> {
    let after = name
        .strip_prefix(PIPE_PREFIX)
        .ok_or(BadPipeName::NoPrefix)?;
    if after.is_empty() || after.contains(['\\', '\0']) {
        return Err(BadPipeName::BadName);
    }
    if name.encode_utf16().count() > MAX_NAME {
        return Err(BadPipeName::TooLong);
    }
    Ok(())
}

/// Where the per-build helper copies live: `<server_dir>\bin` when
/// `BRINDLE_SERVER_DIR` was given, elevated or not, as that directory is
/// used as given ([`choose_windows`]); else under `<local_app_data>\brindle`,
/// in `server`, or in `server-elevated` for an elevated process, so the
/// copies an elevated server runs are never the ones an unelevated process
/// wrote.
pub fn helper_copies(server_dir: Option<&Path>, local_app_data: &Path, elevated: bool) -> PathBuf {
    match server_dir {
        Some(dir) => dir.join("bin"),
        None => local_app_data
            .join("brindle")
            .join(if elevated { "server-elevated" } else { "server" }),
    }
}

/// The name a directory an elevated process doesn't trust is set aside
/// under, beside where it was: `<name>.untrusted-<pid>-<n>`, by process
/// `pid`'s `n`th, so no two set aside share one. A fresh directory then
/// takes the name.
pub fn untrusted_name(name: &str, pid: u32, n: u64) -> String {
    format!("{name}{UNTRUSTED}{pid}-{n}")
}

/// What follows a directory's own name in the name it is set aside under
/// ([`untrusted_name`]).
const UNTRUSTED: &str = ".untrusted-";

/// Whether `name` is one [`untrusted_name`] gave: a directory set aside,
/// which a later start may remove.
pub fn is_untrusted_name(name: &str) -> bool {
    let Some((original, suffix)) = name.rsplit_once(UNTRUSTED) else {
        return false;
    };
    let number = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    !original.is_empty()
        && suffix
            .split_once('-')
            .is_some_and(|(pid, n)| number(pid) && number(n))
}

/// The copy of the helper for one build, in `copies` (see [`helper_copies`]).
pub fn helper_copy(copies: &Path, build: &str) -> PathBuf {
    copies.join(build).join("brindle-remote.exe")
}

/// The temporary name a copy is written under, beside where it goes, by
/// process `pid`'s `n`th copy, so no two copies in flight share one.
pub fn copy_temp_name(pid: u32, n: u64) -> String {
    format!("brindle-remote.exe.{pid}-{n}.tmp")
}

/// Whether a copy whose build id is `copy_build` (`None`: it can't be read)
/// is the copy of `build`.
pub fn is_current(copy_build: Option<&str>, build: &str) -> bool {
    copy_build == Some(build)
}

/// What a copy does after its rename into place failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostRename {
    /// Another process won the rename with the same build: use its file.
    UseWinner,
    /// Something else is under the copy's name: replace it.
    Replace,
    /// Nothing is there: the rename's own error stands.
    Fail,
}

/// What follows a failed rename of a copy of `build`, from what is under the
/// copy's name now: its build id (`None` when it can't be read) and whether
/// anything is there.
pub fn after_lost_rename(copy_build: Option<&str>, build: &str, copy_exists: bool) -> LostRename {
    if is_current(copy_build, build) {
        LostRename::UseWinner
    } else if copy_exists {
        LostRename::Replace
    } else {
        LostRename::Fail
    }
}

/// What an entry of the copies' directory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A directory: one build's copy.
    Dir,
    /// Anything else, or an entry whose kind can't be read.
    Other,
}

/// The build directories a prune removes, in the order listed: every
/// directory but `keep`'s.
pub fn pruned_builds(entries: Vec<(OsString, EntryKind)>, keep: &str) -> Vec<OsString> {
    entries
        .into_iter()
        .filter(|(name, kind)| *kind == EntryKind::Dir && name != keep)
        .map(|(name, _)| name)
        .collect()
}

/// How removing a build's image, its `brindle-remote.exe`, went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRemoval {
    Removed,
    /// There was none.
    Missing,
    /// It couldn't be removed: Windows refuses to delete a running image.
    Failed,
}

/// What a prune does with a build's directory once its image is dealt
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneDir {
    Remove,
    /// Left whole for a later prune.
    Keep,
}

/// Whether a prune removes a build's directory after `removal` of its
/// image: a build that still has a role running keeps its directory whole.
pub fn after_image_removal(removal: ImageRemoval) -> PruneDir {
    match removal {
        ImageRemoval::Removed | ImageRemoval::Missing => PruneDir::Remove,
        ImageRemoval::Failed => PruneDir::Keep,
    }
}

/// Splits a holder's lock file name, `holder-<build>-<n>.lock`, into the
/// build and the holder's number ([`parse_holder_stem`]).
pub fn parse_holder_lock(name: &str) -> Option<(String, u32)> {
    parse_holder_stem(name.strip_suffix(".lock")?)
}

/// Splits the stem a holder's files share, `holder-<build>-<n>`, into the
/// build and the holder's number. The build is everything between the prefix
/// and the last `-`, so it may contain dashes itself.
pub fn parse_holder_stem(stem: &str) -> Option<(String, u32)> {
    let (build, n) = stem.strip_prefix("holder-")?.rsplit_once('-')?;
    if build.is_empty() || n.is_empty() || !n.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((build.to_string(), n.parse().ok()?))
}

/// The stem of a holder's socket file name on Linux, `holder-….sock`, which
/// its lock file and log share: `holder-b1-0.sock` gives `holder-b1-0`.
/// `None` for any other name.
pub fn holder_socket_stem(name: &str) -> Option<&str> {
    name.strip_suffix(".sock")
        .filter(|stem| stem.starts_with("holder-"))
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
            choose(os("/srv/b"), os("/run/user/1"), home.clone()).unwrap(),
            PathBuf::from("/srv/b")
        );
        assert_eq!(
            choose(None, os("/run/user/1"), home.clone()).unwrap(),
            PathBuf::from("/run/user/1").join("brindle")
        );
        assert_eq!(
            choose(os(""), os(""), home.clone()).unwrap(),
            PathBuf::from("/home/u").join(".cache/brindle/run")
        );
    }

    #[test]
    fn nothing_set_is_an_error_and_a_relative_directory_comes_back_as_given() {
        assert!(choose(None, None, None).is_err());
        assert!(choose(os(""), os(""), Some(PathBuf::new())).is_err());
        assert_eq!(
            choose(os("rel/dir"), None, None).unwrap(),
            PathBuf::from("rel/dir")
        );
    }

    #[test]
    fn resolving_makes_the_chosen_directory_absolute() {
        let relative = resolve(os("rel/dir"), None, None).unwrap();
        assert!(relative.is_absolute() && relative.ends_with("rel/dir"));
    }

    // Windows-style values are only built with `join`, so these hold on any
    // host.
    #[test]
    fn windows_directory_follows_the_variables() {
        let base = || os(r"C:\Users\u\AppData\Local");
        let local = PathBuf::from(r"C:\Users\u\AppData\Local");
        assert_eq!(
            choose_windows(None, base(), false).unwrap(),
            local.join("brindle").join("run")
        );
        assert_eq!(
            choose_windows(os(""), base(), true).unwrap(),
            local.join("brindle").join("run-elevated")
        );
        // BRINDLE_SERVER_DIR is used as given, even when elevated.
        assert_eq!(
            choose_windows(os("rel/dir"), base(), true).unwrap(),
            PathBuf::from("rel/dir")
        );
        assert_eq!(
            choose_windows(os("srv"), None, true).unwrap(),
            PathBuf::from("srv")
        );
    }

    #[test]
    fn windows_directory_with_nothing_set_names_both_variables() {
        for local in [None, os("")] {
            let message = choose_windows(None, local, false).unwrap_err().to_string();
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
        // Without `BRINDLE_SERVER_DIR`: an elevated process has its own.
        assert_eq!(
            helper_copies(None, local, false),
            local.join("brindle").join("server")
        );
        assert_eq!(
            helper_copies(None, local, true),
            local.join("brindle").join("server-elevated")
        );
        // With it, the directory is used as given, elevated or not.
        assert_eq!(helper_copies(Some(dir), local, false), dir.join("bin"));
        assert_eq!(helper_copies(Some(dir), local, true), dir.join("bin"));
        let copies = helper_copies(Some(dir), local, false);
        assert_eq!(
            helper_copy(&copies, "b1"),
            dir.join("bin").join("b1").join("brindle-remote.exe")
        );
        assert_eq!(
            helper_copy(&helper_copies(None, local, true), "b1"),
            local
                .join("brindle")
                .join("server-elevated")
                .join("b1")
                .join("brindle-remote.exe")
        );
    }

    /// An elevated process's directories are beside the unelevated ones,
    /// never the same: its server directory and its helper copies.
    #[test]
    fn an_elevated_process_shares_no_directory_with_an_unelevated_one() {
        let base = || os(r"C:\Users\u\AppData\Local");
        let local = PathBuf::from(r"C:\Users\u\AppData\Local");
        assert_ne!(
            choose_windows(None, base(), true).unwrap(),
            choose_windows(None, base(), false).unwrap()
        );
        assert_ne!(
            helper_copies(None, &local, true),
            helper_copies(None, &local, false)
        );
    }

    #[test]
    fn a_directory_is_set_aside_under_its_name_a_pid_and_a_count() {
        assert_eq!(
            untrusted_name("server-elevated", 4242, 0),
            "server-elevated.untrusted-4242-0"
        );
        assert_eq!(untrusted_name("1.25.0", 7, 12), "1.25.0.untrusted-7-12");
        assert_ne!(untrusted_name("bin", 7, 1), untrusted_name("bin", 8, 1));
        assert_ne!(untrusted_name("bin", 7, 1), untrusted_name("bin", 7, 2));
        // One set aside again keeps both marks, and is still one.
        let twice = untrusted_name(&untrusted_name("bin", 7, 1), 9, 0);
        assert_eq!(twice, "bin.untrusted-7-1.untrusted-9-0");
        assert!(is_untrusted_name(&twice));
    }

    #[test]
    fn only_a_set_aside_name_is_one() {
        for name in [
            "server-elevated.untrusted-4242-0",
            "bin.untrusted-1-18446744073709551615",
            "1.25.0.untrusted-7-12",
            "a.b.untrusted-0-0",
        ] {
            assert!(is_untrusted_name(name), "{name}");
        }
        for name in [
            "server-elevated",
            "server",
            ".untrusted-1-2",
            "bin.untrusted-",
            "bin.untrusted-1",
            "bin.untrusted-1-",
            "bin.untrusted--2",
            "bin.untrusted-x-2",
            "bin.untrusted-1-2x",
            "bin.untrusted-1-2-3",
            "bin.UNTRUSTED-1-2",
            "bin.untrusted-1-2.tmp",
            "",
        ] {
            assert!(!is_untrusted_name(name), "{name:?}");
        }
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

    #[test]
    fn a_holder_socket_gives_its_stem() {
        assert_eq!(
            holder_socket_stem("holder-0.3.1-abc-2.sock"),
            Some("holder-0.3.1-abc-2")
        );
        // Any name after the prefix, as the directory shows it.
        assert_eq!(holder_socket_stem("holder-.sock"), Some("holder-"));
        for other in [
            "holder-b1-0.lock",
            "holder-b1-0.log",
            "core.sock",
            "xholder-b1-0.sock",
        ] {
            assert_eq!(holder_socket_stem(other), None, "{other}");
        }
    }

    #[test]
    fn a_holders_stem_gives_its_build_and_number() {
        assert_eq!(parse_holder_stem("holder-b1-0"), Some(("b1".to_string(), 0)));
        assert_eq!(
            parse_holder_stem("holder-0.3.1-abc-12"),
            Some(("0.3.1-abc".to_string(), 12))
        );
        for other in ["holder-odd", "holder-", "holder--0", "holder-b1-", "holder-b1-0.lock", "b1-0"] {
            assert_eq!(parse_holder_stem(other), None, "{other}");
        }
    }

    #[test]
    fn a_pipe_name_needs_the_prefix_a_plain_name_and_at_most_256_characters() {
        assert_eq!(
            check_pipe_name(r"\\.\pipe\brindle-server-0123456789abcdef-core"),
            Ok(())
        );
        let cases = [
            (r"C:\Users\me\run\core.sock", BadPipeName::NoPrefix),
            (r"\\server\pipe\name", BadPipeName::NoPrefix),
            (r"\\.\pipe\", BadPipeName::BadName),
            (r"\\.\pipe\a\b", BadPipeName::BadName),
            ("\\\\.\\pipe\\a\0b", BadPipeName::BadName),
        ];
        for (name, problem) in cases {
            assert_eq!(check_pipe_name(name), Err(problem), "{name:?}");
        }
        let longest = format!(r"\\.\pipe\{}", "n".repeat(MAX_NAME - PIPE_PREFIX.len()));
        assert_eq!(longest.len(), 256);
        assert_eq!(check_pipe_name(&longest), Ok(()));
        assert_eq!(
            check_pipe_name(&format!("{longest}n")),
            Err(BadPipeName::TooLong)
        );
        // Characters are UTF-16 units, not bytes.
        let wide = format!(r"\\.\pipe\{}", "é".repeat(MAX_NAME - PIPE_PREFIX.len()));
        assert_eq!(check_pipe_name(&wide), Ok(()));
        let beyond = format!(r"\\.\pipe\{}", "😀".repeat(124));
        assert_eq!(check_pipe_name(&beyond), Err(BadPipeName::TooLong));

        assert_eq!(
            BadPipeName::NoPrefix.to_string(),
            r"it doesn't start with \\.\pipe\"
        );
        assert_eq!(
            BadPipeName::BadName.to_string(),
            "the name after the prefix is empty or has a backslash"
        );
        assert_eq!(
            BadPipeName::TooLong.to_string(),
            "it is longer than 256 characters"
        );
    }

    #[test]
    fn every_pipe_the_server_names_is_a_pipe_name() {
        let dir = Path::new(r"C:\Users\someone\AppData\Local\brindle\run-elevated");
        let sid = "S-1-5-21-1234567890-1234567890-1234567890-1001";
        let build = "b".repeat(64);
        for name in [
            core_pipe(dir, sid),
            holder_pipe(dir, sid, &build, u32::MAX),
            pty_pipe(u32::MAX, u64::MAX),
        ] {
            assert_eq!(check_pipe_name(&name), Ok(()), "{name}");
        }
    }

    #[test]
    fn the_temporary_copy_is_named_by_pid_and_count() {
        assert_eq!(copy_temp_name(4242, 0), "brindle-remote.exe.4242-0.tmp");
        assert_eq!(copy_temp_name(7, 12), "brindle-remote.exe.7-12.tmp");
        assert_ne!(copy_temp_name(7, 1), copy_temp_name(8, 1));
    }

    #[test]
    fn a_lost_rename_uses_the_winner_replaces_another_file_or_fails() {
        assert!(is_current(Some("b1"), "b1"));
        assert!(!is_current(Some("b2"), "b1"));
        assert!(!is_current(None, "b1"));

        // Another process put the same build there first.
        assert_eq!(
            after_lost_rename(Some("b1"), "b1", true),
            LostRename::UseWinner
        );
        // Something else is under the copy's name: a cut-short file, or one
        // that can't be read.
        assert_eq!(
            after_lost_rename(Some("b2"), "b1", true),
            LostRename::Replace
        );
        assert_eq!(after_lost_rename(None, "b1", true), LostRename::Replace);
        // Nothing is there, so the rename failed for another reason.
        assert_eq!(after_lost_rename(None, "b1", false), LostRename::Fail);
    }

    #[test]
    fn pruning_keeps_the_current_build_and_skips_files() {
        let entries = vec![
            (OsString::from("old"), EntryKind::Dir),
            (OsString::from("kept"), EntryKind::Dir),
            (OsString::from("a-file"), EntryKind::Other),
            (OsString::from("older"), EntryKind::Dir),
            // A file named like the build is skipped too.
            (OsString::from("kept"), EntryKind::Other),
        ];
        assert_eq!(
            pruned_builds(entries, "kept"),
            [OsString::from("old"), OsString::from("older")]
        );
        assert_eq!(pruned_builds(Vec::new(), "kept"), Vec::<OsString>::new());
    }

    #[test]
    fn a_running_image_keeps_its_build_directory() {
        assert_eq!(after_image_removal(ImageRemoval::Removed), PruneDir::Remove);
        assert_eq!(after_image_removal(ImageRemoval::Missing), PruneDir::Remove);
        // Windows refuses to delete a running image: the directory stays
        // whole for a later prune.
        assert_eq!(after_image_removal(ImageRemoval::Failed), PruneDir::Keep);
    }
}
