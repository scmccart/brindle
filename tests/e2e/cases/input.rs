//! Text inputs in the palettes and the settings dialog: `--send` types into
//! the focused input, editing actions move within it, and the surrounding
//! palette or dialog still gets its navigation keys.
//!
//! Steps run 1 s apart starting about 1.5 s after launch, so each dump is
//! taken a second after the last step.

use super::server;
use crate::brindle::{Input, Run};
use crate::{Ctx, Outcome, Result, ensure};

pub fn palette(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.type_line("%0", "clear")?;
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run")
        .action("open_command_palette")
        .send("splt")
        .action("text_input_left")
        .send("i")
        .dump_at(5.5)
        .run()?;
    let want = Input { text: "split".into(), cursor: 4, selection: (4, 4) };
    ensure!(out.dump.input.as_ref() == Some(&want), "input {:?}, want {want:?}", out.dump.input);
    ensure!(
        out.dump.palette.first().is_some_and(|l| l == "list query=\"split\""),
        "palette {:?}",
        out.dump.palette
    );
    ensure!(
        out.dump.palette.iter().any(|l| l.starts_with("tmux: Split Right")),
        "the filter doesn't list Split Right: {:?}",
        out.dump.palette
    );
    ensure!(!out.dump.screen.iter().any(|l| l.contains("spl")), "typed into the terminal: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// ctrl-a's action selects the whole filter, and typing replaces it.
pub fn select_all(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .action("open_command_palette")
        .send("splt")
        .action("text_input_left")
        .action("text_input_select_all")
        .send("tab")
        .dump_at(6.5)
        .run()?;
    let want = Input { text: "tab".into(), cursor: 3, selection: (3, 3) };
    ensure!(out.dump.input.as_ref() == Some(&want), "input {:?}, want {want:?}", out.dump.input);
    Ok(Outcome::Pass)
}

pub fn palette_run(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let before = Run::new(ctx, &server, "before").dump_at(3.0).run()?;
    let out = Run::new(ctx, &server, "run")
        .action("open_command_palette")
        .send("incfs")
        .action("picker_confirm")
        .dump_at(4.5)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "the palette is still open: {:?}", out.dump.palette);
    ensure!(out.dump.input.is_none(), "an input still has focus: {:?}", out.dump.input);
    let (before, after) = (before.dump.grid()?, out.dump.grid()?);
    ensure!(
        after.cell_width > before.cell_width,
        "Increase Font Size didn't run: cell width {} before, {} after",
        before.cell_width,
        after.cell_width
    );
    Ok(Outcome::Pass)
}

pub fn settings_tab(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .action("open_settings")
        .action("settings_new")
        .action("settings_next_field")
        .send("zz")
        .dump_at(5.5)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("mode=editor") && settings.contains("field=\"Foreground\""), "dialog: {settings:?}");
    ensure!(settings.contains("invalid=[\"Foreground\"]"), "the edited field isn't invalid: {settings:?}");
    let input = out.dump.input.as_ref().ok_or("no focused input")?;
    ensure!(input.text.starts_with('#') && input.text.ends_with("zz"), "foreground field holds {:?}", input.text);
    ensure!(input.cursor == input.text.len(), "cursor at {} in {:?}", input.cursor, input.text);
    Ok(Outcome::Pass)
}

pub fn prompt_preselected(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run").action("tmux_rename_window").send("x").dump_at(4.0).run()?;
    ensure!(
        out.dump.palette.first().is_some_and(|l| l.starts_with("prompt ")),
        "no prompt open: {:?}",
        out.dump.palette
    );
    let want = Input { text: "x".into(), cursor: 1, selection: (1, 1) };
    ensure!(out.dump.input.as_ref() == Some(&want), "input {:?}, want {want:?}", out.dump.input);
    Ok(Outcome::Pass)
}
