//! Where the terminal server's processes listen, for the server (the core
//! and the holders), its clients (Brindle, the management commands) and the
//! tests, so all of them name an endpoint the same way.
//!
//! An endpoint is what a server binds and a client connects to, as a path:
//!
//! - **Linux:** a Unix socket in the server directory, `core.sock` and
//!   `holder-<build>-<n>.sock`. Holders are found by their sockets.
//! - **Windows:** a named pipe, `\\.\pipe\brindle-server-<h>-core` and
//!   `…-holder-<build>-<n>` ([`server_dir::core_pipe`],
//!   [`server_dir::holder_pipe`]), where `<h>` hashes the directory's
//!   canonical path and this user's SID. Nothing in the directory stands
//!   for a pipe, so holders are found by their lock files,
//!   `holder-<build>-<n>.lock`, and each one's pipe name follows from its
//!   file's name. The directory must exist: its canonical path is what is
//!   hashed, so that every spelling of it names the same pipes.
//!
//! A holder's lock file and log are beside each other in the directory on
//! both platforms, named like its endpoint.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::server_dir;

/// A holder's endpoint and its files in the server directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    /// Where it listens.
    pub endpoint: PathBuf,
    /// `holder-<build>-<n>.lock`, which it holds for its whole life.
    pub lock: PathBuf,
    /// `holder-<build>-<n>.log`.
    pub log: PathBuf,
}

/// The endpoints of one server directory.
#[derive(Debug, Clone)]
pub struct Endpoints {
    dir: PathBuf,
    /// The directory as the pipe names hash it.
    #[cfg(windows)]
    canonical: PathBuf,
    #[cfg(windows)]
    sid: String,
}

impl Endpoints {
    /// The endpoints of the server directory `dir`. On Windows the
    /// directory must exist (`NotFound` otherwise, which for a client means
    /// that no server runs there).
    pub fn of(dir: &Path) -> io::Result<Self> {
        Ok(Self {
            dir: dir.to_path_buf(),
            #[cfg(windows)]
            canonical: fs::canonicalize(dir)?,
            #[cfg(windows)]
            sid: crate::win::own_sid()?,
        })
    }

    /// Where the core listens.
    pub fn core(&self) -> PathBuf {
        #[cfg(not(windows))]
        return self.dir.join(server_dir::CORE_SOCKET);
        #[cfg(windows)]
        return server_dir::core_pipe(&self.canonical, &self.sid).into();
    }

    /// Holder number `n` of build `build`: where it listens, and its files.
    pub fn holder(&self, build: &str, n: u32) -> Holder {
        let file = |extension: &str| self.dir.join(format!("holder-{build}-{n}.{extension}"));
        Holder {
            #[cfg(not(windows))]
            endpoint: file("sock"),
            #[cfg(windows)]
            endpoint: server_dir::holder_pipe(&self.canonical, &self.sid, build, n).into(),
            lock: file("lock"),
            log: file("log"),
        }
    }

    /// The holders the directory shows, in the order of their file names:
    /// on Linux every `holder-*.sock`, on Windows every
    /// `holder-<build>-<n>.lock`. One of them may be stale (its holder was
    /// killed); the lock tells.
    pub fn holders(&self) -> io::Result<Vec<Holder>> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)?
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with("holder-"))
            .collect();
        names.sort();
        Ok(names.iter().filter_map(|name| self.shown(name)).collect())
    }

    /// The holder that the directory entry `name` stands for, if it is the
    /// kind of file holders are found by.
    #[cfg(not(windows))]
    fn shown(&self, name: &str) -> Option<Holder> {
        let stem = name.strip_suffix(".sock")?;
        Some(Holder {
            endpoint: self.dir.join(name),
            lock: self.dir.join(format!("{stem}.lock")),
            log: self.dir.join(format!("{stem}.log")),
        })
    }

    #[cfg(windows)]
    fn shown(&self, name: &str) -> Option<Holder> {
        let (build, n) = server_dir::parse_holder_lock(name)?;
        Some(self.holder(&build, n))
    }
}

/// Takes a server's endpoint away once it stops listening, so that nothing
/// finds it: the socket file on Linux. A pipe's name goes with its last
/// handle, so there is nothing to remove on Windows.
pub fn remove(endpoint: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    return fs::remove_file(endpoint);
    #[cfg(windows)]
    {
        let _ = endpoint;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let name = format!("brindle-endpoint-{tag}-{}", std::process::id());
            let path = std::env::temp_dir().join(name);
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

    #[test]
    fn a_holders_files_are_named_alike() {
        let temp = TempDir::new("names");
        let endpoints = Endpoints::of(&temp.0).unwrap();
        let holder = endpoints.holder("0.3.1-abc", 2);
        assert_eq!(holder.lock, temp.0.join("holder-0.3.1-abc-2.lock"));
        assert_eq!(holder.log, temp.0.join("holder-0.3.1-abc-2.log"));
        assert_ne!(holder.endpoint, endpoints.holder("0.3.1-abc", 3).endpoint);
        assert_ne!(holder.endpoint, endpoints.core());
        assert_eq!(
            holder,
            Endpoints::of(&temp.0).unwrap().holder("0.3.1-abc", 2)
        );
    }

    #[cfg(unix)]
    #[test]
    fn sockets_are_in_the_directory_and_show_the_holders() {
        let temp = TempDir::new("sockets");
        let endpoints = Endpoints::of(&temp.0).unwrap();
        assert_eq!(endpoints.core(), temp.0.join("core.sock"));
        let holder = endpoints.holder("b1", 0);
        assert_eq!(holder.endpoint, temp.0.join("holder-b1-0.sock"));
        assert_eq!(endpoints.holders().unwrap(), []);

        // Only sockets count, whatever comes after `holder-`.
        for name in [
            "holder-b1-1.sock",
            "holder-b1-0.sock",
            "holder-b1-0.lock",
            "core.sock",
        ] {
            fs::write(temp.0.join(name), b"").unwrap();
        }
        fs::write(temp.0.join("holder-odd.sock"), b"").unwrap();
        let found = endpoints.holders().unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[0], holder);
        assert_eq!(found[1], endpoints.holder("b1", 1));
        assert_eq!(found[2].endpoint, temp.0.join("holder-odd.sock"));
        assert_eq!(found[2].lock, temp.0.join("holder-odd.lock"));

        remove(&holder.endpoint).unwrap();
        assert_eq!(endpoints.holders().unwrap().len(), 2);
        assert_eq!(
            remove(&holder.endpoint).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    // No directory is needed to name a socket: a client asks before any
    // server has created it.
    #[cfg(unix)]
    #[test]
    fn a_missing_directory_still_names_its_sockets() {
        let endpoints = Endpoints::of(Path::new("/nonexistent/brindle")).unwrap();
        assert_eq!(
            endpoints.core(),
            Path::new("/nonexistent/brindle/core.sock")
        );
        assert!(endpoints.holders().is_err());
    }

    #[cfg(windows)]
    #[test]
    fn pipes_are_named_from_the_directory_and_lock_files_show_the_holders() {
        let temp = TempDir::new("pipes");
        let endpoints = Endpoints::of(&temp.0).unwrap();
        let canonical = fs::canonicalize(&temp.0).unwrap();
        let sid = crate::win::own_sid().unwrap();
        assert_eq!(
            endpoints.core(),
            PathBuf::from(server_dir::core_pipe(&canonical, &sid))
        );
        let holder = endpoints.holder("b1", 0);
        assert_eq!(
            holder.endpoint,
            PathBuf::from(server_dir::holder_pipe(&canonical, &sid, "b1", 0))
        );
        assert_eq!(endpoints.holders().unwrap(), []);

        // Only lock files that name a build and a number count.
        for name in [
            "holder-b1-1.lock",
            "holder-b1-0.lock",
            "holder-b1-0.log",
            "core.lock",
        ] {
            fs::write(temp.0.join(name), b"").unwrap();
        }
        fs::write(temp.0.join("holder-odd.lock"), b"").unwrap();
        assert_eq!(
            endpoints.holders().unwrap(),
            [holder.clone(), endpoints.holder("b1", 1)]
        );

        // Nothing stands for a pipe in the directory.
        remove(&holder.endpoint).unwrap();
        assert_eq!(endpoints.holders().unwrap().len(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn every_spelling_of_the_directory_names_the_same_pipes() {
        let temp = TempDir::new("spelling");
        let core = Endpoints::of(&temp.0).unwrap().core();
        let upper = PathBuf::from(temp.0.to_str().unwrap().to_uppercase());
        assert_eq!(Endpoints::of(&upper).unwrap().core(), core);
        let dotted = temp.0.join(".").join("sub").join("..");
        fs::create_dir(temp.0.join("sub")).unwrap();
        assert_eq!(Endpoints::of(&dotted).unwrap().core(), core);
        assert_ne!(Endpoints::of(&temp.0.join("sub")).unwrap().core(), core);

        let missing = Endpoints::of(&temp.0.join("missing")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }
}
