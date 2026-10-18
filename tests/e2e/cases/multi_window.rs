//! Several windows, in one Brindle or in two, stay independent: what is
//! done in one window doesn't happen in another, and a session is shown
//! by one window at most, whichever process opened it.

use super::server;
use crate::brindle::{Output, Run};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

const LIST_CLIENTS: &[&str] = &["list-clients", "-t", "e2e"];

/// The lines a `record_at` command of the run saved.
fn recorded_lines(ctx: &Ctx, label: &str, name: &str) -> Result<Vec<String>> {
    Ok(Run::recorded(ctx, label, name)?.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect())
}

fn tmux_windows(server: &TmuxServer) -> Result<usize> {
    Ok(server.run(&["list-windows", "-t", "e2e"])?.lines().count())
}

/// Runs `first`, then `second` once `first` has had `gap` seconds to
/// attach, as two Brindle processes. Both runs must use the same config:
/// they share the case's `config.toml`.
fn two_processes<'a>(ctx: &Ctx, gap: f64, first: Run<'a>, second: Run<'a>) -> Result<(Output, Output)> {
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| first.run());
        ctx.sleep(gap);
        let second = second.run();
        (first.join().unwrap_or_else(|_| Err("first run panicked".into())), second)
    });
    Ok((first?, second?))
}

/// Window 1 is a shell, window 2 (ctrl-shift-n) the session. Tabs opened
/// and closed in window 1 stay there, and the session gets no window.
pub fn local_tabs(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .action("new_window")
        .send("")
        .action("new_tab_with_profile_1")
        .action("new_tab_with_profile_1")
        .action("close_tab")
        .send("echo one\r")
        .record_at(7.0, "clients", LIST_CLIENTS)
        .dump_at(7.5)
        .run()?;
    ensure!(out.dump.tabs.len() == 2, "window 1 tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows.len() == 1 && out.dump.windows[0].1.len() == 1, "windows: {:?}", out.dump.windows);
    ensure!(tmux_windows(&server)? == 1, "tmux windows: {:?}", server.run(&["list-windows", "-t", "e2e"])?);
    let clients = recorded_lines(ctx, "run", "clients")?;
    ensure!(clients.len() == 1, "clients: {clients:?}");
    Ok(Outcome::Pass)
}

/// Window 1 shows the session, window 2 a shell (ctrl-shift-n, "New
/// window: Shell"). Splitting and adding tmux windows in window 1 leaves
/// window 2 with its one tab.
pub fn tmux_commands(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .send("")
        .action("new_window")
        .action("picker_down")
        .action("picker_down")
        .action("picker_confirm")
        .action("tmux_split_right")
        .action("tmux_new_window")
        .record_at(8.5, "clients", LIST_CLIENTS)
        .dump_at(9.0)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 2, "window 1 tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.windows.len() == 1 && out.dump.windows[0].1 == ["bash"], "windows: {:?}", out.dump.windows);
    let panes = server.run(&["list-panes", "-s", "-t", "e2e", "-F", "#{window_index}:#{pane_id}"])?;
    ensure!(panes.lines().count() == 3, "panes: {panes:?}");
    let clients = recorded_lines(ctx, "run", "clients")?;
    ensure!(clients.len() == 1, "clients: {clients:?}");
    Ok(Outcome::Pass)
}

/// A second Brindle launched with the session's profile while the first
/// shows it doesn't attach it again: tmux keeps one current window per
/// session, so a second client would mirror the first window.
pub fn second_process_one_client(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(8.0);
    let second = Run::new(ctx, &server, "second").record_at(3.0, "clients", LIST_CLIENTS).dump_at(3.5);
    let (first, _) = two_processes(ctx, 3.0, first, second)?;
    let clients = recorded_lines(ctx, "second", "clients")?;
    ensure!(clients.len() == 1, "clients while both ran: {clients:?}");
    ensure!(first.dump.tabs.len() == 1 && first.dump.tabs[0].1, "first tabs: {:?}", first.dump.tabs);
    Ok(Outcome::Pass)
}

/// What a second Brindle launched with the session's profile does (a new
/// tmux window, a split) doesn't show up in the first one's window.
pub fn second_process_independent(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(9.0);
    let second = Run::new(ctx, &server, "second").action("tmux_new_window").action("tmux_split_right").dump_at(4.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    ctx.save("second-tabs.txt", format!("{:?}", second.dump.tabs));
    ensure!(first.dump.tabs.len() == 1 && first.dump.tabs[0].1, "first tabs: {:?}", first.dump.tabs);
    ensure!(tmux_windows(&server)? == 1, "tmux windows: {:?}", server.run(&["list-windows", "-t", "e2e"])?);
    Ok(Outcome::Pass)
}

/// Two Brindles on local shells: tabs opened, typed into and closed in the
/// second leave the first as it was.
pub fn second_process_local(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").profile("Shell").dump_at(9.0);
    let second = Run::new(ctx, &server, "second")
        .profile("Shell")
        .action("new_tab_with_profile_1")
        .send("echo from-second\r")
        .action("new_tab_with_profile_1")
        .dump_at(4.5);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    ensure!(second.dump.tabs.len() == 3, "second tabs: {:?}", second.dump.tabs);
    ensure!(first.dump.tabs.len() == 1, "first tabs: {:?}", first.dump.tabs);
    ensure!(!first.dump.screen.concat().contains("from-second"), "first screen: {:?}", first.dump.screen);
    Ok(Outcome::Pass)
}
