//! Throwaway tmux servers.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use super::Result;

/// The shell in every pane: no rc files, so prompts and colors are plain.
pub const SHELL: &str = "bash --norc --noprofile";

/// A tmux server on its own socket, with no config, killed when dropped.
pub struct TmuxServer {
    pub socket: String,
    /// The socket file, which `kill-server` leaves behind.
    path: Option<String>,
}

impl TmuxServer {
    /// Starts a server with session `e2e` (one window, `cols`x`rows`).
    pub fn new(cols: u16, rows: u16) -> Result<TmuxServer> {
        Self::with_prefix("brindle-e2e", cols, rows)
    }

    /// Like [`TmuxServer::new`], on socket `<prefix>-<pid>-<n>`.
    pub fn with_prefix(prefix: &str, cols: u16, rows: u16) -> Result<TmuxServer> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let socket = format!("{prefix}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let mut server = TmuxServer { socket, path: None };
        server.run(&["new-session", "-d", "-s", "e2e", "-x", &cols.to_string(), "-y", &rows.to_string(), SHELL])?;
        server.path = Some(server.display("e2e", "#{socket_path}")?);
        server.run(&["set", "-g", "default-command", SHELL])?;
        Ok(server)
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        command.args(["-L", &self.socket, "-f", "/dev/null"]);
        command
    }

    /// Runs a tmux command, returning its stdout.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.command().args(args).output().map_err(|e| format!("tmux: {e}"))?;
        if !out.status.success() {
            return Err(format!("tmux {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Types a command line into a pane.
    pub fn type_line(&self, pane: &str, line: &str) -> Result {
        self.run(&["send-keys", "-t", pane, "-l", line])?;
        self.run(&["send-keys", "-t", pane, "Enter"]).map(drop)
    }

    /// `display-message -p` for a target.
    pub fn display(&self, target: &str, format: &str) -> Result<String> {
        Ok(self.run(&["display-message", "-p", "-t", target, format])?.trim_end_matches('\n').to_string())
    }

    pub fn active_pane(&self) -> Result<String> {
        self.display("e2e", "#{pane_id}")
    }

    /// A pane's left, top, width and height.
    pub fn geometry(&self, pane: &str) -> Result<[u16; 4]> {
        let value = self.display(pane, "#{pane_left} #{pane_top} #{pane_width} #{pane_height}")?;
        let fields: Vec<u16> = value.split(' ').filter_map(|f| f.parse().ok()).collect();
        fields.try_into().map_err(|_| format!("bad pane geometry {value:?}"))
    }

    /// A pane's visible screen, without trailing blanks.
    pub fn capture(&self, pane: &str) -> Result<Vec<String>> {
        Ok(trim_screen(self.run(&["capture-pane", "-p", "-t", pane])?.lines()))
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        self.command().arg("kill-server").stderr(Stdio::null()).status().ok();
        if let Some(path) = &self.path {
            std::fs::remove_file(path).ok();
        }
    }
}

/// Lines with trailing spaces and trailing empty lines removed.
pub fn trim_screen<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = lines.map(|l| l.trim_end().to_string()).collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}
