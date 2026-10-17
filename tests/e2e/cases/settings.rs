//! The settings dialog: a theme previews on the tabs behind it, cancel puts
//! the old theme back, and apply writes it to the config.
//!
//! The test config's custom themes list as `e2e` (active) then `e2e-alt`,
//! after the built-ins, so one `settings_down` selects `e2e-alt`.

use super::{cells, server};
use crate::brindle::{self, Output, Run, theme};
use crate::capture::{PxRect, Rgb};
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

/// Opens the dialog (on General) and goes to the Theme section, the last
/// one, which sections wrap around to.
pub fn open_theme(run: Run) -> Run {
    run.action("open_settings").action("settings_section_prev")
}

pub fn preview(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out =
        open_theme(Run::new(ctx, &server, "run")).action("settings_down").capture_at(4.3).dump_at(4.8).run()?;
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
    let out = open_theme(Run::new(ctx, &server, "run"))
        .action("settings_down")
        .action("settings_cancel")
        .capture_at(5.3)
        .dump_at(5.8)
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
    let out = open_theme(Run::new(ctx, &server, "run"))
        .action("settings_down")
        .action("settings_confirm")
        .capture_at(5.5)
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_background("background after apply", &server, &out, theme::ALT_BACKGROUND)?;
    // Only the theme line changed; everything else is byte for byte the same.
    let want = brindle::config(&server.socket, "e2e-alt");
    ensure!(config_text(ctx)? == want, "config after apply:\n{}", config_text(ctx)?);
    Ok(Outcome::Pass)
}

/// Opens the profile editor on the `tmux` profile (the run's profile and the
/// default, so it is selected) and moves focus to its theme choice.
fn edit_tmux_theme(run: Run) -> Run {
    let run = run.action("open_settings").action("settings_section_next").action("settings_edit");
    // Name, Command, Arguments, Directory, Environment, then Theme.
    (0..5).fold(run, |run, _| run.action("settings_down"))
}

/// After the theme choice has focus: "follow global" wraps back to the last
/// theme, `e2e-alt`.
fn choose_alt_theme(run: Run) -> Run {
    run.action("settings_choice_prev")
}

pub fn profile_preview(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out = choose_alt_theme(edit_tmux_theme(Run::new(ctx, &server, "preview"))).capture_at(10.3).dump_at(10.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("section=Profiles mode=editor name=\"tmux\" field=\"Theme\" theme=\"e2e-alt\""),
        "dialog state: {settings:?}"
    );
    expect_background("profile preview", &server, &out, theme::ALT_BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config(&server.socket, "e2e"), "previewing changed the config");

    let out = choose_alt_theme(edit_tmux_theme(Run::new(ctx, &server, "cancel")))
        .action("settings_cancel")
        .action("settings_cancel")
        .capture_at(12.3)
        .dump_at(12.8)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_background("background after cancel", &server, &out, theme::BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config(&server.socket, "e2e"), "cancelling changed the config");
    Ok(Outcome::Pass)
}

/// The test config with `theme = "e2e-alt"` added to the tmux profile, as
/// the editor writes it.
fn with_tmux_profile_theme(config: &str) -> Result<String> {
    let anchor = "tmux_args = [";
    let start = config.find(anchor).ok_or("no tmux_args in the test config")?;
    let end = start + config[start..].find('\n').ok_or("no newline")? + 1;
    Ok(format!("{}theme = \"e2e-alt\"\n{}", &config[..end], &config[end..]))
}

pub fn profile_save(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out = choose_alt_theme(edit_tmux_theme(Run::new(ctx, &server, "run")))
        .action("settings_confirm")
        .capture_at(11.3)
        .dump_at(11.8)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("section=Profiles mode=list selected=\"tmux\""), "dialog state: {settings:?}");
    expect_background("background after save", &server, &out, theme::ALT_BACKGROUND)?;
    let want = with_tmux_profile_theme(&brindle::config(&server.socket, "e2e"))?;
    ensure!(config_text(ctx)? == want, "config after save:\n{}", config_text(ctx)?);
    Ok(Outcome::Pass)
}

pub fn profile_rename(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    // Rename tmux to work and save, then edit `work`'s theme: the open tab
    // only shows it if it followed the rename.
    let run = Run::new(ctx, &server, "run")
        .action("open_settings")
        .action("settings_section_next")
        .action("settings_edit")
        .action("text_input_select_home")
        .send("work")
        .action("settings_confirm")
        .action("settings_edit");
    let run = (0..5).fold(run, |run, _| run.action("settings_down"));
    let out = choose_alt_theme(run).action("settings_confirm").capture_at(16.3).dump_at(16.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("section=Profiles mode=list selected=\"work\""), "dialog state: {settings:?}");
    let config = config_text(ctx)?;
    ensure!(
        config.contains("default_profile = \"work\"") && config.contains("name = \"work\"\ntmux = \"control\""),
        "config after rename:\n{config}"
    );
    expect_background("renamed profile's theme", &server, &out, theme::ALT_BACKGROUND)?;
    Ok(Outcome::Pass)
}

/// The dialog opens on General; right changes `tmux_session_end` and writes
/// it at once, before the first table. The focused choice field's outline
/// encloses its three options.
pub fn general(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .action("open_settings")
        .action("settings_choice_next")
        .capture_at(3.3)
        .dump_at(3.8)
        .run()?;
    expect_choice_outline(&out)?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings == "section=General tmux_session_end=\"new_tab\" error=None", "dialog state: {settings:?}");
    let config = config_text(ctx)?;
    let want = brindle::config(&server.socket, "e2e").replacen("padding = 6\n", "padding = 6\ntmux_session_end = \"new_tab\"\n", 1);
    ensure!(config == want, "config after the change:\n{config}");
    Ok(Outcome::Pass)
}

/// The focused choice field's accent outline: its top edge is the longest
/// accent run in the capture (the active tab's marker is far shorter), and
/// it is wide enough for "Close window", "Open a new tab" and "Ask", with no
/// label text just past its right edge.
fn expect_choice_outline(out: &Output) -> Result {
    let image = out.image.as_ref().ok_or("no capture")?;
    let scale = out.dump.grid()?.scale;
    let px = |v: f32| (v * scale).round() as u32;
    let mut top = (0, 0, 0);
    for y in 0..image.height {
        let mut x = 0;
        while x < image.width {
            let start = x;
            while x < image.width && image.pixel(x, y).near(theme::ACCENT, 8) {
                x += 1;
            }
            if x - start > top.2 - top.1 {
                top = (y, start, x);
            }
            x += 1;
        }
    }
    let (y0, x0, x1) = top;
    ensure!(x1 - x0 >= px(250.0), "choice outline is {} px wide at y={y0}, want at least {}", x1 - x0, px(250.0));
    // The frame is 26 logical pixels tall; its labels sit inside it.
    let rows = |a: u32, b: u32| PxRect { x0: a, y0: y0 + px(4.0), x1: b, y1: y0 + px(22.0) };
    ensure!(image.count(rows(x0, x1), theme::FOREGROUND) > 0, "no label text inside the choice outline");
    let outside = image.count(rows(x1 + px(2.0), (x1 + px(80.0)).min(image.width)), theme::FOREGROUND);
    ensure!(outside == 0, "{outside} label pixels just right of the choice outline");
    Ok(())
}
