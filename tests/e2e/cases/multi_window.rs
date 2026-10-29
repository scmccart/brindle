//! Several windows, in one Brindle or in two, stay independent: what is
//! done in one window doesn't happen in another, and a session is shown
//! by one window at most, whichever process opened it.

use super::server;
use crate::brindle::{Output, Run};
use crate::tmux::{ControlClient, TmuxServer};
use crate::{Ctx, Outcome, Result, ensure};

const LIST_CLIENTS: &[&str] = &["list-clients", "-t", "e2e"];

/// The lines a `record_at` command of the run saved.
fn recorded_lines(ctx: &Ctx, label: &str, name: &str) -> Result<Vec<String>> {
    Ok(Run::recorded(ctx, label, name)?.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect())
}

/// The palette's row labels, without details and shortcuts.
fn rows(out: &Output) -> Vec<String> {
    out.dump.palette.iter().skip(1).map(|r| r.split("  ").next().unwrap_or(r).to_string()).collect()
}

/// Whether the dump shows the palette asking about another Brindle's
/// session: it starts with "New tmux session…", with no "Go to" row.
fn asks(out: &Output) -> bool {
    rows(out).first().is_some_and(|r| r == "New tmux session…")
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
    let second =
        Run::new(ctx, &server, "second").record_at(3.0, "clients", LIST_CLIENTS).dump_at(3.5).without_tabs();
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    let clients = recorded_lines(ctx, "second", "clients")?;
    ensure!(clients.len() == 1, "clients while both ran: {clients:?}");
    ensure!(first.dump.tabs.len() == 1 && first.dump.tabs[0].1, "first tabs: {:?}", first.dump.tabs);
    ensure!(second.dump.tabs.is_empty(), "second tabs: {:?}", second.dump.tabs);
    ensure!(asks(&second), "second palette: {:?}", second.dump.palette);
    ensure!(rows(&second) == ["New tmux session…", "New tab: Shell"], "second palette: {:?}", second.dump.palette);
    Ok(Outcome::Pass)
}

/// What a second Brindle launched with the session's profile does (a new
/// tmux window, a split) doesn't show up in the first one's window.
pub fn second_process_independent(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(9.0);
    let second = Run::new(ctx, &server, "second")
        .action("tmux_new_window")
        .action("tmux_split_right")
        .dump_at(4.0)
        .without_tabs();
    let (first, _) = two_processes(ctx, 3.0, first, second)?;
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

/// Escape in the second Brindle's window, which has no tabs, closes it, and
/// the second Brindle quits.
pub fn second_process_dismiss(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(8.0);
    let second = Run::new(ctx, &server, "second").send("").send("").action("picker_cancel").dump_at(4.5).expect_exit();
    let (first, _) = two_processes(ctx, 3.0, first, second)?;
    ensure!(first.dump.tabs.len() == 1 && first.dump.tabs[0].1, "first tabs: {:?}", first.dump.tabs);
    Ok(Outcome::Pass)
}

/// "New tmux session…" in the second Brindle's tab-less window creates the
/// session and shows it there.
pub fn second_process_new_session(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(10.0);
    let second = Run::new(ctx, &server, "second")
        .send("")
        .action("picker_confirm")
        .send("scratch")
        .action("picker_confirm")
        .record_at(5.5, "clients", LIST_CLIENTS)
        .dump_at(6.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    server.run(&["has-session", "-t", "=scratch"]).map_err(|e| format!("no session scratch: {e}"))?;
    ensure!(second.dump.palette.is_empty(), "second palette: {:?}", second.dump.palette);
    ensure!(second.dump.tabs.len() == 1, "second tabs: {:?}", second.dump.tabs);
    let clients = recorded_lines(ctx, "second", "clients")?;
    ensure!(clients.len() == 1, "clients on e2e while both ran: {clients:?}");
    ensure!(first.dump.tabs.len() == 1, "first tabs: {:?}", first.dump.tabs);
    Ok(Outcome::Pass)
}

/// The session's ctrl-alt-N in a local tab of a second Brindle asks there;
/// the local tab stays and nothing attaches.
pub fn second_process_shortcut(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(9.0);
    let second = Run::new(ctx, &server, "second")
        .profile("Shell")
        .action("new_tab_with_profile_2")
        .record_at(3.5, "clients", LIST_CLIENTS)
        .dump_at(4.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    ensure!(second.dump.tabs.len() == 1, "second tabs: {:?}", second.dump.tabs);
    ensure!(asks(&second), "second palette: {:?}", second.dump.palette);
    let clients = recorded_lines(ctx, "second", "clients")?;
    ensure!(clients.len() == 1, "clients while both ran: {clients:?}");
    ensure!(first.dump.tabs.len() == 1 && first.dump.tabs[0].1, "first tabs: {:?}", first.dump.tabs);
    Ok(Outcome::Pass)
}

/// ctrl-shift-n in a second Brindle opens the window, which asks: it has
/// no tabs.
pub fn second_process_new_window(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let first = Run::new(ctx, &server, "first").dump_at(9.0);
    let second = Run::new(ctx, &server, "second")
        .profile("Shell")
        .action("new_window")
        .record_at(3.5, "clients", LIST_CLIENTS)
        .dump_at(4.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    ensure!(second.dump.tabs.len() == 1, "second tabs: {:?}", second.dump.tabs);
    ensure!(second.dump.windows == [(2, Vec::new())], "second's windows: {:?}", second.dump.windows);
    let clients = recorded_lines(ctx, "second", "clients")?;
    ensure!(clients.len() == 1, "clients while both ran: {clients:?}");
    ensure!(first.dump.tabs.len() == 1, "first tabs: {:?}", first.dump.tabs);
    Ok(Outcome::Pass)
}

/// A control client that isn't Brindle's (`tmux -C` from a shell) doesn't
/// count: Brindle attaches beside it.
pub fn other_client(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let _client = ControlClient::attach(&server, 113, 33)?;
    let out = Run::new(ctx, &server, "run").record_at(3.5, "clients", LIST_CLIENTS).dump_at(4.0).run()?;
    ensure!(out.dump.palette.is_empty(), "palette: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    let clients = recorded_lines(ctx, "run", "clients")?;
    ensure!(clients.len() == 2, "clients: {clients:?}");
    Ok(Outcome::Pass)
}

/// In a second Brindle, after its own session `work` ended, "New tab: tmux"
/// (session `e2e`, which the first shows) asks; choosing "New tab: Shell"
/// leaves one live `Shell` tab, and the ended tab goes.
pub fn second_process_session_ended(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["new-session", "-d", "-s", "work"])?;
    let work = format!(
        "\n[[profiles]]\nname = \"work\"\ntmux = \"control\"\ntmux_session = \"work\"\n\
         tmux_args = [\"-L\", \"{}\", \"-f\", \"/dev/null\"]\n",
        server.socket
    );
    let top = "tmux_session_end = \"ask\"";
    let first = Run::new(ctx, &server, "first").top(top).config(&work).dump_at(14.0);
    let second = Run::new(ctx, &server, "second")
        .top(top)
        .config(&work)
        .profile("work")
        .at(2.0, &["kill-session", "-t", "work"])
        .send("")
        .send("")
        // "Close window", "New tab: Shell", "New tab: tmux"
        .action("picker_down")
        .action("picker_down")
        .action("picker_confirm")
        .send("")
        .send("")
        // "New tmux session…", "New tab: Shell"
        .action("picker_down")
        .action("picker_confirm")
        .send("echo $((6*7))-local\r")
        .record_at(7.5, "clients", LIST_CLIENTS)
        .dump_at(12.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    let clients = recorded_lines(ctx, "second", "clients")?;
    ensure!(clients.len() == 1, "clients while both ran: {clients:?}");
    let pane = server.run(&["capture-pane", "-p", "-t", "e2e"])?;
    ensure!(!pane.contains("42-local"), "typed into the session: {pane:?}");
    ensure!(second.dump.palette.is_empty(), "second palette: {:?}", second.dump.palette);
    ensure!(second.dump.tabs.len() == 1, "second tabs: {:?}", second.dump.tabs);
    ensure!(second.dump.screen.iter().any(|l| l.trim() == "42-local"), "second screen: {:?}", second.dump.screen);
    ensure!(first.dump.tabs.len() == 1, "first tabs: {:?}", first.dump.tabs);
    Ok(Outcome::Pass)
}
