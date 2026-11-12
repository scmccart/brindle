//! `brindle-remote launch` and `--ready-handle` as the built binary takes
//! them. How a role is started detached, and what survives, is tested
//! beside the code (`server/platform/windows/daemon.rs`).

#![cfg(windows)]

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_brindle-remote");

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
