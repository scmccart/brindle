//! The attachment store's file work on Linux (OpenSpec change
//! `agents-prompt-context`, task 1.6a): directories private to the user
//! with the server directory's checks (a real directory, the user's own,
//! mode 0700), texts written once and read-only (0400), and the reads,
//! removals and listings the core's store plan asks for
//! (`server/core/attachments.rs` decides; this only does).

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::UNIX_EPOCH;

use super::dir::own_uid;
use crate::server::platform::Listed;

/// Whether paths on this host are Windows paths.
pub const WINDOWS: bool = false;

/// The home an agent run with `env` has, which a link's and an
/// attachments directory's `~` stand for; with an empty `env`, the core's.
pub fn agent_home(env: &BTreeMap<String, String>) -> Option<String> {
    crate::pty::home_dir(env).map(|home| home.to_string_lossy().into_owned())
}

/// Creates `dir` (and missing parents) with mode 0700, or checks that the
/// existing one is a real directory of this user's that no one else can
/// enter. The error says why, for the client.
pub fn make_private(dir: &Path) -> io::Result<()> {
    make_private_for(dir, own_uid())
}

fn make_private_for(dir: &Path, uid: u32) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let metadata = fs::symlink_metadata(dir)?;
    let refuse = |why: String| Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
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

/// Writes `text` to `file`, which must not exist yet, read-only (0400).
pub fn write_once(file: &Path, text: &[u8]) -> io::Result<()> {
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(libc::O_NOFOLLOW)
        .open(file)?;
    out.write_all(text)?;
    out.sync_data()
}

/// Writes a store's marker, replacing one there.
pub fn write_marker(file: &Path, text: &str) -> io::Result<()> {
    fs::write(file, text)
}

/// Reads at most `limit` bytes of `file`, refusing a link.
pub fn read(file: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(file)?;
    let mut bytes = Vec::new();
    input.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Removes a kept text; one already gone is removed.
pub fn remove(file: &Path) -> io::Result<()> {
    match fs::remove_file(file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Removes a store directory and what is in it; one already gone is
/// removed.
pub fn remove_dir(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// What `dir` holds, links not followed; nothing when it doesn't exist.
pub fn list(dir: &Path) -> io::Result<Vec<Listed>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut listed = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs());
        listed.push(Listed {
            name: entry.file_name().to_string_lossy().into_owned(),
            dir: metadata.is_dir(),
            bytes: metadata.len(),
            modified,
        });
    }
    Ok(listed)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::server::dir::tests::TempDir;

    #[test]
    fn a_store_directory_is_private_and_a_wider_or_linked_one_is_refused() {
        let temp = TempDir::new("store-dir");
        let dir = temp.0.join("cache/attachments/s");
        make_private(&dir).unwrap();
        for made in [&temp.0.join("cache"), &temp.0.join("cache/attachments"), &dir] {
            assert_eq!(fs::metadata(made).unwrap().permissions().mode() & 0o777, 0o700);
        }
        make_private(&dir).unwrap();
        let open = temp.0.join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(make_private(&open).unwrap_err().to_string().contains("0755"));
        assert!(
            make_private_for(&dir, own_uid() + 1)
                .unwrap_err()
                .to_string()
                .contains("belongs to uid")
        );
        let link = temp.0.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(make_private(&link).unwrap_err().to_string().contains("not a directory"));
    }

    #[test]
    fn a_text_is_written_once_read_only_read_back_and_removed() {
        let temp = TempDir::new("store-file");
        let file = temp.0.join("1");
        write_once(&file, b"ok\n").unwrap();
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o400);
        assert_eq!(write_once(&file, b"again").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&file, 100).unwrap(), b"ok\n");
        assert_eq!(read(&file, 2).unwrap(), b"ok");
        let link = temp.0.join("2");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read(&link, 100).is_err(), "a link isn't read");
        assert!(write_once(&temp.0.join("3"), b"x").is_ok());
        let listed = list(&temp.0).unwrap();
        let mut names: Vec<_> = listed.iter().map(|l| l.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["1", "2", "3"]);
        assert!(listed.iter().all(|l| !l.dir && l.modified.is_some()));
        remove(&file).unwrap();
        remove(&file).unwrap();
        assert!(read(&file, 100).is_err());
        write_marker(&temp.0.join("expires"), "1\n").unwrap();
        write_marker(&temp.0.join("expires"), "2\n").unwrap();
        assert_eq!(fs::read_to_string(temp.0.join("expires")).unwrap(), "2\n");
        remove_dir(&temp.0).unwrap();
        remove_dir(&temp.0).unwrap();
        assert!(list(&temp.0).unwrap().is_empty());
    }
}
