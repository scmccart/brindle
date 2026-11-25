//! The fake ACP agent the agent cases play agents with,
//! `brindle-fake-acp-agent` (`remote-agent/src/fake_acp/`): where the
//! suite finds it and whether it is fresh enough to run.
//!
//! It is [`VARIABLE`] when that is set, else the one `cargo build` put
//! beside the suite's own build of Brindle (`CARGO_BIN_EXE_brindle`'s
//! directory), never beside `BRINDLE_E2E_BIN`: an archive doesn't carry it.
//! It has no version to match (it speaks ACP only), so a found binary is
//! stale only when it is older than one of its sources, since a test target
//! doesn't rebuild it. A runner checks it only when a selected case needs
//! it (`Case::needs_fake_agent` in the e2e runner).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::Result;
use crate::ensure;

/// The variable that names the fake agent to run.
pub const VARIABLE: &str = "BRINDLE_FAKE_ACP_AGENT";

/// Its sources, relative to the repository: a binary older than any file
/// under here is stale.
pub const SOURCES: &str = "remote-agent/src/fake_acp";

/// The binary's file name on this platform.
pub fn file_name() -> String {
    format!("brindle-fake-acp-agent{}", std::env::consts::EXE_SUFFIX)
}

/// The fake agent a run uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeAgent {
    pub path: PathBuf,
    /// [`VARIABLE`] named it.
    pub named: bool,
}

/// Where the fake agent is: `variable`'s value when it isn't empty (an
/// empty one counts as unset, as Brindle's variables do), else beside
/// `default_bin`, the suite's own build of Brindle.
pub fn locate(variable: Option<&OsStr>, default_bin: &Path) -> FakeAgent {
    match variable.filter(|value| !value.is_empty()) {
        Some(path) => FakeAgent { path: PathBuf::from(path), named: true },
        None => FakeAgent { path: default_bin.with_file_name(file_name()), named: false },
    }
}

/// [`locate`] with [`VARIABLE`] as this process has it.
pub fn find(default_bin: &Path) -> FakeAgent {
    locate(std::env::var_os(VARIABLE).as_deref(), default_bin)
}

/// The command that builds the fake agent for the suite's own build
/// `default_bin`: with `--release` when that is a release build.
pub fn build_command(default_bin: &Path) -> String {
    let release = default_bin.parent().and_then(Path::file_name).is_some_and(|name| name == "release");
    let profile = if release { " --release" } else { "" };
    format!("cargo build{profile} -p brindle-remote-agent --bin brindle-fake-acp-agent")
}

/// Whether `agent` can run, from when it was built (`None`: it is missing)
/// and the newest of its sources with when that was changed; `build` is
/// the command that builds it, for the message.
pub fn verdict(
    agent: &FakeAgent,
    built: Option<SystemTime>,
    newest_source: Option<(&Path, SystemTime)>,
    build: &str,
) -> Result {
    let Some(built) = built else {
        return Err(if agent.named {
            format!("{VARIABLE} names {}, which is missing", agent.path.display())
        } else {
            format!(
                "the fake ACP agent isn't built ({} is missing): run `{build}`, or set {VARIABLE}",
                agent.path.display()
            )
        });
    };
    match newest_source {
        Some((source, changed)) if changed > built => Err(format!(
            "{} is older than {}: run `{build}` (the test target doesn't rebuild it)",
            agent.path.display(),
            source.display()
        )),
        _ => Ok(()),
    }
}

/// The newest file under `dir`, with when it was changed.
fn newest_file(dir: &Path) -> Option<(PathBuf, SystemTime)> {
    super::server::files(dir)
        .into_iter()
        .filter_map(|file| {
            let changed = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
            Some((file, changed))
        })
        .max_by_key(|(_, changed)| *changed)
}

/// [`verdict`] for `agent` as it is on disk, against [`SOURCES`] under the
/// repository; `default_bin` is the suite's own build of Brindle.
pub fn check(agent: &FakeAgent, default_bin: &Path) -> Result {
    check_built_by(agent, &build_command(default_bin))
}

/// [`check`] with `build` as the command that builds `agent`.
pub fn check_built_by(agent: &FakeAgent, build: &str) -> Result {
    let built = std::fs::metadata(&agent.path).and_then(|m| m.modified()).ok();
    let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join(SOURCES);
    let newest = newest_file(&sources);
    verdict(agent, built, newest.as_ref().map(|(path, changed)| (path.as_path(), *changed)), build)
}

/// The static Linux build the remote agent case copies into its container,
/// relative to the repository: README.md's static helper build, which
/// builds every binary of `brindle-remote-agent`, puts it there.
pub const STATIC_LINUX: &str = "target/x86_64-unknown-linux-musl/release/brindle-fake-acp-agent";

/// What builds [`STATIC_LINUX`], for the messages.
pub const STATIC_LINUX_BUILD: &str =
    "cargo build --locked --release -p brindle-remote-agent --target x86_64-unknown-linux-musl (README.md's Docker command)";

/// The lookup's own checks (`--self-test`): found, missing, stale and the
/// variable.
pub fn self_test() -> Result {
    let debug_bin = Path::new("/repo/target/debug/brindle");
    let release_bin = Path::new("/repo/target/release/brindle");
    let name = file_name();

    // The variable wins; empty is unset; never beside another binary.
    let named = locate(Some(OsStr::new("/opt/fake")), debug_bin);
    ensure!(named == FakeAgent { path: PathBuf::from("/opt/fake"), named: true }, "the variable: {named:?}");
    let beside = locate(None, debug_bin);
    ensure!(
        beside == FakeAgent { path: PathBuf::from(format!("/repo/target/debug/{name}")), named: false },
        "beside the suite's build: {beside:?}"
    );
    ensure!(locate(Some(OsStr::new("")), debug_bin) == beside, "an empty variable is unset");

    // The build command follows the suite's build.
    let debug = build_command(debug_bin);
    ensure!(debug == "cargo build -p brindle-remote-agent --bin brindle-fake-acp-agent", "debug: {debug}");
    let release = build_command(release_bin);
    ensure!(
        release == "cargo build --release -p brindle-remote-agent --bin brindle-fake-acp-agent",
        "release: {release}"
    );

    let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let later = then + Duration::from_secs(60);
    let source = Path::new("/repo/remote-agent/src/fake_acp/script.rs");

    // Found: built after its newest source, or with no source to compare.
    ensure!(verdict(&beside, Some(later), Some((source, then)), &debug).is_ok(), "fresh refused");
    ensure!(verdict(&beside, Some(then), Some((source, then)), &debug).is_ok(), "built with its source refused");
    ensure!(verdict(&beside, Some(then), None, &debug).is_ok(), "no sources refused");

    // Missing: the message says how to build it, or what the variable named.
    let missing = verdict(&beside, None, Some((source, then)), &debug).unwrap_err();
    ensure!(missing.contains("isn't built") && missing.contains(&debug) && missing.contains(VARIABLE), "{missing}");
    let missing = verdict(&named, None, None, &debug).unwrap_err();
    ensure!(missing == format!("{VARIABLE} names /opt/fake, which is missing"), "{missing}");

    // Stale: older than a source, the variable's included.
    let stale = verdict(&beside, Some(then), Some((source, later)), &release).unwrap_err();
    ensure!(stale.contains("is older than") && stale.contains("script.rs") && stale.contains(&release), "{stale}");
    ensure!(verdict(&named, Some(then), Some((source, later)), &debug).is_err(), "a stale named agent accepted");

    // On disk: a file made now is newer than the sources, and a path that
    // isn't there is missing.
    let dir = std::env::temp_dir().join(format!("brindle-fake-agent-self-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let made = dir.join(&name);
    std::fs::write(&made, b"").map_err(|e| format!("writing {}: {e}", made.display()))?;
    let found = check(&FakeAgent { path: made, named: true }, debug_bin);
    let gone = check(&FakeAgent { path: dir.join("gone"), named: true }, debug_bin);
    std::fs::remove_dir_all(&dir).ok();
    ensure!(found.is_ok(), "a fresh file on disk: {found:?}");
    ensure!(gone.is_err(), "a missing file on disk accepted");
    Ok(())
}
