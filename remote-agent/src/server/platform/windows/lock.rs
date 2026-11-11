//! The single-instance and holder locks: `LockFileEx` on a file in the
//! server directory, held for the process's whole life and released by
//! Windows when it ends.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

use brindle_remote_protocol::win::file_identity;

/// A held lock. Dropping it releases the lock.
pub struct Lock(File);

impl Lock {
    /// Takes the lock at `path`, creating the file if needed. `None` means
    /// another process holds it.
    pub fn create(path: &Path) -> io::Result<Option<Self>> {
        Self::take(Self::options().create(true).truncate(false).open(path)?)
    }

    /// Takes the lock at `path` if the file exists (`NotFound` otherwise).
    /// `None` means another process holds it.
    pub fn existing(path: &Path) -> io::Result<Option<Self>> {
        Self::take(Self::options().open(path)?)
    }

    /// std opens with `FILE_SHARE_READ | FILE_SHARE_WRITE |
    /// FILE_SHARE_DELETE`: others can open the file to try the lock, and
    /// an exiting holder can delete its own lock file while it is open.
    fn options() -> OpenOptions {
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        options
    }

    /// `try_lock` is `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK |
    /// LOCKFILE_FAIL_IMMEDIATELY)` over the whole file.
    fn take(file: File) -> io::Result<Option<Self>> {
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// Whether `path` still names the locked file. An exiting holder
    /// deletes its lock file; if that happened between the open and the
    /// lock, this lock is on a deleted file and protects nothing.
    pub fn is_at(&self, path: &Path) -> io::Result<bool> {
        let locked = file_identity(&self.0)?;
        Ok(matches!(
            File::open(path).and_then(|now| file_identity(&now)),
            Ok(now) if now == locked
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::*;

    const CHILD_LOCK: &str = "BRINDLE_TEST_LOCK_CHILD";

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The child's half of the test below. Run as a test of its own it does
    /// nothing; re-run with the variable it takes the lock, says so with a
    /// file, and holds the lock until it is killed.
    #[test]
    fn child_holds_the_lock_until_killed() {
        let Some(path) = std::env::var_os(CHILD_LOCK) else {
            return;
        };
        let path = PathBuf::from(path);
        let lock = Lock::create(&path)
            .unwrap()
            .expect("the child takes the lock");
        std::fs::write(path.with_extension("ready"), b"").unwrap();
        std::thread::sleep(Duration::from_secs(120));
        drop(lock);
    }

    #[test]
    fn a_lock_held_by_another_process_is_refused_until_it_dies() {
        let dir = temp_dir("child");
        let path = dir.join("core.lock");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("child_holds_the_lock_until_killed")
            .env(CHILD_LOCK, &path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let ready = path.with_extension("ready");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            assert!(child.try_wait().unwrap().is_none(), "the child exited");
            assert!(Instant::now() < deadline, "the child never took the lock");
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(Lock::create(&path).unwrap().is_none());
        assert!(Lock::existing(&path).unwrap().is_none());

        child.kill().unwrap();
        child.wait().unwrap();
        let lock = Lock::existing(&path)
            .unwrap()
            .expect("free once the child died");
        assert!(lock.is_at(&path).unwrap());
        // Another handle is another owner, in this process too.
        assert!(Lock::create(&path).unwrap().is_none());
        drop(lock);
        assert!(Lock::create(&path).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_needs_the_file() {
        let dir = temp_dir("missing");
        let error = Lock::existing(&dir.join("holder-b-0.lock")).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deleted_and_recreated_file_is_not_the_locked_one() {
        let dir = temp_dir("identity");
        let path = dir.join("holder-b-0.lock");
        let lock = Lock::create(&path).unwrap().unwrap();
        assert!(lock.is_at(&path).unwrap());

        // The holder can delete its own lock file while holding it.
        std::fs::remove_file(&path).unwrap();
        assert!(!lock.is_at(&path).unwrap());

        let next = Lock::create(&path)
            .unwrap()
            .expect("a new file, a new lock");
        assert!(next.is_at(&path).unwrap());
        assert!(!lock.is_at(&path).unwrap());
        drop((lock, next));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
