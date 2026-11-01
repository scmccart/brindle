//! Remote Linux profiles exercised against disposable Docker containers.
//! The fixture deliberately contains no Brindle helper: each PTY run must
//! deploy the sibling `brindle-remote` binary before it can show a shell.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::brindle::{self, Dump};
use crate::{Ctx, Outcome, Result, ensure};

const CONTAINER_PROFILE: &str = "Remote container";
const TMUX_PROFILE: &str = "Remote tmux";
const SSH_PROFILE: &str = "Remote SSH";
const REMOTE_CWD: &str = "/tmp/brindle-e2e-osc7";
const TMUX_SOCKET: &str = "brindle-e2e-remote";
const TMUX_SESSION: &str = "remote";

static CONTAINER_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIXTURE_IMAGE: OnceLock<Result<String>> = OnceLock::new();

/// A vanilla container profile covers helper injection, PTY input/output,
/// OSC 7 directory inheritance, OSC 7501, and direct remote tmux control
/// mode. The two Brindle launches share only this disposable container.
pub fn container_exec(ctx: &Ctx) -> Result<Outcome> {
    if let Some(outcome) = docker_outcome()? {
        return Ok(outcome);
    }
    let Some(agent) = remote_agent(ctx)? else {
        return Ok(Outcome::Skip(
            "Linux remote helper was not built; run `cargo build -p brindle-remote-agent`".into(),
        ));
    };
    let image = fixture_image(ctx)?;
    let container = Container::start(ctx, &image, false)?;
    let planted = format!("/tmp/brindle-remote-{}", env!("CARGO_PKG_VERSION"));
    let marker = "/tmp/brindle-e2e-planted-helper-ran";
    container.exec(&[
        "sh",
        "-lc",
        &format!(
            "printf '#!/bin/sh\\ntouch {marker}\\necho {}\\n' > {planted}; chmod 755 {planted}",
            env!("CARGO_PKG_VERSION")
        ),
    ])?;

    let mut config = format!(
        "{}\n[[profiles]]\n\
         name = {profile:?}\n\
         command = \"/bin/bash\"\n\
         args = [\"--norc\", \"--noprofile\", \"-i\"]\n\
         env = {{ SHELL = \"/bin/bash\", PS1 = \"REMOTE_CONTAINER_READY> \", BRINDLE_E2E_MARKER = \"container-exec\" }}\n\
         remote = {{ type = \"container\", engine = \"docker\", container = {container_name:?}, user = \"remote\" }}\n",
        brindle::config_with("remote-unused", "e2e", CONTAINER_PROFILE, ""),
        profile = CONTAINER_PROFILE,
        container_name = container.name,
    );
    config.push_str("\n[notifications]\nwhen = \"always\"\n");
    // The marker is assembled by printf so --wait-for cannot match the
    // shell's echo of the command before OSC 7 has actually been consumed.
    let set_cwd = format!(
        r#"mkdir -p {REMOTE_CWD}; cd {REMOTE_CWD}; printf '\033]7;file://remote%s\033\\' "$PWD"; printf 'REMOTE_%s' CWD_SET; echo\r"#
    );
    let exercise = concat!(
        "printf 'REMOTE_CONTAINER_IO:%s' \"$BRINDLE_E2E_MARKER\"; echo; ",
        "printf 'REMOTE_CONTAINER_CWD:%s' \"$PWD\"; echo; ",
        "printf '\\033]7501;state=working:id=remote:progress=73:app=remote-build\\033\\\\'; ",
        "printf '\\033]9;remote osc9\\033\\\\'; ",
        "printf '\\033]777;notify;Remote build;done\\033\\\\'; ",
        "printf 'REMOTE_%s' CONTAINER_STATUS_READY; echo; sleep 30\\r"
    )
    .to_string();
    let steps = vec![
        "--wait-for".into(),
        "REMOTE_CONTAINER_READY>".into(),
        "--send".into(),
        set_cwd,
        "--wait-for".into(),
        "REMOTE_CWD_SET".into(),
        "--action".into(),
        "split_zone_right".into(),
        "--wait-for".into(),
        "REMOTE_CONTAINER_READY>".into(),
        "--send".into(),
        exercise,
        "--wait-for".into(),
        "REMOTE_CONTAINER_STATUS_READY".into(),
    ];
    let dump = run_brindle(
        ctx,
        "container-pty",
        &config,
        CONTAINER_PROFILE,
        &steps,
        &agent,
        16.0,
    )?;
    ensure!(
        dump.zones.as_deref() == Some("count=2 active=2"),
        "remote split zones: {:?}",
        dump.zones
    );
    ensure!(
        dump.screen
            .iter()
            .any(|line| line == "REMOTE_CONTAINER_IO:container-exec"),
        "remote PTY output missing from screen: {:?}",
        dump.screen
    );
    let expected_cwd = format!("REMOTE_CONTAINER_CWD:{REMOTE_CWD}");
    ensure!(
        dump.screen.iter().any(|line| line == &expected_cwd),
        "the split did not inherit the OSC 7 directory: {:?}",
        dump.screen
    );
    let expected_status = ["state=working id=\"remote\" progress=73 app=\"remote-build\""];
    ensure!(
        dump.status == expected_status,
        "remote status {:?}, expected {expected_status:?}",
        dump.status
    );
    let notifications =
        fs::read_to_string(ctx.dir.join("container-pty.notify")).unwrap_or_default();
    ensure!(
        notifications.lines().any(|line| line.starts_with("osc9\t")),
        "remote OSC 9 notification missing: {notifications:?}"
    );
    ensure!(
        notifications
            .lines()
            .any(|line| line.starts_with("osc777\tRemote build")),
        "remote OSC 777 notification missing: {notifications:?}"
    );
    container.assert_absent(marker)?;
    container.exec(&["rm", "--force", &planted])?;
    // Remote control mode is a direct tmux byte stream, so it needs no
    // deployed PTY helper at all.

    setup_tmux(ctx, &container)?;
    let config = format!(
        "{}\n[[profiles]]\n\
         name = {profile:?}\n\
         tmux = \"control\"\n\
         tmux_session = {session:?}\n\
         tmux_args = [\"-L\", {socket:?}, \"-f\", \"/dev/null\"]\n\
         remote = {{ type = \"container\", engine = \"docker\", container = {container_name:?} }}\n",
        brindle::config_with("remote-unused", "e2e", TMUX_PROFILE, ""),
        profile = TMUX_PROFILE,
        session = TMUX_SESSION,
        socket = TMUX_SOCKET,
        container_name = container.name,
    );
    let steps = vec![
        "--wait-for".into(),
        "REMOTE_TMUX_ALPHA".into(),
        "--action".into(),
        "next_tab".into(),
        "--wait-for".into(),
        "REMOTE_TMUX_BETA".into(),
        "--send".into(),
        "printf 'REMOTE_TMUX_%s' INPUT; echo\\r".into(),
        "--wait-for".into(),
        "REMOTE_TMUX_INPUT".into(),
    ];
    let dump = run_brindle(
        ctx,
        "container-tmux",
        &config,
        TMUX_PROFILE,
        &steps,
        &agent,
        14.0,
    )?;
    ensure!(
        dump.tabs.len() == 2
            && dump.tabs[0].0 == "alpha"
            && !dump.tabs[0].1
            && dump.tabs[1].0 == "beta"
            && dump.tabs[1].1,
        "remote tmux native tabs: {:?}",
        dump.tabs
    );
    ensure!(
        dump.screen.iter().any(|line| line == "REMOTE_TMUX_BETA")
            && dump.screen.iter().any(|line| line == "REMOTE_TMUX_INPUT"),
        "remote tmux output/input missing from active native tab: {:?}",
        dump.screen
    );

    Ok(Outcome::Pass)
}

/// An SSH endpoint starts without a cached helper. Brindle must upload it
/// through the generated key before the remote interactive shell can run.
pub fn ssh_container(ctx: &Ctx) -> Result<Outcome> {
    if let Some(outcome) = docker_outcome()? {
        return Ok(outcome);
    }
    let Some(agent) = remote_agent(ctx)? else {
        return Ok(Outcome::Skip(
            "Linux remote helper was not built; run `cargo build -p brindle-remote-agent`".into(),
        ));
    };
    let image = fixture_image(ctx)?;
    let key = KeyPair::generate(ctx)?;
    let container = Container::start(ctx, &image, true)?;
    let cache_dir = "/root/.cache/brindle";
    container.assert_absent(cache_dir)?;
    container.copy_to(&key.public, "/root/.ssh/authorized_keys")?;
    container.exec(&["chmod", "700", "/root/.ssh"])?;
    container.exec(&["chmod", "600", "/root/.ssh/authorized_keys"])?;
    container.exec(&["/usr/sbin/sshd", "-t"])?;
    container.exec_detached(&["/usr/sbin/sshd", "-D", "-e"])?;
    let port = container.ssh_port()?;
    let host = std::env::var("BRINDLE_REMOTE_E2E_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    wait_for_ssh(ctx, &key.private, &host, port)?;

    let identity = key.private.to_string_lossy();
    let config = format!(
        "{}\n[[profiles]]\n\
         name = {profile:?}\n\
         command = \"/bin/bash\"\n\
         args = [\"--norc\", \"--noprofile\", \"-i\"]\n\
         env = {{ SHELL = \"/bin/bash\", PS1 = \"REMOTE_SSH_READY> \", BRINDLE_E2E_MARKER = \"ssh-upload\" }}\n\
         remote = {{ type = \"ssh\", host = {host:?}, user = \"root\", port = {port}, identity_file = {identity:?}, ssh_args = [\"-F\", \"/dev/null\", \"-o\", \"BatchMode=yes\", \"-o\", \"IdentitiesOnly=yes\", \"-o\", \"StrictHostKeyChecking=no\", \"-o\", \"UserKnownHostsFile=/dev/null\", \"-o\", \"GlobalKnownHostsFile=/dev/null\", \"-o\", \"LogLevel=ERROR\"] }}\n",
        brindle::config_with("remote-unused", "e2e", SSH_PROFILE, ""),
        profile = SSH_PROFILE,
    );
    let steps = vec![
        "--wait-for".into(),
        "REMOTE_SSH_READY>".into(),
        "--send".into(),
        "printf 'SSH_REMOTE_IO:%s' \"$BRINDLE_E2E_MARKER\"; echo\\r".into(),
        "--wait-for".into(),
        "SSH_REMOTE_IO:ssh-upload".into(),
    ];
    let dump = run_brindle(ctx, "ssh-pty", &config, SSH_PROFILE, &steps, &agent, 12.0)?;
    ensure!(
        dump.screen
            .iter()
            .any(|line| line == "SSH_REMOTE_IO:ssh-upload"),
        "SSH remote PTY output missing from screen: {:?}",
        dump.screen
    );
    container.assert_cached_agent(cache_dir)?;

    Ok(Outcome::Pass)
}

fn docker_outcome() -> Result<Option<Outcome>> {
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output();
    let unavailable = match output {
        Ok(output) if output.status.success() => return Ok(None),
        Ok(output) => format!("docker daemon is unavailable: {}", output_message(&output)),
        Err(error) => format!("docker is unavailable: {error}"),
    };
    if std::env::var("BRINDLE_REMOTE_E2E_REQUIRED").as_deref() == Ok("1") {
        Err(format!("{unavailable} (BRINDLE_REMOTE_E2E_REQUIRED=1)"))
    } else {
        Ok(Some(Outcome::Skip(unavailable)))
    }
}

fn remote_agent(ctx: &Ctx) -> Result<Option<PathBuf>> {
    let path = std::env::var_os("BRINDLE_REMOTE_AGENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.bin.with_file_name("brindle-remote"));
    if !path.is_file() {
        if std::env::var("BRINDLE_REMOTE_E2E_REQUIRED").as_deref() == Ok("1") {
            return Err(format!(
                "Linux remote helper not found at {}; run `cargo build -p brindle-remote-agent`",
                path.display()
            ));
        }
        return Ok(None);
    }
    fs::canonicalize(&path)
        .map(Some)
        .map_err(|error| format!("resolving remote helper {}: {error}", path.display()))
}

fn fixture_image(ctx: &Ctx) -> Result<String> {
    FIXTURE_IMAGE
        .get_or_init(|| build_fixture_image(ctx))
        .clone()
}

fn build_fixture_image(ctx: &Ctx) -> Result<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote");
    let mut hasher = DefaultHasher::new();
    for name in ["Dockerfile", "sshd_config"] {
        name.hash(&mut hasher);
        let path = root.join(name);
        fs::read(&path)
            .map_err(|error| format!("reading remote fixture {}: {error}", path.display()))?
            .hash(&mut hasher);
    }
    let digest = format!("{:016x}", hasher.finish());
    let image = format!("brindle-e2e/remote-fixture:{digest}");
    let label = format!("org.brindle.e2e.fixture={digest}");
    let inspect = Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{ index .Config.Labels \"org.brindle.e2e.fixture\" }}",
            &image,
        ])
        .output()
        .map_err(|error| format!("inspecting Docker fixture image: {error}"))?;
    if inspect.status.success() && String::from_utf8_lossy(&inspect.stdout).trim() == digest {
        return Ok(image);
    }

    let output = Command::new("docker")
        .args(["build", "--label", &label, "--tag", &image])
        .arg(&root)
        .output()
        .map_err(|error| format!("building Docker fixture image: {error}"))?;
    let mut log = output.stdout.clone();
    log.extend_from_slice(&output.stderr);
    ctx.save("docker-build.log", log);
    ensure!(
        output.status.success(),
        "building Docker fixture image failed: {}",
        output_message(&output)
    );
    Ok(image)
}

struct Container {
    name: String,
    owner: String,
}

impl Container {
    fn start(ctx: &Ctx, image: &str, publish_ssh: bool) -> Result<Self> {
        let sequence = CONTAINER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let owner = format!("{}-{nonce}-{sequence}", std::process::id());
        let name = format!("brindle-e2e-remote-{owner}");
        let mut command = Command::new("docker");
        command
            .args(["run", "--detach", "--name"])
            .arg(&name)
            .args(["--label", &format!("org.brindle.e2e.case={}", ctx.name)])
            .args(["--label", &format!("org.brindle.e2e.owner={owner}")]);
        if publish_ssh {
            command.args(["--publish", "127.0.0.1::22"]);
        }
        command.args([image, "sleep", "infinity"]);
        let output = match command.output() {
            Ok(output) => output,
            Err(error) => {
                remove_container(&name, &owner);
                return Err(format!("starting Docker fixture container: {error}"));
            }
        };
        if !output.status.success() {
            remove_container(&name, &owner);
            return Err(format!(
                "starting Docker fixture container failed: {}",
                output_message(&output)
            ));
        }
        let container = Self { name, owner };
        container.wait_running(ctx)?;
        Ok(container)
    }

    fn wait_running(&self, ctx: &Ctx) -> Result {
        let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(10.0));
        loop {
            let output = Command::new("docker")
                .args(["inspect", "--format", "{{.State.Running}}", &self.name])
                .output()
                .map_err(|error| format!("probing Docker fixture container: {error}"))?;
            let message = output_message(&output);
            if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("Docker fixture container was not ready: {message}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn exec_output(&self, args: &[&str]) -> Result<Output> {
        Command::new("docker")
            .arg("exec")
            .arg(&self.name)
            .args(args)
            .output()
            .map_err(|error| format!("docker exec {}: {error}", args.join(" ")))
    }

    fn exec(&self, args: &[&str]) -> Result<String> {
        let output = self.exec_output(args)?;
        ensure!(
            output.status.success(),
            "docker exec {} failed: {}",
            args.join(" "),
            output_message(&output)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn exec_detached(&self, args: &[&str]) -> Result {
        let output = Command::new("docker")
            .args(["exec", "--detach", &self.name])
            .args(args)
            .output()
            .map_err(|error| {
                format!("starting detached docker exec {}: {error}", args.join(" "))
            })?;
        ensure!(
            output.status.success(),
            "detached docker exec {} failed: {}",
            args.join(" "),
            output_message(&output)
        );
        Ok(())
    }

    fn copy_to(&self, source: &Path, destination: &str) -> Result {
        let target = format!("{}:{destination}", self.name);
        let output = Command::new("docker")
            .arg("cp")
            .arg(source)
            .arg(&target)
            .output()
            .map_err(|error| {
                format!("copying {} into Docker fixture: {error}", source.display())
            })?;
        ensure!(
            output.status.success(),
            "docker cp failed: {}",
            output_message(&output)
        );
        Ok(())
    }

    fn assert_absent(&self, path: &str) -> Result {
        self.exec(&["test", "!", "-e", path]).map(|_| ())
    }

    fn assert_cached_agent(&self, cache_dir: &str) -> Result {
        let path = self.exec(&[
            "sh",
            "-lc",
            &format!("find {cache_dir} -type f -name brindle-remote -perm /111 -print -quit"),
        ])?;
        let path = path.trim();
        ensure!(!path.is_empty(), "no cached SSH helper under {cache_dir}");
        let version = self.exec(&[path, "--version"])?;
        ensure!(
            version.trim() == env!("CARGO_PKG_VERSION"),
            "deployed helper version {:?}, expected {}",
            version.trim(),
            env!("CARGO_PKG_VERSION")
        );
        Ok(())
    }

    fn ssh_port(&self) -> Result<u16> {
        let output = Command::new("docker")
            .args(["port", &self.name, "22/tcp"])
            .output()
            .map_err(|error| format!("reading Docker SSH port: {error}"))?;
        ensure!(
            output.status.success(),
            "docker port failed: {}",
            output_message(&output)
        );
        let address = String::from_utf8_lossy(&output.stdout);
        address
            .lines()
            .find_map(|line| line.trim().rsplit_once(':')?.1.parse().ok())
            .ok_or_else(|| format!("docker port returned no TCP port: {address:?}"))
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        remove_container(&self.name, &self.owner);
    }
}

fn remove_container(name: &str, owner: &str) {
    let inspect = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{ index .Config.Labels \"org.brindle.e2e.owner\" }}",
            name,
        ])
        .output();
    let ours = inspect.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == owner
    });
    if ours {
        Command::new("docker")
            .args(["rm", "--force", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok();
    }
}

struct KeyPair {
    private: PathBuf,
    public: PathBuf,
}

impl KeyPair {
    fn generate(ctx: &Ctx) -> Result<Self> {
        let private = ctx.dir.join("ssh-key");
        let public = ctx.dir.join("ssh-key.pub");
        fs::remove_file(&private).ok();
        fs::remove_file(&public).ok();
        let output = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&private)
            .output()
            .map_err(|error| format!("generating temporary SSH key: {error}"))?;
        if !output.status.success() {
            fs::remove_file(&private).ok();
            fs::remove_file(&public).ok();
            return Err(format!("ssh-keygen failed: {}", output_message(&output)));
        }
        Ok(Self { private, public })
    }
}

impl Drop for KeyPair {
    fn drop(&mut self) {
        fs::remove_file(&self.private).ok();
        fs::remove_file(&self.public).ok();
    }
}

fn wait_for_ssh(ctx: &Ctx, identity: &Path, host: &str, port: u16) -> Result {
    let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(15.0));
    loop {
        let output = ssh_command(identity, port)
            .arg(format!("root@{host}"))
            .arg("printf BRINDLE_SSH_READY")
            .output()
            .map_err(|error| format!("probing fixture SSH server: {error}"))?;
        let message = output_message(&output);
        if output.status.success() && String::from_utf8_lossy(&output.stdout) == "BRINDLE_SSH_READY"
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fixture SSH server was not ready: {message}"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn ssh_command(identity: &Path, port: u16) -> Command {
    let mut command = Command::new("ssh");
    command
        .args(["-F", "/dev/null", "-p", &port.to_string(), "-i"])
        .arg(identity)
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
            "LogLevel=ERROR",
            "-o",
            "ConnectTimeout=2",
        ]);
    command
}

fn setup_tmux(ctx: &Ctx, container: &Container) -> Result {
    container.exec(&[
        "tmux",
        "-L",
        TMUX_SOCKET,
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        TMUX_SESSION,
        "-n",
        "alpha",
        "printf 'REMOTE_TMUX_ALPHA\\n'; exec bash --norc --noprofile",
    ])?;
    container.exec(&[
        "tmux",
        "-L",
        TMUX_SOCKET,
        "-f",
        "/dev/null",
        "new-window",
        "-d",
        "-t",
        TMUX_SESSION,
        "-n",
        "beta",
        "printf 'REMOTE_TMUX_BETA\\n'; exec bash --norc --noprofile",
    ])?;
    container.exec(&[
        "tmux",
        "-L",
        TMUX_SOCKET,
        "-f",
        "/dev/null",
        "select-window",
        "-t",
        &format!("{TMUX_SESSION}:alpha"),
    ])?;
    wait_for_tmux_text(ctx, container, "alpha", "REMOTE_TMUX_ALPHA")?;
    wait_for_tmux_text(ctx, container, "beta", "REMOTE_TMUX_BETA")
}

fn wait_for_tmux_text(ctx: &Ctx, container: &Container, window: &str, text: &str) -> Result {
    let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(10.0));
    let target = format!("{TMUX_SESSION}:{window}");
    loop {
        let output = container.exec_output(&[
            "tmux",
            "-L",
            TMUX_SOCKET,
            "-f",
            "/dev/null",
            "capture-pane",
            "-p",
            "-t",
            &target,
        ])?;
        let message = output_message(&output);
        if output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line == text)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("tmux window {window:?} was not ready: {message}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_brindle(
    ctx: &Ctx,
    label: &str,
    config: &str,
    profile: &str,
    steps: &[String],
    agent: &Path,
    dump_after: f64,
) -> Result<Dump> {
    let config_path = ctx.dir.join(format!("{label}.toml"));
    fs::write(&config_path, config)
        .map_err(|error| format!("writing {}: {error}", config_path.display()))?;
    let log_path = ctx.dir.join(format!("{label}.log"));
    let log = fs::File::create(&log_path)
        .map_err(|error| format!("creating {}: {error}", log_path.display()))?;
    let dump_after = ctx.secs(dump_after);
    let mut command = Command::new(&ctx.bin);
    let child = command
        .args(["-p", profile])
        .args(steps)
        .args(ctx.step_interval())
        .args(["--dump-screen-after", &dump_after.to_string()])
        .env("BRINDLE_CONFIG", &config_path)
        .env("BRINDLE_REMOTE_AGENT", agent)
        .env(
            "BRINDLE_NOTIFY_LOG",
            ctx.dir.join(format!("{label}.notify")),
        )
        .env("DISPLAY", &ctx.display)
        .env_remove("WAYLAND_DISPLAY")
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(|error| format!("starting {}: {error}", ctx.bin.display()))?;
    let mut child = ChildGuard { child };
    let deadline = Instant::now() + Duration::from_secs_f64(dump_after + 30.0);
    let status = loop {
        if let Some(status) = child
            .child
            .try_wait()
            .map_err(|error| format!("waiting for Brindle: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(format!("Brindle did not exit; see {}", log_path.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout = String::new();
    child
        .child
        .stdout
        .take()
        .ok_or_else(|| "Brindle stdout was not piped".to_string())?
        .read_to_string(&mut stdout)
        .map_err(|error| format!("reading Brindle screen dump: {error}"))?;
    ctx.save(&format!("{label}.dump"), &stdout);
    ensure!(
        status.success(),
        "Brindle exited with {status}; see {}",
        log_path.display()
    );
    let dump = Dump::parse(&stdout);
    ensure!(
        dump.size.is_some(),
        "no screen in {label}.dump; see {}",
        log_path.display()
    );
    Ok(dump)
}

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }
}

fn output_message(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match (stdout.trim(), stderr.trim()) {
        ("", "") => output.status.to_string(),
        (stdout, "") => stdout.to_owned(),
        ("", stderr) => stderr.to_owned(),
        (stdout, stderr) => format!("{stdout}; {stderr}"),
    }
}
