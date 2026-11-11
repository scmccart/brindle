//! The single-instance and holder locks: to be `LockFileEx` on a file in
//! the server directory.

use std::io;
use std::path::Path;

use super::unsupported;

/// A held lock. Dropping it releases the lock.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Lock(());

impl Lock {
    /// Takes the lock at `path`, creating the file if needed. `None` means
    /// another process holds it.
    pub fn create(_path: &Path) -> io::Result<Option<Self>> {
        Err(unsupported())
    }

    /// Takes the lock at `path` if the file exists (`NotFound` otherwise).
    /// `None` means another process holds it.
    pub fn existing(_path: &Path) -> io::Result<Option<Self>> {
        Err(unsupported())
    }

    /// Whether `path` still names the locked file.
    pub fn is_at(&self, _path: &Path) -> io::Result<bool> {
        Err(unsupported())
    }
}
