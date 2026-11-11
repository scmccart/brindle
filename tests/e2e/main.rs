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
//! `:0`), so ask before running them on someone's desktop. They never send
//! synthetic input. `BRINDLE_E2E_BIN` picks the binary (default: this
//! build's), `BRINDLE_E2E_SLOW` stretches every delay. Each case leaves its
//! config, dumps, captures and transcripts in `target/tmp/e2e/<case>/`.

mod brindle;
#[path = "../common/mod.rs"]
mod common;
mod capture;
mod cases;
mod query;

use std::path::PathBuf;
use std::time::{Duration, Instant};

pub use common::Result;

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
    /// The terminal server helper Brindle runs (`BRINDLE_REMOTE_AGENT`, else
    /// `brindle-remote` beside `bin`).
    pub helper: PathBuf,
    pub display: String,
    /// The case's own terminal server directory (`BRINDLE_SERVER_DIR`), so
    /// local tabs never use the user's server. It is under the system temp
    /// directory, not `dir`, because a holder's socket path must fit in
    /// `sun_path` (108 bytes).
    pub server_dir: PathBuf,
    slow: f64,
}

impl Ctx {
    /// `secs`, stretched by `BRINDLE_E2E_SLOW`.
    pub fn secs(&self, secs: f64) -> f64 {
        secs * self.slow
    }

    /// Brindle's arguments that stretch its step spacing the same way. None
    /// at the normal speed, so `BRINDLE_E2E_BIN` can name an older build.
    pub fn step_interval(&self) -> Vec<String> {
        if self.slow == 1.0 { Vec::new() } else { vec!["--step-interval".to_string(), self.secs(1.0).to_string()] }
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
    let mut filters: Vec<&String> = Vec::new();
    let mut shard = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--shard" {
            match it.next().map(|v| parse_shard(v)) {
                Some(Ok(s)) => shard = Some(s),
                Some(Err(why)) => usage(&why),
                None => usage("--shard needs K/N"),
            }
        } else if !arg.starts_with("--") {
            filters.push(arg);
        }
    }
    let mut selected: Vec<&Case> =
        cases.iter().filter(|c| filters.is_empty() || filters.iter().any(|f| c.name.contains(f.as_str()))).collect();
    if let Some((k, n)) = shard {
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
    // Local tabs run in the terminal server on Linux, so every case that
    // runs Brindle needs the helper.
    if cfg!(target_os = "linux") && has_display && selected.iter().any(|case| case.display) {
        if let Err(why) = check_helper(&helper) {
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
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("case directory");
            let server_dir = std::env::temp_dir().join(format!("brindle-e2e-{}-{n}", std::process::id()));
            let ctx = Ctx {
                name: case.name,
                dir,
                bin: bin.clone(),
                helper: helper.clone(),
                display: display.clone(),
                server_dir,
                slow,
            };
            let outcome = (case.run)(&ctx);
            match stop_server(&ctx) {
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

/// The helper Brindle runs its terminal server with: `BRINDLE_REMOTE_AGENT`,
/// else the `brindle-remote` beside the binary, as Brindle looks for it.
fn helper(bin: &std::path::Path) -> PathBuf {
    std::env::var_os("BRINDLE_REMOTE_AGENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| bin.with_file_name(if cfg!(windows) { "brindle-remote.exe" } else { "brindle-remote" }))
}

/// The helper must be there, and when it is this build's, no older than
/// its sources (`cargo test --test e2e` doesn't rebuild it).
fn check_helper(helper: &std::path::Path) -> Result {
    let built = std::fs::metadata(helper).and_then(|m| m.modified()).map_err(|_| {
        format!(
            "the terminal server helper isn't built ({} is missing): run `cargo build`, which builds it, \
             or set BRINDLE_REMOTE_AGENT",
            helper.display()
        )
    })?;
    let own = PathBuf::from(env!("CARGO_BIN_EXE_brindle")).with_file_name("brindle-remote");
    if helper != own {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let newer = ["remote-agent/src", "remote-protocol/src", "term-core/src"]
        .iter()
        .flat_map(|dir| files(&root.join(dir)))
        .find(|file| std::fs::metadata(file).and_then(|m| m.modified()).is_ok_and(|time| time > built));
    match newer {
        Some(file) => Err(format!(
            "{} is older than {}: run `cargo build` (the e2e target doesn't rebuild it)",
            helper.display(),
            file.display()
        )),
        None => Ok(()),
    }
}

fn files(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .flat_map(|entry| {
            let path = entry.path();
            if path.is_dir() { files(&path) } else { vec![path] }
        })
        .collect()
}

/// Stops the case's terminal server, if one started, and removes its
/// directory, keeping its log in the case's directory. Fails when a core
/// or holder of the case's is still running afterwards, killing it.
fn stop_server(ctx: &Ctx) -> Result {
    if !ctx.server_dir.exists() {
        return Ok(());
    }
    let stopped = std::process::Command::new(&ctx.helper)
        .arg("stop")
        .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
        .stdin(std::process::Stdio::null())
        .output();
    let mut problem = match stopped {
        Err(error) => Some(format!("can't stop the server with {}: {error}", ctx.helper.display())),
        Ok(_) => None,
    };
    // Holders hang their sessions up first (SIGHUP, then SIGKILL after 1 s).
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut left = server_processes(&ctx.server_dir);
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        left = server_processes(&ctx.server_dir);
    }
    if !left.is_empty() {
        let names: Vec<String> = left.iter().map(|(pid, role)| format!("{role} {pid}")).collect();
        problem.get_or_insert(format!("left running after `brindle-remote stop`: {}", names.join(", ")));
        for (pid, _) in &left {
            std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status().ok();
        }
    }
    std::fs::copy(ctx.server_dir.join("core.log"), ctx.dir.join("server-core.log")).ok();
    std::fs::remove_dir_all(&ctx.server_dir).ok();
    problem.map_or(Ok(()), Err)
}

/// The `serve` and `hold` processes using `dir` (their environment names
/// it), with their roles.
pub fn server_processes(dir: &std::path::Path) -> Vec<(u32, String)> {
    let wanted = format!("BRINDLE_SERVER_DIR={}", dir.display());
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            let role = cmdline.split(|&b| b == 0).nth(1).map(|arg| String::from_utf8_lossy(arg).into_owned())?;
            if role != "serve" && role != "hold" {
                return None;
            }
            let environ = std::fs::read(entry.path().join("environ")).ok()?;
            environ.split(|&b| b == 0).any(|var| var == wanted.as_bytes()).then_some((pid, role))
        })
        .collect()
}

fn usage(why: &str) -> ! {
    eprintln!("e2e: {why}");
    std::process::exit(2);
}

/// `K/N` for `--shard`: shard K (1-based) of N.
fn parse_shard(value: &str) -> std::result::Result<(usize, usize), String> {
    let bad = || format!("--shard needs K/N with 1 <= K <= N, got {value:?}");
    let (k, n) = value.split_once('/').ok_or_else(bad)?;
    let (k, n): (usize, usize) = (k.parse().map_err(|_| bad())?, n.parse().map_err(|_| bad())?);
    if n == 0 || k == 0 || k > n {
        return Err(bad());
    }
    Ok((k, n))
}

/// Every Nth item from the Kth: round-robin, so each area of the suite (its
/// cases are registered by area) spreads across the shards.
fn take_shard<T>(items: Vec<T>, k: usize, n: usize) -> Vec<T> {
    items.into_iter().enumerate().filter(|(i, _)| i % n == k - 1).map(|(_, item)| item).collect()
}

fn self_test_shard() -> Result {
    let shards: Vec<Vec<usize>> = (1..=3).map(|k| take_shard((0..10).collect(), k, 3)).collect();
    ensure!(shards == [vec![0, 3, 6, 9], vec![1, 4, 7], vec![2, 5, 8]], "10 cases in 3 shards: {shards:?}");
    ensure!(parse_shard("2/4") == Ok((2, 4)), "2/4");
    for bad in ["0/4", "5/4", "1/0", "x", "1/", "/4", "-1/4"] {
        ensure!(parse_shard(bad).is_err(), "{bad:?} accepted");
    }
    Ok(())
}

/// The harness's own unit tests (`--self-test`); there is no libtest here.
fn self_test() -> i32 {
    let tests: &[(&str, fn() -> Result)] = &[
        ("shards", self_test_shard),
        ("dump parsing", brindle::self_test_dump),
        ("query parsing", query::self_test),
        ("cell rectangles", capture::self_test),
        ("pixel scans", capture::self_test_scan),
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
