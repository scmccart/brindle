//! The settings dialog: a theme previews on the tabs behind it, cancel puts
//! the old theme back, and apply writes it to the config.
//!
//! The test config's custom themes list as `e2e` (active) then `e2e-alt`,
//! after the built-ins, so one `settings_down` selects `e2e-alt`.

use super::{cells, server};
use crate::brindle::{self, Output, Run, theme};
use crate::capture::Rgb;
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

/// Checks the terminal background in the tab's bottom-left cells, which the
/// dialog (centred near the top) doesn't cover.
fn expect_background(what: &str, server: &TmuxServer, out: &Output, color: Rgb) -> Result {
    let image = out.image.as_ref().ok_or("no capture")?;
    let cells = cells(server, out, "%0")?;
    let (_, rows) = out.dump.size.ok_or("no size")?;
    let r = cells.rect(0, rows - 2, 12, 2);
    let area = ((r.x1 - r.x0) * (r.y1 - r.y0)) as usize;
    image.expect(what, r, color, area * 3 / 4)
}

fn config_text(ctx: &Ctx) -> Result<String> {
    std::fs::read_to_string(ctx.dir.join("config.toml")).map_err(|e| e.to_string())
}

pub fn preview(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out =
        Run::new(ctx, &server, "run").action("open_settings").action("settings_down").capture_at(3.3).dump_at(3.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("mode=list selected=\"e2e-alt\" active=\"e2e\""),
        "dialog state: {settings:?}"
    );
    expect_background("previewed background", &server, &out, theme::ALT_BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config(&server.socket, "e2e"), "previewing changed the config");
    Ok(Outcome::Pass)
}

pub fn cancel(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run")
        .action("open_settings")
        .action("settings_down")
        .action("settings_cancel")
        .capture_at(4.3)
        .dump_at(4.8)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_background("background after cancel", &server, &out, theme::BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config(&server.socket, "e2e"), "cancelling changed the config");
    Ok(Outcome::Pass)
}

pub fn apply(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run")
        .action("open_settings")
        .action("settings_down")
        .action("settings_confirm")
        .capture_at(4.5)
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_background("background after apply", &server, &out, theme::ALT_BACKGROUND)?;
    // Only the theme line changed; everything else is byte for byte the same.
    let want = brindle::config(&server.socket, "e2e-alt");
    ensure!(config_text(ctx)? == want, "config after apply:\n{}", config_text(ctx)?);
    Ok(Outcome::Pass)
}
