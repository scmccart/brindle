//! Helper deployment for remote endpoints, and the launch of their relays.
//!
//! **Deployment.** An endpoint keeps this build's Linux `brindle-remote`
//! cached by version and digest ([`Helper`]), so it is copied there once per
//! build:
//! - an SSH host in the user's `~/.cache/brindle/<version>-<digest>/`,
//!   uploaded with user-only permissions;
//! - a container in `/tmp/brindle-helper-<version>-<digest>/`, a directory
//!   of root's that every user of the container runs the helper from and
//!   none of them can write. Root makes it, takes the copied file over and
//!   removes other builds' directories. A directory that someone else made
//!   is refused and left alone. Nothing is removed when a connection ends.
//!
//! **Launch** ([`launch`]). A relay starts with one command, which is its
//! connection: a script that runs the cached helper's `relay`, or exits
//! [`NOT_CACHED`] when the endpoint has no helper it may run (in a
//! container: one that isn't root's own, or is a symbolic link). Only then
//! is the helper deployed, and the launch run once more. The relay's link
//! itself is `remote::local`'s.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

use super::{hide_window, wait_process};
use crate::config::RemoteTarget;

const REMOTE_DIR: &str = ".cache/brindle";

const STDERR_DIAGNOSTIC_LIMIT: usize = 64 * 1024;
/// How long to wait for a stderr pipe to close after its process has ended.
/// A backgrounded ssh mux master (ControlPersist) can inherit the pipe and
/// keep it open indefinitely.
const STDERR_DRAIN_WAIT: Duration = Duration::from_secs(1);

/// Drains `stderr` on a thread and delivers the bounded text through a
/// channel, so the caller can stop waiting if the pipe never closes.
fn spawn_stderr_drain(stderr: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    // If the thread can't start, `tx` is dropped with the closure and the
    // receiver simply reports no output.
    thread::Builder::new()
        .name("remote-stderr".into())
        .spawn(move || {
            tx.send(drain_stderr(stderr)).ok();
        })
        .ok();
    rx
}

/// Takes what the drain has collected, giving up (and leaving the thread to
/// finish on its own) if the pipe stays open.
fn await_stderr(rx: &mpsc::Receiver<String>) -> String {
    rx.recv_timeout(STDERR_DRAIN_WAIT).unwrap_or_default()
}

fn drain_stderr(mut stderr: impl Read) -> String {
    let mut retained = Vec::new();
    let mut buffer = [0u8; 4096];
    while let Ok(read) = stderr.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let keep = (STDERR_DIAGNOSTIC_LIMIT - retained.len()).min(read);
        retained.extend_from_slice(&buffer[..keep]);
    }
    String::from_utf8_lossy(&retained).into_owned()
}

fn agent_path() -> Result<PathBuf> {
    super::find_helper()?.context(
        "Linux remote helper not found beside Brindle; set BRINDLE_REMOTE_AGENT to a Linux brindle-remote binary",
    )
}

/// Where the Linux helper a relay's launch deploys is, asked at each launch
/// ([`launch`]), so a rebuilt or newly named helper is the next launch's.
pub(crate) type HelperLookup = Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>;

/// The helper `BRINDLE_REMOTE_AGENT` names, else the one beside Brindle.
pub(crate) fn default_helper() -> HelperLookup {
    Arc::new(agent_path)
}

/// What a relay's launch exits with when the endpoint has no helper of this
/// build that it may run (design decision 8). A helper never exits with it,
/// so before the relay's frame marker it always means: deploy, then launch
/// again.
pub const NOT_CACHED: i32 = 97;
/// What a container's directory step exits with when the cache directory is
/// taken: something is there that isn't a directory of root's.
const TAKEN: i32 = 98;
/// How long each deployment command may take.
const DEPLOY_LIMIT: Duration = Duration::from_secs(30);

/// This build's Linux helper: the file on this machine, and the version and
/// digest that endpoints cache it by.
struct Helper {
    local: PathBuf,
    version: &'static str,
    digest: String,
}

impl Helper {
    /// The helper at `local`, hashed as it is now: a rebuilt helper is
    /// another build.
    fn of(local: PathBuf) -> Result<Self> {
        let digest = file_sha256(&local)?;
        Ok(Self { local, version: env!("CARGO_PKG_VERSION"), digest })
    }

    /// Where an SSH host keeps it, with `$HOME` left for the host's shell.
    fn ssh_path(&self) -> String {
        format!("$HOME/{REMOTE_DIR}/{}-{}/brindle-remote", self.version, self.digest)
    }

    /// The directory a container keeps it in, for all of its users.
    fn container_dir(&self) -> String {
        format!("/tmp/brindle-helper-{}-{}", self.version, self.digest)
    }
}

/// A relay's launch ended, or was ended, before the relay's frame marker.
#[derive(Debug)]
pub struct Unstarted {
    /// The process's exit code; `None` when it was killed or didn't exit in
    /// time. A launch script's [`NOT_CACHED`] is one.
    pub code: Option<i32>,
    /// What the process wrote to stderr.
    pub said: String,
    /// The whole failure, as a tab shows it.
    pub message: String,
}

impl std::fmt::Display for Unstarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Unstarted {}

/// Runs the deployment's commands: for real ([`Commands`]), or as a test
/// scripts them.
trait Run {
    /// Runs `command` to its end, with the file at `input` as its stdin when
    /// one is given. `what` names the command in errors.
    fn run(&self, command: Command, input: Option<&Path>, what: &str) -> Result<Ran>;
}

/// How a deployment command ended.
#[derive(Debug)]
struct Ran {
    /// Its exit code; `None` when it was killed.
    code: Option<i32>,
    stderr: String,
}

/// Runs each command for at most [`DEPLOY_LIMIT`], ending it early once
/// `cancel` is set.
struct Commands<'a> {
    cancel: &'a AtomicBool,
}

impl Run for Commands<'_> {
    fn run(&self, mut command: Command, input: Option<&Path>, what: &str) -> Result<Ran> {
        let input = match input {
            Some(path) => Some(
                File::open(path)
                    .with_context(|| format!("opening remote helper {}", path.display()))?,
            ),
            None => None,
        };
        command
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("starting {what}"))?;
        let stderr = child.stderr.take().map(spawn_stderr_drain);
        let sending = match (input, child.stdin.take()) {
            (Some(mut file), Some(mut stdin)) => {
                Some(thread::spawn(move || std::io::copy(&mut file, &mut stdin)))
            }
            _ => None,
        };
        let status =
            wait_process(child, self.cancel, DEPLOY_LIMIT).with_context(|| what.to_string())?;
        let sent = match sending {
            Some(sending) => sending
                .join()
                .map_err(|_| anyhow::anyhow!("{what}: the thread sending its input panicked"))?,
            None => Ok(0),
        };
        let stderr = stderr.map(|rx| await_stderr(&rx)).unwrap_or_default();
        // A command that failed says why itself. Under one that succeeded, a
        // pipe that broke is the failure.
        if status.success()
            && let Err(error) = sent
        {
            bail!("{what}: sending its input failed: {error}");
        }
        Ok(Ran { code: status.code(), stderr })
    }
}

fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    hide_window(&mut command);
    command
}

/// Splits what a failed command wrote to stderr into the line that names its
/// cause and the other lines (in their order, joined by newlines, possibly
/// empty). The cause is the last non-empty line: docker and podman print
/// just one, ssh prints its warnings before its error, and a login script
/// that printed before a failing `exec` ends with the `exec`'s error. `None`
/// when nothing was written.
pub(crate) fn split_cause(stderr: &str) -> Option<(&str, String)> {
    let lines: Vec<&str> = stderr.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    let (cause, rest) = lines.split_last()?;
    Some((cause.trim(), rest.join("\n")))
}

/// "what failed: <cause>", with the command's other stderr lines below it
/// when it printed more than one: the first line, which is all a palette row
/// shows, names the cause.
fn bootstrap_failure(what: &str, stderr: &str) -> anyhow::Error {
    match split_cause(stderr) {
        None => anyhow::anyhow!("{what}"),
        Some((cause, rest)) if rest.is_empty() => anyhow::anyhow!("{what}: {cause}"),
        Some((cause, rest)) => anyhow::anyhow!("{what}: {cause}\n{rest}"),
    }
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening remote helper {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What an SSH host's `sh` runs for a relay: the cached helper, when it is
/// there and says it is this version, else exit [`NOT_CACHED`]. A helper
/// that is there but says something else (another build, or an exec error
/// for another architecture) has what it said written to stderr, which the
/// error after a deployment shows. It isn't a login shell, whose startup
/// files could print into the framed stream. The path and the answer are
/// positional parameters, not variables: a variable that the user's
/// environment already had would be changed for the relay, and the relay's
/// environment is its sessions'.
fn ssh_launch_script(helper: &Helper) -> String {
    format!(
        "set -- \"{path}\"; if test -x \"$1\"; then set -- \"$1\" \"$(\"$1\" --version 2>&1)\"; \
         if test \"$2\" = {version}; then exec \"$1\" relay; fi; \
         echo \"$1 --version printed: $2\" >&2; fi; exit {NOT_CACHED}",
        path = helper.ssh_path(),
        version = helper.version,
    )
}

/// What an SSH host's `sh` runs to take the helper from its stdin: into a
/// user-only directory and through a temporary name, so the cached path only
/// ever holds a whole file.
fn ssh_upload_script(helper: &Helper) -> String {
    let remote = helper.ssh_path();
    format!(
        "umask 077; mkdir -p \"$HOME/{REMOTE_DIR}/{version}-{digest}\" && tmp=\"{remote}.tmp.$$\" \
         && cat > \"$tmp\" && chmod 700 \"$tmp\" && mv \"$tmp\" \"{remote}\"",
        version = helper.version,
        digest = helper.digest,
    )
}

/// What a container's `sh` runs for a relay, as the profile's user, with the
/// cache directory as `$1`: the helper in it, when the directory and the
/// file are root's own and neither is a symbolic link, else exit
/// [`NOT_CACHED`]. Only root can put anything in a directory of root's, or
/// (`/tmp` being sticky) put another in its place, so what passed the check
/// is what runs. Like the SSH launch it sets no variable.
const CONTAINER_LAUNCH: &str = concat!(
    r#"test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0"#,
    r#" && test -f "$1/brindle-remote" && test ! -L "$1/brindle-remote" && test -x "$1/brindle-remote""#,
    r#" && test "$(stat -c %u "$1/brindle-remote")" = 0"#,
    r#" && exec "$1/brindle-remote" relay; exit 97"#,
);

/// Root's first deployment step in a container, with the cache directory as
/// `$1`: make it (mode 755), or keep one that is already a directory of
/// root's and not a symbolic link, with its mode put right. Anything else
/// there is someone else's: exit [`TAKEN`], touching nothing.
const CONTAINER_DIRECTORY: &str = concat!(
    r#"mkdir -m 755 "$1" 2>/dev/null && exit 0; "#,
    r#"if test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0; then chmod 755 "$1"; exit; fi; "#,
    r#"if test -e "$1" || test -L "$1"; then exit 98; fi; "#,
    // Nothing is there: again, for mkdir's own message.
    r#"mkdir -m 755 "$1""#,
);

/// Root's last deployment step, with the cache directory as `$1` and the
/// copied file's name in it as `$2`: take the file over (`docker cp` can
/// keep the host file's owner), check that it is a regular file of root's,
/// and rename it into place. So the helper's path only ever holds a whole,
/// root-owned file, and a helper that is running is never written to. Then
/// remove the other builds' directories beside it, but only those that
/// plainly are directories of root's.
const CONTAINER_INSTALL: &str = concat!(
    r#"if test -f "$1/$2" && test ! -L "$1/$2" && chown 0:0 "$1/$2" && chmod 755 "$1/$2""#,
    r#" && test "$(stat -c %u "$1/$2")" = 0 && mv -f "$1/$2" "$1/brindle-remote"; "#,
    r#"then :; else rm -f "$1/$2"; exit 1; fi; "#,
    r#"for d in "${1%/*}"/brindle-helper-*; do "#,
    r#"test "$d" != "$1" && test -d "$d" && test ! -L "$d" && test "$(stat -c %u "$d")" = 0 && rm -rf "$d"; "#,
    r#"done; exit 0"#,
);

/// The command whose process is `target`'s relay, when the endpoint has this
/// build's helper; it exits [`NOT_CACHED`] when it hasn't. It is the warm
/// path's only connection: an SSH host checks the helper it has, and a
/// container (as the profile's user) that the helper is root's.
fn launch_command(target: &RemoteTarget, helper: &Helper) -> Result<Command> {
    match target {
        RemoteTarget::Ssh { .. } => {
            super::remote_command(target, "sh", &["-c".into(), ssh_launch_script(helper)])
        }
        RemoteTarget::Container { .. } => super::remote_command(
            target,
            "sh",
            &[
                "-c".into(),
                CONTAINER_LAUNCH.into(),
                "sh".into(),
                helper.container_dir(),
            ],
        ),
    }
}

/// Starts the relay at `target`, where `start` runs a launch command up to
/// the relay's frame marker (`local::relay::start`). The warm path is that
/// one command. When it exits [`NOT_CACHED`], the helper `helper` finds now
/// is deployed and the launch run once more. Any other failure is the
/// endpoint's or the transport's own (ssh's 255 among them), which deploying
/// wouldn't mend.
pub(crate) fn launch<L>(
    target: &RemoteTarget,
    helper: &HelperLookup,
    start: impl Fn(Command) -> std::result::Result<L, Unstarted>,
) -> Result<L> {
    let helper = Helper::of(helper()?)?;
    // Nothing cancels a relay's launch: its commands end by their limits.
    let never = AtomicBool::new(false);
    launch_with(&Commands { cancel: &never }, target, &helper, start)
}

fn launch_with<L>(
    run: &impl Run,
    target: &RemoteTarget,
    helper: &Helper,
    start: impl Fn(Command) -> std::result::Result<L, Unstarted>,
) -> Result<L> {
    let first = match start(launch_command(target, helper)?) {
        Ok(link) => return Ok(link),
        Err(unstarted) => unstarted,
    };
    if first.code != Some(NOT_CACHED) {
        return Err(unlaunched(target, first));
    }
    match target {
        RemoteTarget::Ssh { .. } => upload_ssh(run, target, helper)?,
        RemoteTarget::Container {
            engine, container, ..
        } => deploy_container(run, engine, container, helper)?,
    }
    start(launch_command(target, helper)?).map_err(|again| match again.code {
        Some(NOT_CACHED) => undeployed(target, helper, &again),
        _ => unlaunched(target, again),
    })
}

/// A launch that failed for another reason than a missing helper.
fn unlaunched(target: &RemoteTarget, unstarted: Unstarted) -> anyhow::Error {
    match (target, unstarted.code) {
        // ssh itself failed (auth, host key, network), not the host's shell.
        (RemoteTarget::Ssh { .. }, Some(255)) => {
            bootstrap_failure("SSH connection failed", &unstarted.said)
        }
        _ => unstarted.into(),
    }
}

/// The endpoint still has no helper it may run, right after getting one.
fn undeployed(target: &RemoteTarget, helper: &Helper, again: &Unstarted) -> anyhow::Error {
    let what = match target {
        RemoteTarget::Ssh { .. } => format!(
            "the helper {} was uploaded, but on the host it doesn't print version {}: \
             replace it with this Brindle's Linux helper, built for the host's architecture",
            helper.local.display(),
            helper.version
        ),
        RemoteTarget::Container { .. } => format!(
            "the helper was copied into the container, but the profile's user there doesn't \
             find {} and the helper in it to be root's own",
            helper.container_dir()
        ),
    };
    bootstrap_failure(&what, &again.said)
}

/// Uploads the helper to an SSH host's cache.
fn upload_ssh(run: &impl Run, target: &RemoteTarget, helper: &Helper) -> Result<()> {
    let command = super::remote_command(target, "sh", &["-c".into(), ssh_upload_script(helper)])?;
    let uploaded = run.run(command, Some(&helper.local), "SSH helper upload")?;
    if uploaded.code != Some(0) {
        return Err(bootstrap_failure("SSH helper upload failed", &uploaded.stderr));
    }
    Ok(())
}

/// `script` for root's `sh` in the container, with `args` as `$1` onwards.
fn container_root(engine: &str, container: &str, script: &str, args: &[&str]) -> Command {
    let mut command = hidden_command(engine);
    command
        .args(["exec", "--user", "0", container, "sh", "-c", script, "sh"])
        .args(args);
    command
}

/// A name that no other deployment gives its copy of the helper.
fn temporary_name() -> Result<String> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).context("naming the container helper's copy")?;
    Ok(format!("brindle-remote.tmp-{}", hex(&random)))
}

/// Puts the helper in a container's cache, as root: the directory
/// ([`CONTAINER_DIRECTORY`]), the copy under a name of its own, then the
/// take-over, the rename into place and the removal of other builds
/// ([`CONTAINER_INSTALL`]). A directory that is taken is an error naming it,
/// and nothing more runs.
fn deploy_container(run: &impl Run, engine: &str, container: &str, helper: &Helper) -> Result<()> {
    let directory = helper.container_dir();
    let made = run.run(
        container_root(engine, container, CONTAINER_DIRECTORY, &[&directory]),
        None,
        "creating the container's helper directory",
    )?;
    match made.code {
        Some(0) => {}
        Some(TAKEN) => bail!(
            "{directory} exists in the container but isn't a directory owned by root, \
             so Brindle runs nothing from it and leaves it as it is; remove it there to connect"
        ),
        _ => return Err(bootstrap_failure("container helper directory failed", &made.stderr)),
    }
    let temporary = temporary_name()?;
    let mut copy = hidden_command(engine);
    copy.arg("cp")
        .arg(&helper.local)
        .arg(format!("{container}:{directory}/{temporary}"));
    let copied = run.run(copy, None, "copying container helper")?;
    if copied.code != Some(0) {
        return Err(bootstrap_failure("container helper upload failed", &copied.stderr));
    }
    let installed = run.run(
        container_root(engine, container, CONTAINER_INSTALL, &[&directory, &temporary]),
        None,
        "verifying container helper",
    )?;
    if installed.code != Some(0) {
        return Err(bootstrap_failure(
            "container helper is not a root-owned regular file",
            &installed.stderr,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    // ---- the relay's launch and the deployment behind it ----

    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// The cache directory of [`helper`] in a container.
    const CACHE: &str = "/tmp/brindle-helper-1.2.3-abc123";
    /// The SSH launch script for [`helper`].
    const SSH_LAUNCH: &str = r#"set -- "$HOME/.cache/brindle/1.2.3-abc123/brindle-remote"; if test -x "$1"; then set -- "$1" "$("$1" --version 2>&1)"; if test "$2" = 1.2.3; then exec "$1" relay; fi; echo "$1 --version printed: $2" >&2; fi; exit 97"#;
    /// The SSH upload script for [`helper`].
    const SSH_UPLOAD: &str = r#"umask 077; mkdir -p "$HOME/.cache/brindle/1.2.3-abc123" && tmp="$HOME/.cache/brindle/1.2.3-abc123/brindle-remote.tmp.$$" && cat > "$tmp" && chmod 700 "$tmp" && mv "$tmp" "$HOME/.cache/brindle/1.2.3-abc123/brindle-remote""#;

    fn helper() -> Helper {
        Helper {
            local: PathBuf::from("helpers/brindle-remote-linux"),
            version: "1.2.3",
            digest: "abc123".into(),
        }
    }

    fn ssh_host() -> RemoteTarget {
        RemoteTarget::Ssh {
            host: "A8".into(),
            user: Some("dev".into()),
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        }
    }

    fn container_of(engine: &str, user: Option<&str>) -> RemoteTarget {
        RemoteTarget::Container {
            engine: engine.into(),
            container: "devbox".into(),
            user: user.map(Into::into),
            connect_at_launch: false,
        }
    }

    /// A command's program and arguments.
    fn line(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    /// `ssh` running `script` by the host's `sh`, for [`ssh_host`].
    fn ssh_line(script: &str) -> Vec<String> {
        let mut line = strings(&[
            "ssh",
            "-T",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=3",
            "--",
            "dev@A8",
        ]);
        line.push(format!("sh -c '{script}'"));
        line
    }

    #[test]
    fn the_ssh_launch_runs_the_cached_relay_or_exits_97() {
        let command = launch_command(&ssh_host(), &helper()).unwrap();
        // One connection, and not a login shell.
        assert_eq!(line(&command), ssh_line(SSH_LAUNCH));
        assert_eq!(ssh_launch_script(&helper()), SSH_LAUNCH);
        assert!(SSH_LAUNCH.ends_with(&format!("; exit {NOT_CACHED}")));
        assert_eq!(ssh_upload_script(&helper()), SSH_UPLOAD);
    }

    #[test]
    fn a_container_launch_checks_the_cache_as_the_profiles_user() {
        let with_user = launch_command(&container_of("docker", Some("1000")), &helper()).unwrap();
        assert_eq!(
            line(&with_user),
            ["docker", "exec", "-i", "--user", "1000", "devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", CACHE]
        );
        let without = launch_command(&container_of("podman", None), &helper()).unwrap();
        assert_eq!(
            line(&without),
            ["podman", "exec", "-i", "devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", CACHE]
        );
        assert!(launch_command(&container_of("lxc", None), &helper()).is_err());
        assert_eq!(
            CONTAINER_LAUNCH,
            r#"test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0 && test -f "$1/brindle-remote" && test ! -L "$1/brindle-remote" && test -x "$1/brindle-remote" && test "$(stat -c %u "$1/brindle-remote")" = 0 && exec "$1/brindle-remote" relay; exit 97"#
        );
        assert!(CONTAINER_LAUNCH.ends_with(&format!("; exit {NOT_CACHED}")));
    }

    #[test]
    fn roots_container_scripts_are_pinned() {
        assert_eq!(
            CONTAINER_DIRECTORY,
            r#"mkdir -m 755 "$1" 2>/dev/null && exit 0; if test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0; then chmod 755 "$1"; exit; fi; if test -e "$1" || test -L "$1"; then exit 98; fi; mkdir -m 755 "$1""#
        );
        assert!(CONTAINER_DIRECTORY.contains(&format!("then exit {TAKEN}; fi")));
        assert_eq!(
            CONTAINER_INSTALL,
            r#"if test -f "$1/$2" && test ! -L "$1/$2" && chown 0:0 "$1/$2" && chmod 755 "$1/$2" && test "$(stat -c %u "$1/$2")" = 0 && mv -f "$1/$2" "$1/brindle-remote"; then :; else rm -f "$1/$2"; exit 1; fi; for d in "${1%/*}"/brindle-helper-*; do test "$d" != "$1" && test -d "$d" && test ! -L "$d" && test "$(stat -c %u "$d")" = 0 && rm -rf "$d"; done; exit 0"#
        );
    }

    /// An endpoint that runs nothing: it logs each launch and each
    /// deployment command, and ends each as the test scripted.
    #[derive(Default)]
    struct Scripted {
        log: RefCell<Vec<Step>>,
        launches: RefCell<VecDeque<std::result::Result<(), Unstarted>>>,
        runs: RefCell<VecDeque<(i32, &'static str)>>,
    }

    #[derive(Debug, PartialEq)]
    enum Step {
        Launch(Vec<String>),
        /// A deployment command and the file that is its stdin.
        Run(Vec<String>, Option<PathBuf>),
    }

    impl Scripted {
        /// How each launch ends, and each deployment command's exit code
        /// and stderr.
        fn new(
            launches: impl IntoIterator<Item = std::result::Result<(), Unstarted>>,
            runs: impl IntoIterator<Item = (i32, &'static str)>,
        ) -> Self {
            Self {
                log: RefCell::default(),
                launches: RefCell::new(launches.into_iter().collect()),
                runs: RefCell::new(runs.into_iter().collect()),
            }
        }

        fn launch(&self, target: &RemoteTarget) -> Result<()> {
            launch_with(self, target, &helper(), |command| {
                self.log.borrow_mut().push(Step::Launch(line(&command)));
                self.launches.borrow_mut().pop_front().expect("a launch the test didn't script")
            })
        }

        fn log(&self) -> Vec<Step> {
            self.log.take()
        }
    }

    impl Run for Scripted {
        fn run(&self, command: Command, input: Option<&Path>, _what: &str) -> Result<Ran> {
            self.log
                .borrow_mut()
                .push(Step::Run(line(&command), input.map(Path::to_path_buf)));
            let (code, stderr) = self.runs.borrow_mut().pop_front().expect("a command the test didn't script");
            Ok(Ran { code: Some(code), stderr: stderr.into() })
        }
    }

    /// A launch that exited with `code` before the relay's frame marker.
    fn exited(code: i32, said: &str) -> Unstarted {
        Unstarted {
            code: Some(code),
            said: said.into(),
            message: format!("the relay didn't start (exited with {code})\n{said}"),
        }
    }

    fn container_launch(user: Option<&str>) -> Step {
        let mut line = strings(&["docker", "exec", "-i"]);
        if let Some(user) = user {
            line.extend(strings(&["--user", user]));
        }
        line.extend(strings(&["devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", CACHE]));
        Step::Launch(line)
    }

    fn root_step(script: &str, args: &[&str]) -> Vec<String> {
        let mut line = strings(&["docker", "exec", "--user", "0", "devbox", "sh", "-c", script, "sh"]);
        line.extend(strings(args));
        line
    }

    #[test]
    fn a_cached_helper_takes_one_command() {
        let host = Scripted::new([Ok(())], []);
        host.launch(&ssh_host()).unwrap();
        assert_eq!(host.log(), [Step::Launch(ssh_line(SSH_LAUNCH))]);

        let devbox = Scripted::new([Ok(())], []);
        devbox.launch(&container_of("docker", Some("dev"))).unwrap();
        assert_eq!(devbox.log(), [container_launch(Some("dev"))]);
    }

    #[test]
    fn a_host_without_the_helper_gets_it_uploaded_and_is_launched_again() {
        let host = Scripted::new([Err(exited(NOT_CACHED, "")), Ok(())], [(0, "")]);
        host.launch(&ssh_host()).unwrap();
        assert_eq!(
            host.log(),
            [
                Step::Launch(ssh_line(SSH_LAUNCH)),
                Step::Run(ssh_line(SSH_UPLOAD), Some(helper().local)),
                Step::Launch(ssh_line(SSH_LAUNCH)),
            ]
        );
    }

    #[test]
    fn a_container_without_the_helper_gets_it_from_root_and_is_launched_again() {
        let devbox = Scripted::new([Err(exited(NOT_CACHED, "")), Ok(())], [(0, ""), (0, ""), (0, "")]);
        devbox.launch(&container_of("docker", Some("dev"))).unwrap();
        let log = devbox.log();
        assert_eq!(log.len(), 5, "{log:?}");
        // The launch runs as the profile's user, the deployment as root.
        assert_eq!(log[0], container_launch(Some("dev")));
        assert_eq!(log[1], Step::Run(root_step(CONTAINER_DIRECTORY, &[CACHE]), None));
        let Step::Run(copy, None) = &log[2] else { panic!("{:?}", log[2]) };
        assert_eq!(copy[..3], ["docker", "cp", "helpers/brindle-remote-linux"]);
        // The copy has a name of its own, and is renamed into place.
        let temporary = copy[3]
            .strip_prefix(&format!("devbox:{CACHE}/"))
            .unwrap_or_else(|| panic!("{copy:?}"));
        let random = temporary.strip_prefix("brindle-remote.tmp-").unwrap_or_else(|| panic!("{temporary}"));
        assert!(random.len() == 16 && random.bytes().all(|b| b.is_ascii_hexdigit()), "{temporary}");
        assert_eq!(copy.len(), 4);
        assert_eq!(log[3], Step::Run(root_step(CONTAINER_INSTALL, &[CACHE, temporary]), None));
        assert_eq!(log[4], container_launch(Some("dev")));
        assert_ne!(temporary_name().unwrap(), temporary);
    }

    #[test]
    fn a_taken_container_directory_is_refused_and_nothing_more_runs() {
        let devbox = Scripted::new([Err(exited(NOT_CACHED, ""))], [(TAKEN, "")]);
        let error = devbox.launch(&container_of("docker", None)).unwrap_err().to_string();
        assert!(error.starts_with(&format!("{CACHE} exists in the container but isn't a directory owned by root")), "{error}");
        // No copy into it, nothing removed, and no second launch.
        assert_eq!(
            devbox.log(),
            [container_launch(None), Step::Run(root_step(CONTAINER_DIRECTORY, &[CACHE]), None)]
        );
    }

    #[test]
    fn an_ssh_failure_is_a_connection_error_and_not_a_missing_helper() {
        let said = "ssh: connect to host a8 port 22: Connection refused";
        let host = Scripted::new([Err(exited(255, said))], []);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        assert_eq!(error, format!("SSH connection failed: {said}"));
        assert_eq!(host.log(), [Step::Launch(ssh_line(SSH_LAUNCH))]);

        // Any other exit is the launch's own failure, shown as it said it.
        let said = "Error response from daemon: No such container: devbox";
        let devbox = Scripted::new([Err(exited(1, said))], []);
        let error = devbox.launch(&container_of("docker", None)).unwrap_err();
        assert_eq!(error.downcast_ref::<Unstarted>().and_then(|unstarted| unstarted.code), Some(1));
        assert!(error.to_string().ends_with(said), "{error}");
        assert_eq!(devbox.log(), [container_launch(None)]);
        // A container's 255 is its own, too.
        let devbox = Scripted::new([Err(exited(255, "boom"))], []);
        let error = devbox.launch(&container_of("docker", None)).unwrap_err();
        assert!(error.downcast_ref::<Unstarted>().is_some(), "{error}");
    }

    /// The relay's launch hashes the helper its lookup names, so an endpoint
    /// caches that file, and the lookup is asked at the launch.
    #[test]
    fn a_relay_launch_deploys_the_helper_its_lookup_names() {
        let dir = std::env::temp_dir().join(format!("brindle-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("brindle-remote");
        std::fs::write(&file, "a helper of known bytes\n").unwrap();
        let named = file.clone();
        let lookup: HelperLookup = Arc::new(move || Ok(named.clone()));
        let launched = RefCell::new(Vec::new());
        let error = launch(&container_of("docker", None), &lookup, |command| {
            launched.borrow_mut().push(line(&command));
            Err::<(), _>(exited(1, "no such container"))
        })
        .unwrap_err();
        std::fs::remove_dir_all(&dir).ok();
        assert!(error.downcast_ref::<Unstarted>().is_some(), "{error}");
        let digest = "0f39ba63db5203814a6b8f623897a84840ada2bbdbab67462d4c9685cfc0913b";
        let cache = format!("/tmp/brindle-helper-{}-{digest}", env!("CARGO_PKG_VERSION"));
        assert_eq!(
            launched.take(),
            [strings(&["docker", "exec", "-i", "devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", &cache])]
        );
    }

    /// A failure's first line names its cause, which is the last line of
    /// what the command wrote; the other lines follow it.
    #[test]
    fn the_cause_is_the_last_line_of_stderr() {
        // docker's single line.
        let docker = "Error response from daemon: container abc is not running";
        assert_eq!(split_cause(docker), Some((docker, String::new())));
        // ssh's warnings come before its error.
        let ssh = "Warning: Permanently added '127.0.0.1' (ED25519) to the list of known hosts.\r\n\
                   ssh: connect to host 127.0.0.1 port 2222: Connection refused\r\n";
        let (cause, rest) = split_cause(ssh).unwrap();
        assert_eq!(cause, "ssh: connect to host 127.0.0.1 port 2222: Connection refused");
        assert_eq!(rest, "Warning: Permanently added '127.0.0.1' (ED25519) to the list of known hosts.");
        // Blank lines and indentation don't count.
        assert_eq!(split_cause("noise\n\n  boom  \n \n"), Some(("boom", "noise".to_string())));
        // Nothing written: no cause.
        assert_eq!(split_cause(""), None);
        assert_eq!(split_cause(" \n\r\n"), None);

        assert_eq!(bootstrap_failure("what failed", "").to_string(), "what failed");
        assert_eq!(bootstrap_failure("what failed", docker).to_string(), format!("what failed: {docker}"));
        assert_eq!(
            bootstrap_failure("what failed", "one\ntwo\nthree").to_string(),
            "what failed: three\none\ntwo"
        );
    }

    #[test]
    fn an_ssh_failure_names_its_error_before_ssh_warnings() {
        let said = "Warning: Permanently added 'a8' (ED25519) to the list of known hosts.\n\
                    kex_exchange_identification: read: Connection reset by peer";
        let host = Scripted::new([Err(exited(255, said))], []);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        let (first, others) = error.split_once('\n').expect("the other lines follow");
        assert_eq!(first, "SSH connection failed: kex_exchange_identification: read: Connection reset by peer");
        assert_eq!(others, "Warning: Permanently added 'a8' (ED25519) to the list of known hosts.");

        // Nothing on stderr: today's text.
        let host = Scripted::new([Err(exited(255, ""))], []);
        assert_eq!(host.launch(&ssh_host()).unwrap_err().to_string(), "SSH connection failed");
    }

    #[test]
    fn a_helper_that_still_is_not_there_after_deploying_is_an_error() {
        let said = "/home/dev/.cache/brindle/1.2.3-abc123/brindle-remote --version printed: 1.2.2";
        let host = Scripted::new([Err(exited(NOT_CACHED, "")), Err(exited(NOT_CACHED, said))], [(0, "")]);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        // It names the local file that was uploaded, and what the host's copy said.
        let local = PathBuf::from("helpers/brindle-remote-linux");
        assert!(
            error.starts_with(&format!(
                "the helper {} was uploaded, but on the host it doesn't print version 1.2.3",
                local.display()
            )),
            "{error}"
        );
        assert!(error.ends_with(said), "{error}");
        // Deployed once, launched twice.
        assert_eq!(host.log().len(), 3);

        let devbox = Scripted::new([Err(exited(NOT_CACHED, "")), Err(exited(NOT_CACHED, ""))], [(0, ""), (0, ""), (0, "")]);
        let error = devbox.launch(&container_of("docker", Some("dev"))).unwrap_err().to_string();
        assert!(error.starts_with("the helper was copied into the container"), "{error}");
        assert!(error.contains(CACHE), "{error}");
        assert_eq!(devbox.log().len(), 5);
    }

    #[test]
    fn a_failed_deployment_is_not_launched_again() {
        let host = Scripted::new([Err(exited(NOT_CACHED, ""))], [(1, "cat: write error: No space left on device\n")]);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        assert_eq!(error, "SSH helper upload failed: cat: write error: No space left on device");
        assert_eq!(host.log().len(), 2);

        // Each of root's steps in a container.
        let steps: [(&[(i32, &str)], &str); 3] = [
            (&[(1, "mkdir: cannot create directory: Read-only file system")], "container helper directory failed: mkdir"),
            (&[(0, ""), (1, "no such container")], "container helper upload failed: no such container"),
            (&[(0, ""), (0, ""), (1, "")], "container helper is not a root-owned regular file"),
        ];
        for (runs, expected) in steps {
            let devbox = Scripted::new([Err(exited(NOT_CACHED, ""))], runs.iter().copied());
            let error = devbox.launch(&container_of("docker", None)).unwrap_err().to_string();
            assert!(error.starts_with(expected), "{error}");
            assert_eq!(devbox.log().len(), 1 + runs.len());
        }
    }

    /// A command that copies its stdin to the file `to`, writes `said` to
    /// stderr and exits with `code`.
    fn copying(to: &Path, said: &str, code: i32) -> Command {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;

            let mut command = hidden_command("cmd");
            command.args(["/d", "/c"]).raw_arg(format!(
                "findstr \"^\" > \"{}\" & echo {said} 1>&2 & exit {code}",
                to.display()
            ));
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command
                .args(["-c", "cat > \"$1\"; echo \"$2\" >&2; exit \"$3\"", "sh"])
                .arg(to)
                .args([said, &code.to_string()]);
            command
        }
    }

    /// A command that waits a minute.
    fn waiting() -> Command {
        if cfg!(windows) {
            let mut command = hidden_command("powershell");
            command.args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep 60"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        }
    }

    #[test]
    fn a_deployment_command_gets_its_input_and_says_how_it_ended() {
        let dir = std::env::temp_dir().join(format!("brindle-deploy-run-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let (input, copy) = (dir.join("helper"), dir.join("copy"));
        std::fs::write(&input, "the helper\n").unwrap();
        let never = AtomicBool::new(false);
        let commands = Commands { cancel: &never };

        // Its input is the file, and its exit code and stderr come back.
        let ran = commands.run(copying(&copy, "all of it", 0), Some(&input), "copying").unwrap();
        assert_eq!((ran.code, ran.stderr.trim()), (Some(0), "all of it"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap().trim(), "the helper");
        let ran = commands.run(copying(&copy, "no space left", 3), Some(&input), "copying").unwrap();
        assert_eq!((ran.code, ran.stderr.trim()), (Some(3), "no space left"));
        // Without one, it reads nothing.
        let ran = commands.run(copying(&copy, "nothing", 0), None, "copying").unwrap();
        assert_eq!((ran.code, ran.stderr.trim()), (Some(0), "nothing"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "");

        // What can't run at all is an error naming it.
        let error = commands.run(Command::new("brindle-no-such-program"), None, "probing").unwrap_err();
        assert!(format!("{error:#}").starts_with("starting probing"), "{error:#}");
        let error = commands
            .run(copying(&copy, "unread", 0), Some(&dir.join("missing")), "copying")
            .unwrap_err();
        assert!(format!("{error:#}").starts_with("opening remote helper"), "{error:#}");

        // A cancelled command is ended at once, and doesn't count as done.
        let cancelled = AtomicBool::new(true);
        let started = Instant::now();
        let ran = Commands { cancel: &cancelled }.run(waiting(), None, "waiting").unwrap();
        assert_ne!(ran.code, Some(0));
        assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The scripts as a real `sh` runs them, in a directory of the test's
    /// own. Ownership is the one thing faked, by a `stat` and a `chown`
    /// first on `PATH`, since tests don't run as root; Docker's opt-in
    /// scenarios below check the real thing. They run on Linux, which the
    /// scripts are for, and pass unrun elsewhere.
    mod scripts {
        use std::process::Output;

        use super::*;

        /// Files matching this aren't root's, to the fake `stat`.
        const THEIRS: &str = "*-theirs*";

        struct Sandbox(PathBuf);

        impl Sandbox {
            fn new(tag: &str) -> Option<Self> {
                if !cfg!(target_os = "linux") {
                    return None;
                }
                let root = std::env::temp_dir().join(format!("brindle-deploy-{tag}-{}", std::process::id()));
                std::fs::remove_dir_all(&root).ok();
                std::fs::create_dir_all(&root).unwrap();
                let sandbox = Self(root);
                // `stat -c %u FILE`: root's, unless FILE matches $THEIRS.
                sandbox.executable("bin/stat", "#!/bin/sh\ncase \"$3\" in $THEIRS) echo 1000;; *) echo 0;; esac\n");
                sandbox.executable("bin/chown", "#!/bin/sh\nexit 0\n");
                Some(sandbox)
            }

            fn path(&self, name: &str) -> PathBuf {
                self.0.join(name)
            }

            fn text(&self, name: &str) -> String {
                self.path(name).to_string_lossy().into_owned()
            }

            /// Runs `script` with `args` by `sh`, with the tools as they are.
            fn plain(&self, script: &str, args: &[&str]) -> String {
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(script)
                    .arg("sh")
                    .args(args)
                    .stdin(Stdio::null())
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{script} {args:?}: {output:?}");
                String::from_utf8_lossy(&output.stdout).into_owned()
            }

            /// Runs `script` with `args` by `sh` as an endpoint would, where
            /// files matching the pattern `theirs` aren't root's.
            fn sh(&self, script: &str, args: &[&str], theirs: &str) -> Output {
                let own = std::env::var_os("PATH").unwrap_or_default();
                let path = std::env::join_paths(
                    std::iter::once(self.path("bin")).chain(std::env::split_paths(&own)),
                )
                .unwrap();
                Command::new("sh")
                    .arg("-c")
                    .arg(script)
                    .arg("sh")
                    .args(args)
                    .env("PATH", path)
                    .env("THEIRS", theirs)
                    .env("HOME", self.path("home"))
                    // A variable of the user's that a script could clobber.
                    .env("h", "the user's")
                    .stdin(Stdio::null())
                    .output()
                    .unwrap()
            }

            /// Writes an executable file. The shell writes it, so that no
            /// child another test forks holds it open for writing while it
            /// runs (`ETXTBSY`).
            fn executable(&self, name: &str, contents: &str) {
                let write = r#"mkdir -p "${1%/*}" && printf %s "$2" > "$1" && chmod 755 "$1""#;
                self.plain(write, &[&self.text(name), contents]);
            }

            /// A helper that says `version`, and what it was run with.
            fn helper(&self, name: &str, version: &str) {
                let script = format!(
                    "#!/bin/sh\nif test \"$1\" = --version; then echo {version}; else echo \"ran $* h=$h\"; fi\n"
                );
                self.executable(name, &script);
            }

            fn symlink(&self, target: &str, link: &str) {
                self.plain(r#"ln -s "$1" "$2""#, &[&self.text(target), &self.text(link)]);
            }

            fn mode(&self, name: &str) -> String {
                self.plain(r#"stat -c %a "$1""#, &[&self.text(name)]).trim().to_string()
            }

            fn chmod(&self, name: &str, mode: &str) {
                self.plain(r#"chmod "$1" "$2""#, &[mode, &self.text(name)]);
            }

            /// The names in a directory.
            fn names(&self, directory: &str) -> Vec<String> {
                let mut names: Vec<String> = std::fs::read_dir(self.path(directory))
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                names
            }
        }

        impl Drop for Sandbox {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }

        /// A script's exit code and what it printed.
        fn out(output: &Output) -> (Option<i32>, String) {
            (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned())
        }

        /// A script that ran nothing and said the helper isn't cached.
        fn not_cached() -> (Option<i32>, String) {
            (Some(NOT_CACHED), String::new())
        }

        #[test]
        fn the_ssh_launch_script_runs_the_cached_helper_or_exits_97() {
            let Some(sandbox) = Sandbox::new("ssh-launch") else { return };
            let script = ssh_launch_script(&helper());
            let cached = "home/.cache/brindle/1.2.3-abc123/brindle-remote";
            // Nothing cached.
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), not_cached());
            // Cached: the relay runs, with the user's environment as it was.
            sandbox.helper(cached, "1.2.3");
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), (Some(0), "ran relay h=the user's\n".into()));
            // Not executable, or another version: not this build's helper.
            sandbox.chmod(cached, "644");
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), not_cached());
            sandbox.helper(cached, "1.2.2");
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), not_cached());
        }

        #[test]
        fn the_ssh_upload_script_caches_its_stdin_for_the_user_only() {
            let Some(sandbox) = Sandbox::new("ssh-upload") else { return };
            std::fs::write(sandbox.path("source"), "the helper").unwrap();
            let uploaded = Command::new("sh")
                .arg("-c")
                .arg(ssh_upload_script(&helper()))
                .env("HOME", sandbox.path("home"))
                .stdin(File::open(sandbox.path("source")).unwrap())
                .output()
                .unwrap();
            assert!(uploaded.status.success(), "{uploaded:?}");
            let directory = "home/.cache/brindle/1.2.3-abc123";
            let cached = format!("{directory}/brindle-remote");
            assert_eq!(std::fs::read_to_string(sandbox.path(&cached)).unwrap(), "the helper");
            assert_eq!((sandbox.mode(directory).as_str(), sandbox.mode(&cached).as_str()), ("700", "700"));
            // Nothing is left under a temporary name.
            assert_eq!(sandbox.names(directory), ["brindle-remote"]);
        }

        #[test]
        fn the_container_launch_script_runs_only_roots_helper() {
            let Some(sandbox) = Sandbox::new("container-launch") else { return };
            let cache = "brindle-helper-1.2.3-abc123";
            let helper = format!("{cache}/brindle-remote");
            let launch = |directory: &str, theirs: &str| {
                out(&sandbox.sh(CONTAINER_LAUNCH, &[&sandbox.text(directory)], theirs))
            };
            // No directory, then no helper in it.
            assert_eq!(launch(cache, THEIRS), not_cached());
            std::fs::create_dir(sandbox.path(cache)).unwrap();
            assert_eq!(launch(cache, THEIRS), not_cached());
            // Root's directory and root's helper.
            sandbox.helper(&helper, "1.2.3");
            assert_eq!(launch(cache, THEIRS), (Some(0), "ran relay h=the user's\n".into()));
            // A directory, or a helper, that isn't root's.
            assert_eq!(launch(cache, &sandbox.text(cache)), not_cached());
            assert_eq!(launch(cache, "*/brindle-remote"), not_cached());
            // A symbolic link to root's directory, or to root's helper.
            sandbox.symlink(cache, "brindle-helper-link");
            assert_eq!(launch("brindle-helper-link", THEIRS), not_cached());
            std::fs::create_dir(sandbox.path("brindle-helper-linked")).unwrap();
            sandbox.symlink(&helper, "brindle-helper-linked/brindle-remote");
            assert_eq!(launch("brindle-helper-linked", THEIRS), not_cached());
            // A helper that can't be run.
            sandbox.chmod(&helper, "644");
            assert_eq!(launch(cache, THEIRS), not_cached());
        }

        #[test]
        fn the_directory_script_makes_or_keeps_roots_directory_and_refuses_any_other() {
            let Some(sandbox) = Sandbox::new("container-directory") else { return };
            let run = |name: &str| sandbox.sh(CONTAINER_DIRECTORY, &[&sandbox.text(name)], THEIRS);
            let done = (Some(0), String::new());
            let taken = (Some(TAKEN), String::new());
            // Made.
            assert_eq!(out(&run("made")), done);
            assert_eq!(sandbox.mode("made"), "755");
            // Root's already: kept, with its mode put right.
            sandbox.chmod("made", "700");
            std::fs::write(sandbox.path("made/kept"), "").unwrap();
            assert_eq!(out(&run("made")), done);
            assert_eq!(sandbox.mode("made"), "755");
            assert_eq!(sandbox.names("made"), ["kept"]);
            // Someone else's directory: refused, and as it was.
            std::fs::create_dir(sandbox.path("dir-theirs")).unwrap();
            std::fs::write(sandbox.path("dir-theirs/canary"), "").unwrap();
            sandbox.chmod("dir-theirs", "700");
            assert_eq!(out(&run("dir-theirs")), taken);
            assert_eq!(sandbox.mode("dir-theirs"), "700");
            assert_eq!(sandbox.names("dir-theirs"), ["canary"]);
            // A symbolic link, to root's directory or to nothing.
            sandbox.symlink("made", "link");
            assert_eq!(out(&run("link")), taken);
            sandbox.symlink("nowhere", "dangling");
            assert_eq!(out(&run("dangling")), taken);
            assert!(!sandbox.path("nowhere").exists());
            // A file.
            std::fs::write(sandbox.path("file"), "theirs").unwrap();
            assert_eq!(out(&run("file")), taken);
            assert_eq!(std::fs::read_to_string(sandbox.path("file")).unwrap(), "theirs");
            // Nowhere to make it: mkdir's own failure, with its message.
            let failed = run("missing/made");
            assert!(!matches!(failed.status.code(), Some(0) | Some(TAKEN)), "{failed:?}");
            assert!(!failed.stderr.is_empty());
        }

        #[test]
        fn the_install_script_puts_the_helper_in_place_and_removes_roots_other_builds() {
            let Some(sandbox) = Sandbox::new("container-install") else { return };
            let cache = "brindle-helper-1.2.3-abc123";
            let helper = format!("{cache}/brindle-remote");
            let install = |copy: &str| out(&sandbox.sh(CONTAINER_INSTALL, &[&sandbox.text(cache), copy], THEIRS));
            let failed = (Some(1), String::new());
            std::fs::create_dir(sandbox.path(cache)).unwrap();
            std::fs::write(sandbox.path(&helper), "old").unwrap();
            // Beside it: root's other build, someone else's, a link, a file,
            // and directories that aren't a helper's.
            for other in ["brindle-helper-1.2.2-old", "brindle-helper-theirs", "elsewhere", "other"] {
                std::fs::create_dir(sandbox.path(other)).unwrap();
                std::fs::write(sandbox.path(other).join("kept"), "").unwrap();
            }
            sandbox.symlink("elsewhere", "brindle-helper-link");
            std::fs::write(sandbox.path("brindle-helper-file"), "").unwrap();
            let beside = sandbox.names("");

            // A copy that is missing, isn't root's or is a symbolic link is
            // removed, and nothing else changes.
            assert_eq!(install("brindle-remote.tmp-none"), failed);
            std::fs::write(sandbox.path(&format!("{cache}/brindle-remote.tmp-theirs")), "new").unwrap();
            assert_eq!(install("brindle-remote.tmp-theirs"), failed);
            sandbox.symlink("elsewhere/kept", &format!("{cache}/brindle-remote.tmp-link"));
            assert_eq!(install("brindle-remote.tmp-link"), failed);
            assert_eq!(sandbox.names(cache), ["brindle-remote"]);
            assert_eq!(std::fs::read_to_string(sandbox.path(&helper)).unwrap(), "old");
            assert_eq!(sandbox.names(""), beside);
            assert_eq!(sandbox.names("elsewhere"), ["kept"]);

            // The copy becomes the helper, and root's other build goes.
            std::fs::write(sandbox.path(&format!("{cache}/brindle-remote.tmp-1")), "new").unwrap();
            sandbox.chmod(&format!("{cache}/brindle-remote.tmp-1"), "600");
            assert_eq!(install("brindle-remote.tmp-1"), (Some(0), String::new()));
            assert_eq!(std::fs::read_to_string(sandbox.path(&helper)).unwrap(), "new");
            assert_eq!(sandbox.mode(&helper), "755");
            assert_eq!(sandbox.names(cache), ["brindle-remote"]);
            let left: Vec<String> = beside.into_iter().filter(|name| name != "brindle-helper-1.2.2-old").collect();
            assert_eq!(sandbox.names(""), left);
            for kept in ["brindle-helper-theirs", "elsewhere", "other"] {
                assert_eq!(sandbox.names(kept), ["kept"], "{kept}");
            }
            assert!(sandbox.path("brindle-helper-link").is_symlink());
        }
    }

    /// Real endpoints (opt-in: `BRINDLE_REMOTE_DOCKER_TEST=1`): the relay's
    /// launch and deployment in Docker containers and over OpenSSH, how they
    /// fail, and sessions through the relay as a remote tab runs them
    /// ([`Session::start`] on the profile's [`Host`]).
    #[cfg(any(target_os = "linux", windows))]
    mod docker {
        use std::sync::Mutex;

        use super::*;
        use crate::config::Profile;
        use crate::remote::Host;
        use crate::remote::local::{self, Link, Session, SessionSink, SpawnRequest, dial_with, start_relay};
        use crate::remote::protocol::Frame;
        use crate::remote::protocol::client::Policy;
        use crate::remote::protocol::relay::{RelayHost, RelayMessage};
        use crate::terminal::GridSize;

        type Scenario = std::result::Result<(), String>;

        /// The Docker tests take turns: `ssh_transport_scenarios` and
        /// `relay_launch_scenarios` both build and run the SSH fixture's
        /// image ([`FIXTURE_TAG`]).
        static DOCKER_TEST_LOCK: Mutex<()> = Mutex::new(());

        /// The static helper the tests deploy, built with the docker command
        /// in README.md. They pass it to the relay's dial, never through the
        /// environment.
        fn static_helper() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/x86_64-unknown-linux-musl/release/brindle-remote")
        }

        /// A lookup that always names `path`.
        fn lookup_of(path: PathBuf) -> HelperLookup {
            Arc::new(move || Ok(path.clone()))
        }

        /// With the opt-in set: this test's turn, once the static helper is
        /// known to be there.
        fn opted_in() -> Option<std::sync::MutexGuard<'static, ()>> {
            if std::env::var_os("BRINDLE_REMOTE_DOCKER_TEST").is_none_or(|v| v != "1") {
                return None;
            }
            let turn = DOCKER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let helper = static_helper();
            assert!(
                helper.is_file(),
                "{} not found; build it with the docker command in README.md",
                helper.display()
            );
            Some(turn)
        }

        /// A directory for the test's files, under target/ rather than the
        /// system temp directory: a snap Docker's `docker cp` can't see the
        /// host's /tmp.
        fn scratch(tag: &str) -> PathBuf {
            let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/tmp")
                .join(format!("brindle-{tag}-test-{}", std::process::id()));
            std::fs::create_dir_all(&scratch).unwrap();
            scratch
        }

        /// The scenarios that failed, each said as it ends.
        #[derive(Default)]
        struct Failures(Vec<String>);

        impl Failures {
            fn record(&mut self, name: &str, result: Scenario) {
                eprintln!("scenario {name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
                if let Err(e) = result {
                    self.0.push(format!("{name}: {e}"));
                }
            }

            fn check(self) {
                assert!(self.0.is_empty(), "failures:\n{}", self.0.join("\n"));
            }
        }

        struct Container(String);

        impl Container {
            fn start(image: &str, tag: &str) -> std::result::Result<Self, String> {
                Self::start_with(image, tag, &[])
            }

            fn start_with(
                image: &str,
                tag: &str,
                extra: &[&str],
            ) -> std::result::Result<Self, String> {
                let name = format!("brindle-client-test-{}-{tag}", std::process::id());
                // Own the name first so a half-started container is still removed.
                let guard = Self(name.clone());
                let out = Command::new("docker")
                    .args(["run", "-d", "--rm", "--name", &name])
                    .args(extra)
                    .args([image, "sleep", "600"])
                    .output()
                    .map_err(|e| format!("docker run: {e}"))?;
                if !out.status.success() {
                    return Err(format!(
                        "docker run {image}: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                }
                Ok(guard)
            }

            fn exec_sh(&self, script: &str) -> String {
                let out = Command::new("docker")
                    .args(["exec", &self.0, "sh", "-c", script])
                    .output()
                    .expect("docker exec");
                String::from_utf8_lossy(&out.stdout).into_owned()
            }
        }

        impl Drop for Container {
            fn drop(&mut self) {
                Command::new("docker")
                    .args(["rm", "-f", &self.0])
                    .output()
                    .ok();
            }
        }

        fn container_target(c: &Container, user: Option<&str>) -> RemoteTarget {
            RemoteTarget::Container {
                engine: "docker".into(),
                container: c.0.clone(),
                user: user.map(Into::into),
                connect_at_launch: false,
            }
        }

        fn container_profile(container: &str) -> Profile {
            Profile {
                remote: Some(RemoteTarget::Container {
                    engine: "docker".into(),
                    container: container.into(),
                    user: None,
                    connect_at_launch: false,
                }),
                ..Profile::default_shell()
            }
        }

        fn profile_for(target: &RemoteTarget) -> Profile {
            Profile { remote: Some(target.clone()), ..Profile::default_shell() }
        }

        const FIXTURE_TAG: &str = "brindle-client-test/ssh-fixture:1";

        fn ssh_target(port: u16, key: &Path) -> RemoteTarget {
            // An empty config and a throwaway known_hosts, so the machine's own
            // ssh configuration cannot interfere (no portable null device).
            let config = key.with_file_name("ssh_config");
            std::fs::write(&config, "").unwrap();
            let config = config.to_string_lossy().into_owned();
            let known_hosts = key.with_file_name("known_hosts").to_string_lossy().into_owned();
            RemoteTarget::Ssh {
                host: "127.0.0.1".into(),
                user: Some("root".into()),
                port: Some(port),
                identity_file: Some(key.to_string_lossy().into_owned()),
                ssh_args: [
                    "-F",
                    &config,
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "IdentitiesOnly=yes",
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    &format!("UserKnownHostsFile={known_hosts}"),
                    "-o",
                    "LogLevel=ERROR",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                connect_at_launch: false,
            }
        }

        fn generate_key(path: &Path) -> std::result::Result<(), String> {
            std::fs::remove_file(path).ok();
            std::fs::remove_file(path.with_extension("pub")).ok();
            let out = Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(path)
                .output()
                .map_err(|e| format!("ssh-keygen: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(format!("ssh-keygen: {}", String::from_utf8_lossy(&out.stderr)))
            }
        }

        struct SshFixture {
            container: Container,
            target: RemoteTarget,
            scratch: PathBuf,
        }

        impl SshFixture {
            /// Starts the fixture in a container named with `tag`. Each test
            /// has a tag of its own, so one whose container is still being
            /// removed is never in the next one's way.
            fn start(scratch: &Path, tag: &str) -> std::result::Result<Self, String> {
                let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote");
                let build = Command::new("docker")
                    .args(["build", "-q", "-t", FIXTURE_TAG])
                    .arg(&fixture_dir)
                    .output()
                    .map_err(|e| format!("docker build: {e}"))?;
                if !build.status.success() {
                    return Err(format!("docker build: {}", String::from_utf8_lossy(&build.stderr)));
                }
                let container = Container::start_with(FIXTURE_TAG, tag, &["-p", "127.0.0.1::22"])?;
                let key = scratch.join("id_test");
                generate_key(&key)?;
                let copy = Command::new("docker")
                    .args(["cp"])
                    .arg(key.with_extension("pub"))
                    .arg(format!("{}:/root/.ssh/authorized_keys", container.0))
                    .output()
                    .map_err(|e| format!("docker cp: {e}"))?;
                if !copy.status.success() {
                    return Err(format!("docker cp: {}", String::from_utf8_lossy(&copy.stderr)));
                }
                container.exec_sh(
                    "chown root:root /root/.ssh/authorized_keys; chmod 600 /root/.ssh/authorized_keys; chmod 700 /root/.ssh",
                );
                let sshd = Command::new("docker")
                    .args(["exec", "-d", &container.0, "/usr/sbin/sshd", "-D", "-e"])
                    .output()
                    .map_err(|e| format!("starting sshd: {e}"))?;
                if !sshd.status.success() {
                    return Err(format!("starting sshd: {}", String::from_utf8_lossy(&sshd.stderr)));
                }
                let port_out = Command::new("docker")
                    .args(["port", &container.0, "22/tcp"])
                    .output()
                    .map_err(|e| format!("docker port: {e}"))?;
                let text = String::from_utf8_lossy(&port_out.stdout).into_owned();
                let port: u16 = text
                    .lines()
                    .find_map(|line| line.trim().rsplit_once(':')?.1.parse().ok())
                    .ok_or_else(|| format!("no published ssh port: {text:?}"))?;
                let target = ssh_target(port, &key);
                let fixture = Self { container, target, scratch: scratch.to_path_buf() };
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    let mut probe = crate::remote::remote_command(
                        &fixture.target,
                        "printf",
                        &["BRINDLE_SSH_READY".into()],
                    )
                    .map_err(|e| e.to_string())?;
                    let out = probe.output().map_err(|e| format!("ssh probe: {e}"))?;
                    if String::from_utf8_lossy(&out.stdout) == "BRINDLE_SSH_READY" {
                        return Ok(fixture);
                    }
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "sshd never became ready: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ));
                    }
                    thread::sleep(Duration::from_millis(300));
                }
            }
        }

        // ---- the relay's link ----

        /// What a launched relay says first, which is `RELAY_HOST` once it
        /// has reached its core. Letting go of the link ends the relay.
        fn host_of(link: Link) -> std::result::Result<RelayHost, String> {
            let Link { mut reader, wire } = link;
            let (tx, first) = mpsc::channel();
            thread::spawn(move || {
                tx.send(Frame::read_from(&mut reader)).ok();
            });
            let frame = first.recv_timeout(Duration::from_secs(60));
            drop(wire);
            let frame = frame
                .map_err(|_| "no frame from the relay within 60 s".to_string())?
                .map_err(|e| format!("reading the relay's first frame: {e}"))?;
            match RelayMessage::decode(&frame) {
                Ok(RelayMessage::Host(host)) if host.version == env!("CARGO_PKG_VERSION") => Ok(host),
                other => Err(format!("the relay's first frame was {other:?}")),
            }
        }

        /// Connects as the relay transport does: the production dial, with
        /// the deployment it needs, of the static helper. The error is what
        /// a new tab would show.
        fn connect(target: &RemoteTarget) -> std::result::Result<RelayHost, String> {
            connect_with(target, static_helper())
        }

        /// [`connect`], deploying the helper at `path`.
        fn connect_with(target: &RemoteTarget, path: PathBuf) -> std::result::Result<RelayHost, String> {
            let link = dial_with("test".into(), target, lookup_of(path)).launch().map_err(|e| format!("{e:#}"))?;
            host_of(link)
        }

        /// Checks that the container keeps the helper at `helper` (the one
        /// deployed last), in root's own directory, and nothing else of
        /// Brindle's: no other build, no copy left under a temporary name.
        fn scenario_one_helper_kept(c: &Container, helper: PathBuf) -> Scenario {
            let directory = Helper::of(helper).map_err(|e| format!("{e:#}"))?.container_dir();
            let expected = format!("{directory} 0 755\n{directory}/brindle-remote 0 755\n");
            let kept = c.exec_sh("stat -c '%n %u %a' /tmp/brindle-* /tmp/brindle-*/* 2>&1");
            if kept == expected {
                Ok(())
            } else {
                Err(format!("kept {kept:?}, expected {expected:?}"))
            }
        }

        /// The file [`scenario_bad_helper`] deploys, in `dir`.
        fn bad_helper(dir: &Path) -> PathBuf {
            dir.join("not-a-helper")
        }

        /// A helper that isn't one is deployed as any other build is, and the
        /// relay it can't run is an error that shows what the endpoint said.
        fn scenario_bad_helper(c: &Container, dir: &Path) -> Scenario {
            let bad = bad_helper(dir);
            std::fs::write(&bad, b"this is not an executable\n").map_err(|e| e.to_string())?;
            match connect_with(&container_target(c, None), bad) {
                Ok(host) => Err(format!("connected: {host:?}")),
                Err(error) if error.contains("/brindle-remote: ") => Ok(()),
                Err(error) => Err(error),
            }
        }

        // ---- sessions through the relay, as a tab has them ----

        /// What a tab's sink was told.
        enum Seen {
            Output(Vec<u8>),
            Exited(Option<i32>),
            Lost(String),
            Failed(String),
            Foreground,
        }

        struct Sink(mpsc::Sender<Seen>);

        impl SessionSink for Sink {
            fn output(&mut self, bytes: &[u8]) {
                self.0.send(Seen::Output(bytes.to_vec())).ok();
            }

            fn note(&mut self, _text: &str) {}

            fn replay(&mut self, _active: bool) {}

            fn exited(&mut self, code: Option<i32>) {
                self.0.send(Seen::Exited(code)).ok();
            }

            fn lost(&mut self, reason: &str) {
                self.0.send(Seen::Lost(reason.into())).ok();
            }

            fn failed(&mut self, error: &anyhow::Error) {
                self.0.send(Seen::Failed(format!("{error:#}"))).ok();
            }

            fn foreground(&mut self) {
                self.0.send(Seen::Foreground).ok();
            }

            fn broken(&mut self, reason: &str) {
                self.0.send(Seen::Lost(format!("broken: {reason}"))).ok();
            }
        }

        #[derive(Debug, PartialEq)]
        enum End {
            /// What was waited for happened.
            Matched,
            Exited(Option<i32>),
            Lost(String),
            Failed(String),
            /// The sink went away.
            Closed,
            TimedOut,
        }

        /// A remote tab's session.
        struct Tab {
            session: Session,
            seen: mpsc::Receiver<Seen>,
            /// The latest output (its last 64 KiB once it passes 1 MiB).
            output: String,
            /// How many bytes of output came.
            received: u64,
        }

        impl Tab {
            /// Starts `profile`'s session on its host as `Terminal::spawn`
            /// does: `command` (program and arguments) instead of the
            /// profile's when it isn't empty, and `cwd` instead of its
            /// directory.
            fn start(profile: &Profile, command: &[&str], cwd: Option<&str>) -> Self {
                let command = command
                    .split_first()
                    .map(|(program, args)| (program.to_string(), args.iter().map(|a| a.to_string()).collect()));
                let size = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };
                let request = SpawnRequest::remote(profile, command, cwd.map(PathBuf::from), size, 1000, Policy::Kill);
                let (tx, seen) = mpsc::channel();
                let (host, lookup) = (Host::of(profile), lookup_of(static_helper()));
                let session =
                    Session::start_with(move || local::shared_with(&host, &lookup), request, Box::new(Sink(tx)));
                Self { session, seen, output: String::new(), received: 0 }
            }

            fn input(&self, text: &str) {
                self.session.input(text.as_bytes());
            }

            /// Takes what the sink is told until `done` holds, the session
            /// ends or `limit` passes.
            fn until(&mut self, limit: Duration, done: impl Fn(&Self) -> bool) -> End {
                let deadline = Instant::now() + limit;
                loop {
                    if done(self) {
                        return End::Matched;
                    }
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return End::TimedOut;
                    }
                    match self.seen.recv_timeout(left.min(Duration::from_millis(100))) {
                        Ok(Seen::Output(bytes)) => self.take(&bytes),
                        Ok(Seen::Foreground) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Ok(Seen::Exited(code)) => return End::Exited(code),
                        Ok(Seen::Lost(reason)) => return End::Lost(reason),
                        Ok(Seen::Failed(error)) => return End::Failed(error),
                        Err(mpsc::RecvTimeoutError::Disconnected) => return End::Closed,
                    }
                }
            }

            fn take(&mut self, bytes: &[u8]) {
                self.received += bytes.len() as u64;
                self.output.push_str(&String::from_utf8_lossy(bytes));
                if self.output.len() > 1 << 20 {
                    let mut cut = self.output.len() - (64 << 10);
                    while !self.output.is_char_boundary(cut) {
                        cut += 1;
                    }
                    self.output.drain(..cut);
                }
            }

            /// Waits up to 30 s for `needle` in the output.
            fn wait_for(&mut self, needle: &str) -> Scenario {
                match self.until(Duration::from_secs(30), |tab| tab.output.contains(needle)) {
                    End::Matched => Ok(()),
                    end => Err(format!("no {needle:?}: {end:?}; output ends {:?}", self.tail())),
                }
            }

            /// Runs to the end of the session.
            fn run_to_end(&mut self, limit: Duration) -> End {
                self.until(limit, |_| false)
            }

            fn tail(&self) -> &str {
                let mut at = self.output.len().saturating_sub(512);
                while !self.output.is_char_boundary(at) {
                    at += 1;
                }
                &self.output[at..]
            }
        }

        impl Drop for Tab {
            fn drop(&mut self) {
                self.session.close();
            }
        }

        /// Sessions that print and exit at once, `rounds` of them, through
        /// the endpoint's one connection.
        fn scenario_clean_exit(profile: &Profile, rounds: usize) -> Scenario {
            for round in 0..rounds {
                let mut tab = Tab::start(profile, &["sh", "-c", "printf READY; exit 0"], None);
                match tab.run_to_end(Duration::from_secs(60)) {
                    End::Exited(_) if tab.output.contains("READY") => {}
                    end => return Err(format!("round {round}: {end:?}; output {:?}", tab.output)),
                }
            }
            Ok(())
        }

        /// A tab on a container that isn't there fails, saying so.
        fn scenario_missing_container() -> Scenario {
            let mut tab = Tab::start(&container_profile("brindle-no-such-container-xyz"), &["sh"], None);
            match tab.run_to_end(Duration::from_secs(60)) {
                End::Failed(error) if error.to_lowercase().contains("no such container") => Ok(()),
                end => Err(format!("{end:?}; output {:?}", tab.output)),
            }
        }

        /// A container killed under a session ends the relay's link: the
        /// tab says the connection was lost and that it is reconnecting.
        fn scenario_killed_container() -> Scenario {
            let c = Container::start("ubuntu:24.04", "kill")?;
            let mut tab = Tab::start(&container_profile(&c.0), &["sh"], None);
            tab.input("echo up-$((1+1))\n");
            tab.wait_for("up-2\r\n")?;
            Command::new("docker").args(["kill", &c.0]).output().ok();
            tab.wait_for("; reconnecting")
        }

        /// Input, output, a resize, the foreground program the host reports
        /// (`PROCESS`) and the exit, through the relay.
        fn scenario_round_trip(profile: &Profile) -> Scenario {
            let mut tab = Tab::start(profile, &["sh"], None);
            tab.input("echo round-trip-$((1+1))\n");
            tab.wait_for("round-trip-2\r\n")?;
            tab.session.resize(120, 40, 8, 16);
            thread::sleep(Duration::from_millis(500));
            tab.input("stty size\n");
            tab.wait_for("40 120\r\n")?;
            let named = |tab: &Tab| {
                tab.session.foreground().and_then(|foreground| foreground.name).is_some_and(|name| !name.is_empty())
            };
            if tab.until(Duration::from_secs(10), named) != End::Matched {
                return Err(format!("no foreground program: {:?}", tab.session.foreground()));
            }
            tab.input("exit\n");
            match tab.run_to_end(Duration::from_secs(20)) {
                End::Exited(_) => Ok(()),
                end => Err(format!("{end:?}; output ends {:?}", tab.tail())),
            }
        }

        /// ^C stops a flood: input still gets through while the session's
        /// output is backed up.
        fn scenario_ctrl_c_during_flood(profile: &Profile) -> Scenario {
            let mut tab = Tab::start(profile, &["sh"], None);
            tab.input("yes\n");
            let flooding = tab.until(Duration::from_secs(30), |tab| tab.received > 5_000_000);
            if flooding != End::Matched {
                return Err(format!("the flood never started: {flooding:?} after {} bytes", tab.received));
            }
            tab.input("\u{3}");
            tab.input("echo CTRLC-OK-$((2+3))\n");
            tab.wait_for("CTRLC-OK-5\r\n")?;
            tab.input("exit\n");
            Ok(())
        }

        /// The profile's own program (the endpoint's login shell) answers
        /// with what it sees: `$0`, `$TERM` and `$PWD`.
        fn default_shell(profile: &Profile, cwd: Option<&str>) -> std::result::Result<String, String> {
            let mut tab = Tab::start(profile, &[], cwd);
            tab.input("echo S1=$0 S2=$TERM S3=$PWD E$((1+1))\n");
            tab.wait_for("E2\r\n")?;
            tab.input("exit\n");
            let out = &tab.output;
            let reply = out.rfind("S1=").map_or(out.as_str(), |at| &out[at..]);
            Ok(reply.lines().next().unwrap_or_default().to_string())
        }

        #[test]
        fn docker_transport_scenarios() {
            let Some(_turn) = opted_in() else { return };
            let started = Instant::now();
            let scratch = scratch("client");
            let ubuntu = Container::start("ubuntu:24.04", "main").unwrap();
            let profile = container_profile(&ubuntu.0);
            let mut failures = Failures::default();
            failures.record("a clean exit x20", scenario_clean_exit(&profile, 20));
            failures.record("b missing container", scenario_missing_container());
            failures.record("c docker kill", scenario_killed_container());
            failures.record("e/f round trip, resize, foreground", scenario_round_trip(&profile));
            match default_shell(&profile, None) {
                Ok(line) => failures.record(
                    "the default shell is the passwd login shell with a sane TERM",
                    if line.starts_with("S1=-bash S2=xterm-256color") { Ok(()) } else { Err(line) },
                ),
                Err(e) => failures.record("default shell", Err(e)),
            }
            match default_shell(&profile, Some("/no/such/dir")) {
                Ok(line) => failures.record(
                    "a missing cwd falls back to home",
                    if line.contains(" S3=/root ") { Ok(()) } else { Err(line) },
                ),
                Err(e) => failures.record("default shell, missing cwd", Err(e)),
            }
            failures.record("d the one helper is kept, root's own", scenario_one_helper_kept(&ubuntu, static_helper()));
            failures.record("g bad helper", scenario_bad_helper(&ubuntu, &scratch));
            // Deploying another build removed the first one's directory.
            failures.record(
                "g the other build is all that is kept",
                scenario_one_helper_kept(&ubuntu, bad_helper(&scratch)),
            );
            match Container::start("alpine", "alpine") {
                Ok(alpine) => {
                    // busybox's sh, stat, mkdir and mv run the scripts here.
                    failures.record("h alpine run", scenario_clean_exit(&container_profile(&alpine.0), 1));
                    failures.record(
                        "h alpine keeps the helper, root's own",
                        scenario_one_helper_kept(&alpine, static_helper()),
                    );
                }
                Err(e) => eprintln!("scenario h skipped: {e}"),
            }
            std::fs::remove_dir_all(&scratch).ok();
            failures.check();
            assert!(started.elapsed() < Duration::from_secs(180), "took {:?}", started.elapsed());
        }

        // ---- over OpenSSH ----

        fn scenario_ssh_wrong_key(fx: &SshFixture) -> Scenario {
            let wrong = fx.scratch.join("id_wrong");
            generate_key(&wrong)?;
            let RemoteTarget::Ssh { port, .. } = &fx.target else { unreachable!() };
            let target = ssh_target(port.unwrap(), &wrong);
            let result = connect(&target);
            std::fs::remove_file(&wrong).ok();
            std::fs::remove_file(wrong.with_extension("pub")).ok();
            match result {
                Err(error) if error.contains("Permission denied") => Ok(()),
                other => Err(format!("{other:?}")),
            }
        }

        /// Startup files that print don't reach the relay's frames: the
        /// launch isn't a login shell, and the frame marker skips what a
        /// non-interactive shell still prints (bash reads `~/.bashrc` under
        /// sshd).
        fn scenario_ssh_noise(fx: &SshFixture, file: &str, line: &str) -> Scenario {
            let c = &fx.container;
            c.exec_sh(&format!(
                "cp {file} {file}.orig; printf '{line}\\n' | cat - {file}.orig > {file}"
            ));
            let result = connect(&fx.target);
            c.exec_sh(&format!("mv {file}.orig {file}"));
            result.map(drop)
        }

        #[test]
        fn ssh_transport_scenarios() {
            let Some(_turn) = opted_in() else { return };
            let scratch = scratch("ssh");
            let fx = SshFixture::start(&scratch, "ssh").expect("ssh fixture");
            let profile = profile_for(&fx.target);
            let mut failures = Failures::default();
            failures.record("ssh round trip and exit", scenario_round_trip(&profile));
            failures.record("ssh ctrl-c during flood", scenario_ctrl_c_during_flood(&profile));
            failures.record("ssh wrong key", scenario_ssh_wrong_key(&fx));
            failures.record("ssh ~/.profile noise", scenario_ssh_noise(&fx, "/root/.profile", "echo NOISE_PROFILE"));
            failures.record("ssh ~/.bashrc noise", scenario_ssh_noise(&fx, "/root/.bashrc", "echo NOISE_BASHRC"));
            std::fs::remove_dir_all(&scratch).ok();
            failures.check();
        }

        // ---- the relay's launch and deployment ----

        /// The cached helper's inode and modification time.
        fn helper_file(c: &Container, path: &str) -> String {
            c.exec_sh(&format!("stat -c '%i %Y' {path} 2>&1"))
        }

        fn exec_as(c: &Container, user: &str, script: &str) -> String {
            let out = Command::new("docker")
                .args(["exec", "--user", user, &c.0, "sh", "-c", script])
                .output()
                .expect("docker exec");
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
        }

        /// The first connection to a container deploys the helper, as root
        /// and for every user; later ones, root's or another user's, run that
        /// same copy.
        fn scenario_container_helper_is_reused(c: &Container) -> Scenario {
            let directory = Helper::of(static_helper()).map_err(|e| format!("{e:#}"))?.container_dir();
            let path = format!("{directory}/brindle-remote");
            if !c.exec_sh("ls -d /tmp/brindle-* 2>/dev/null").is_empty() {
                return Err("a helper was there before the first connection".into());
            }
            let first = connect(&container_target(c, None))?;
            scenario_one_helper_kept(c, static_helper())?;
            let copied = helper_file(c, &path);
            // Later than any second copy's time could equal.
            thread::sleep(Duration::from_millis(1200));
            let second = connect(&container_target(c, None))?;
            let other = connect(&container_target(c, Some("remote")))?;
            let after = helper_file(c, &path);
            if copied != after {
                return Err(format!("the helper was copied again: {copied:?} then {after:?}"));
            }
            scenario_one_helper_kept(c, static_helper())?;
            if first.home != "/root" || second.home != "/root" || other.home != "/home/remote" {
                return Err(format!("homes: {:?} {:?} {:?}", first.home, second.home, other.home));
            }
            Ok(())
        }

        /// A directory another user of the container made where the helper
        /// would go is refused by name, and nothing in it runs or changes.
        fn scenario_taken_directory_is_refused(c: &Container) -> Scenario {
            let directory = Helper::of(static_helper()).map_err(|e| format!("{e:#}"))?.container_dir();
            let planted = exec_as(
                c,
                "remote",
                &format!(
                    "mkdir {directory} && printf '#!/bin/sh\\ntouch /tmp/canary-ran\\n' > {directory}/brindle-remote \
                     && chmod 755 {directory}/brindle-remote"
                ),
            );
            if !planted.is_empty() {
                return Err(format!("planting the directory: {planted}"));
            }
            let before = c.exec_sh(&format!("stat -c '%n %U %a %i %Y' {directory} {directory}/* 2>&1"));
            for user in [None, Some("remote")] {
                match connect(&container_target(c, user)) {
                    Ok(host) => return Err(format!("connected as {user:?}: {host:?}")),
                    Err(error) if error.contains(&format!("{directory} exists in the container")) => {}
                    Err(error) => return Err(format!("as {user:?}, the error doesn't name it: {error}")),
                }
            }
            let after = c.exec_sh(&format!("stat -c '%n %U %a %i %Y' {directory} {directory}/* 2>&1"));
            if before != after || !before.contains(" remote ") {
                return Err(format!("the directory changed: {before:?} then {after:?}"));
            }
            let ran = c.exec_sh("ls /tmp/canary-ran /tmp/brindle-helper-*/brindle-remote.tmp-* 2>/dev/null");
            if !ran.is_empty() {
                return Err(format!("something ran in it, or was copied into it: {ran}"));
            }
            Ok(())
        }

        /// A runner that counts what the real one runs.
        struct Counted<'a> {
            commands: Commands<'a>,
            ran: RefCell<Vec<Vec<String>>>,
        }

        impl Run for Counted<'_> {
            fn run(&self, command: Command, input: Option<&Path>, what: &str) -> Result<Ran> {
                self.ran.borrow_mut().push(line(&command));
                self.commands.run(command, input, what)
            }
        }

        /// Commands that were started, each as its words.
        type CommandLines = Vec<Vec<String>>;

        /// Launches the relay with the real commands, returning each launch
        /// and each deployment command that was started. Every process of a
        /// launch is one of the two.
        fn counted_launch(target: &RemoteTarget) -> std::result::Result<(CommandLines, CommandLines), String> {
            let helper = Helper::of(static_helper()).map_err(|e| format!("{e:#}"))?;
            let never = AtomicBool::new(false);
            let counted = Counted { commands: Commands { cancel: &never }, ran: RefCell::default() };
            let launched = RefCell::new(Vec::new());
            let link = launch_with(&counted, target, &helper, |command| {
                launched.borrow_mut().push(line(&command));
                start_relay(command, Duration::from_secs(60))
            })
            .map_err(|e| format!("{e:#}"))?;
            host_of(link)?;
            Ok((launched.into_inner(), counted.ran.into_inner()))
        }

        /// A host without the helper takes three `ssh` processes (the launch
        /// that finds none, the upload, the launch); with it cached, the
        /// launch is the only one. Counted where the launch starts them.
        fn scenario_warm_ssh_launch_is_one_process(fx: &SshFixture) -> Scenario {
            let c = &fx.container;
            if !c.exec_sh("test -e /root/.cache/brindle && echo present").trim().is_empty() {
                return Err("a cache was there before the first connection".into());
            }
            let (launched, ran) = counted_launch(&fx.target)?;
            if launched.len() != 2 || ran.len() != 1 {
                return Err(format!("the first connection: launched {launched:?}, ran {ran:?}"));
            }
            // Everything in the build's directory, which is the helper alone.
            // (The host's server directory is beside it.)
            let cached = || {
                c.exec_sh(&format!(
                    "stat -c '%n %i %Y' /root/.cache/brindle/{}-*/* 2>&1",
                    env!("CARGO_PKG_VERSION")
                ))
            };
            let uploaded = cached();
            if uploaded.lines().count() != 1 || !uploaded.contains("/brindle-remote ") {
                return Err(format!("the cache holds {uploaded:?}"));
            }
            thread::sleep(Duration::from_millis(1200));
            let (launched, ran) = counted_launch(&fx.target)?;
            if launched.len() != 1 || launched[0][0] != "ssh" || !ran.is_empty() {
                return Err(format!("the warm connection: launched {launched:?}, ran {ran:?}"));
            }
            // And through the production dial, which uploads nothing either.
            connect(&fx.target)?;
            let after = cached();
            if uploaded != after {
                return Err(format!("the helper was uploaded again: {uploaded:?} then {after:?}"));
            }
            Ok(())
        }

        #[test]
        fn relay_launch_scenarios() {
            let Some(_turn) = opted_in() else { return };
            let scratch = scratch("relay");
            // The fixture's image has a second user, `remote`.
            let fx = SshFixture::start(&scratch, "relay-ssh").expect("ssh fixture");
            let reused = Container::start(FIXTURE_TAG, "reuse").unwrap();
            let taken = Container::start(FIXTURE_TAG, "taken").unwrap();
            let mut failures = Failures::default();
            failures.record("a warm ssh launch is one ssh process", scenario_warm_ssh_launch_is_one_process(&fx));
            failures.record("a container's helper is reused", scenario_container_helper_is_reused(&reused));
            failures.record("a taken directory is refused", scenario_taken_directory_is_refused(&taken));
            std::fs::remove_dir_all(&scratch).ok();
            failures.check();
        }
    }
}
