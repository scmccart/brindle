//! One window shows each tmux session once: opening a profile whose
//! session it shows switches to its tab, "tmux: New Session" creates
//! sessions, and restoring an old state file with several windows attaches
//! each session once, in the one window.

use super::server;
use crate::brindle::{Run, state_dir};
use crate::{Ctx, Outcome, Result, ensure};

/// How many clients `list-clients` recorded for the session.
fn clients(ctx: &Ctx, label: &str) -> Result<usize> {
    Ok(Run::recorded(ctx, label, "clients")?.lines().filter(|l| !l.trim().is_empty()).count())
}

const LIST_CLIENTS: &[&str] = &["list-clients", "-t", "e2e"];

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

/// With a shell tab and the session's tab, ctrl-alt-2 (the session's
/// profile) from the shell, and then picking the profile in the picker,
/// switch to the session's tab: no second tab, no second client.
pub fn shown_switches(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let shortcut = Run::new(ctx, &server, "shortcut")
        .profile("Shell")
        .action("new_tab_with_profile_2")
        .send("")
        .action("prev_tab")
        .action("new_tab_with_profile_2")
        .record_at(6.0, "clients", LIST_CLIENTS)
        .dump_at(6.5)
        .run()?;
    ensure!(shortcut.dump.palette.is_empty(), "a palette is open: {:?}", shortcut.dump.palette);
    ensure!(shortcut.dump.tabs.len() == 2 && shortcut.dump.tabs[1].1, "tabs: {:?}", shortcut.dump.tabs);
    ensure!(clients(ctx, "shortcut")? == 1, "clients: {:?}", Run::recorded(ctx, "shortcut", "clients")?);

    let picker = Run::new(ctx, &server, "picker")
        .profile("Shell")
        .action("new_tab_with_profile_2")
        .send("")
        .action("prev_tab")
        .action("open_profile_picker")
        .send("")
        .action("picker_down")
        .action("picker_confirm")
        .record_at(8.0, "clients", LIST_CLIENTS)
        .dump_at(8.5)
        .run()?;
    ensure!(picker.dump.palette.is_empty(), "a palette is open: {:?}", picker.dump.palette);
    ensure!(picker.dump.tabs.len() == 2 && picker.dump.tabs[1].1, "tabs: {:?}", picker.dump.tabs);
    ensure!(clients(ctx, "picker")? == 1, "clients: {:?}", Run::recorded(ctx, "picker", "clients")?);
    Ok(Outcome::Pass)
}

/// A state file from before Brindle had one window, recording the session
/// in two windows, comes back as one window: the session once, then the
/// second window's local tab.
pub fn restore_merge(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let dir = state_dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmux = "[[windows.tabs]]\nkind = \"tmux\"\nprofile = \"tmux\"\nsession = \"e2e\"\n";
    let local = format!("[[windows.tabs]]\nkind = \"local\"\nprofile = \"Shell\"\ncwd = \"{}\"\n", ctx.dir.display());
    let window = |active: usize| format!("[[windows]]\nwidth = 1000.0\nheight = 660.0\nactive = {active}\n");
    let state = format!("version = 1\n\n{}{tmux}\n{}{tmux}{local}", window(0), window(1));
    std::fs::write(dir.join("session.toml"), state).map_err(|e| e.to_string())?;
    let out = Run::new(ctx, &server, "run").restore().record_at(4.5, "clients", LIST_CLIENTS).dump_at(5.0).run()?;
    let titles: Vec<&str> = out.dump.tabs.iter().map(|(t, _)| t.as_str()).collect();
    ensure!(titles.len() == 2 && titles[1] == "bash", "tabs: {:?}", out.dump.tabs);
    ensure!(clients(ctx, "run")? == 1, "clients: {:?}", Run::recorded(ctx, "run", "clients")?);
    Ok(Outcome::Pass)
}
