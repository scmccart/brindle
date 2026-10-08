//! The cases: keystroke latency and vtebench throughput, each in a local
//! shell tab and in a tmux control-mode tab.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::common::tmux::TmuxServer;
use crate::report::{LatencyResult, ThroughputResult, parse_dat, parse_latency};
use crate::{Ctx, Measured, Result, ensure};

fn config(socket: &str) -> String {
    format!(
        r#"default_profile = "Shell"
padding = 6

[font]
size = 13.0

[cursor]
blink = false

[[profiles]]
name = "Shell"
command = "bash"
args = ["--norc", "--noprofile"]

[[profiles]]
name = "tmux"
tmux = "control"
tmux_session = "e2e"
tmux_args = ["-L", "{socket}", "-f", "/dev/null"]
"#
    )
}

/// Starts Brindle with the perf config; its stderr goes to `<label>.log`.
fn spawn(ctx: &Ctx, socket: &str, label: &str, args: &[&str]) -> Result<Child> {
    let config_path = ctx.dir.join("config.toml");
    std::fs::write(&config_path, config(socket)).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(ctx.dir.join(format!("{label}.log"))).map_err(|e| e.to_string())?;
    Command::new(&ctx.bin)
        .args(args)
        .env("BRINDLE_CONFIG", &config_path)
        .env("DISPLAY", &ctx.display)
        .env_remove("WAYLAND_DISPLAY")
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

fn stop(mut child: Child) {
    child.kill().ok();
    child.wait().ok();
}

fn latency(ctx: &Ctx, profile: &str, socket: &str) -> Result<Measured> {
    let log = ctx.dir.join("latency.jsonl");
    let samples = ctx.samples.to_string();
    let child = spawn(
        ctx,
        socket,
        "brindle",
        &["-p", profile, "--latency-log", log.to_str().expect("utf-8 path"), "--latency-bench", &samples],
    )?;
    // Each keystroke waits at most 1 s for its echo, plus a short gap.
    let limit = Duration::from_secs(60) + Duration::from_millis(1050) * (ctx.samples as u32 + 10);
    let stdout = wait(child, "brindle", limit)?;
    ctx.save("brindle.out", &stdout);
    let result: LatencyResult = parse_latency(&stdout).map_err(|e| format!("{e}; see brindle.log"))?;
    ensure!(result.samples == ctx.samples as u64, "{} samples, expected {}", result.samples, ctx.samples);
    Ok(Measured::Latency(result))
}

pub fn latency_shell(ctx: &Ctx) -> Result<Measured> {
    latency(ctx, "Shell", "unused")
}

pub fn latency_tmux(ctx: &Ctx) -> Result<Measured> {
    let server = TmuxServer::with_prefix("brindle-perf", 80, 24)?;
    latency(ctx, "tmux", &server.socket)
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
        "-p",
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
    let child = spawn(ctx, "unused", "brindle", &args)?;
    wait(child, "brindle", VTEBENCH_LIMIT)?;
    throughput_result(ctx, &dat)
}

pub fn throughput_tmux(ctx: &Ctx) -> Result<Measured> {
    let vtebench = match ctx.vtebench() {
        Ok(v) => v,
        Err(why) => return Ok(Measured::Skip(why)),
    };
    let server = TmuxServer::with_prefix("brindle-perf", 80, 24)?;
    let child = spawn(ctx, &server.socket, "brindle", &["-p", "tmux"])?;

    // vtebench should only start once Brindle is attached.
    let attached = Instant::now() + Duration::from_secs(15);
    while server.run(&["list-clients"]).unwrap_or_default().trim().is_empty() {
        if Instant::now() > attached {
            drop(server);
            stop(child);
            return Err("Brindle didn't attach to tmux; see brindle.log".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_secs(1));

    let dat = ctx.dir.join("results.dat");
    let done = ctx.dir.join("done");
    let quote = |p: &Path| format!("'{}'", p.display());
    let line = format!(
        "stty size > {}; {} -b {} --dat {}; touch {}",
        quote(&ctx.dir.join("grid")),
        quote(&vtebench.bin),
        quote(&vtebench.benchmarks),
        quote(&dat),
        quote(&done)
    );
    let pane = server.active_pane()?;
    server.type_line(&pane, &line)?;
    let deadline = Instant::now() + VTEBENCH_LIMIT;
    while !done.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
    }
    // End the server first, so Brindle leaves no control client behind with
    // format subscriptions (see CLAUDE.md). Brindle then falls back to a
    // shell tab instead of quitting, so it is stopped here.
    drop(server);
    std::thread::sleep(Duration::from_secs(1));
    stop(child);
    ensure!(done.exists(), "vtebench didn't finish within {}s", VTEBENCH_LIMIT.as_secs());
    throughput_result(ctx, &dat)
}
