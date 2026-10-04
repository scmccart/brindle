//! Pane geometry: Brindle's grid and text match tmux's pane, including
//! rows tmux keeps for border status lines.

use super::{require, server};
use crate::brindle::{Run, matches_tmux};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result};

fn seq(server: &TmuxServer, pane: &str, from: u32, to: u32) -> Result {
    server.type_line(pane, &format!("clear; seq {from} {to}"))
}

/// Two panes side by side, `seq` output in the right one (active).
fn split(ctx: &Ctx, border_status: &str) -> Result<TmuxServer> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    server.run(&["set", "-w", "-t", "e2e", "pane-border-status", border_status])?;
    seq(&server, "%1", 1, 60)?;
    ctx.sleep(0.5);
    Ok(server)
}

fn check_active(server: &TmuxServer, out: &crate::brindle::Output) -> Result<Outcome> {
    matches_tmux(server, &out.dump, &server.active_pane()?)?;
    Ok(Outcome::Pass)
}

pub fn plain(ctx: &Ctx) -> Result<Outcome> {
    let server = split(ctx, "off")?;
    let out = Run::new(ctx, &server, "run").dump_at(3.0).run()?;
    check_active(&server, &out)
}

pub fn top_before_attach(ctx: &Ctx) -> Result<Outcome> {
    let server = split(ctx, "top")?;
    let out = Run::new(ctx, &server, "run").dump_at(3.0).run()?;
    check_active(&server, &out)
}

pub fn top_while_attached(ctx: &Ctx) -> Result<Outcome> {
    let server = split(ctx, "off")?;
    let out = Run::new(ctx, &server, "run")
        .at(3.0, &["set", "-w", "-t", "e2e", "pane-border-status", "top"])
        .dump_at(6.0)
        .run()?;
    check_active(&server, &out)
}

pub fn fullscreen_less(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = require("less") {
        return Ok(skip);
    }
    let file = ctx.dir.join("long.txt");
    let text: String = (1..=500).map(|n| format!("line {n} of a long file\n")).collect();
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    let server = split(ctx, "off")?;
    server.type_line("%1", &format!("clear; less {}", file.display()))?;
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run")
        .at(3.0, &["set", "-w", "-t", "e2e", "pane-border-status", "top"])
        .dump_at(6.0)
        .run()?;
    check_active(&server, &out)
}

pub fn bottom(ctx: &Ctx) -> Result<Outcome> {
    let server = split(ctx, "bottom")?;
    let out = Run::new(ctx, &server, "run").dump_at(3.0).run()?;
    check_active(&server, &out)
}

pub fn zoom(ctx: &Ctx) -> Result<Outcome> {
    let server = split(ctx, "top")?;
    seq(&server, "%1", 1, 80)?;
    ctx.sleep(0.3);
    let out = Run::new(ctx, &server, "run").at(2.0, &["resize-pane", "-Z", "-t", "%1"]).dump_at(5.0).run()?;
    check_active(&server, &out)
}

/// Border status turned on in a window whose tab is in the background.
pub fn background_window(ctx: &Ctx) -> Result<Outcome> {
    let server = split(ctx, "off")?;
    server.run(&["new-window", "-t", "e2e"])?;
    ctx.sleep(0.3);
    let out = Run::new(ctx, &server, "run")
        .at(2.5, &["set", "-w", "-t", "e2e:0", "pane-border-status", "top"])
        .at(4.5, &["select-window", "-t", "e2e:0"])
        .dump_at(7.0)
        .run()?;
    check_active(&server, &out)
}

/// A vertical split made while attached, then rotated: both panes change
/// size, and tmux sends no layout change for the rotation.
fn rotate_once(ctx: &Ctx, label: &'static str, show_other: bool) -> Result {
    let server = server()?;
    server.run(&["set", "-w", "-t", "e2e", "pane-border-status", "top"])?;
    seq(&server, "%0", 1, 60)?;
    ctx.sleep(0.3);
    let mut run = Run::new(ctx, &server, label)
        .at(2.5, &["split-window", "-v", "-d", "-t", "%0"])
        .at(2.8, &["send-keys", "-t", "%1", "-l", "clear; seq 100 160"])
        .at(2.9, &["send-keys", "-t", "%1", "Enter"])
        .at(4.5, &["rotate-window", "-t", "e2e"])
        .dump_at(8.0);
    if show_other {
        // The pane that isn't active after the rotation.
        run = run.at(6.0, &["select-pane", "-t", "e2e:.+"]);
    }
    let out = run.run()?;
    matches_tmux(&server, &out.dump, &server.active_pane()?)
}

pub fn rotate(ctx: &Ctx) -> Result<Outcome> {
    rotate_once(ctx, "active", false)?;
    rotate_once(ctx, "other", true)?;
    Ok(Outcome::Pass)
}

/// Unequal panes swap places and sizes when rotated, with no border status.
pub fn rotate_unequal(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["split-window", "-h", "-l", "30", "-t", "e2e"])?;
    seq(&server, "%0", 1, 60)?;
    seq(&server, "%1", 100, 160)?;
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run")
        .at(2.5, &["rotate-window", "-t", "e2e"])
        .at(4.0, &["select-pane", "-t", "%0"])
        .dump_at(6.0)
        .run()?;
    matches_tmux(&server, &out.dump, "%0")?;
    Ok(Outcome::Pass)
}
