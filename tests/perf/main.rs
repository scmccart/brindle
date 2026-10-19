//! Opt-in performance suite: keystroke latency (`--latency-bench`) and
//! vtebench throughput, each in a local shell tab and a tmux tab. It
//! reports numbers and never fails on them.
//!
//! ```sh
//! cargo build --release
//! BRINDLE_PERF=1 cargo test --test perf                 # every case
//! BRINDLE_PERF=1 cargo test --test perf -- latency      # cases matching "latency"
//! BRINDLE_PERF=1 cargo test --test perf -- --list
//! BRINDLE_PERF=1 cargo test --test perf -- --self-test  # the harness's own tests
//! ```
//!
//! Cases open Brindle windows under Xwayland (`BRINDLE_PERF_DISPLAY`,
//! default `:0`), so ask before running them on someone's desktop. They
//! never send synthetic OS input.
//! - `BRINDLE_PERF_BIN` picks the binary (default: `target/release/brindle`;
//!   the dev build would distort the numbers).
//! - `BRINDLE_PERF_SAMPLES` sets the keystrokes per latency case (500).
//! - `BRINDLE_PERF_VTEBENCH` uses an existing vtebench checkout instead of
//!   fetching the pinned commit into `target/perf-tools/vtebench`.
//! - `BRINDLE_PERF_BASELINE=<run dir>` shows changes against an earlier run.
//!
//! Each run is kept in `target/tmp/perf/<UTC time>/` (`latest` links to the
//! newest), with `summary.toml` and every case's logs and raw data.

#[path = "../common/mod.rs"]
mod common;
mod cases;
mod report;
mod vtebench;

use std::cell::OnceCell;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

pub use common::Result;
use report::{LatencyResult, Summary, ThroughputResult};

pub enum Measured {
    Latency(LatencyResult),
    Throughput(ThroughputResult),
    Skip(String),
}

pub struct Case {
    pub name: &'static str,
    pub run: fn(&Ctx) -> Result<Measured>,
}

/// What a case gets from the runner.
pub struct Ctx {
    /// Scratch and artifact directory for this case.
    pub dir: PathBuf,
    pub bin: PathBuf,
    pub display: String,
    pub samples: usize,
    target_dir: PathBuf,
    vtebench: std::rc::Rc<OnceCell<std::result::Result<vtebench::Vtebench, String>>>,
}

impl Ctx {
    /// Writes a file into the case's directory.
    pub fn save(&self, name: &str, contents: impl AsRef<[u8]>) {
        std::fs::write(self.dir.join(name), contents).ok();
    }

    /// vtebench, fetched and built on first use; `Err` says why it's missing.
    pub fn vtebench(&self) -> std::result::Result<&vtebench::Vtebench, String> {
        self.vtebench.get_or_init(|| vtebench::resolve(&self.target_dir)).as_ref().map_err(Clone::clone)
    }
}

fn cases() -> Vec<Case> {
    vec![
        Case { name: "latency-shell", run: cases::latency_shell },
        Case { name: "latency-tmux", run: cases::latency_tmux },
        Case { name: "throughput-brindle", run: cases::throughput_brindle },
        Case { name: "throughput-tmux", run: cases::throughput_tmux },
    ]
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Notes kept with the results: anything that moves the numbers.
fn environment(bin: &std::path::Path, display: &str) -> Vec<(String, String)> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let head = command_output("git", &["-C", manifest, "rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let dirty = command_output("git", &["-C", manifest, "status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let governor = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    vec![
        ("brindle".into(), command_output(bin.to_str().unwrap_or(""), &["--version"]).unwrap_or_default()),
        ("binary".into(), bin.display().to_string()),
        ("git".into(), format!("{head}{}", if dirty { "-dirty" } else { "" })),
        ("cpu_governor".into(), governor),
        ("display".into(), format!("X11 {display} (session: {session})")),
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var("BRINDLE_PERF").as_deref() != Ok("1") {
        println!("perf: skipped (set BRINDLE_PERF=1 to run; it opens Brindle windows)");
        return;
    }
    if args.iter().any(|a| a == "--self-test") {
        std::process::exit(self_test());
    }
    let cases = cases();
    if args.iter().any(|a| a == "--list") {
        for case in &cases {
            println!("{}", case.name);
        }
        return;
    }
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let selected: Vec<&Case> =
        cases.iter().filter(|c| filters.is_empty() || filters.iter().any(|f| c.name.contains(f.as_str()))).collect();

    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let target_dir = tmp.parent().expect("target/tmp has a parent").to_path_buf();
    let bin = std::env::var_os("BRINDLE_PERF_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| target_dir.join("release/brindle"));
    if !bin.exists() {
        println!("perf: {} doesn't exist; run `cargo build --release` (or set BRINDLE_PERF_BIN)", bin.display());
        std::process::exit(1);
    }
    if cfg!(not(target_os = "linux")) {
        println!("perf: the cases need X11 and tmux, so they run on Linux only");
        return;
    }
    let display = std::env::var("BRINDLE_PERF_DISPLAY").unwrap_or_else(|_| ":0".into());
    #[cfg(target_os = "linux")]
    if x11rb::connect(Some(&display)).is_err() {
        println!("perf: no X display at {display}");
        std::process::exit(1);
    }
    let samples = std::env::var("BRINDLE_PERF_SAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(500);
    // Read before `latest` moves, so `BRINDLE_PERF_BASELINE=…/latest` works.
    let baseline = match std::env::var_os("BRINDLE_PERF_BASELINE") {
        Some(dir) => match Summary::load(&PathBuf::from(dir).join("summary.toml")) {
            Ok(summary) => Some(summary),
            Err(why) => {
                println!("perf: baseline: {why}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    let root = tmp.join("perf");
    let stamp = command_output("date", &["-u", "+%Y%m%dT%H%M%SZ"]).unwrap_or_else(|| "run".into());
    let run_dir = root.join(&stamp);
    std::fs::create_dir_all(&run_dir).expect("run directory");
    let latest = root.join("latest");
    std::fs::remove_file(&latest).ok();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&stamp, &latest).ok();

    let mut summary = Summary::default();
    for (key, value) in environment(&bin, &display) {
        println!("{key:<13} {value}");
        summary.meta.insert(key, value);
    }
    summary.meta.insert("latency_samples".into(), samples.to_string());
    println!("results       {}\n", run_dir.display());

    let vtebench_cell = std::rc::Rc::new(OnceCell::new());
    let mut failed = 0;
    for case in selected {
        let started = Instant::now();
        let dir = run_dir.join(case.name);
        std::fs::create_dir_all(&dir).expect("case directory");
        let ctx = Ctx {
            dir,
            bin: bin.clone(),
            display: display.clone(),
            samples,
            target_dir: target_dir.clone(),
            vtebench: vtebench_cell.clone(),
        };
        let outcome = (case.run)(&ctx);
        let took = started.elapsed().as_secs_f64();
        match outcome {
            Ok(Measured::Latency(result)) => {
                println!("measured {} ({took:.0}s)", case.name);
                summary.latency.insert(case.name.into(), result);
            }
            Ok(Measured::Throughput(result)) => {
                println!("measured {} ({took:.0}s)", case.name);
                summary.throughput.insert(case.name.into(), result);
            }
            Ok(Measured::Skip(why)) => println!("skipped  {}: {why}", case.name),
            Err(why) => {
                println!("FAILED   {} ({took:.0}s): {why}", case.name);
                failed += 1;
            }
        }
        if let Some(Ok(v)) = vtebench_cell.get() {
            summary.meta.insert("vtebench".into(), v.commit.clone());
        }
    }

    std::fs::write(run_dir.join("summary.toml"), summary.to_toml()).ok();
    println!("\n{}", report::render(&summary, baseline.as_ref()));
    if baseline.is_some() {
        println!("(changes against the baseline in parentheses: ms for latency, % for throughput; lower is better)");
    }
    // Numbers never fail the suite, but a case that couldn't measure does.
    if failed > 0 {
        std::process::exit(1);
    }
}

/// The harness's own unit tests (`--self-test`); there is no libtest here.
fn self_test() -> i32 {
    type Test = (&'static str, fn() -> Result);
    let tests: &[Test] = &[
        ("latency parsing", report::self_test_latency),
        ("dat parsing", report::self_test_dat),
        ("report rendering", report::self_test_render),
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
