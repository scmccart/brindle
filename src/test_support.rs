//! Helpers shared by this crate's unit tests.

use std::fs::File;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long [`write_executable`] waits for the file's writers to go.
const WRITERS_LIMIT: Duration = Duration::from_secs(5);
/// How long it waits instead where no lease can show that they are gone.
const WITHOUT_LEASE: Duration = Duration::from_millis(100);

/// Writes `contents` to `path` as an executable file that can be run at once.
///
/// Tests run on parallel threads, and some of them fork children. A child
/// forked while the file was open for writing here holds a copy of that fd
/// until its own exec, close-on-exec or not, and until then running the file
/// fails with `ETXTBSY`. So this returns once nothing has it open for writing.
pub fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    wait_until_unwritten(path);
}

/// Waits, for at most [`WRITERS_LIMIT`], until nothing has `path` open for
/// writing. The kernel grants a read lease only then, so the check is taking
/// one. Where it grants none at all (`EINVAL`: the filesystem has no leases),
/// this waits [`WITHOUT_LEASE`] instead, long enough for a child to exec.
fn wait_until_unwritten(path: &Path) {
    let file = File::open(path).unwrap();
    let deadline = Instant::now() + WRITERS_LIMIT;
    loop {
        match read_lease(&file) {
            Ok(()) => return,
            Err(error) if error.raw_os_error() != Some(libc::EAGAIN) => {
                eprintln!(
                    "no read lease on {} ({error}): waiting {} ms for its writers instead",
                    path.display(),
                    WITHOUT_LEASE.as_millis()
                );
                std::thread::sleep(WITHOUT_LEASE);
                return;
            }
            Err(_) if Instant::now() >= deadline => {
                eprintln!(
                    "{} is still open for writing after {} s, so running it may fail",
                    path.display(),
                    WRITERS_LIMIT.as_secs()
                );
                return;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

/// Takes a read lease on `file`, which must be open read-only, and gives it
/// back. `EAGAIN` means the file is open for writing somewhere.
#[cfg(target_os = "linux")]
fn read_lease(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;

    let fd = file.as_raw_fd();
    // SAFETY: a valid fd and integer arguments.
    if unsafe { libc::fcntl(fd, libc::F_SETLEASE, libc::F_RDLCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // Closing `file` isn't enough: the lease belongs to the open file, which
    // a child forked meanwhile shares until its exec.
    // SAFETY: as above.
    unsafe { libc::fcntl(fd, libc::F_SETLEASE, libc::F_UNLCK) };
    Ok(())
}

/// Leases are Linux's.
#[cfg(not(target_os = "linux"))]
fn read_lease(_: &File) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[test]
fn an_executable_is_ready_only_once_its_writers_are_gone() {
    let dir = std::env::temp_dir().join(format!("brindle-test-support-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("script");
    // Stands in for the copy of the writer's fd that a forked child holds.
    let writer = File::create(&path).unwrap();
    let refused = read_lease(&File::open(&path).unwrap()).unwrap_err();
    if refused.raw_os_error() == Some(libc::EAGAIN) {
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let closing = Instant::now();
            drop(writer);
            closing
        });
        wait_until_unwritten(&path);
        let returned = Instant::now();
        assert!(returned >= closer.join().unwrap());
    } else {
        eprintln!("not checked: no read leases in {} ({refused})", dir.display());
    }
    std::fs::remove_dir_all(&dir).ok();
}
