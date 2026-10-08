//! Getting vtebench: it isn't on crates.io, so a pinned commit is cloned
//! into `target/perf-tools/vtebench` and built there with its own lockfile
//! (Brindle's `Cargo.lock` is never touched). `BRINDLE_PERF_VTEBENCH`
//! points at an existing checkout instead.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;

const REPO: &str = "https://github.com/alacritty/vtebench";
/// Bump deliberately: results from different commits aren't comparable.
pub const PINNED: &str = "ead80032e57dee2e75f0b51f2ea67528647d9944";

pub struct Vtebench {
    pub bin: PathBuf,
    pub benchmarks: PathBuf,
    pub commit: String,
}

fn run(command: &mut Command) -> Result {
    let out = command.output().map_err(|e| format!("{command:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{command:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

fn head(dir: &Path) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Finds or fetches vtebench and builds it.
pub fn resolve(target_dir: &Path) -> Result<Vtebench> {
    let dir = match std::env::var_os("BRINDLE_PERF_VTEBENCH") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let dir = target_dir.join("perf-tools/vtebench");
            if !dir.join(".git").exists() {
                println!("perf: fetching vtebench into {}", dir.display());
                std::fs::create_dir_all(dir.parent().expect("has a parent")).map_err(|e| e.to_string())?;
                run(Command::new("git").args(["clone", "-q", REPO]).arg(&dir))?;
            }
            if head(&dir)? != PINNED {
                run(Command::new("git").arg("-C").arg(&dir).args(["fetch", "-q", "origin"]))?;
                run(Command::new("git").arg("-C").arg(&dir).args(["checkout", "-q", PINNED]))?;
            }
            dir
        }
    };
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(Command::new(cargo)
        .args(["build", "-q", "--release", "--locked", "--manifest-path"])
        .arg(dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(dir.join("target")))
    .map_err(|e| format!("building vtebench: {e}"))?;
    Ok(Vtebench {
        bin: dir.join("target/release/vtebench"),
        benchmarks: dir.join("benchmarks"),
        commit: head(&dir)?,
    })
}
