//! Opt-in end-to-end suite: runs Brindle against throwaway tmux servers and
//! checks what it shows against tmux (screen text, window pixels, replies
//! to color queries), plus the tmux behaviour Brindle relies on.
//!
//! ```sh
//! BRINDLE_E2E=1 cargo test --test e2e                 # every case
//! BRINDLE_E2E=1 cargo test --test e2e -- titles       # cases matching "titles"
//! BRINDLE_E2E=1 cargo test --test e2e -- --list
//! BRINDLE_E2E=1 cargo test --test e2e -- --self-test  # the harness's own tests
//! ```
//!
//! Cases open Brindle windows under Xwayland (`BRINDLE_E2E_DISPLAY`, default
//! `:0`), so ask before running them on someone's desktop. They never send
//! synthetic input. `BRINDLE_E2E_BIN` picks the binary (default: this
//! build's), `BRINDLE_E2E_SLOW` stretches every delay. Each case leaves its
//! config, dumps, captures and transcripts in `target/tmp/e2e/<case>/`.

mod brindle;
mod capture;
mod cases;
mod query;
mod tmux;

use std::path::PathBuf;
use std::time::{Duration, Instant};

pub type Result<T = ()> = std::result::Result<T, String>;

/// Fails the case with a message unless `cond` holds.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}

pub enum Outcome {
    Pass,
    Skip(String),
}

pub struct Case {
    pub name: &'static str,
    /// Runs Brindle, so it needs an X display.
    pub display: bool,
    pub run: fn(&Ctx) -> Result<Outcome>,
}

/// What a case gets from the runner.
pub struct Ctx {
    pub name: &'static str,
    /// Scratch and artifact directory for this case, emptied before it runs.
    pub dir: PathBuf,
    pub bin: PathBuf,
    pub display: String,
    slow: f64,
}

impl Ctx {
    /// `secs`, stretched by `BRINDLE_E2E_SLOW`.
    pub fn secs(&self, secs: f64) -> f64 {
        secs * self.slow
    }

    pub fn sleep(&self, secs: f64) {
        std::thread::sleep(Duration::from_secs_f64(self.secs(secs)));
    }

    /// Writes a file into the case's directory.
    pub fn save(&self, name: &str, contents: impl AsRef<[u8]>) {
        std::fs::write(self.dir.join(name), contents).ok();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var("BRINDLE_E2E").as_deref() != Ok("1") {
        println!("e2e: skipped (set BRINDLE_E2E=1 to run; it opens Brindle windows)");
        return;
    }
    if args.iter().any(|a| a == "--self-test") {
        std::process::exit(self_test());
    }
    let cases = cases::all();
    if args.iter().any(|a| a == "--list") {
        for case in &cases {
            println!("{}{}", case.name, if case.display { "" } else { "  (no display)" });
        }
        return;
    }
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let selected: Vec<&Case> =
        cases.iter().filter(|c| filters.is_empty() || filters.iter().any(|f| c.name.contains(f.as_str()))).collect();

    let display = std::env::var("BRINDLE_E2E_DISPLAY").unwrap_or_else(|_| ":0".into());
    let has_display = x11rb::connect(Some(&display)).is_ok();
    let bin = std::env::var_os("BRINDLE_E2E_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_brindle")));
    let slow = std::env::var("BRINDLE_E2E_SLOW").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("e2e");

    let (mut passed, mut failed, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for case in selected {
        let started = Instant::now();
        let outcome = if case.display && !has_display {
            Ok(Outcome::Skip(format!("no X display at {display}")))
        } else {
            let dir = root.join(case.name);
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("case directory");
            let ctx = Ctx { name: case.name, dir, bin: bin.clone(), display: display.clone(), slow };
            (case.run)(&ctx)
        };
        let took = started.elapsed().as_secs_f64();
        match outcome {
            Ok(Outcome::Pass) => {
                println!("ok       {} ({took:.1}s)", case.name);
                passed.push(case.name);
            }
            Ok(Outcome::Skip(why)) => {
                println!("skipped  {}: {why}", case.name);
                skipped.push(case.name);
            }
            Err(why) => {
                println!("FAILED   {} ({took:.1}s): {why}", case.name);
                println!("         artifacts: {}", root.join(case.name).display());
                failed.push(case.name);
            }
        }
    }
    println!("\ne2e: {} passed, {} failed, {} skipped", passed.len(), failed.len(), skipped.len());
    if !failed.is_empty() {
        println!("failed: {}", failed.join(" "));
        std::process::exit(1);
    }
}

/// The harness's own unit tests (`--self-test`); there is no libtest here.
fn self_test() -> i32 {
    let tests: &[(&str, fn() -> Result)] = &[
        ("dump parsing", brindle::self_test_dump),
        ("query parsing", query::self_test),
        ("cell rectangles", capture::self_test),
    ];
    let mut failed = 0;
    for (name, test) in tests {
        match test() {
            Ok(()) => println!("ok       {name}"),
            Err(why) => {
                println!("FAILED   {name}: {why}");
                failed += 1;
            }
        }
    }
    i32::from(failed > 0)
}
