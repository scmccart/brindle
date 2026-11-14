//! The single-instance and holder locks: `flock` on a file in the server
//! directory, held for the process's whole life and released by the kernel
//! when it ends.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

/// A held lock. Dropping it releases the lock.
pub struct Lock(File);

impl Lock {
    /// Takes the lock at `path`, creating the file (user-only) if needed.
    /// `None` means another process holds it.
    pub fn create(path: &Path) -> io::Result<Option<Self>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        Self::take(file)
    }

    /// Takes the lock at `path` if the file exists (`NotFound` otherwise).
    /// `None` means another process holds it.
    pub fn existing(path: &Path) -> io::Result<Option<Self>> {
        Self::take(OpenOptions::new().read(true).write(true).open(path)?)
    }

    fn take(file: File) -> io::Result<Option<Self>> {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(Self(file)));
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            Ok(None)
        } else {
            Err(error)
        }
    }

    /// Whether `path` still names the locked file. An exiting holder
    /// unlinks its lock file; if that happened between the open and the
    /// lock, this lock is on a dead inode and protects nothing.
    pub fn is_at(&self, path: &Path) -> io::Result<bool> {
        let locked = self.0.metadata()?;
        Ok(matches!(
            fs::metadata(path),
            Ok(now) if now.ino() == locked.ino() && now.dev() == locked.dev()
        ))
    }
}
