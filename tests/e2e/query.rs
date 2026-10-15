//! Asking a pane's program side for the terminal's default colors (OSC
//! 10/11) or palette colors (OSC 4) and reading the replies back from the
//! screen.

use std::path::PathBuf;

use crate::{Ctx, Result, ensure};

const SCRIPT: &str = r#"#!/bin/bash
# Queries the default background (OSC 11) and foreground (OSC 10) and
# prints the replies as bg=... and fg=... lines.
q() { printf '\033]%s;?\a' "$1"; read -t1 -rsd $'\a' r; printf '%s=%s\n' "$2" "${r#*;}"; }
q 11 bg; q 10 fg
"#;

const PALETTE_SCRIPT: &str = r#"#!/bin/bash
# Queries each palette index given (OSC 4) and prints p<index>=<first reply>
# extra=<a second reply arriving within half a second>, colors only.
for i in "$@"; do
  printf '\033]4;%s;?\a' "$i"
  r=; read -t1 -rsd $'\a' r; e=; read -t0.5 -rsd $'\a' e
  printf 'p%s=%s extra=%s\n' "$i" "${r##*;}" "${e##*;}"
done
"#;

/// Writes the query script into the case directory; type its path into a pane.
pub fn script(ctx: &Ctx) -> Result<PathBuf> {
    write_script(ctx, "query.sh", SCRIPT)
}

/// Writes the palette query script; run it with the indexes to query.
pub fn palette_script(ctx: &Ctx) -> Result<PathBuf> {
    write_script(ctx, "palette.sh", PALETTE_SCRIPT)
}

fn write_script(ctx: &Ctx, name: &str, text: &str) -> Result<PathBuf> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("chmod").arg("+x").arg(&path).status().map_err(|e| e.to_string())?;
    ensure!(status.success(), "chmod {name}");
    Ok(path)
}

/// The last palette reply for `index` on a screen, as (first, extra); both
/// empty when there was none.
pub fn palette_reply(screen: &[String], index: u8) -> (String, String) {
    let key = format!("p{index}=");
    screen
        .iter()
        .rev()
        .find_map(|l| {
            let rest = l.trim().strip_prefix(&key)?;
            let (first, extra) = rest.split_once(" extra=").unwrap_or((rest, ""));
            Some((first.trim().to_string(), extra.trim().to_string()))
        })
        .unwrap_or_default()
}

/// Checks that `index` got exactly one reply, carrying `want`.
pub fn expect_palette(what: &str, screen: &[String], index: u8, want: &str) -> Result {
    let (first, extra) = palette_reply(screen, index);
    ensure!(first == want, "{what}: index {index} answered {first:?}, want {want}");
    ensure!(extra.is_empty(), "{what}: index {index} got a second reply {extra:?}");
    Ok(())
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

    let screen: Vec<String> = [
        "$ ./palette.sh 4 196",
        "p4=rgb:0000/0000/0000 extra=",
        "p4=rgb:2020/4040/8080 extra=",
        "p196=rgb:ffff/0000/0000 extra=rgb:ffff/0000/0000",
        "p1= extra=",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    ensure!(palette_reply(&screen, 4) == ("rgb:2020/4040/8080".into(), String::new()), "last reply wins");
    ensure!(expect_palette("one", &screen, 4, "rgb:2020/4040/8080").is_ok(), "single reply accepted");
    ensure!(expect_palette("two", &screen, 196, "rgb:ffff/0000/0000").is_err(), "second reply caught");
    ensure!(palette_reply(&screen, 1) == (String::new(), String::new()), "no reply");
    ensure!(palette_reply(&screen, 19) == (String::new(), String::new()), "p19 isn't p196");
    Ok(())
}
