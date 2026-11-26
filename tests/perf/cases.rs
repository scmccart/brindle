//! The cases: keystroke latency and vtebench throughput in a local shell
//! tab. The agent zone's case is in `agent.rs`.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::common::server::{Isolation, isolate};
use crate::report::{LatencyResult, ThroughputResult, parse_dat, parse_latency};
use crate::{Ctx, Measured, Result, ensure};

pub const CONFIG: &str = r#"default_profile = "Shell"
padding = 6

[font]
size = 13.0

[cursor]
blink = false

[[profiles]]
name = "Shell"
command = "bash"
args = ["--norc", "--noprofile"]
"#;

/// Starts Brindle with the perf config; its stderr goes to `<label>.log`.
fn spawn(ctx: &Ctx, label: &str, args: &[&str]) -> Result<Child> {
    spawn_with(ctx, label, CONFIG, args)
}

/// Starts Brindle with `config` (written as `<label>-config.toml`); its
/// stderr goes to `<label>.log`.
pub fn spawn_with(ctx: &Ctx, label: &str, config: &str, args: &[&str]) -> Result<Child> {
    let config_path = ctx.dir.join(format!("{label}-config.toml"));
    std::fs::write(&config_path, config).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(ctx.dir.join(format!("{label}.log"))).map_err(|e| e.to_string())?;
    let mut command = Command::new(&ctx.bin);
    // Every launch records the session and runs its tab in a terminal
    // server: never the user's directories or server.
    let state = ctx.dir.join("state");
    isolate(
        &mut command,
        &Isolation { config: &config_path, state_dir: &state, server_dir: &ctx.server_dir, display: &ctx.display },
    );
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(|e| format!("starting {}: {e}", ctx.bin.display()))
}

/// Waits for Brindle to exit, killing it after `limit`. Returns its stdout.
fn wait(mut child: Child, label: &str, limit: Duration) -> Result<String> {
    let deadline = Instant::now() + limit;
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            return Err(format!("Brindle didn't exit within {}s; see {label}.log", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut stdout = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().expect("piped"), &mut stdout).ok();
    Ok(stdout)
}

pub fn latency_shell(ctx: &Ctx) -> Result<Measured> {
    let log = ctx.dir.join("latency.jsonl");
    let samples = ctx.samples.to_string();
    let child = spawn(
        ctx,
        "brindle",
        &["terminal", "Shell", "--latency-log", log.to_str().expect("utf-8 path"), "--latency-bench", &samples],
    )?;
    // Each keystroke waits at most 1 s for its echo, plus a short gap.
    let limit = Duration::from_secs(60) + Duration::from_millis(1050) * (ctx.samples as u32 + 10);
    let stdout = wait(child, "brindle", limit)?;
    ctx.save("brindle.out", &stdout);
    let result: LatencyResult = parse_latency(&stdout).map_err(|e| format!("{e}; see brindle.log"))?;
    ensure!(result.samples == ctx.samples as u64, "{} samples, expected {}", result.samples, ctx.samples);
    Ok(Measured::Latency(result))
}

/// `stty size` output (`ROWS COLS`) as `COLSxROWS`.
fn grid(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut fields = text.split_whitespace();
    let (rows, cols) = (fields.next().ok_or("empty stty size")?, fields.next().ok_or("no columns in stty size")?);
    Ok(format!("{cols}x{rows}"))
}

/// Each vtebench benchmark runs for up to 10 s.
const VTEBENCH_LIMIT: Duration = Duration::from_secs(600);

fn throughput_result(ctx: &Ctx, dat: &Path) -> Result<Measured> {
    let text = std::fs::read_to_string(dat).map_err(|e| format!("no results ({e}); see brindle.log"))?;
    let mean_ms = parse_dat(&text).map_err(|e| format!("{}: {e}", dat.display()))?;
    ensure!(!mean_ms.is_empty(), "{} has no samples", dat.display());
    Ok(Measured::Throughput(ThroughputResult { grid: grid(&ctx.dir.join("grid"))?, mean_ms }))
}

pub fn throughput_brindle(ctx: &Ctx) -> Result<Measured> {
    let vtebench = match ctx.vtebench() {
        Ok(v) => v,
        Err(why) => return Ok(Measured::Skip(why)),
    };
    let dat = ctx.dir.join("results.dat");
    let grid = ctx.dir.join("grid");
    // The PTY starts at 80x24 until the window's first layout resizes it.
    let script = r#"sleep 1; stty size > "$0"; exec "$1" -b "$2" --dat "$3""#;
    let args = [
        "terminal",
        "Shell",
        "-e",
        "sh",
        "-c",
        script,
        grid.to_str().expect("utf-8 path"),
        vtebench.bin.to_str().expect("utf-8 path"),
        vtebench.benchmarks.to_str().expect("utf-8 path"),
        dat.to_str().expect("utf-8 path"),
    ];
    let child = spawn(ctx, "brindle", &args)?;
    wait(child, "brindle", VTEBENCH_LIMIT)?;
    throughput_result(ctx, &dat)
}
