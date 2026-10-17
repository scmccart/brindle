//! Throwaway tmux servers, and raw control-mode connections to them.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Ctx, Result};

pub use crate::common::tmux::{TmuxServer, trim_screen};

/// A raw `tmux -u -C` client, recording everything tmux sends. `-u` marks
/// it UTF-8 whatever the locale, as Brindle's client is.
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
            .args(["-u", "-C", "attach", "-t", "e2e"])
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
