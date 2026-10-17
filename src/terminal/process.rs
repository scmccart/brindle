//! Process info for tab titles and new-tab directories: which process is in
//! the foreground on a tab's PTY, its name, and its working directory.
//!
//! Linux reads `/proc`. Other platforms report nothing yet, so tabs fall
//! back to their profile name and new tabs to the profile's directory.

use std::path::PathBuf;

/// What a local tab knows about the program on its PTY.
#[derive(Clone, Copy, Debug)]
pub struct PtyProcess {
    child_pid: u32,
    /// The PTY master, for `tcgetpgrp`. It stays open for as long as the
    /// event loop owns the PTY.
    #[cfg(unix)]
    master_fd: i32,
}

impl PtyProcess {
    #[cfg(unix)]
    pub fn new(child_pid: u32, master_fd: i32) -> Self {
        Self { child_pid, master_fd }
    }

    #[cfg(not(unix))]
    pub fn new(child_pid: u32) -> Self {
        Self { child_pid }
    }

    /// The PTY's foreground process group, which is its leader's pid.
    #[cfg(unix)]
    fn foreground_pid(&self) -> Option<u32> {
        // SAFETY: tcgetpgrp has no memory-safety preconditions; a stale fd
        // only makes it fail.
        let pgrp = unsafe { libc::tcgetpgrp(self.master_fd) };
        (pgrp > 0).then_some(pgrp as u32)
    }

    #[cfg(not(unix))]
    fn foreground_pid(&self) -> Option<u32> {
        None
    }

    /// Name of the foreground process on the PTY (e.g. `vim`).
    pub fn foreground_name(&self) -> Option<String> {
        process_name(self.foreground_pid()?)
    }

    /// Current working directory of the foreground process (falls back to
    /// the program the tab started).
    pub fn working_directory(&self) -> Option<PathBuf> {
        let pid = self.foreground_pid().unwrap_or(self.child_pid);
        process_cwd(pid).or_else(|| process_cwd(self.child_pid))
    }
}

#[cfg(target_os = "linux")]
pub fn process_name(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim().to_string())
}

#[cfg(target_os = "linux")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(target_os = "linux"))]
pub fn process_name(_pid: u32) -> Option<String> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn reads_this_process() {
        let pid = std::process::id();
        assert!(process_name(pid).is_some_and(|name| !name.is_empty()));
        assert_eq!(process_cwd(pid), std::env::current_dir().ok());
    }
}
