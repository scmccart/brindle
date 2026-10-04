//! Asking a pane's program side for the terminal's default colors (OSC
//! 10/11) and reading the replies back from the screen.

use std::path::PathBuf;

use crate::{Ctx, Result, ensure};

const SCRIPT: &str = r#"#!/bin/bash
# Queries the default background (OSC 11) and foreground (OSC 10) and
# prints the replies as bg=... and fg=... lines.
q() { printf '\033]%s;?\a' "$1"; read -t1 -rsd $'\a' r; printf '%s=%s\n' "$2" "${r#*;}"; }
q 11 bg; q 10 fg
"#;

/// Writes the query script into the case directory; type its path into a pane.
pub fn script(ctx: &Ctx) -> Result<PathBuf> {
    let path = ctx.dir.join("query.sh");
    std::fs::write(&path, SCRIPT).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("chmod").arg("+x").arg(&path).status().map_err(|e| e.to_string())?;
    ensure!(status.success(), "chmod query.sh");
    Ok(path)
}

/// The last `bg=` and `fg=` replies on a screen; empty when there was none.
pub fn replies(screen: &[String]) -> (String, String) {
    let last = |key: &str| {
        screen.iter().rev().find_map(|l| l.trim().strip_prefix(key).map(str::to_string)).unwrap_or_default()
    };
    (last("bg="), last("fg="))
}

pub fn self_test() -> Result {
    let screen: Vec<String> =
        ["$ ./query.sh", "bg=rgb:0000/0000/0000", "fg=", "$ ./query.sh", "bg=rgb:1010/1414/1818", "fg=rgb:c0c0/c0c0/c0c0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let (bg, fg) = replies(&screen);
    ensure!(bg == "rgb:1010/1414/1818" && fg == "rgb:c0c0/c0c0/c0c0", "{bg} {fg}");
    ensure!(replies(&[]) == (String::new(), String::new()), "empty screen");
    Ok(())
}
