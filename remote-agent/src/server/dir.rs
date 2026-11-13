//! The server directory: where the core's and holders' locks and logs live,
//! and on Linux their sockets (Windows names its pipes from the directory
//! instead). The platform says where it is and who may enter it.

use std::io;
use std::path::PathBuf;

use super::platform::dir;
pub use super::platform::dir::resolve_from_env;

/// The server directory for this process's environment, created if needed.
pub fn from_env() -> io::Result<PathBuf> {
    let path = resolve_from_env()?;
    dir::ensure(&path)?;
    Ok(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
}
