//! One window per tmux session: attaching a session another window shows
//! asks first, new windows don't mirror it, "tmux: New Session" creates
//! sessions, and restore attaches each session once.

use super::server;
use crate::brindle::{Output, Run, state_dir};
use crate::{Ctx, Outcome, Result, ensure};

/// The palette's row labels, without details and shortcuts.
fn rows(out: &Output) -> Vec<String> {
    out.dump.palette.iter().skip(1).map(|r| r.split("  ").next().unwrap_or(r).to_string()).collect()
}

/// How many clients `list-clients` recorded for the session.
fn clients(ctx: &Ctx, label: &str) -> Result<usize> {
    Ok(Run::recorded(ctx, label, "clients")?.lines().filter(|l| !l.trim().is_empty()).count())
}

const LIST_CLIENTS: &[&str] = &["list-clients", "-t", "e2e"];

/// Window 1 is a shell; ctrl-shift-n opens window 2 on the session (the
/// default profile). Picking that profile again in window 1 asks; "Go to"
/// leaves window 1 as it was and attaches nothing.
pub fn open_elsewhere_goto(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let ask = |label| {
        Run::new(ctx, &server, label)
            .profile("Shell")
            .action("new_window")
            .send("")
            .action("new_tab_with_profile_2")
    };
    let out = ask("ask").record_at(5.0, "clients", LIST_CLIENTS).dump_at(5.5).run()?;
    let rows = rows(&out);
    ensure!(rows.first().is_some_and(|r| r == "Go to e2e"), "palette: {:?}", out.dump.palette);
    ensure!(rows.iter().any(|r| r == "New tmux session…"), "palette: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 1, "window 1 tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows.len() == 1, "windows: {:?}", out.dump.windows);
    ensure!(clients(ctx, "ask")? == 1, "clients: {:?}", Run::recorded(ctx, "ask", "clients")?);

    let out = ask("goto").action("picker_confirm").record_at(6.0, "clients", LIST_CLIENTS).dump_at(6.5).run()?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 1, "window 1 tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows.len() == 1 && out.dump.windows[0].1.len() == 1, "windows: {:?}", out.dump.windows);
    ensure!(clients(ctx, "goto")? == 1, "clients: {:?}", Run::recorded(ctx, "goto", "clients")?);
    Ok(Outcome::Pass)
}

/// Picking that profile in window 1's picker, whose row says it switches,
/// goes to window 2 without asking.
pub fn picker_goes_to_session(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .action("new_window")
        .send("")
        .action("open_profile_picker")
        .send("")
        .action("picker_down")
        .action("picker_confirm")
        .record_at(8.0, "clients", LIST_CLIENTS)
        .dump_at(8.5)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "a palette is open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 1, "window 1 tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows.len() == 1 && out.dump.windows[0].1.len() == 1, "windows: {:?}", out.dump.windows);
    ensure!(clients(ctx, "run")? == 1, "clients: {:?}", Run::recorded(ctx, "run", "clients")?);
    Ok(Outcome::Pass)
}

/// ctrl-shift-n with the default profile's session open asks instead of
/// mirroring it; choosing another profile opens that in the new window.
pub fn new_window_asks(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let ask = |label| Run::new(ctx, &server, label).send("").action("new_window");
    let out = ask("ask").dump_at(4.0).run()?;
    ensure!(rows(&out) == ["Go to e2e", "New tmux session…", "New window: Shell"], "palette: {:?}", out.dump.palette);
    ensure!(out.dump.windows.is_empty(), "a window opened: {:?}", out.dump.windows);

    let out = ask("shell")
        .action("picker_down")
        .action("picker_down")
        .action("picker_confirm")
        .record_at(6.5, "clients", LIST_CLIENTS)
        .dump_at(7.0)
        .run()?;
    ensure!(out.dump.windows.len() == 1 && out.dump.windows[0].1.len() == 1, "windows: {:?}", out.dump.windows);
    ensure!(out.dump.windows[0].1[0] == "bash", "window 2 tabs: {:?}", out.dump.windows);
    ensure!(clients(ctx, "shell")? == 1, "clients: {:?}", Run::recorded(ctx, "shell", "clients")?);
    Ok(Outcome::Pass)
}

/// Escape on that palette opens nothing.
pub fn open_elsewhere_dismiss(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .send("")
        .action("new_window")
        .action("picker_cancel")
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(out.dump.windows.is_empty(), "a window opened: {:?}", out.dump.windows);
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

/// "New tmux session…" from the ctrl-shift-n palette opens a new window on
/// a new session of the same server.
pub fn new_window_new_session(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .send("")
        .action("new_window")
        .action("picker_down")
        .action("picker_confirm")
        .send("scratch")
        .action("picker_confirm")
        .dump_at(9.0)
        .run()?;
    server.run(&["has-session", "-t", "=scratch"]).map_err(|e| format!("no session scratch: {e}"))?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(out.dump.windows.len() == 1 && out.dump.windows[0].1.len() == 1, "windows: {:?}", out.dump.windows);
    Ok(Outcome::Pass)
}

/// The profile picker says when a control-mode profile's session is open.
pub fn profile_picker_state(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run").send("").action("open_profile_picker").dump_at(4.0).run()?;
    let row = out.dump.palette.iter().find(|r| r.starts_with("tmux  "));
    ensure!(
        row.is_some_and(|r| r.contains("tmux control mode · e2e · open in this window")),
        "palette: {:?}",
        out.dump.palette
    );
    let shell = out.dump.palette.iter().find(|r| r.starts_with("Shell  "));
    ensure!(shell.is_some_and(|r| !r.contains("open in")), "palette: {:?}", out.dump.palette);
    Ok(Outcome::Pass)
}

/// "tmux: New Session" from a local tab creates the named session and
/// shows it in the current window.
pub fn new_session(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    // Listed in a local tab, since the config has a control-mode profile.
    let out = Run::new(ctx, &server, "listed")
        .profile("Shell")
        .action("open_command_palette")
        .send("new session")
        .dump_at(3.5)
        .run()?;
    ensure!(out.dump.palette.iter().any(|r| r.starts_with("tmux: New Session")), "palette: {:?}", out.dump.palette);

    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .action("tmux_new_session")
        .send("work")
        .action("picker_confirm")
        .dump_at(6.0)
        .run()?;
    server.run(&["has-session", "-t", "=work"]).map_err(|e| format!("no session work: {e}"))?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows.is_empty(), "a window opened: {:?}", out.dump.windows);
    Ok(Outcome::Pass)
}

/// A taken name keeps the prompt open with tmux's error, and no
/// attach-failure tab opens.
pub fn new_session_duplicate(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["new-session", "-d", "-s", "work"])?;
    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .action("tmux_new_session")
        .send("work")
        .action("picker_confirm")
        .dump_at(5.5)
        .run()?;
    let prompt = out.dump.palette.first().map(String::as_str).unwrap_or("");
    ensure!(
        prompt.starts_with("prompt ") && prompt.contains("error=Some(\"duplicate session: work\")"),
        "palette: {:?}",
        out.dump.palette
    );
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

/// A state file recording the session in two windows attaches it once:
/// the second window keeps only its local tab.
pub fn restore_dedupe(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let dir = state_dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmux = "[[windows.tabs]]\nkind = \"tmux\"\nprofile = \"tmux\"\nsession = \"e2e\"\n";
    let local = format!("[[windows.tabs]]\nkind = \"local\"\nprofile = \"Shell\"\ncwd = \"{}\"\n", ctx.dir.display());
    let window = |active: usize| format!("[[windows]]\nwidth = 1000.0\nheight = 660.0\nactive = {active}\n");
    let state = format!("version = 1\n\n{}{tmux}\n{}{tmux}{local}", window(0), window(1));
    std::fs::write(dir.join("session.toml"), state).map_err(|e| e.to_string())?;
    let out = Run::new(ctx, &server, "run").restore().record_at(4.5, "clients", LIST_CLIENTS).dump_at(5.0).run()?;
    ensure!(out.dump.tabs.len() == 1, "window 1 tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows == [(2, vec!["bash".to_string()])], "windows: {:?}", out.dump.windows);
    ensure!(clients(ctx, "run")? == 1, "clients: {:?}", Run::recorded(ctx, "run", "clients")?);
    Ok(Outcome::Pass)
}
