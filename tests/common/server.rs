//! What both GUI suites do around each case's Brindle: the case's own
//! directories and hermetic environment ([`isolate`]), the terminal server
//! helper it needs, found as Brindle finds it ([`helper`], [`check_helper`]),
//! and the stop step that ends the case's server and fails a case that left
//! a role running or ran one in a systemd scope ([`stop`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::Result;
use crate::ensure;

/// The suite a private directory is for. Closed, so a third suite names
/// its own prefix.
#[derive(Clone, Copy)]
pub enum Suite {
    E2e,
    Perf,
}

impl Suite {
    fn prefix(self) -> &'static str {
        match self {
            Suite::E2e => "brindle-e2e",
            Suite::Perf => "brindle-perf",
        }
    }
}

/// Case number `n`'s terminal server directory, given its directory
/// `case_dir`: on Linux under the system temp directory, not `case_dir`,
/// because a holder's socket path must fit in `sun_path` (108 bytes); on
/// Windows `case_dir`'s `server`.
pub fn private_dir(suite: Suite, case_dir: &Path, n: usize) -> PathBuf {
    if cfg!(windows) {
        case_dir.join("server")
    } else {
        std::env::temp_dir().join(format!("{}-{}-{n}", suite.prefix(), std::process::id()))
    }
}

/// The unit a role ran in, from its `/proc/<pid>/cgroup`, when that is one
/// of the terminal server's own scopes (`brindle-serve-….scope`,
/// `brindle-hold-….scope`); `None` for any other cgroup.
pub fn scoped_unit(cgroup: &str) -> Option<&str> {
    cgroup.lines().find_map(|line| {
        let path = line.splitn(3, ':').nth(2)?;
        let unit = path.rsplit('/').next()?;
        let role_scope = ["brindle-serve-", "brindle-hold-"].iter().any(|prefix| unit.starts_with(prefix));
        (role_scope && unit.ends_with(".scope")).then_some(unit)
    })
}

/// What every Brindle a suite launches is given, so it never reads the
/// user's config, session or terminal server, starts no systemd scope and
/// opens on the case's display.
pub struct Isolation<'a> {
    pub config: &'a Path,
    pub state_dir: &'a Path,
    pub server_dir: &'a Path,
    pub display: &'a str,
}

/// Sets the case's isolation on `command`. Callers apply it first and the
/// case's own removals and additions after, so a case can still override
/// a setting.
pub fn isolate(command: &mut Command, run: &Isolation) {
    command
        .env("BRINDLE_CONFIG", run.config)
        .env("BRINDLE_STATE_DIR", run.state_dir)
        .env("BRINDLE_SERVER_DIR", run.server_dir)
        .env("BRINDLE_NO_SCOPE", "1")
        .env("DISPLAY", run.display)
        .env_remove("WAYLAND_DISPLAY");
}

/// The variable that names another terminal server helper. On Windows
/// `BRINDLE_REMOTE_AGENT` is the Linux helper the `remote-*` cases deploy,
/// and Brindle doesn't read it for the local server.
pub const HELPER_VARIABLE: &str = if cfg!(windows) { "BRINDLE_SERVER_HELPER" } else { "BRINDLE_REMOTE_AGENT" };

/// The helper's file name as cargo builds it.
pub fn helper_name() -> String {
    format!("brindle-remote{}", std::env::consts::EXE_SUFFIX)
}

/// The names Brindle looks for the Linux helper by beside its binary, in
/// order (`HELPER_NAMES` in `src/remote/mod.rs`): the archives' name, then
/// cargo's. Remote tabs deploy it, and on Linux it is the terminal server's
/// helper too.
pub const LINUX_HELPER_NAMES: [&str; 2] = ["brindle-remote-linux-x86_64", "brindle-remote"];

/// The names Brindle looks for its terminal server helper by beside its
/// binary, in order: on Windows only `brindle-remote.exe`, else
/// [`LINUX_HELPER_NAMES`].
pub fn server_helper_names() -> &'static [&'static str] {
    if cfg!(windows) { &["brindle-remote.exe"] } else { &LINUX_HELPER_NAMES }
}

/// The paths Brindle looks beside: `bin` as given, then the file behind it
/// when that is elsewhere (a link, such as WinGet's), as `exe_paths` in
/// `src/remote/mod.rs` does.
pub fn bin_paths(bin: &Path) -> Vec<PathBuf> {
    let mut paths = vec![bin.to_path_buf()];
    if let Ok(resolved) = std::fs::canonicalize(bin)
        && resolved != bin
    {
        paths.push(resolved);
    }
    paths
}

/// The first of `names` that `is_file` accepts beside one of `bins`, every
/// name beside each path in turn: where Brindle finds a helper.
pub fn beside(bins: &[PathBuf], names: &[&str], is_file: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    bins.iter().flat_map(|bin| names.iter().map(move |name| bin.with_file_name(name))).find(|path| is_file(path))
}

/// The helper named by [`HELPER_VARIABLE`], when that is set and not empty
/// (Brindle takes an empty one for unset).
fn named_helper() -> Option<PathBuf> {
    std::env::var_os(HELPER_VARIABLE).filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// The helper Brindle runs its terminal server with: [`HELPER_VARIABLE`],
/// else the first of [`server_helper_names`] beside the binary, as Brindle
/// looks for it (so an archive's `brindle-remote-linux-x86_64` on Linux);
/// cargo's name beside it when there is none, for [`check_helper`] to report.
pub fn helper(bin: &Path) -> PathBuf {
    named_helper().unwrap_or_else(|| {
        beside(&bin_paths(bin), server_helper_names(), &|path: &Path| path.is_file())
            .unwrap_or_else(|| bin.with_file_name(helper_name()))
    })
}

/// What [`check_helper`] asks of a helper beyond its being there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// This build's own: no older than its sources, since a test target
    /// doesn't rebuild it.
    Sources,
    /// Beside another binary, such as an installed or unpacked release
    /// (`BRINDLE_E2E_BIN` outside `target/`): it has no sources here to be
    /// newer than, so it must be that binary's own release, its `--version`
    /// the binary's.
    SameRelease,
    /// Another build [`HELPER_VARIABLE`] chose: taken as it is.
    Chosen,
}

/// The rule for `helper`, Brindle's helper when the suite runs `bin`, and
/// `default_bin`, the suite's own build of Brindle; `named` when
/// [`HELPER_VARIABLE`] gave it. The default binary's helper (cargo's name
/// beside it, however it was found) and any helper found beside the default
/// binary are this build's.
pub fn freshness(helper: &Path, bin: &Path, default_bin: &Path, named: bool) -> Freshness {
    if helper == default_bin.with_file_name(helper_name()) || (!named && bin == default_bin) {
        Freshness::Sources
    } else if named {
        Freshness::Chosen
    } else {
        Freshness::SameRelease
    }
}

/// Whether a binary's `--version` (`brindle X.Y.Z`) and its helper's
/// (`X.Y.Z`) name one release; `Err` says what each printed.
pub fn same_release(bin_said: &str, helper_said: &str) -> std::result::Result<(), String> {
    let (bin_said, helper_said) = (bin_said.trim(), helper_said.trim());
    match bin_said.strip_prefix("brindle ") {
        Some(version) if !version.is_empty() && version == helper_said => Ok(()),
        _ => Err(format!("the binary says {bin_said:?} and the helper {helper_said:?}")),
    }
}

/// The helper must be there, and then hold to its [`freshness`] rule. The
/// suite runs `bin`; `default_bin` is its own build of Brindle (the e2e
/// suite's `CARGO_BIN_EXE_brindle`, perf's `target/release/brindle`).
pub fn check_helper(helper: &Path, bin: &Path, default_bin: &Path) -> Result {
    let build = if default_bin.parent().and_then(Path::file_name).is_some_and(|name| name == "release") {
        "cargo build --release"
    } else {
        "cargo build"
    };
    let rule = freshness(helper, bin, default_bin, named_helper().is_some());
    let Ok(built) = std::fs::metadata(helper).and_then(|m| m.modified()) else {
        return Err(match rule {
            Freshness::Sources => format!(
                "the terminal server helper isn't built ({} is missing): run `{build}`, which builds it, \
                 or set {HELPER_VARIABLE}",
                helper.display()
            ),
            Freshness::SameRelease => format!(
                "no terminal server helper beside {} (Brindle looks for {}), or set {HELPER_VARIABLE}",
                bin.display(),
                server_helper_names().join(", ")
            ),
            Freshness::Chosen => format!("{HELPER_VARIABLE} names {}, which is missing", helper.display()),
        });
    };
    match rule {
        Freshness::Sources => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let sources = ["remote-agent/src", "remote-protocol/src", "term-core/src", "transcript/src"];
            let newer = sources
                .iter()
                .flat_map(|dir| files(&root.join(dir)))
                .find(|file| std::fs::metadata(file).and_then(|m| m.modified()).is_ok_and(|time| time > built));
            match newer {
                Some(file) => Err(format!(
                    "{} is older than {}: run `{build}` (the test target doesn't rebuild it)",
                    helper.display(),
                    file.display()
                )),
                None => Ok(()),
            }
        }
        Freshness::SameRelease => same_release(&version_of(bin)?, &version_of(helper)?).map_err(|why| {
            format!(
                "{} isn't the release of {} beside it ({why}): a release's archive carries both, \
                 or set {HELPER_VARIABLE}",
                helper.display(),
                bin.display()
            )
        }),
        Freshness::Chosen => Ok(()),
    }
}

/// What `binary --version` prints, or why it couldn't say.
fn version_of(binary: &Path) -> Result<String> {
    let output = Command::new(binary)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("can't run {} --version: {error}", binary.display()))?;
    ensure!(output.status.success(), "{} --version exited with {}", binary.display(), output.status);
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Every file under `dir`.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .flat_map(|entry| {
            let path = entry.path();
            if path.is_dir() { files(&path) } else { vec![path] }
        })
        .collect()
}

/// Stops the case's terminal server in `server_dir`, if one started, with
/// `helper`, and removes the directory, keeping its logs in `case_dir`.
/// Fails when a core or holder of the case's ran in a systemd scope, or is
/// still running afterwards, killing it.
pub fn stop(helper: &Path, server_dir: &Path, case_dir: &Path) -> Result {
    if !server_dir.exists() {
        return Ok(());
    }
    // A role in a scope of the user's own manager means a launch missed
    // `isolate`: on a machine with a user manager it would outlive the case.
    let mut problem = if cfg!(target_os = "linux") { scoped_roles(server_dir) } else { None };
    let stopped = Command::new(helper)
        .arg("stop")
        .env("BRINDLE_SERVER_DIR", server_dir)
        .stdin(std::process::Stdio::null())
        .output();
    if let Err(error) = stopped {
        problem.get_or_insert(format!("can't stop the server with {}: {error}", helper.display()));
    }
    // Holders hang their sessions up first (SIGHUP, then SIGKILL after 1 s).
    // A process list that can't be read fails the case: roles left running
    // would go unnoticed.
    match roles_left(server_dir, Duration::from_secs(5)) {
        Ok(left) if left.is_empty() => {}
        Ok(left) => {
            let names: Vec<String> = left.iter().map(|(pid, role)| format!("{role} {pid}")).collect();
            problem.get_or_insert(format!("left running after `brindle-remote stop`: {}", names.join(", ")));
            if let Err(error) = end_roles(server_dir) {
                problem.get_or_insert(error);
            }
        }
        Err(error) => {
            problem.get_or_insert(format!("can't tell whether roles are left running: {error}"));
        }
    }
    save_logs(server_dir, case_dir);
    // Windows keeps a role's image (its copy under the directory's `bin`)
    // from being deleted for a moment after the role is gone.
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Err(error) = std::fs::remove_dir_all(server_dir) {
        if !cfg!(windows) {
            break;
        }
        if Instant::now() > deadline {
            problem.get_or_insert(format!("can't remove {}: {error}", server_dir.display()));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    problem.map_or(Ok(()), Err)
}

/// The roles using `server_dir` that run in one of the server's own
/// systemd scopes, as a problem to report; `None` when there are none.
fn scoped_roles(server_dir: &Path) -> Option<String> {
    let roles = match roles(server_dir) {
        Ok(roles) => roles,
        Err(error) => return Some(format!("can't list the roles to check their scopes: {error}")),
    };
    let scoped: Vec<String> = roles
        .into_iter()
        .filter_map(|(pid, role)| {
            let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
            scoped_unit(&cgroup).map(|unit| format!("{role} {pid} in {unit}"))
        })
        .collect();
    (!scoped.is_empty()).then(|| format!("ran in a systemd scope: {}", scoped.join(", ")))
}

/// Copies the core's log to `server-core.log` in the case's directory, and
/// on Windows each holder's to `server-holder-….log`: a holder's says which
/// console host its sessions got.
fn save_logs(server_dir: &Path, case_dir: &Path) {
    std::fs::copy(server_dir.join("core.log"), case_dir.join("server-core.log")).ok();
    if cfg!(windows) {
        for log in files(server_dir) {
            let name = log.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if log.parent() == Some(server_dir) && name.starts_with("holder-") && name.ends_with(".log") {
                std::fs::copy(&log, case_dir.join(format!("server-{name}"))).ok();
            }
        }
    }
}

/// The roles still using `dir` once `limit` has passed; none as soon as
/// they are all gone. Fails when the processes can't be listed.
fn roles_left(dir: &Path, limit: Duration) -> std::io::Result<Vec<(u32, String)>> {
    let deadline = Instant::now() + limit;
    let mut left = roles(dir)?;
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        left = roles(dir)?;
    }
    Ok(left)
}

/// Kills the roles using `dir`, and on Windows waits for them to be gone.
pub fn end_roles(dir: &Path) -> Result {
    let running = roles(dir).map_err(|e| format!("can't list the roles using {}: {e}", dir.display()))?;
    for (pid, _) in running {
        #[cfg(unix)]
        Command::new("kill").args(["-KILL", &pid.to_string()]).status().ok();
        #[cfg(windows)]
        super::winproc::kill(pid);
    }
    if cfg!(windows) {
        roles_left(dir, Duration::from_secs(5)).map_err(|e| format!("can't list the roles using {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// The roles using `dir`, for a case that polls for one: the list is empty
/// when the processes can't be read, which the case's own wait then reports.
/// The harness's checks use [`roles`] and fail on that error.
pub fn server_processes(dir: &Path) -> Vec<(u32, String)> {
    roles(dir).unwrap_or_default()
}

/// The `serve` and `hold` processes using `dir` (their environment names
/// it), with their roles, or why the process list couldn't be read.
#[cfg(unix)]
pub fn roles(dir: &Path) -> std::io::Result<Vec<(u32, String)>> {
    let wanted = format!("BRINDLE_SERVER_DIR={}", dir.display());
    let entries = std::fs::read_dir("/proc")?;
    Ok(entries
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
        .collect())
}

/// The `serve` and `hold` processes using `dir`, with their roles: on
/// Windows a role runs from its build's copy of the helper under the
/// directory's `bin`. A copy running as anything else (the `launch` that
/// starts a role, a role whose arguments can't be read) is listed under the
/// helper's name, so nothing from the directory goes unnoticed.
#[cfg(windows)]
pub fn roles(dir: &Path) -> std::io::Result<Vec<(u32, String)>> {
    use super::winproc;
    let copies = dir.join("bin");
    let name = helper_name();
    Ok(winproc::all()?
        .into_iter()
        .filter(|process| process.name.eq_ignore_ascii_case(&name))
        .filter(|process| winproc::image(process.pid).is_some_and(|image| under(&image, &copies)))
        .map(|process| {
            let line = winproc::command_line(process.pid);
            let role = line.as_deref().and_then(winproc::first_argument).filter(|role| ["serve", "hold"].contains(role));
            (process.pid, role.map_or_else(|| name.clone(), str::to_string))
        })
        .collect())
}

/// Whether the Windows path `path` is under `dir`, however each is spelled.
#[cfg(windows)]
pub fn under(path: &Path, dir: &Path) -> bool {
    let spelled = |path: &Path| {
        let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
        text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text)
    };
    spelled(path).strip_prefix(&spelled(dir)).is_some_and(|rest| rest.starts_with('\\'))
}

#[cfg(windows)]
pub fn self_test_under() -> Result {
    let under = |path: &str, dir: &str| under(Path::new(path), Path::new(dir));
    ensure!(under(r"C:\Case\server\bin\b1\brindle-remote.exe", r"c:\case/server\bin"), "another spelling");
    ensure!(under(r"\\?\C:\case\server\bin\b1\x.exe", r"C:\case\server\bin"), "a verbatim path");
    ensure!(!under(r"C:\case\server\binary\x.exe", r"C:\case\server\bin"), "a sibling with the same prefix");
    ensure!(!under(r"C:\case\server\bin", r"C:\case\server\bin"), "the directory itself");
    Ok(())
}

/// The pure helpers' tests, run by both suites' `--self-test`.
pub fn self_test() -> Result {
    let case = Path::new("cases").join("one");
    let dirs = [
        private_dir(Suite::E2e, &case, 0),
        private_dir(Suite::E2e, &case, 1),
        private_dir(Suite::Perf, &case, 0),
        private_dir(Suite::Perf, &case, 1),
    ];
    if cfg!(windows) {
        for dir in &dirs {
            ensure!(dir.starts_with(&case), "{} isn't under the case directory", dir.display());
        }
    } else {
        for (i, dir) in dirs.iter().enumerate() {
            ensure!(dir.starts_with(std::env::temp_dir()), "{} isn't under the temp directory", dir.display());
            ensure!(!dirs[..i].contains(dir), "{} given twice", dir.display());
        }
    }

    let unit = |cgroup: &str| scoped_unit(cgroup).map(str::to_string);
    let serve = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/brindle-serve-1a2b.scope";
    ensure!(unit(serve).as_deref() == Some("brindle-serve-1a2b.scope"), "a serve scope: {:?}", unit(serve));
    let hold = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/brindle-hold-3c4d.scope\n";
    ensure!(unit(hold).as_deref() == Some("brindle-hold-3c4d.scope"), "a hold scope: {:?}", unit(hold));
    for other in [
        "0::/user.slice/user-1000.slice/session-3.scope",
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-brindle-1234.scope",
        "",
    ] {
        ensure!(unit(other).is_none(), "{other:?} taken for a role's scope: {:?}", unit(other));
    }
    self_test_helpers()
}

/// [`beside`], [`freshness`] and [`same_release`].
fn self_test_helpers() -> Result {
    let opt = Path::new("opt");
    let (bin, link) = (opt.join("brindle"), Path::new("links").join("brindle"));
    let (archive, cargo) = (opt.join(LINUX_HELPER_NAMES[0]), opt.join(LINUX_HELPER_NAMES[1]));
    let find = |bins: &[PathBuf], files: &[&PathBuf]| {
        beside(bins, &LINUX_HELPER_NAMES, &|path| files.iter().any(|file| file.as_path() == path))
    };
    ensure!(find(std::slice::from_ref(&bin), &[]).is_none(), "a helper found where there is none");
    ensure!(find(std::slice::from_ref(&bin), &[&cargo]) == Some(cargo.clone()), "cargo's name alone");
    ensure!(find(std::slice::from_ref(&bin), &[&cargo, &archive]) == Some(archive.clone()), "the archive's name comes first");
    // Beside the file a link leads to, after the link's own directory.
    ensure!(find(&[link.clone(), bin.clone()], &[&archive]) == Some(archive.clone()), "beside the resolved binary");
    ensure!(find(&[link.clone(), bin.clone()], &[&link.with_file_name("brindle-remote"), &archive]) == Some(link.with_file_name("brindle-remote")), "the link's directory first");

    let target = Path::new("target").join("debug");
    let default_bin = target.join("brindle");
    let own = default_bin.with_file_name(helper_name());
    let rule = |helper: &Path, bin: &Path, named| freshness(helper, bin, &default_bin, named);
    ensure!(rule(&own, &default_bin, false) == Freshness::Sources, "this build's helper");
    ensure!(rule(&own, &bin, true) == Freshness::Sources, "this build's helper, named for another binary");
    ensure!(rule(&target.join(LINUX_HELPER_NAMES[0]), &default_bin, false) == Freshness::Sources, "another name beside this build");
    ensure!(rule(&archive, &bin, false) == Freshness::SameRelease, "an installed release's helper");
    ensure!(rule(&opt.join(helper_name()), &bin, false) == Freshness::SameRelease, "cargo's name beside an installed binary");
    ensure!(rule(&archive, &bin, true) == Freshness::Chosen, "a helper the variable names");
    ensure!(rule(&archive, &default_bin, true) == Freshness::Chosen, "another helper named for this build");

    ensure!(same_release("brindle 0.4.0\n", "0.4.0\r\n").is_ok(), "one release");
    for (bin_said, helper_said) in
        [("brindle 0.4.0", "0.3.0"), ("brindle 0.4.0", "brindle 0.4.0"), ("0.4.0", "0.4.0"), ("brindle ", ""), ("", "")]
    {
        ensure!(same_release(bin_said, helper_said).is_err(), "{bin_said:?} and {helper_said:?} taken for one release");
    }
    Ok(())
}
