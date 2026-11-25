//! `brindle-fake-acp-agent`: an ACP agent played from a script, for the
//! terminal server's tests (OpenSpec change `agents-sessions`, design
//! decision 16). It is built with the helper and never packaged.
//!
//! Its script's steps are [`script`]'s. For now it reads and checks the
//! script it is given and stops there: the player, a thin loop over the
//! steps, comes with that change's task 6.1.

// railway R3, types R2: no `_ =>` on one of Brindle's own enums. Non-test
// code only, as in the helper's root.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the player (agents-sessions task 6.1) uses what the checker doesn't"
    )
)]
mod script;

use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: brindle-fake-acp-agent SCRIPT");
        return ExitCode::from(2);
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "brindle-fake-acp-agent: can't read {}: {error}",
                path.display()
            );
            return ExitCode::from(2);
        }
    };
    match script::parse(&text) {
        Ok(steps) => {
            eprintln!(
                "brindle-fake-acp-agent: {} has {} steps; playing them comes with agents-sessions task 6.1",
                path.display(),
                steps.len()
            );
            ExitCode::from(2)
        }
        Err(bad) => {
            eprintln!("brindle-fake-acp-agent: {}: {bad}", path.display());
            ExitCode::from(2)
        }
    }
}
