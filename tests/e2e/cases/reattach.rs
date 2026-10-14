//! Pane restore: a program's alternate screen and terminal modes come back
//! when Brindle attaches to a pane it never saw them being set in, and
//! again when it reattaches.

use super::server;
use crate::brindle::{Run, matches_tmux};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

/// tmux's pane flags and the `TermMode` flag each one should restore.
const FLAGS: [(&str, &str); 7] = [
    ("alternate_on", "ALT_SCREEN"),
    ("keypad_cursor_flag", "APP_CURSOR"),
    ("keypad_flag", "APP_KEYPAD"),
    ("mouse_standard_flag", "MOUSE_REPORT_CLICK"),
    ("mouse_button_flag", "MOUSE_DRAG"),
    ("mouse_all_flag", "MOUSE_MOTION"),
    ("mouse_sgr_flag", "SGR_MOUSE"),
];

/// The `TermMode` names of the flags tmux reports as set for `pane`.
fn tmux_modes(server: &TmuxServer, pane: &str) -> Result<Vec<&'static str>> {
    let format: Vec<String> = FLAGS.iter().map(|(f, _)| format!("#{{{f}}}")).collect();
    let values = server.display(pane, &format.join(" "))?;
    let values: Vec<&str> = values.split(' ').collect();
    ensure!(values.len() == FLAGS.len(), "pane flags {values:?}");
    Ok(FLAGS.iter().zip(values).filter(|(_, v)| *v == "1").map(|((_, m), _)| *m).collect())
}

/// `TermMode(SHOW_CURSOR | APP_CURSOR)` → the restorable flags it holds.
fn dumped_modes(mode: &str) -> Vec<&str> {
    let inner = mode.trim_start_matches("TermMode(").trim_end_matches(')');
    inner.split(" | ").filter(|m| FLAGS.iter().any(|(_, name)| name == m)).collect()
}

/// Runs a program that sets modes in the only pane, then attaches twice.
/// Each time the restored modes must equal tmux's and `want`, and the
/// screen must match tmux's.
fn reattach(ctx: &Ctx, program: &str, want: &[&str]) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", &format!("clear; {program}; cat"))?;
    ctx.sleep(0.5);
    let mut tmux = tmux_modes(&server, "%0")?;
    tmux.sort();
    let mut want = want.to_vec();
    want.sort();
    ensure!(tmux == want, "tmux reports {tmux:?} for the program, want {want:?}");
    for label in ["attach", "reattach"] {
        let out = Run::new(ctx, &server, label).dump_at(3.0).run()?;
        let mut got = dumped_modes(&out.dump.mode);
        got.sort();
        ensure!(got == want, "{label}: Brindle restored {got:?} (mode {}), tmux has {want:?}", out.dump.mode);
        matches_tmux(&server, &out.dump, "%0").map_err(|e| format!("{label}: {e}"))?;
    }
    Ok(Outcome::Pass)
}

/// The alternate screen with application cursor and keypad modes, as an
/// editor leaves them.
pub fn alt_screen(ctx: &Ctx) -> Result<Outcome> {
    reattach(
        ctx,
        r"printf '\e[?1049h\e[?1h\e=on the alternate screen\n'",
        &["ALT_SCREEN", "APP_CURSOR", "APP_KEYPAD"],
    )
}

/// Button-drag tracking (1002) in SGR form (1006) comes back as exactly
/// that: not click-only and not any-motion (1003).
pub fn mouse_modes(ctx: &Ctx) -> Result<Outcome> {
    reattach(ctx, r"printf '\e[?1002h\e[?1006hbutton-drag mouse\n'", &["MOUSE_DRAG", "SGR_MOUSE"])
}
