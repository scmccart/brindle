//! A temporary directory per test, removed when the test ends (`testing R7`).

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory, `temp_dir()/brindle-<name>-<pid>-<n>`, removed with
/// everything in it on drop, so a failed assertion leaves nothing behind.
/// `n` counts per process, so two tests (or two directories of one test)
/// never share it, and whatever a recycled pid left there is removed first.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(test_name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("brindle-{test_name}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_temp_dir_is_fresh_and_removed_on_drop() {
        let first = TempDir::new("test-support-dir");
        let second = TempDir::new("test-support-dir");
        assert_ne!(first.path(), second.path());
        assert!(first.is_dir());
        std::fs::write(first.join("file"), "x").unwrap();
        let path = first.path().to_path_buf();
        drop(first);
        assert!(!path.exists());
    }
}
