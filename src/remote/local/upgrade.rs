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
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, bail};

use super::{Connected, Endpoint, Stream, connection::RECONNECT_LIMIT, hello, start_core, sys, try_connect};
use crate::remote::hide_window;
use crate::remote::protocol::build_id;
use crate::remote::protocol::client::{Request, ToClient, ToCore, Welcome};
use crate::remote::protocol::Frame;

/// How long a stepping-down core may take to checkpoint its sessions and
/// answer (it waits up to 10 s per holder).
const STEP_DOWN_LIMIT: Duration = Duration::from_secs(60);
/// How long `--version` and `check` may take.
const HELPER_LIMIT: Duration = Duration::from_secs(5);

/// A server binary's release, build id and file time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    pub build: String,
    pub mtime: Option<SystemTime>,
}

/// Whether `helper` should replace `core`.
pub fn is_newer(helper: &Build, core: &Build) -> bool {
    match (semver(&helper.version), semver(&core.version)) {
        (Some(offered), Some(running)) if offered != running => offered > running,
        (Some(_), Some(_)) => {
            helper.build != core.build
                && matches!((helper.mtime, core.mtime), (Some(offered), Some(running)) if offered > running)
        }
        _ => false,
    }
}

/// `MAJOR.MINOR.PATCH`, ignoring any pre-release or build suffix.
fn semver(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

/// Upgrades the core behind `connected` when the helper is newer, and
/// returns the connection to use: the new core's, or the running one's
/// when the helper isn't newer, fails its check, or the step-down is
/// refused.
pub fn offer(endpoint: &Endpoint, token: &str, connected: Connected) -> Result<Connected> {
    let helper = match helper_build(&endpoint.helper) {
        Ok(helper) => helper,
        Err(error) => {
            log::warn!("not offering an upgrade: {error:#}");
            return Ok(connected);
        }
    };
    let core = core_build(&connected.welcome);
    log::debug!("terminal server upgrade check: helper {helper:?}, running core {core:?}");
    if !is_newer(&helper, &core) {
        return Ok(connected);
    }
    log::info!(
        "upgrading the terminal server from {} ({}) to {} ({})",
        core.version,
        core.build,
        helper.version,
        helper.build
    );
    if let Err(error) = check(&endpoint.helper, &connected.welcome.holder_protocols) {
        log::warn!("keeping the running terminal server: {error:#}");
        return Ok(connected);
    }
    let Connected { mut stream, welcome } = connected;
    if let Err(error) = step_down(&mut stream) {
        log::warn!("keeping the running terminal server: {error:#}");
        return Ok(Connected { stream, welcome });
    }
    drop(stream);
    connect_new(endpoint, token)
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
    use super::*;

    fn build(version: &str, build: &str, secs: Option<u64>) -> Build {
        Build {
            version: version.into(),
            build: build.into(),
            mtime: secs.map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
        }
    }

    #[test]
    fn a_higher_version_is_newer_and_a_lower_one_older() {
        let core = build("0.3.0", "aaa", Some(100));
        assert!(is_newer(&build("0.4.0", "bbb", Some(50)), &core));
        assert!(is_newer(&build("0.3.1", "aaa", None), &core));
        assert!(is_newer(&build("1.0.0", "bbb", Some(100)), &build("0.10.9", "aaa", Some(100))));
        assert!(!is_newer(&build("0.2.9", "bbb", Some(200)), &core));
        assert!(!is_newer(&build("0.3.0", "bbb", Some(200)), &build("0.3.1", "aaa", Some(100))));
    }

    #[test]
    fn an_equal_version_needs_another_build_and_a_later_file() {
        let core = build("0.3.0", "aaa", Some(100));
        assert!(is_newer(&build("0.3.0", "bbb", Some(101)), &core));
        // The same build, however new its file.
        assert!(!is_newer(&build("0.3.0", "aaa", Some(500)), &core));
        // Another build, but an older file.
        assert!(!is_newer(&build("0.3.0", "bbb", Some(99)), &core));
        assert!(!is_newer(&build("0.3.0", "bbb", Some(100)), &core));
        // Either time unknown.
        assert!(!is_newer(&build("0.3.0", "bbb", None), &core));
        assert!(!is_newer(&build("0.3.0", "bbb", Some(101)), &build("0.3.0", "aaa", None)));
        // Pre-release suffixes compare by their numbers.
        assert!(is_newer(&build("0.3.0-rc.1", "bbb", Some(101)), &core));
    }

    #[test]
    fn unparsable_versions_are_never_newer() {
        let core = build("0.3.0", "aaa", Some(100));
        assert!(!is_newer(&build("dev", "bbb", Some(200)), &core));
        assert!(!is_newer(&build("0.4", "bbb", Some(200)), &core));
        assert!(!is_newer(&build("0.4.0", "bbb", Some(200)), &build("", "aaa", Some(1))));
        assert_eq!(semver(" 1.2.3\n"), Some((1, 2, 3)));
        assert_eq!(semver("1.2.3.4"), None);
    }
}
