//! Offering the running core an upgrade to the helper Brindle found.
//!
//! After `WELCOME`, Brindle compares its helper with the core:
//! - a higher semver is newer and a lower one older;
//! - with equal versions (dogfooding builds keep the version), the helper is
//!   newer when its build id differs and its file is newer than the running
//!   core's binary. That binary's mtime is read through `/proc/<core
//!   pid>/exe`, which still reaches the running file after an install
//!   replaced it, so `WELCOME` needs no extra field. On Windows the core
//!   runs from a per-build copy that keeps the helper's file time, found
//!   with `QueryFullProcessImageNameW`.
//!
//! A newer helper must pass `<helper> check --holder-protocols LIST` (the
//! running holders' H1 versions), else the running core stays and is used.
//! Then Brindle sends `STEP_DOWN`, waits for the core to close the
//! connection, and starts the new core from its helper. Holders start a
//! core of their own build again if the new one won't start, so Brindle
//! connects to whichever core answers. The start thread of the tab that
//! asked waits for all of this, with the tab's input queued.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use super::{Connected, Endpoint, Stream, connection::RECONNECT_LIMIT, hello, start_core, sys, try_connect};
use crate::remote::hide_window;
use crate::remote::protocol::build_id;
use crate::remote::protocol::upgrade::{Build, is_newer};
use crate::remote::protocol::client::{Request, ToClient, ToCore, Welcome};
use crate::remote::protocol::Frame;

/// How long a stepping-down core may take to checkpoint its sessions and
/// answer (it waits up to 10 s per holder).
const STEP_DOWN_LIMIT: Duration = Duration::from_secs(60);
/// How long `--version` and `check` may take.
const HELPER_LIMIT: Duration = Duration::from_secs(5);

/// What [`offer`] did, and the connection to use.
pub enum Offered {
    /// The core stepped down for the helper's: the new core's connection.
    Upgraded(Connected),
    /// The helper isn't newer: the running core's.
    Current(Connected),
    /// The helper is newer, or couldn't be read, but the running core stays,
    /// and why: the caller logs it.
    Kept { connected: Connected, why: String },
}

impl Offered {
    /// The connection to use.
    pub fn connected(self) -> Connected {
        match self {
            Self::Upgraded(connected) | Self::Current(connected) | Self::Kept { connected, .. } => connected,
        }
    }
}

/// What to do with the running core.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Keep it: the helper isn't newer.
    Current,
    /// Keep it, though the helper may be newer, and why.
    Keep(String),
    /// Step it down for the helper's core.
    StepDown,
}

/// Whether the running core (`core`) steps down for the helper (`helper`,
/// or why its build couldn't be read): only for a newer helper, and only
/// once `check` (the helper's own check against the running holders, run
/// only then) has passed.
fn plan(helper: &Result<Build, String>, core: &Build, check: impl FnOnce() -> Result<(), String>) -> Plan {
    let helper = match helper {
        Ok(helper) => helper,
        Err(error) => return Plan::Keep(format!("reading the helper's build: {error}")),
    };
    if !is_newer(helper, core) {
        return Plan::Current;
    }
    match check() {
        Ok(()) => Plan::StepDown,
        Err(error) => Plan::Keep(format!(
            "the helper {} ({}) refused the running holders: {error}",
            helper.version, helper.build
        )),
    }
}

/// Upgrades the core behind `connected` when the helper is newer, and
/// returns the connection to use: the new core's, or the running one's
/// when the helper isn't newer, can't be read, fails its check, or the
/// step-down is refused (a [`Offered::Kept`], with why).
pub fn offer(endpoint: &Endpoint, token: &str, connected: Connected) -> Result<Offered> {
    let helper = helper_build(&endpoint.helper).map_err(|error| format!("{error:#}"));
    let core = core_build(&connected.welcome);
    log::debug!("terminal server upgrade check: helper {helper:?}, running core {core:?}");
    let check = || check(&endpoint.helper, &connected.welcome.holder_protocols).map_err(|error| format!("{error:#}"));
    match plan(&helper, &core, check) {
        Plan::Current => return Ok(Offered::Current(connected)),
        Plan::Keep(why) => return Ok(Offered::Kept { connected, why }),
        Plan::StepDown => {}
    }
    if let Ok(helper) = &helper {
        log::info!(
            "upgrading the terminal server from {} ({}) to {} ({})",
            core.version,
            core.build,
            helper.version,
            helper.build
        );
    }
    let Connected { mut stream, welcome } = connected;
    if let Err(error) = step_down(&mut stream) {
        return Ok(Offered::Kept { connected: Connected { stream, welcome }, why: format!("{error:#}") });
    }
    drop(stream);
    connect_new(endpoint, token).map(Offered::Upgraded)
}

fn helper_build(helper: &Path) -> Result<Build> {
    let bytes = std::fs::read(helper).with_context(|| format!("reading {}", helper.display()))?;
    let mtime = std::fs::metadata(helper).and_then(|metadata| metadata.modified()).ok();
    let output = run(hide_window(&mut Command::new(helper)).arg("--version"))?;
    if !output.status.success() {
        bail!("{} --version failed", helper.display());
    }
    Ok(Build {
        version: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        build: build_id::of_binary(&bytes),
        mtime,
    })
}

fn core_build(welcome: &Welcome) -> Build {
    let exe = sys::image_of(welcome.pid);
    Build {
        version: welcome.core_version.clone(),
        build: welcome.core_build.clone(),
        mtime: exe.and_then(std::fs::metadata).and_then(|metadata| metadata.modified()).ok(),
    }
}

/// `<helper> check --holder-protocols LIST`; it logs a refusal to
/// `core.log` itself.
fn check(helper: &Path, protocols: &[u32]) -> Result<()> {
    let list = protocols.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let output = run(hide_window(&mut Command::new(helper)).args(["check", "--holder-protocols", &list]))?;
    if output.status.success() {
        return Ok(());
    }
    bail!("{}", String::from_utf8_lossy(&output.stderr).trim())
}

/// Runs a helper command with no input, for at most [`HELPER_LIMIT`].
fn run(command: &mut Command) -> Result<std::process::Output> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running the terminal server helper")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let pid = child.id();
    std::thread::spawn(move || tx.send(child.wait_with_output()).ok());
    match rx.recv_timeout(HELPER_LIMIT) {
        Ok(output) => Ok(output?),
        Err(_) => {
            sys::kill(pid);
            bail!("the terminal server helper didn't answer within {} s", HELPER_LIMIT.as_secs())
        }
    }
}

/// Sends `STEP_DOWN` and waits for `OK`, then for the core to close.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "railway R3 exception: frames other than this request's `Ok` or `Error` are skipped while the core closes"
)]
fn step_down(stream: &mut Stream) -> Result<()> {
    let req = 1;
    stream.set_read_timeout(Some(STEP_DOWN_LIMIT))?;
    ToCore::StepDown(Request { req }).encode()?.write_to(stream)?;
    loop {
        match Frame::read_from_or_eof(stream) {
            Ok(Some(frame)) => match ToClient::decode(&frame)? {
                ToClient::Ok(ok) if ok.req == req => break,
                ToClient::Error(error) if error.req == Some(req) => bail!("STEP_DOWN refused: {}", error.message),
                _ => {}
            },
            Ok(None) => return Ok(()),
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                bail!("no answer to STEP_DOWN within {} s", STEP_DOWN_LIMIT.as_secs())
            }
            Err(error) => return Err(error.into()),
        }
    }
    // The core closes its clients as it exits.
    let mut rest = Vec::new();
    io::Read::read_to_end(stream, &mut rest).ok();
    Ok(())
}

/// Starts the new core and connects to it. If it won't start, the holders
/// start one of their own build within a few seconds; connect to that.
fn connect_new(endpoint: &Endpoint, token: &str) -> Result<Connected> {
    let deadline = Instant::now() + RECONNECT_LIMIT;
    let mut started = false;
    loop {
        if let Some(stream) = try_connect(endpoint)? {
            match hello(stream, token) {
                Ok(connected) => return Ok(connected),
                Err(error) if Instant::now() >= deadline => return Err(error),
                // A core still exiting, or one starting.
                Err(error) => log::debug!("after the upgrade: {error:#}"),
            }
        } else if !started {
            started = true;
            if let Err(error) = start_core(endpoint) {
                log::warn!("the upgraded terminal server didn't start: {error:#}");
            }
            continue;
        }
        if Instant::now() >= deadline {
            bail!(
                "terminal server unreachable after an upgrade: no core within {} s; its log is {}",
                RECONNECT_LIMIT.as_secs(),
                endpoint.log().display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn build(version: &str, build: &str) -> Build {
        Build { version: version.into(), build: build.into(), mtime: None }
    }

    /// A newer helper whose check fails keeps the running core, and says
    /// which helper refused and why; the check runs only for a newer one.
    #[test]
    fn a_failed_check_keeps_the_running_core_with_its_reason() {
        let core = build("0.4.0", "aaa");
        let refused = || Err("holder protocol 1 is not supported".to_string());
        assert_eq!(
            plan(&Ok(build("0.5.0", "bbb")), &core, refused),
            Plan::Keep("the helper 0.5.0 (bbb) refused the running holders: holder protocol 1 is not supported".into())
        );
        assert_eq!(plan(&Ok(build("0.5.0", "bbb")), &core, || Ok(())), Plan::StepDown);

        let checked = Cell::new(false);
        let check = || {
            checked.set(true);
            Ok(())
        };
        assert_eq!(plan(&Ok(build("0.3.0", "bbb")), &core, check), Plan::Current);
        assert!(!checked.get(), "an older helper isn't checked");
        assert_eq!(
            plan(&Err("reading brindle-remote: not found".into()), &core, refused),
            Plan::Keep("reading the helper's build: reading brindle-remote: not found".into())
        );
    }
}
