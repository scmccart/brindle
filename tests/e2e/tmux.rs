//! Throwaway tmux servers, and raw control-mode connections to them.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Ctx, Result};

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
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let socket = format!("brindle-e2e-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
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

/// A raw `tmux -C` client, recording everything tmux sends.
pub struct ControlClient {
    child: Child,
    stdin: ChildStdin,
    lines: Arc<Mutex<Vec<String>>>,
    started: Instant,
    transcript: Arc<Mutex<String>>,
}

impl ControlClient {
    /// Attaches to session `e2e` of `server` at `cols`x`rows`.
    pub fn attach(server: &TmuxServer, cols: u16, rows: u16) -> Result<ControlClient> {
        let mut child = server
            .command()
            .args(["-C", "attach", "-t", "e2e"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("tmux -C: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let transcript = Arc::new(Mutex::new(String::new()));
        let started = Instant::now();
        {
            let (lines, transcript) = (lines.clone(), transcript.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let t = started.elapsed().as_secs_f64();
                    transcript.lock().unwrap().push_str(&format!("{t:6.2} {line}\n"));
                    lines.lock().unwrap().push(line);
                }
            });
        }
        let mut client = ControlClient { child, stdin, lines, started, transcript };
        client.send(&format!("refresh-client -C {cols}x{rows}"))?;
        Ok(client)
    }

    pub fn send(&mut self, command: &str) -> Result {
        let t = self.started.elapsed().as_secs_f64();
        self.transcript.lock().unwrap().push_str(&format!("{t:6.2} >> {command}\n"));
        writeln!(self.stdin, "{command}").map_err(|e| format!("tmux -C write: {e}"))
    }

    /// How many lines have arrived so far; a mark for `lines_since`.
    pub fn mark(&self) -> usize {
        self.lines.lock().unwrap().len()
    }

    pub fn lines_since(&self, mark: usize) -> Vec<String> {
        self.lines.lock().unwrap()[mark..].to_vec()
    }

    /// Waits up to `secs` for a line after `mark` matching `pred`.
    pub fn wait_for(&self, ctx: &Ctx, mark: usize, secs: f64, pred: impl Fn(&str) -> bool) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(secs));
        while Instant::now() < deadline {
            if let Some(line) = self.lines_since(mark).into_iter().find(|l| pred(l)) {
                return Some(line);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    pub fn transcript(&self) -> String {
        self.transcript.lock().unwrap().clone()
    }
}

impl Drop for ControlClient {
    fn drop(&mut self) {
        writeln!(self.stdin, "detach-client").ok();
        std::thread::sleep(Duration::from_millis(100));
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
