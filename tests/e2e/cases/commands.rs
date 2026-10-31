//! Native tmux commands from control-mode tabs: break pane, the layouts,
//! rotate and swap, and detach beside a local tab. They're checked by what
//! tmux did; how the result is drawn is the `geometry-*` cases' job.
//!
//! Each command gets its own short run, so the checks don't depend on step
//! timing.

use super::server;
use crate::brindle::Run;
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

/// `%0`, `%1` and `%2` side by side.
fn three_panes(ctx: &Ctx) -> Result<TmuxServer> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "%0"])?;
    server.run(&["split-window", "-h", "-t", "%1"])?;
    ctx.sleep(0.3);
    Ok(server)
}

/// The window's pane ids in tmux's order (left to right for a row).
fn order(server: &TmuxServer) -> Result<Vec<String>> {
    Ok(server.run(&["list-panes", "-t", "e2e", "-F", "#{pane_id}"])?.lines().map(str::to_string).collect())
}

/// Breaking the active pane out gives a new active tab, and leaves the
/// other pane in the original window.
pub fn break_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "%0"])?;
    let out = Run::new(ctx, &server, "run").action("tmux_break_pane").dump_at(3.5).run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs {:?}", out.dump.tabs);
    let panes = server.run(&["list-panes", "-a", "-F", "#{window_index}:#{pane_id}"])?;
    ensure!(panes.lines().collect::<Vec<_>>() == ["0:%0", "1:%1"], "panes by window: {panes:?}");
    ensure!(server.display("e2e", "#{window_index}")? == "1", "tmux's current window isn't the new one");
    Ok(Outcome::Pass)
}

/// Each pane's left, top, width and height, and the window's size.
fn layout(server: &TmuxServer) -> Result<(Vec<[u16; 4]>, u16, u16)> {
    let panes = server.run(&["list-panes", "-t", "e2e", "-F", "#{pane_left} #{pane_top} #{pane_width} #{pane_height}"])?;
    let panes = panes
        .lines()
        .map(|l| {
            let v: Vec<u16> = l.split(' ').filter_map(|f| f.parse().ok()).collect();
            v.try_into().map_err(|_| format!("bad pane line {l:?}"))
        })
        .collect::<Result<Vec<[u16; 4]>>>()?;
    let size = server.display("e2e", "#{window_width} #{window_height}")?;
    let (w, h) = size.split_once(' ').ok_or(format!("window size {size:?}"))?;
    Ok((panes, w.parse().map_err(|_| "width")?, h.parse().map_err(|_| "height")?))
}

/// Runs one layout action, and returns the geometry tmux reports after it.
fn apply(ctx: &Ctx, server: &TmuxServer, action: &'static str) -> Result<(Vec<[u16; 4]>, u16, u16)> {
    Run::new(ctx, server, action).action(action).dump_at(3.0).run()?;
    let layout = layout(server)?;
    ctx.save(&format!("{action}.txt"), format!("{layout:?}\n"));
    Ok(layout)
}

pub fn layouts(ctx: &Ctx) -> Result<Outcome> {
    let server = three_panes(ctx)?;

    let (panes, _, height) = apply(ctx, &server, "tmux_layout_even_horizontal")?;
    ensure!(panes.iter().all(|p| p[1] == 0 && p[3] == height), "even-horizontal: {panes:?}");

    let (panes, width, _) = apply(ctx, &server, "tmux_layout_even_vertical")?;
    ensure!(panes.iter().all(|p| p[0] == 0 && p[2] == width), "even-vertical: {panes:?}");

    let (panes, _, height) = apply(ctx, &server, "tmux_layout_main_vertical")?;
    let main = panes[0];
    ensure!(main[0] == 0 && main[1] == 0 && main[3] == height, "main-vertical main pane: {panes:?}");
    ensure!(panes[1..].iter().all(|p| p[0] > main[2]), "main-vertical: {panes:?}");

    let (tiled, _, _) = apply(ctx, &server, "tmux_layout_tiled")?;
    let distinct = |field: usize| {
        let mut v: Vec<u16> = tiled.iter().map(|p| p[field]).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    ensure!(distinct(0) == 2 && distinct(1) == 2, "tiled isn't two rows of two columns: {tiled:?}");

    let (next, _, _) = apply(ctx, &server, "tmux_next_layout")?;
    ensure!(next != tiled, "next layout left the tiled layout: {next:?}");
    Ok(Outcome::Pass)
}

/// Rotating moves every pane one place; swapping moves the active pane one
/// place and keeps it active.
pub fn rotate_swap(ctx: &Ctx) -> Result<Outcome> {
    let server = three_panes(ctx)?;
    let before = order(&server)?;
    Run::new(ctx, &server, "rotate").action("tmux_rotate_panes").dump_at(3.0).run()?;
    let after = order(&server)?;
    let rotated = |by: usize| {
        let mut v = before.clone();
        v.rotate_left(by);
        v
    };
    ensure!(after == rotated(1) || after == rotated(2), "rotate: {before:?} -> {after:?}");

    // The middle pane, so both directions have a neighbour.
    let middle = after[1].clone();
    server.run(&["select-pane", "-t", &middle])?;
    let position = |server: &TmuxServer| -> Result<usize> {
        let order = order(server)?;
        order.iter().position(|p| *p == middle).ok_or(format!("{middle} is gone: {order:?}"))
    };
    for (action, want) in [("tmux_swap_pane_next", 2), ("tmux_swap_pane_prev", 1)] {
        Run::new(ctx, &server, action).action(action).dump_at(3.0).run()?;
        let at = position(&server)?;
        ensure!(at == want, "{action}: {middle} is at {at}, want {want} ({:?})", order(&server)?);
        let active = server.active_pane()?;
        ensure!(active == middle, "{action}: the active pane is {active}, not {middle} that moved");
    }
    Ok(Outcome::Pass)
}

/// Typed into the shell tab once detached: prints only in a live terminal.
const PROBE: &str = "echo $((6*7))-local\r";

/// A shell tab, then the session's two windows as tabs 2 and 3. Detaching
/// does nothing from the shell tab; from a tmux tab it closes both tmux
/// tabs, keeps the shell tab, and leaves the session running.
pub fn detach_beside_local(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["new-window", "-t", "e2e"])?;
    let attached = |label| Run::new(ctx, &server, label).profile("Shell").action("new_tab_with_profile_2").send("");

    let out = attached("local")
        .action("activate_tab_1")
        .action("tmux_detach")
        .record_at(6.0, "clients", &["list-clients", "-t", "e2e"])
        .dump_at(6.5)
        .run()?;
    ensure!(out.dump.tabs.len() == 3 && out.dump.tabs[0].1, "tabs after detach in the shell tab: {:?}", out.dump.tabs);
    let clients = Run::recorded(ctx, "local", "clients")?;
    ensure!(clients.lines().count() == 1, "clients after detach in the shell tab: {clients:?}");

    let out = attached("tmux").action("activate_tab_2").action("tmux_detach").send("").send(PROBE).dump_at(7.5).run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs after detach in a tmux tab: {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| l.trim() == "42-local"), "the shell tab isn't live: {:?}", out.dump.screen);
    server.run(&["has-session", "-t", "e2e"]).map_err(|e| format!("after detaching: {e}"))?;
    Ok(Outcome::Pass)
}

/// `rename_tab` trims the submitted text, overrides OSC title changes on a
/// local tab, clears when given only whitespace so automatic titles resume,
/// and on a tmux control-mode tab renames only Brindle's tab while leaving
/// the tmux window name untouched.
pub fn rename_tab(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let title = |out: &crate::brindle::Output| out.dump.tabs.first().map(|(t, _)| t.clone()).unwrap_or_default();

    let renamed = Run::new(ctx, &server, "local-rename")
        .profile("Shell")
        .action("rename_tab")
        .send("  server  ")
        .action("picker_confirm")
        .send("printf '\\033]2;osc-title\\007'\r")
        .dump_at(6.5)
        .run()?;
    ensure!(title(&renamed) == "server", "local renamed title: {:?}", renamed.dump.tabs);

    let cleared = Run::new(ctx, &server, "local-clear")
        .profile("Shell")
        .send("printf '\\033]2;auto-title\\007'\r")
        .action("rename_tab")
        .send("  server  ")
        .action("picker_confirm")
        .action("rename_tab")
        .send("   ")
        .action("picker_confirm")
        .dump_at(9.5)
        .run()?;
    ensure!(title(&cleared) == "auto-title", "local cleared title: {:?}", cleared.dump.tabs);

    server.run(&["rename-window", "-t", "e2e:0", "editor"])?;
    let tmux = Run::new(ctx, &server, "tmux-rename")
        .action("rename_tab")
        .send("  review  ")
        .action("picker_confirm")
        .dump_at(5.5)
        .run()?;
    ensure!(title(&tmux) == "review", "tmux tab title: {:?}", tmux.dump.tabs);
    let window_name = server.run(&["display-message", "-p", "-t", "e2e:0", "#{window_name}"])?;
    ensure!(window_name.trim() == "editor", "tmux window name changed to {window_name:?}");
    Ok(Outcome::Pass)
}
