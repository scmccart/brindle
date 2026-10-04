//! Pane title lines (`pane-border-status` with `pane-border-format`).

use super::{cells, server};
use crate::brindle::{Output, Run, theme};
use crate::capture::Cells;
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result};

const CLAUDE_CODE_FORMAT: &str = "#[fg=blue,bold] #{pane_title} #[default]";

/// Two panes side by side with titles on top, the left one active.
fn split(border_status: &str) -> Result<TmuxServer> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    server.run(&["set", "-w", "-t", "e2e", "pane-border-status", border_status])?;
    server.run(&["select-pane", "-t", "%0"])?;
    Ok(server)
}

fn capture(server: &TmuxServer, run: Run) -> Result<(Output, Cells)> {
    let out = run.capture_at(4.5).dump_at(6.0).run()?;
    let cells = cells(server, &out, &server.active_pane()?)?;
    Ok((out, cells))
}

pub fn default_format(ctx: &Ctx) -> Result<Outcome> {
    let server = split("top")?;
    let (out, cells) = capture(&server, Run::new(ctx, &server, "run"))?;
    let image = out.image.as_ref().unwrap();
    // Active pane: its index in reverse video, in the accent color.
    let index = cells.rect(2, 0, 1, 1);
    let area = ((index.x1 - index.x0) * (index.y1 - index.y0)) as usize;
    image.expect("active index", index, theme::ACCENT, area / 3)?;
    // Inactive pane: `1 "title"` in the foreground color.
    let [left, ..] = server.geometry("%1")?;
    image.expect("inactive title", cells.rect(left + 2, 0, 10, 1), theme::FOREGROUND, 10)?;
    Ok(Outcome::Pass)
}

pub fn claude_code_format(ctx: &Ctx) -> Result<Outcome> {
    let server = split("top")?;
    server.run(&["select-pane", "-t", "%1", "-T", "@researcher"])?;
    server.run(&["set", "-p", "-t", "%1", "pane-border-format", CLAUDE_CODE_FORMAT])?;
    server.run(&["select-pane", "-t", "%0"])?;
    let (out, cells) = capture(&server, Run::new(ctx, &server, "run"))?;
    let [left, ..] = server.geometry("%1")?;
    out.image.as_ref().unwrap().expect("@researcher", cells.rect(left + 2, 0, 13, 1), theme::BLUE, 10)?;
    Ok(Outcome::Pass)
}

/// A title renamed while attached: the longer name reaches columns the old
/// one didn't.
pub fn rename(ctx: &Ctx) -> Result<Outcome> {
    let server = split("top")?;
    server.run(&["select-pane", "-t", "%1", "-T", "@a"])?;
    server.run(&["set", "-p", "-t", "%1", "pane-border-format", CLAUDE_CODE_FORMAT])?;
    server.run(&["select-pane", "-t", "%0"])?;
    let run = Run::new(ctx, &server, "run").at(2.0, &["select-pane", "-t", "%1", "-T", "@a-much-longer-name"]);
    // select-pane -T doesn't change the active pane.
    let (out, cells) = capture(&server, run)?;
    let [left, ..] = server.geometry("%1")?;
    out.image.as_ref().unwrap().expect("renamed title", cells.rect(left + 8, 0, 10, 1), theme::BLUE, 10)?;
    Ok(Outcome::Pass)
}

/// Titles move below the panes with `bottom`, and disappear with `off`.
pub fn bottom_and_off(ctx: &Ctx) -> Result<Outcome> {
    let magenta_titles = |server: &TmuxServer| {
        server.run(&["set", "-g", "pane-border-format", "#[fg=magenta] title #[default]"]).map(drop)
    };
    let server = split("bottom")?;
    magenta_titles(&server)?;
    let (out, cells) = capture(&server, Run::new(ctx, &server, "bottom"))?;
    let image = out.image.as_ref().unwrap();
    let [_, top, _, height] = server.geometry("%0")?;
    image.expect("bottom title", cells.rect(2, top + height, 8, 1), theme::MAGENTA, 10)?;
    image.expect_none("top row", cells.rect(0, 0, 113, 1), theme::MAGENTA)?;
    drop(server);

    let server = split("off")?;
    magenta_titles(&server)?;
    let (out, cells) = capture(&server, Run::new(ctx, &server, "off"))?;
    out.image.as_ref().unwrap().expect_none("any row", cells.rect(0, 0, 113, 33), theme::MAGENTA)?;
    Ok(Outcome::Pass)
}

/// A stacked pane's title shares the divider row above it.
pub fn stacked(ctx: &Ctx) -> Result<Outcome> {
    let server = split("top")?;
    server.run(&["split-window", "-v", "-t", "%1"])?;
    server.run(&["select-pane", "-t", "%2", "-T", "@tester"])?;
    server.run(&["set", "-p", "-t", "%2", "pane-border-format", "#[fg=magenta,bold] #{pane_title} #[default]"])?;
    let (out, cells) = capture(&server, Run::new(ctx, &server, "run"))?;
    let [left, top, ..] = server.geometry("%2")?;
    out.image.as_ref().unwrap().expect("@tester", cells.rect(left + 2, top - 1, 9, 1), theme::MAGENTA, 10)?;
    Ok(Outcome::Pass)
}
