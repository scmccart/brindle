//! Opt-in performance suite: keystroke latency (`--latency-bench`) and
//! vtebench throughput, in a local shell tab, a Markdown reply streamed
//! into an agent zone by the fake ACP agent, a long agent conversation
//! attached again, and a command's output streamed into a tool call's row.
//! It reports numbers and never fails on them.
//!
//! ```sh
//! cargo build --release
//! BRINDLE_PERF=1 cargo test --test perf                 # every case
//! BRINDLE_PERF=1 cargo test --test perf -- latency      # cases matching "latency"
//! BRINDLE_PERF=1 cargo test --test perf -- --list       # the cases selected
//! BRINDLE_PERF=1 cargo test --test perf -- --shard 1/2  # every 2nd case from the 1st
//! BRINDLE_PERF=1 cargo test --test perf -- --self-test  # the harness's own tests
//! ```
//!
//! `--shard K/N` takes every Nth case from the Kth, after the filters, as
//! the e2e suite's does, so the cases can be spread over CI jobs; `--list`
//! prints the cases the filters and shard select.
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
//! newest), with `summary.toml` and every case's logs and raw data. Each
//! case runs in a terminal server of its own (`tests/common/server.rs`),
//! which needs `brindle-remote` beside the release binary. The agent cases
//! need the fake ACP agent (`tests/common/fake_agent.rs`: beside the
//! release build, or `BRINDLE_FAKE_ACP_AGENT`) and skips without it.

#[path = "../common/mod.rs"]
mod common;
mod agent;
mod cases;
mod report;
mod vtebench;

use std::cell::OnceCell;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use common::Selection;
use common::server::{Suite, private_dir};
pub use common::Result;
use report::{AttachResult, LatencyResult, StreamResult, Summary, ThroughputResult};

pub enum Measured {
    Latency(LatencyResult),
    Throughput(ThroughputResult),
    Stream(StreamResult),
    Attach(AttachResult),
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
    /// This build's release binary, beside which `cargo build --release`
    /// puts the fake ACP agent.
    pub default_bin: PathBuf,
    /// The terminal server helper Brindle runs (`BRINDLE_REMOTE_AGENT`,
    /// else `brindle-remote` beside `bin`).
    pub helper: PathBuf,
    pub display: String,
    /// The case's own terminal server directory (`BRINDLE_SERVER_DIR`), so
    /// its tab never runs in the user's server.
    pub server_dir: PathBuf,
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
        Case { name: "throughput-brindle", run: cases::throughput_brindle },
        Case { name: "agent-markdown-stream", run: agent::markdown_stream },
        Case { name: "agent-attach-long", run: agent::attach_long },
        Case { name: "agent-tool-output", run: agent::tool_output },
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
        // Runs before this note was kept used the user's own server.
        ("server".into(), "private per case".into()),
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
    let selection = Selection::parse(&args).unwrap_or_else(|why| usage(&why));
    let selected: Vec<&Case> = selection.select(cases.iter().collect(), |case| case.name);
    if args.iter().any(|a| a == "--list") {
        for case in &selected {
            println!("{}", case.name);
        }
        return;
    }
    if let Some((k, n)) = selection.shard {
        println!("perf: shard {k}/{n}, {} case(s)", selected.len());
    }

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
        println!("perf: the cases need X11, so they run on Linux only");
        return;
    }
    // The case's local tab runs in a terminal server of its own.
    let helper = common::server::helper(&bin);
    let default_bin = target_dir.join("release").join(format!("brindle{}", std::env::consts::EXE_SUFFIX));
    if let Err(why) = common::server::check_helper(&helper, &bin, &default_bin) {
        println!("perf: {why}");
        std::process::exit(1);
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
    for (n, case) in selected.into_iter().enumerate() {
        let started = Instant::now();
        let dir = run_dir.join(case.name);
        std::fs::create_dir_all(&dir).expect("case directory");
        let ctx = Ctx {
            server_dir: private_dir(Suite::Perf, &dir, n),
            dir,
            bin: bin.clone(),
            default_bin: default_bin.clone(),
            helper: helper.clone(),
            display: display.clone(),
            samples,
            target_dir: target_dir.clone(),
            vtebench: vtebench_cell.clone(),
        };
        let outcome = (case.run)(&ctx);
        // A run that left a role running, or ran one in the user's systemd
        // manager, isn't one to compare: it fails, so its numbers stay out
        // of the summary.
        let outcome = match common::server::stop(&ctx.helper, &ctx.server_dir, &ctx.dir) {
            Err(why) if outcome.is_ok() => Err(why),
            Err(why) => {
                println!("         {why}");
                outcome
            }
            Ok(()) => outcome,
        };
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
            Ok(Measured::Stream(result)) => {
                println!("measured {} ({took:.0}s)", case.name);
                summary.stream.insert(case.name.into(), result);
            }
            Ok(Measured::Attach(result)) => {
                println!("measured {} ({took:.0}s)", case.name);
                summary.attach.insert(case.name.into(), result);
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

    let written = write_summary(&run_dir, &summary);
    println!("\n{}", report::render(&summary, baseline.as_ref()));
    if baseline.is_some() {
        println!("(changes against the baseline in parentheses: ms for latency, % for throughput; lower is better)");
    }
    // Numbers that weren't kept can't be compared with later runs.
    if let Err(why) = written {
        println!("perf: {why}");
        std::process::exit(1);
    }
    // Numbers never fail the suite, but a case that couldn't measure does.
    if failed > 0 {
        std::process::exit(1);
    }
}

/// Writes `summary.toml` into the run directory.
fn write_summary(run_dir: &std::path::Path, summary: &Summary) -> Result {
    let path = run_dir.join("summary.toml");
    std::fs::write(&path, summary.to_toml()).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// A summary written into a directory that can't hold it is an error that
/// names the file, and one into a directory that can is on disk.
fn self_test_summary() -> Result {
    let missing = std::env::temp_dir().join(format!("brindle-perf-self-test-{}", std::process::id())).join("absent");
    let failed = write_summary(&missing, &Summary::default());
    crate::ensure!(
        failed.as_ref().is_err_and(|why| why.contains("summary.toml")),
        "an unwritable directory should fail, naming the file: {failed:?}"
    );
    let dir = std::env::temp_dir().join(format!("brindle-perf-self-test-{}-ok", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let written = write_summary(&dir, &Summary::default());
    let on_disk = dir.join("summary.toml").is_file();
    std::fs::remove_dir_all(&dir).ok();
    crate::ensure!(written.is_ok() && on_disk, "a writable directory should get the file: {written:?}");
    Ok(())
}

fn usage(why: &str) -> ! {
    eprintln!("perf: {why}");
    eprintln!("usage: BRINDLE_PERF=1 cargo test --test perf -- [FILTER…] [--shard K/N] [--list | --self-test]");
    std::process::exit(2);
}

/// The harness's own unit tests (`--self-test`); there is no libtest here.
fn self_test() -> i32 {
    type Test = (&'static str, fn() -> Result);
    let tests: &[Test] = &[
        ("shards", common::self_test_shard),
        ("latency parsing", report::self_test_latency),
        ("dat parsing", report::self_test_dat),
        ("report rendering", report::self_test_render),
        ("server helpers", common::server::self_test),
        ("summary write", self_test_summary),
        ("agent stream", agent::self_test),
        ("agent attach", agent::self_test_attach),
        ("agent tool output", agent::self_test_tool_output),
        ("fake agent lookup", common::fake_agent::self_test),
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
