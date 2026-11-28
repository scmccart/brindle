//! The attachment store's file work on Windows (OpenSpec change
//! `agents-prompt-context`, task 1.6a): directories private to the user
//! and SYSTEM (created with the server directory's protected DACL, or an
//! existing one protected again; a link or junction refused), texts
//! written once and read-only, and the reads, removals and listings the
//! core's store plan asks for (`server/core/attachments.rs` decides; this
//! only does).

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use brindle_remote_protocol::win;

use crate::server::platform::Listed;

/// Whether paths on this host are Windows paths.
pub const WINDOWS: bool = true;

/// `FILE_FLAG_OPEN_REPARSE_POINT`: a link is opened, not followed.
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// The home an agent run with `env` has, which a link's and an
/// attachments directory's `~` stand for: its `USERPROFILE`, else its
/// `HOME`, else this process's `USERPROFILE`; with an empty `env`, the
/// core's.
pub fn agent_home(env: &BTreeMap<String, String>) -> Option<String> {
    ["USERPROFILE", "HOME"]
        .iter()
        .find_map(|name| env.get(*name).filter(|home| !home.is_empty()).cloned())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|home| !home.is_empty()))
}

/// Creates `dir` and each missing parent with the private DACL, or
/// protects an existing directory again; a file, link or junction is
/// refused. The error says why, for the client.
pub fn make_private(dir: &Path) -> io::Result<()> {
    let mut missing: Vec<PathBuf> = Vec::new();
    let mut next = Some(dir);
    while let Some(path) = next.filter(|path| !path.as_os_str().is_empty()) {
        match fs::symlink_metadata(path) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing.push(path.to_path_buf()),
            Err(error) => return Err(error),
        }
        next = path.parent();
    }
    for path in missing.iter().rev() {
        match win::create_private_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let metadata = fs::symlink_metadata(dir)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "it is not a directory",
        ));
    }
    if missing.first().map(PathBuf::as_path) != Some(dir) {
        win::protect_dir(dir)?;
    }
    Ok(())
}

/// Writes `text` to `file`, which must not exist yet, then makes it
/// read-only. It takes the store directory's private DACL.
pub fn write_once(file: &Path, text: &[u8]) -> io::Result<()> {
    let mut out = OpenOptions::new().write(true).create_new(true).open(file)?;
    out.write_all(text)?;
    out.sync_data()?;
    drop(out);
    let mut permissions = fs::metadata(file)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(file, permissions)
}

/// Writes a store's marker, replacing one there.
pub fn write_marker(file: &Path, text: &str) -> io::Result<()> {
    fs::write(file, text)
}

/// Reads at most `limit` bytes of `file`, refusing a link.
pub fn read(file: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(file)?;
    if input.metadata()?.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "it is a link"));
    }
    let mut bytes = Vec::new();
    input.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Removes a kept text, read-only as it is; one already gone is removed.
pub fn remove(file: &Path) -> io::Result<()> {
    match fs::symlink_metadata(file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(metadata) if metadata.permissions().readonly() => {
            let mut permissions = metadata.permissions();
            #[allow(clippy::permissions_set_readonly_false, reason = "Windows: clears the read-only attribute")]
            permissions.set_readonly(false);
            fs::set_permissions(file, permissions)?;
        }
        Ok(_) => {}
    }
    fs::remove_file(file)
}

/// Removes a store directory and what is in it (its read-only texts
/// first made writable); one already gone is removed.
pub fn remove_dir(dir: &Path) -> io::Result<()> {
    for listed in list(dir)? {
        if !listed.dir {
            remove(&dir.join(&listed.name))?;
        }
    }
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
            dir: metadata.is_dir() && !metadata.file_type().is_symlink(),
            bytes: metadata.len(),
            modified,
        });
    }
    Ok(listed)
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
    fn a_store_directory_is_private_and_a_file_is_refused() {
        let temp = TempDir::new("store-dir");
        let dir = temp.0.join("cache\\attachments\\s");
        make_private(&dir).unwrap();
        assert!(private(&dir));
        assert!(private(&temp.0.join("cache\\attachments")));
        make_private(&dir).unwrap();
        let open = temp.0.join("open");
        fs::create_dir(&open).unwrap();
        make_private(&open).unwrap();
        assert!(private(&open), "an existing directory is protected again");
        let file = temp.0.join("file");
        fs::write(&file, b"").unwrap();
        assert!(make_private(&file).is_err());
    }

    #[test]
    fn a_text_is_written_once_read_only_read_back_and_removed() {
        let temp = TempDir::new("store-file");
        let dir = temp.0.join("s");
        make_private(&dir).unwrap();
        let file = dir.join("1");
        write_once(&file, b"ok\r\n").unwrap();
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        assert!(private(&file));
        assert_eq!(write_once(&file, b"again").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&file, 100).unwrap(), b"ok\r\n");
        assert_eq!(read(&file, 2).unwrap(), b"ok");
        assert_eq!(list(&dir).unwrap().len(), 1);
        remove(&file).unwrap();
        remove(&file).unwrap();
        write_once(&dir.join("2"), b"x").unwrap();
        write_marker(&dir.join("expires"), "1\n").unwrap();
        remove_dir(&dir).unwrap();
        remove_dir(&dir).unwrap();
        assert!(list(&dir).unwrap().is_empty());
    }
}
