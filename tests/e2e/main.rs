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
//! build's), `BRINDLE_E2E_SLOW` stretches every delay. Each case leaves its
//! config, dumps, captures and transcripts in `target/tmp/e2e/<case>/`.

mod brindle;
#[path = "../common/mod.rs"]
mod common;
mod capture;
mod cases;
mod query;
#[cfg(windows)]
mod winproc;

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
    /// The terminal server helper Brindle runs (`BRINDLE_REMOTE_AGENT`, on
    /// Windows `BRINDLE_SERVER_HELPER`, else `brindle-remote` beside `bin`).
    pub helper: PathBuf,
    pub display: String,
    /// The case's own terminal server directory (`BRINDLE_SERVER_DIR`), so
    /// local tabs never use the user's server. On Linux it is under the
    /// system temp directory, not `dir`, because a holder's socket path must
    /// fit in `sun_path` (108 bytes); on Windows it is `dir`'s `server`.
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
    // Local tabs run in the terminal server, so every case that runs
    // Brindle needs the helper.
    if cfg!(any(target_os = "linux", windows)) && has_display && selected.iter().any(|case| case.display) {
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
            let server_dir = server_dir(&dir, n);
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

/// The variable that names another terminal server helper. On Windows
/// `BRINDLE_REMOTE_AGENT` is the Linux helper the `remote-*` cases deploy,
/// and Brindle doesn't read it for the local server.
const HELPER_VARIABLE: &str = if cfg!(windows) { "BRINDLE_SERVER_HELPER" } else { "BRINDLE_REMOTE_AGENT" };

/// The helper's file name beside a binary.
fn helper_name() -> String {
    format!("brindle-remote{}", std::env::consts::EXE_SUFFIX)
}

/// The helper Brindle runs its terminal server with: [`HELPER_VARIABLE`],
/// else the `brindle-remote` beside the binary, as Brindle looks for it.
fn helper(bin: &std::path::Path) -> PathBuf {
    std::env::var_os(HELPER_VARIABLE).map(PathBuf::from).unwrap_or_else(|| bin.with_file_name(helper_name()))
}

/// Case number `n`'s terminal server directory, given its directory `dir`.
fn server_dir(dir: &std::path::Path, n: usize) -> PathBuf {
    if cfg!(windows) {
        dir.join("server")
    } else {
        std::env::temp_dir().join(format!("brindle-e2e-{}-{n}", std::process::id()))
    }
}

/// The helper must be there, and when it is this build's, no older than
/// its sources (`cargo test --test e2e` doesn't rebuild it).
fn check_helper(helper: &std::path::Path) -> Result {
    let built = std::fs::metadata(helper).and_then(|m| m.modified()).map_err(|_| {
        format!(
            "the terminal server helper isn't built ({} is missing): run `cargo build`, which builds it, \
             or set {HELPER_VARIABLE}",
            helper.display()
        )
    })?;
    let own = PathBuf::from(env!("CARGO_BIN_EXE_brindle")).with_file_name(helper_name());
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
/// directory, keeping its logs in the case's directory. Fails when a core
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
    let left = roles_left(&ctx.server_dir, Duration::from_secs(5));
    if !left.is_empty() {
        let names: Vec<String> = left.iter().map(|(pid, role)| format!("{role} {pid}")).collect();
        problem.get_or_insert(format!("left running after `brindle-remote stop`: {}", names.join(", ")));
        end_roles(&ctx.server_dir);
    }
    save_logs(ctx);
    // Windows keeps a role's image (its copy under the directory's `bin`)
    // from being deleted for a moment after the role is gone.
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Err(error) = std::fs::remove_dir_all(&ctx.server_dir) {
        if !cfg!(windows) {
            break;
        }
        if Instant::now() > deadline {
            problem.get_or_insert(format!("can't remove {}: {error}", ctx.server_dir.display()));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    problem.map_or(Ok(()), Err)
}

/// Copies the core's log to `server-core.log` in the case's directory, and
/// on Windows each holder's to `server-holder-….log`: a holder's says which
/// console host its sessions got.
fn save_logs(ctx: &Ctx) {
    std::fs::copy(ctx.server_dir.join("core.log"), ctx.dir.join("server-core.log")).ok();
    if cfg!(windows) {
        for log in files(&ctx.server_dir) {
            let name = log.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if log.parent() == Some(&ctx.server_dir) && name.starts_with("holder-") && name.ends_with(".log") {
                std::fs::copy(&log, ctx.dir.join(format!("server-{name}"))).ok();
            }
        }
    }
}

/// The roles still using `dir` once `limit` has passed; none as soon as
/// they are all gone.
fn roles_left(dir: &std::path::Path, limit: Duration) -> Vec<(u32, String)> {
    let deadline = Instant::now() + limit;
    let mut left = server_processes(dir);
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        left = server_processes(dir);
    }
    left
}

/// Kills the roles using `dir`, and on Windows waits for them to be gone.
fn end_roles(dir: &std::path::Path) {
    for (pid, _) in server_processes(dir) {
        #[cfg(unix)]
        std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status().ok();
        #[cfg(windows)]
        winproc::kill(pid);
    }
    if cfg!(windows) {
        roles_left(dir, Duration::from_secs(5));
    }
}

/// The `serve` and `hold` processes using `dir` (their environment names
/// it), with their roles.
#[cfg(unix)]
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

/// The `serve` and `hold` processes using `dir`, with their roles: on
/// Windows a role runs from its build's copy of the helper under the
/// directory's `bin`. A copy running as anything else (the `launch` that
/// starts a role, a role whose arguments can't be read) is listed under the
/// helper's name, so nothing from the directory goes unnoticed.
#[cfg(windows)]
pub fn server_processes(dir: &std::path::Path) -> Vec<(u32, String)> {
    let copies = dir.join("bin");
    let name = helper_name();
    winproc::all()
        .into_iter()
        .filter(|process| process.name.eq_ignore_ascii_case(&name))
        .filter(|process| winproc::image(process.pid).is_some_and(|image| under(&image, &copies)))
        .map(|process| {
            let line = winproc::command_line(process.pid);
            let role = line.as_deref().and_then(winproc::first_argument).filter(|role| ["serve", "hold"].contains(role));
            (process.pid, role.map_or_else(|| name.clone(), str::to_string))
        })
        .collect()
}

/// Whether the Windows path `path` is under `dir`, however each is spelled.
#[cfg(windows)]
fn under(path: &std::path::Path, dir: &std::path::Path) -> bool {
    let spelled = |path: &std::path::Path| {
        let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
        text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text)
    };
    spelled(path).strip_prefix(&spelled(dir)).is_some_and(|rest| rest.starts_with('\\'))
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

#[cfg(windows)]
fn self_test_under() -> Result {
    let under = |path: &str, dir: &str| under(std::path::Path::new(path), std::path::Path::new(dir));
    ensure!(under(r"C:\Case\server\bin\b1\brindle-remote.exe", r"c:\case/server\bin"), "another spelling");
    ensure!(under(r"\\?\C:\case\server\bin\b1\x.exe", r"C:\case\server\bin"), "a verbatim path");
    ensure!(!under(r"C:\case\server\binary\x.exe", r"C:\case\server\bin"), "a sibling with the same prefix");
    ensure!(!under(r"C:\case\server\bin", r"C:\case\server\bin"), "the directory itself");
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
        #[cfg(windows)]
        ("windows processes", winproc::self_test),
        #[cfg(windows)]
        ("windows paths", self_test_under),
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
