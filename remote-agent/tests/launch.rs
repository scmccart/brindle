//! `brindle-remote launch` and `--ready-handle` as the built binary takes
//! them. How a role is started detached, and what survives, is tested
//! beside the code (`server/platform/windows/daemon.rs`).

#![cfg(windows)]

mod support;

use std::process::{Command, Output};

use support::BIN;

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn launch_runs_its_own_binary_and_passes_its_output_on() {
    // The trampoline exits at once; the pipe ends when what it started has
    // exited too, so the version read here is that process's.
    let output = run(&["launch", "--version"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        env!("CARGO_PKG_VERSION")
    );
    // All it may say is that the job these tests run in keeps the process.
    for line in stderr(&output).lines() {
        assert!(line.contains("forbids breakaway"), "{line}");
    }
}

#[test]
fn launch_needs_a_role_and_a_ready_pipe_it_holds() {
    let output = run(&["launch"]);
    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(
        said.contains("usage: brindle-remote launch <role>"),
        "{said}"
    );

    // 3 names no handle.
    let output = run(&["launch", "hold", "--ready-handle", "3"]);
    assert!(!output.status.success());
    let said = stderr(&output);
    assert!(said.contains("inherited pipe"), "{said}");
}

#[test]
fn a_role_refuses_arguments_that_are_not_an_open_ready_pipe() {
    for role in ["serve", "hold"] {
        let output = run(&[role, "--ready-handle", "3"]);
        assert!(!output.status.success());
        let said = stderr(&output);
        assert!(said.contains("is not an open pipe handle"), "{said}");

        let output = run(&[role, "--ready-fd", "3"]);
        assert!(!output.status.success());
        let said = stderr(&output);
        let usage = format!("usage: brindle-remote {role} [--ready-handle N]");
        assert!(said.contains(&usage), "{said}");
    }
}

/// What only an elevated process makes: the protected directories of an
/// elevated server (fix-security-audit-2026-11-20). These run only from an
/// elevated test process, and skip otherwise.
mod elevated {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use brindle_remote_protocol::server_dir;
    use brindle_remote_protocol::win::{self, Contents};

    use super::support::*;

    /// Whether this test process is elevated, recording the skip when not.
    fn elevated() -> bool {
        if win::is_elevated().unwrap() {
            return true;
        }
        skip(Capability::Elevated, "the test process isn't elevated");
        false
    }

    /// Makes `link` a junction to `target`, which needs no privilege.
    fn junction(link: &Path, target: &Path) {
        let root = std::env::var("SystemRoot").unwrap();
        let status = Command::new(format!(r"{root}\System32\cmd.exe"))
            .arg("/c")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn set_aside(dir: &Path) -> usize {
        names(dir).iter().filter(|name| server_dir::is_untrusted_name(name)).count()
    }

    /// A junction, and a directory with the user's access, in the elevated
    /// directory's place are each set aside and replaced; while it is held,
    /// neither it nor its parent can be renamed.
    #[test]
    fn a_planted_junction_and_a_user_directory_are_each_replaced() {
        if !elevated() {
            return;
        }
        let dir = TempDir::new("elevated-protected");
        let target = dir.path.join("elsewhere");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("planted.exe"), b"x").unwrap();
        let parent = dir.path.join("brindle");
        let path = parent.join("server-elevated");
        fs::create_dir(&parent).unwrap();
        junction(&path, &target);

        let held = win::protected_dir(&path, Contents::Executables).unwrap();
        assert!(names(&path).is_empty());
        assert_eq!(names(&target), ["planted.exe"]);
        assert!(win::hold_protected_dir(&path).unwrap().is_some());
        assert!(fs::rename(&path, parent.join("moved")).is_err());
        assert!(fs::rename(&parent, dir.path.join("moved")).is_err());
        drop(held);
        assert_eq!(set_aside(&parent), 1);

        fs::remove_dir(&path).unwrap();
        win::create_private_dir(&path).unwrap();
        fs::write(path.join("brindle-remote.exe"), b"planted").unwrap();
        assert!(win::hold_protected_dir(&path).is_err());
        let held = win::protected_dir(&path, Contents::Executables).unwrap();
        assert!(names(&path).is_empty());
        assert_eq!(set_aside(&parent), 2);
        drop(held);
    }

    /// A `brindle-remote.exe` planted where an elevated Brindle's copy of
    /// its helper goes is set aside with its directory: the copy that is
    /// started is the installed helper's.
    #[test]
    fn a_planted_helper_in_the_elevated_copies_is_never_the_one_run() {
        if !elevated() {
            return;
        }
        let dir = TempDir::new("elevated-copies");
        // Where this build's copy goes, learnt from a copy made elsewhere.
        let scratch = dir.path.join("scratch");
        let build = win::ensure_helper_copy(Path::new(BIN), &scratch).unwrap();
        let build = build.parent().unwrap().file_name().unwrap().to_owned();
        let copies = server_dir::helper_copies(None, &dir.path, true);
        win::create_private_dir(copies.parent().unwrap()).unwrap();
        win::create_private_dir(&copies).unwrap();
        fs::create_dir(copies.join(&build)).unwrap();
        fs::write(copies.join(&build).join("brindle-remote.exe"), b"planted").unwrap();

        // What Brindle's client does before it starts a core.
        let held = win::protected_dir(&copies, Contents::Executables).unwrap();
        let copy = win::ensure_helper_copy(Path::new(BIN), &copies).unwrap();
        assert!(copy.starts_with(&copies), "{}", copy.display());
        assert_eq!(fs::read(&copy).unwrap(), fs::read(BIN).unwrap());
        assert_eq!(set_aside(copies.parent().unwrap()), 1);
        drop(held);
    }

    /// An elevated holder loads the packaged ConPTY from its own protected
    /// extraction, not from one an unelevated process wrote.
    #[test]
    fn an_elevated_holder_s_conpty_host_runs_from_the_protected_extraction() {
        if !elevated() {
            return;
        }
        // A build that embeds the packaged ConPTY has it beside the helper.
        if !Path::new(BIN).with_file_name("conpty.dll").exists() {
            skip(Capability::PackagedConpty, "no conpty.dll beside the helper");
            return;
        }
        let mut dir = TempDir::new("elevated-conpty");
        let local = TempDir::new("elevated-conpty-local");
        // A copy with no ConPTY beside it, so the holder extracts its own.
        let bin = TempDir::new("elevated-conpty-bin");
        let copy = bin.path.join("brindle-remote.exe");
        fs::copy(BIN, &copy).unwrap();
        let local_app_data = local.path.to_str().unwrap();
        start(
            &copy,
            "hold",
            &dir.path,
            &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version"), ("LOCALAPPDATA", local_app_data)],
        )
        .unwrap();
        let mut core = TestCore::connect_to(&dir);
        let holder_pid = core.hello(1).welcome().pid;
        dir.track(holder_pid);
        core.spawn(shell(1, "echo conpty-up"));
        core.output_until(1, b"conpty-up");

        let hosts = children_named(holder_pid, "OpenConsole.exe");
        assert_eq!(hosts.len(), 1, "{hosts:?}");
        let image = fs::canonicalize(win::image_path_of(hosts[0]).unwrap()).unwrap();
        let extraction = fs::canonicalize(local.path.join("brindle").join("conpty-elevated")).unwrap();
        assert!(image.starts_with(&extraction), "{}", image.display());
        assert!(win::hold_protected_dir(image.parent().unwrap()).unwrap().is_some());
        core.hang_up(1);
        core.hung_up(1);
    }
}
