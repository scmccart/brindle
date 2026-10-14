//! Attaching keeps tmux's selection: the current and last window and each
//! window's active pane are left as they were, the tabs follow them, and
//! a window created in the background stays there.

use super::server;
use crate::brindle::{Run, matches_tmux};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

/// Windows w0..w2 (`%0`..`%2`), window 2 current and window 0 the last
/// window. Window 2 is split into `%2`, `%3` and `%4`, left to right, each
/// showing its name, with `%3` active.
fn setup(ctx: &Ctx) -> Result<TmuxServer> {
    let server = server()?;
    server.run(&["rename-window", "-t", "e2e:0", "w0"])?;
    server.run(&["new-window", "-t", "e2e:1", "-n", "w1"])?;
    server.run(&["new-window", "-t", "e2e:2", "-n", "w2"])?;
    server.run(&["set", "-g", "automatic-rename", "off"])?;
    server.run(&["split-window", "-h", "-t", "%2"])?;
    server.run(&["split-window", "-h", "-t", "%3"])?;
    for pane in ["%2", "%3", "%4"] {
        server.type_line(pane, &format!("clear; echo pane {pane}"))?;
    }
    server.run(&["select-pane", "-t", "%3"])?;
    server.run(&["select-window", "-t", "e2e:0"])?;
    server.run(&["select-window", "-t", "e2e:2"])?;
    ctx.sleep(0.5);
    Ok(server)
}

/// Each window's index, current flag and last flag.
fn windows(server: &TmuxServer) -> Result<String> {
    server.run(&["list-windows", "-t", "e2e", "-F", "#{window_index}:#{window_active}#{window_last_flag}"])
}

/// [`windows`], and each pane's active flag.
fn selection(server: &TmuxServer) -> Result<String> {
    let panes = server.run(&["list-panes", "-a", "-F", "#{pane_id}:#{pane_active}"])?;
    Ok(format!("{}{panes}", windows(server)?))
}

fn unchanged(server: &TmuxServer, before: &str) -> Result {
    let after = selection(server)?;
    ensure!(after == before, "tmux's selection changed:\n{before}->\n{after}");
    Ok(())
}

fn active_tab(out: &crate::brindle::Output) -> Option<&str> {
    out.dump.tabs.iter().find(|(_, active)| *active).map(|(title, _)| title.as_str())
}

/// Window 2's tab is active, and tmux still has window 2 current and
/// window 0 last.
pub fn current_window(ctx: &Ctx) -> Result<Outcome> {
    let server = setup(ctx)?;
    let before = selection(&server)?;
    let out = Run::new(ctx, &server, "run").dump_at(3.5).run()?;
    let titles: Vec<&str> = out.dump.tabs.iter().map(|(t, _)| t.as_str()).collect();
    ensure!(titles == ["w0", "w1", "w2"], "tabs {:?}", out.dump.tabs);
    ensure!(active_tab(&out) == Some("w2"), "active tab {:?}", out.dump.tabs);
    unchanged(&server, &before)?;
    Ok(Outcome::Pass)
}

/// Focus is in the second pane, the one tmux has active.
pub fn active_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = setup(ctx)?;
    let before = selection(&server)?;
    let out = Run::new(ctx, &server, "run").dump_at(3.5).run()?;
    matches_tmux(&server, &out.dump, "%3")?;
    unchanged(&server, &before)?;
    Ok(Outcome::Pass)
}

/// Splitting right after attaching splits the pane tmux had active.
pub fn split_active_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = setup(ctx)?;
    Run::new(ctx, &server, "run").action("tmux_split_right").dump_at(3.5).run()?;
    let panes = server.run(&["list-panes", "-t", "e2e:2", "-F", "#{pane_id}"])?;
    let panes: Vec<&str> = panes.lines().collect();
    ensure!(panes == ["%2", "%3", "%5", "%4"], "window 2's panes, left to right: {panes:?}");
    let [left, top, width, height] = server.geometry("%3")?;
    let [new_left, new_top, _, new_height] = server.geometry("%5")?;
    ensure!(
        new_left == left + width + 1 && new_top == top && new_height == height,
        "%5 at {new_left},{new_top} height {new_height} isn't beside %3 at {left},{top} {width}x{height}"
    );
    ensure!(server.active_pane()? == "%5", "the new pane isn't active: {}", server.active_pane()?);
    Ok(Outcome::Pass)
}

/// `new-window -d` from another client adds a background tab; a new
/// window from Brindle comes to the front.
pub fn background_window(ctx: &Ctx) -> Result<Outcome> {
    let server = setup(ctx)?;
    let before = windows(&server)?;
    let out = Run::new(ctx, &server, "background")
        .at(2.0, &["new-window", "-d", "-t", "e2e:3", "-n", "w3"])
        .dump_at(4.0)
        .run()?;
    let titles: Vec<&str> = out.dump.tabs.iter().map(|(t, _)| t.as_str()).collect();
    ensure!(titles == ["w0", "w1", "w2", "w3"], "tabs {:?}", out.dump.tabs);
    ensure!(active_tab(&out) == Some("w2"), "active tab {:?}", out.dump.tabs);
    let after = windows(&server)?;
    ensure!(after == format!("{before}3:00\n"), "tmux's windows changed:\n{before}->\n{after}");

    let out = Run::new(ctx, &server, "foreground").action("tmux_new_window").dump_at(3.5).run()?;
    ensure!(out.dump.tabs.len() == 5, "tabs {:?}", out.dump.tabs);
    let current = server.display("e2e", "#{window_index}")?;
    ensure!(current == "4", "tmux's current window is {current}, not the new window 4");
    ensure!(out.dump.tabs.last().is_some_and(|(_, active)| *active), "the new tab isn't active: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}
