//! What a window does when the tmux session it showed goes away
//! (`tmux_session_end`).

use crate::brindle::{Output, Run};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

fn server() -> Result<TmuxServer> {
    TmuxServer::new(100, 30)
}

fn setting(value: &str) -> String {
    format!("tmux_session_end = \"{value}\"")
}

/// Typed into the active tab once the session is gone: it shows up only in
/// a live local terminal.
const PROBE: &str = "echo $((6*7))-local\r";

/// One tab, a live local terminal that ran [`PROBE`].
fn expect_local_tab(out: &Output) -> Result {
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| l.trim() == "42-local"), "screen: {:?}", out.dump.screen);
    Ok(())
}

/// `exit` in the session's last shell closes the window (the default).
pub fn close(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    Run::new(ctx, &server, "run").send("exit\r").dump_at(6.0).expect_exit().run()?;
    Ok(Outcome::Pass)
}

/// Detaching from a window with only the session's tabs closes it, and the
/// session keeps running.
pub fn detach_close(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    Run::new(ctx, &server, "run")
        .top(&setting("close_window"))
        .action("tmux_detach")
        .dump_at(6.0)
        .expect_exit()
        .run()?;
    server.run(&["has-session", "-t", "e2e"]).map_err(|e| format!("after detaching: {e}"))?;
    Ok(Outcome::Pass)
}

/// With `new_tab`, a session killed elsewhere leaves a local tab (the
/// default profile is the control-mode one, so the first non-tmux profile).
pub fn new_tab(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .top(&setting("new_tab"))
        .at(2.0, &["kill-session", "-t", "e2e"])
        .send("")
        .send("")
        .send(PROBE)
        .dump_at(5.0)
        .run()?;
    expect_local_tab(&out)?;
    Ok(Outcome::Pass)
}

/// With `ask`, the ended tab stays behind a palette offering to close the
/// window or open each profile; confirming the first row closes the window.
pub fn ask(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "ask")
        .top(&setting("ask"))
        .at(2.0, &["kill-session", "-t", "e2e"])
        .dump_at(4.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "the ended tab should stay: {:?}", out.dump.tabs);
    let rows: Vec<&str> = out.dump.palette.iter().skip(1).map(|r| r.split("  ").next().unwrap_or(r)).collect();
    ensure!(rows == ["Close window", "New tab: Shell", "New tab: tmux"], "palette: {:?}", out.dump.palette);

    let server = self::server()?;
    Run::new(ctx, &server, "confirm")
        .top(&setting("ask"))
        .at(2.0, &["kill-session", "-t", "e2e"])
        .send("")
        .send("")
        .action("picker_confirm")
        .dump_at(6.0)
        .expect_exit()
        .run()?;
    Ok(Outcome::Pass)
}

/// Closing the session's only tab ends the session; with `new_tab` the
/// setting still applies (the tab isn't dropped early on `%window-close`).
pub fn close_tab(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .top(&setting("new_tab"))
        .action("close_tab")
        .send("")
        .send(PROBE)
        .dump_at(5.0)
        .run()?;
    expect_local_tab(&out)?;
    ensure!(server.run(&["has-session", "-t", "e2e"]).is_err(), "the session survived closing its last tab");
    Ok(Outcome::Pass)
}
