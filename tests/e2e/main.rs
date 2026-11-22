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
//! someone's desktop. They never send synthetic input. `BRINDLE_E2E_BIN`
//! picks the binary (default: this build's; an installed or unpacked
//! release works too, its helper found beside it as Brindle finds it and
//! checked for its release rather than against sources:
//! `common::server::check_helper`), `BRINDLE_E2E_SLOW` stretches every delay
//! and every wait's limit (the cases wait through `wait.rs`'s [`Scale`],
//! never a fixed sleep). Each case leaves its config, dumps, captures and
//! transcripts in `target/tmp/e2e/<case>/`.

mod brindle;
#[path = "../common/mod.rs"]
mod common;
mod capture;
mod cases;
mod query;
mod switches;
mod wait;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[cfg(windows)]
use common::server::under;
use common::server::{Suite, check_helper, end_roles, helper, private_dir, server_processes};
#[cfg(windows)]
use common::winproc;
use common::{Selection, take_shard};
pub use common::Result;
use switches::{Listing, Readiness, SwitchHost, TestSwitch};
pub use wait::Scale;

pub enum Outcome {
    Pass,
    Skip(String),
}

pub struct Case {
    pub name: &'static str,
    /// Runs Brindle, so it needs an X display.
    pub display: bool,
    /// The fault switches the case sets, which its binaries must list
    /// (`--list-fault-switches`) for it to run (`testing R18`).
    pub needs: &'static [TestSwitch],
    pub run: fn(&Ctx) -> Result<Outcome>,
}

impl Case {
    /// A case that opens windows and needs no switch.
    pub fn display(name: &'static str, run: fn(&Ctx) -> Result<Outcome>) -> Self {
        Case { name, display: true, needs: &[], run }
    }

    /// A case that opens no window.
    pub fn headless(name: &'static str, run: fn(&Ctx) -> Result<Outcome>) -> Self {
        Case { name, display: false, needs: &[], run }
    }

    /// A case that opens windows and sets the fault switches `needs`.
    pub fn needing(name: &'static str, needs: &'static [TestSwitch], run: fn(&Ctx) -> Result<Outcome>) -> Self {
        Case { name, display: true, needs, run }
    }
}

/// What a case gets from the runner.
pub struct Ctx {
    pub name: &'static str,
    /// The fault switches the case declared ([`Case::needs`]); `Run::env`
    /// refuses any other `BRINDLE_TEST_*` variable.
    pub needs: &'static [TestSwitch],
    /// Scratch and artifact directory for this case, emptied before it runs.
    pub dir: PathBuf,
    pub bin: PathBuf,
    /// The terminal server helper Brindle runs (`BRINDLE_REMOTE_AGENT`, on
    /// Windows `BRINDLE_SERVER_HELPER`, else beside `bin` by Brindle's names
    /// for it: on Linux an archive's `brindle-remote-linux-x86_64`, then
    /// `brindle-remote`).
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
    if cfg!(any(target_os = "linux", windows))
        && has_display
        && selected.iter().any(|case| case.display)
        && let Err(why) = check_helper(&helper, &bin, Path::new(env!("CARGO_BIN_EXE_brindle")))
    {
        eprintln!("e2e: {why}");
        std::process::exit(1);
    }
    if std::env::var_os("BRINDLE_E2E_BIN").is_some() {
        println!("e2e: {} with {}", bin.display(), helper.display());
    }

    let required = std::env::var("BRINDLE_E2E_SWITCHES_REQUIRED").as_deref() == Ok("1");
    let mut probes = Probes::new(&bin, &helper, &root, &display, Scale::new(slow));

    let (mut passed, mut failed, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for (n, case) in selected.into_iter().enumerate() {
        let started = Instant::now();
        // Before the case's directories are filled, so no window opens for
        // a case whose binary lacks a switch it needs.
        let ready = if case.display && !has_display { Readiness::Run } else { probes.readiness(case.needs, required) };
        let outcome = if case.display && !has_display {
            Ok(Outcome::Skip(format!("no X display at {display}")))
        } else if let Readiness::Skip(why) = ready {
            Ok(Outcome::Skip(why))
        } else if let Readiness::Fail(why) = ready {
            Err(why)
        } else {
            let dir = root.join(case.name);
            let server_dir = private_dir(Suite::E2e, &dir, n);
            // On Windows the server directory is the same one every run, so
            // roles an interrupted run left would be taken for this case's,
            // and their images would keep the directory from being emptied.
            let cleaned = if cfg!(windows) { end_roles(&server_dir) } else { Ok(()) };
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("case directory");
            let ctx = Ctx {
                name: case.name,
                needs: case.needs,
                dir,
                bin: bin.clone(),
                helper: helper.clone(),
                display: display.clone(),
                server_dir,
                scale: Scale::new(slow),
            };
            let outcome = cleaned.and_then(|()| (case.run)(&ctx));
            match common::server::stop(&ctx.helper, &ctx.server_dir, &ctx.dir) {
                // A server left running fails a case that passed.
                Err(why) if outcome.is_ok() => Err(why),
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

/// What the binaries under test say they honour (`--list-fault-switches`),
/// asked lazily and once per run: a host is probed only when a selected
/// case needs one of its switches.
struct Probes<'a> {
    bin: &'a Path,
    helper: &'a Path,
    scratch: PathBuf,
    display: &'a str,
    scale: Scale,
    brindle: Option<Listing>,
    helper_listing: Option<Listing>,
}

impl<'a> Probes<'a> {
    fn new(bin: &'a Path, helper: &'a Path, root: &Path, display: &'a str, scale: Scale) -> Self {
        Probes { bin, helper, scratch: root.join("probe"), display, scale, brindle: None, helper_listing: None }
    }

    /// Whether a case needing `needs` can run, probing the hosts it needs.
    fn readiness(&mut self, needs: &[TestSwitch], required: bool) -> Readiness {
        let wants = |host| needs.iter().any(|switch| switch.host() == host);
        if wants(SwitchHost::Brindle) && self.brindle.is_none() {
            self.brindle = Some(self.probe(self.bin, "brindle"));
        }
        if wants(SwitchHost::Helper) && self.helper_listing.is_none() {
            self.helper_listing = Some(self.probe(self.helper, "helper"));
        }
        let unasked = |binary: &Path| Listing::unasked(binary);
        let brindle = self.brindle.clone().unwrap_or_else(|| unasked(self.bin));
        let helper = self.helper_listing.clone().unwrap_or_else(|| unasked(self.helper));
        switches::readiness(needs, &brindle, &helper, required)
    }

    /// Runs `binary --list-fault-switches` isolated, killed past 10 s
    /// (stretched), with its output in the probe's scratch directory.
    fn probe(&self, binary: &Path, label: &str) -> Listing {
        let switches = self.ask(binary, label);
        Listing { binary: binary.to_path_buf(), switches }
    }

    fn ask(&self, binary: &Path, label: &str) -> Result<std::collections::BTreeSet<String>> {
        let dir = self.scratch.join(label);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let (out_path, err_path) = (dir.join("stdout"), dir.join("stderr"));
        let file = |path: &Path| std::fs::File::create(path).map_err(|e| format!("creating {}: {e}", path.display()));
        let mut command = Command::new(binary);
        common::server::isolate(
            &mut command,
            &common::server::Isolation {
                config: &dir.join("config.toml"),
                state_dir: &dir.join("state"),
                server_dir: &dir.join("server"),
                display: self.display,
            },
        );
        let mut child = command
            .arg("--list-fault-switches")
            .stdin(Stdio::null())
            .stdout(file(&out_path)?)
            .stderr(file(&err_path)?)
            .spawn()
            .map_err(|e| format!("couldn't run {} --list-fault-switches: {e}", binary.display()))?;
        let exited = self.scale.poll(10.0, &format!("{} --list-fault-switches exiting", binary.display()), || {
            child.try_wait().map_err(|e| e.to_string())
        });
        let status = match exited {
            Ok(status) => status,
            Err(why) => {
                child.kill().ok();
                child.wait().ok();
                return Err(why);
            }
        };
        if !status.success() {
            let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
            return Err(format!("{} --list-fault-switches exited with {status}: {}", binary.display(), stderr.trim()));
        }
        let text = std::fs::read_to_string(&out_path).map_err(|e| format!("reading {}: {e}", out_path.display()))?;
        Ok(switches::parse_listing(&text))
    }
}

fn usage(why: &str) -> ! {
    eprintln!("e2e: {why}");
    std::process::exit(2);
}

/// One of the harness's own unit tests.
type SelfTest = fn() -> Result;

/// The harness's own unit tests (`--self-test`); there is no libtest here.
fn self_test() -> i32 {
    let tests: &[(&str, SelfTest)] = &[
        ("shards", common::self_test_shard),
        ("scaled waits", wait::self_test),
        ("dump after the last step", brindle::self_test_dump_after_last_step),
        ("dump parsing", brindle::self_test_dump),
        ("session listing", brindle::self_test_list),
        ("query parsing", query::self_test),
        ("cell rectangles", capture::self_test),
        ("pixel scans", capture::self_test_scan),
        ("remote parsing", cases::self_test_remote),
        ("clipboard helpers", cases::self_test_clipboard),
        ("server helpers", common::server::self_test),
        ("fault switches", switches::self_test),
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
