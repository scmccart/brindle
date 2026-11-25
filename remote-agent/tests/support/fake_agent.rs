//! Scripts for the fake ACP agent ([`super::FAKE_AGENT`], OpenSpec change
//! `agents-sessions`, design decision 16): [`AgentScript`] writes one into a
//! test's directory, with its record file and the file gates ([`Gate`]) the
//! test opens to let the agent go on, so a test orders the agent's steps
//! against its own without a sleep.

use std::fs;
use std::path::{Path, PathBuf};

use brindle_remote_protocol::serde_json::{self, Value};

/// A fake agent's script, built step by step and written with
/// [`AgentScript::write`]. Steps are the lines of `src/fake_acp/script.rs`.
pub struct AgentScript {
    dir: PathBuf,
    name: String,
    lines: Vec<String>,
    gates: usize,
}

impl AgentScript {
    /// A script to be written as `<dir>/<name>.script`, its record and gates
    /// beside it.
    pub fn new(dir: &Path, name: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            lines: Vec::new(),
            gates: 0,
        }
    }

    /// Adds a step, written as is (`expect initialize`, `respond {…}`).
    pub fn step(&mut self, line: impl Into<String>) -> &mut Self {
        self.lines.push(line.into());
        self
    }

    /// Adds a `record` step and gives its file, which [`recorded`] reads.
    pub fn record(&mut self) -> PathBuf {
        let path = self.dir.join(format!("{}.received.jsonl", self.name));
        self.lines.push(format!("record {}", path.display()));
        path
    }

    /// Adds a `wait-file` step on a new gate: the agent goes past it once
    /// the test calls [`Gate::open`].
    pub fn gate(&mut self) -> Gate {
        self.gates += 1;
        let path = self.dir.join(format!("{}.gate-{}", self.name, self.gates));
        // A gate left from an earlier run would be open already.
        let _ = fs::remove_file(&path);
        self.lines.push(format!("wait-file {}", path.display()));
        Gate { path }
    }

    /// Writes the script and gives its path, the agent's argument.
    pub fn write(&self) -> PathBuf {
        let path = self.dir.join(format!("{}.script", self.name));
        let mut text = self.lines.join("\n");
        text.push('\n');
        fs::write(&path, text).unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
        path
    }
}

/// A file the fake agent waits for; creating it lets the agent go on.
pub struct Gate {
    path: PathBuf,
}

impl Gate {
    pub fn open(&self) {
        fs::write(&self.path, b"").unwrap_or_else(|error| panic!("opening gate {}: {error}", self.path.display()));
    }
}

/// The messages a fake agent recorded so far, each as JSON; none before the
/// file exists. A line still being written is left out.
pub fn recorded(path: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("recorded {line:?}: {error}")))
        .collect()
}
