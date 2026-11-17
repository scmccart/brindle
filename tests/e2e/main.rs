//! Opt-in end-to-end suite: runs Brindle with the debug hooks and checks
//! what it shows (screen text, window pixels, replies to color queries,
//! notifications, the terminal server's sessions).
//!
//! ```sh
//! BRINDLE_E2E=1 cargo test --test e2e                 # every case
//! BRINDLE_E2E=1 cargo test --test e2e -- titles       # cases matching "titles"
//! BRINDLE_E2E=1 cargo test --test e2e -- --list
//! BRINDLE_E2E=1 cargo test --test e2e -- --shard 2/4 # every 4th case from the 2nd
//! BRINDLE_E2E=1 cargo test --test e2e -- --self-test  # the harness's own tests
//! ```
//!
//! Cases open Brindle windows under Xwayland (`BRINDLE_E2E_DISPLAY`, default
//! `:0`; on Windows, on the desktop), so ask before running them on
//! someone's desktop. They never send synthetic input. `BRINDLE_E2E_BIN` picks the binary (default: this
//! build's), `BRINDLE_E2E_SLOW` stretches every delay and every wait's limit
//! (the cases wait through `wait.rs`'s [`Scale`], never a fixed sleep). Each
//! case leaves its config, dumps, captures and transcripts in
//! `target/tmp/e2e/<case>/`.

mod brindle;
#[path = "../common/mod.rs"]
mod common;
mod capture;
mod cases;
mod query;
mod wait;

use std::path::PathBuf;
use std::time::Instant;

#[cfg(windows)]
use common::server::under;
use common::server::{Suite, check_helper, end_roles, helper, helper_name, private_dir, server_processes};
#[cfg(windows)]
use common::winproc;
use common::{Selection, take_shard};
pub use common::Result;
pub use wait::Scale;

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
    /// The terminal server helper Brindle runs (`BRINDLE_REMOTE_AGENT`, on
    /// Windows `BRINDLE_SERVER_HELPER`, else `brindle-remote` beside `bin`).
    pub helper: PathBuf,
    pub display: String,
    /// The case's own terminal server directory (`BRINDLE_SERVER_DIR`), so
    /// local tabs never use the user's server. On Linux it is under the
    /// system temp directory, not `dir`, because a holder's socket path must
    /// fit in `sun_path` (108 bytes); on Windows it is `dir`'s `server`.
    pub server_dir: PathBuf,
    scale: Scale,
}

impl Ctx {
    /// `secs`, stretched by `BRINDLE_E2E_SLOW`.
    pub fn secs(&self, secs: f64) -> f64 {
        self.scale.secs(secs)
    }

    /// The case's waits, stretched by `BRINDLE_E2E_SLOW` ([`wait`]).
    pub fn scale(&self) -> Scale {
        self.scale
    }

    /// Brindle's arguments that stretch its step spacing the same way. None
    /// at the normal speed, so `BRINDLE_E2E_BIN` can name an older build.
    pub fn step_interval(&self) -> Vec<String> {
        if self.scale == Scale::new(1.0) {
            Vec::new()
        } else {
            vec!["--step-interval".to_string(), self.secs(1.0).to_string()]
        }
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
    let selection = Selection::parse(&args).unwrap_or_else(|why| usage(&why));
    let mut selected: Vec<&Case> = selection.matching(cases.iter().collect(), |case| case.name);
    if let Some((k, n)) = selection.shard {
        let total = selected.len();
        selected = take_shard(selected, k, n);
        println!("e2e: shard {k}/{n}, {} of {total} cases", selected.len());
    }

    let display = std::env::var("BRINDLE_E2E_DISPLAY").unwrap_or_else(|_| ":0".into());
    // Cases that open windows need an X display on Linux. On Windows they
    // open on the desktop the suite runs on.
    #[cfg(target_os = "linux")]
    let has_display = x11rb::connect(Some(&display)).is_ok();
    #[cfg(not(target_os = "linux"))]
    let has_display = true;
    let bin = std::env::var_os("BRINDLE_E2E_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_brindle")));
    let slow = std::env::var("BRINDLE_E2E_SLOW").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("e2e");
    let helper = helper(&bin);
    // Local tabs run in the terminal server, so every case that runs
    // Brindle needs the helper.
    if cfg!(any(target_os = "linux", windows)) && has_display && selected.iter().any(|case| case.display) {
        let default = PathBuf::from(env!("CARGO_BIN_EXE_brindle")).with_file_name(helper_name());
        if let Err(why) = check_helper(&helper, &default) {
            eprintln!("e2e: {why}");
            std::process::exit(1);
        }
    }

    let (mut passed, mut failed, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for (n, case) in selected.into_iter().enumerate() {
        let started = Instant::now();
        let outcome = if case.display && !has_display {
            Ok(Outcome::Skip(format!("no X display at {display}")))
        } else {
            let dir = root.join(case.name);
            let server_dir = private_dir(Suite::E2e, &dir, n);
            // On Windows the server directory is the same one every run, so
            // roles an interrupted run left would be taken for this case's,
            // and their images would keep the directory from being emptied.
            if cfg!(windows) {
                end_roles(&server_dir);
            }
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("case directory");
            let ctx = Ctx {
                name: case.name,
                dir,
                bin: bin.clone(),
                helper: helper.clone(),
                display: display.clone(),
                server_dir,
                scale: Scale::new(slow),
            };
            let outcome = (case.run)(&ctx);
            match common::server::stop(&ctx.helper, &ctx.server_dir, &ctx.dir) {
                // A server left running fails a case that passed.
                Err(why) if !matches!(outcome, Err(_)) => Err(why),
                Err(why) => {
                    println!("         {why}");
                    outcome
                }
                Ok(()) => outcome,
            }
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

fn usage(why: &str) -> ! {
    eprintln!("e2e: {why}");
    std::process::exit(2);
}

/// The harness's own unit tests (`--self-test`); there is no libtest here.
fn self_test() -> i32 {
    let tests: &[(&str, fn() -> Result)] = &[
        ("shards", common::self_test_shard),
        ("scaled waits", wait::self_test),
        ("dump after the last step", brindle::self_test_dump_after_last_step),
        ("dump parsing", brindle::self_test_dump),
        ("query parsing", query::self_test),
        ("cell rectangles", capture::self_test),
        ("pixel scans", capture::self_test_scan),
        ("remote parsing", cases::self_test_remote),
        ("server helpers", common::server::self_test),
        #[cfg(windows)]
        ("windows processes", winproc::self_test),
        #[cfg(windows)]
        ("windows paths", common::server::self_test_under),
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
