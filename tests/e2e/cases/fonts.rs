//! Fonts: `[ui_font]` resolves to its first installed family, and the
//! settings dialog's Fonts section previews a size at once and writes it on
//! Apply. Both read the dump's `--- fonts` and grid lines, so they run on
//! Linux and Windows alike.

use super::step_time;
use crate::brindle::{self, Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// A family no machine has.
const MISSING: &str = "Nonexistent Brindle Font";

fn ui_font(families: &[&str]) -> String {
    let list: Vec<String> = families.iter().map(|f| format!("{f:?}")).collect();
    format!("\n[ui_font]\nfamily = [{}]\n", list.join(", "))
}

/// With no `[ui_font]`, and with one naming only a missing family, the
/// chrome uses the built-in list's first installed family; with a missing
/// family then an installed one, it uses the installed one. The installed
/// family is the one the terminal font resolved to, so the case needs no
/// list of the runner's fonts.
pub fn ui_font_config(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "default").run()?;
    let fonts = out.dump.fonts()?;
    let (installed, builtin) = (fonts.terminal.clone(), fonts.ui.clone());
    ensure!(!builtin.is_empty() && builtin != MISSING, "UI font with no [ui_font]: {builtin:?}");

    let out = Run::new(ctx, "missing").config(&ui_font(&[MISSING])).run()?;
    let ui = &out.dump.fonts()?.ui;
    ensure!(*ui == builtin, "UI font for a list of only a missing family: {ui:?}, not the built-in {builtin:?}");

    if installed == builtin {
        return Ok(Outcome::Skip(format!("the terminal font and the built-in UI font are both {installed:?}")));
    }
    let out = Run::new(ctx, "listed").config(&ui_font(&[MISSING, installed.as_str()])).run()?;
    let fonts = out.dump.fonts()?;
    ensure!(fonts.ui == installed, "UI font for [{MISSING:?}, {installed:?}]: {:?}", fonts.ui);
    ensure!(fonts.terminal == installed, "the terminal font changed: {:?}", fonts.terminal);
    Ok(Outcome::Pass)
}

/// The test config's line height, 14px at 1.25, rounded.
const LINE_HEIGHT: f32 = 18.0;
/// Size 20 at 1.25.
const LINE_HEIGHT_AT_20: f32 = 25.0;

fn expect_line_height(what: &str, out: &Output, want: f32) -> Result {
    let grid = out.dump.grid()?;
    ensure!((grid.line_height - want).abs() < 0.01, "{what}: line height {} (want {want})", grid.line_height);
    Ok(())
}

fn config_text(ctx: &Ctx) -> Result<String> {
    std::fs::read_to_string(ctx.dir.join("config.toml")).map_err(|e| e.to_string())
}

/// Opens the dialog, goes to Fonts (the last section, focus on its first
/// field, Terminal font), tabs to Size and types 20 over it. Five steps.
fn type_size(run: Run) -> Run {
    run.action("open_settings")
        .action("settings_section_last")
        .action("settings_next_field")
        .action("text_input_select_all")
        .send("20")
}

/// Size previews at once, re-fitting the grid before anything is written;
/// escape puts the config's size back; Apply writes the size alone and
/// closes the dialog.
pub fn settings_fonts(ctx: &Ctx) -> Result<Outcome> {
    let out = type_size(Run::new(ctx, "preview")).dump_at(step_time(5) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.starts_with("section=Fonts mode=form ") && settings.contains(" size=\"20\" "),
        "dialog state: {settings:?}"
    );
    ensure!(
        settings.contains(" field=\"Size\" ") && settings.contains(" invalid=[] can_apply=true "),
        "dialog state: {settings:?}"
    );
    ensure!(out.dump.focus_stop.as_deref() == Some("Size"), "focus stop: {:?}", out.dump.focus_stop);
    expect_line_height("previewing size 20", &out, LINE_HEIGHT_AT_20)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "previewing changed the config");

    let out = type_size(Run::new(ctx, "cancel")).action("settings_cancel").dump_at(step_time(6) + 0.8).run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_line_height("after cancel", &out, LINE_HEIGHT)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "cancelling changed the config");

    let out = type_size(Run::new(ctx, "apply")).action("settings_confirm").dump_at(step_time(6) + 1.3).run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "focus after apply: {:?}", out.dump.focus);
    expect_line_height("after apply", &out, LINE_HEIGHT_AT_20)?;
    let config = config_text(ctx)?;
    let original = brindle::config("e2e");
    // The test config has no `[font]`: Apply adds it with the size alone,
    // leaving the rest byte for byte.
    ensure!(
        config.starts_with(&original) && config[original.len()..].trim() == "[font]\nsize = 20",
        "config after apply:\n{config}"
    );
    Ok(Outcome::Pass)
}
