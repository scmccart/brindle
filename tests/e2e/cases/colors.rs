//! Replies to default-color queries (OSC 10/11) from programs in panes.

use super::server;
use crate::brindle::{Run, theme};
use crate::capture::Rgb;
use crate::query;
use crate::{Ctx, Outcome, Result, ensure};

fn expect(what: &str, screen: &[String], bg: Rgb, fg: Rgb) -> Result<Outcome> {
    let (got_bg, got_fg) = query::replies(screen);
    ensure!(
        got_bg == bg.report() && got_fg == fg.report(),
        "{what}: replies bg={got_bg:?} fg={got_fg:?}, want {} {}",
        bg.report(),
        fg.report()
    );
    Ok(Outcome::Pass)
}

pub fn attach(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::script(ctx)?;
    let out = Run::new(ctx, &server, "run").send(&format!("clear; {}\\r", script.display())).dump_at(5.0).run()?;
    expect("after attach", &out.dump.screen, theme::BACKGROUND, theme::FOREGROUND)
}

pub fn new_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::script(ctx)?;
    let out = Run::new(ctx, &server, "run")
        .action("tmux_split_right")
        .send(&format!("clear; {}\\r", script.display()))
        .dump_at(5.0)
        .run()?;
    ensure!(server.active_pane()? == "%1", "the split didn't become the active pane");
    expect("in a new pane", &out.dump.screen, theme::BACKGROUND, theme::FOREGROUND)
}

pub fn reload(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::script(ctx)?;
    let out = Run::new(ctx, &server, "run")
        .theme_at(1.5, "e2e-alt")
        .send("clear\\r")
        .send("clear\\r")
        .send("clear\\r")
        .send(&format!("clear; {}\\r", script.display()))
        .dump_at(8.0)
        .run()?;
    expect("after a reload", &out.dump.screen, theme::ALT_BACKGROUND, theme::ALT_FOREGROUND)
}

/// A pane with `window-style` answers with its style's colors; tmux checks
/// Brindle's report before `window-style`, so the report must carry them.
pub fn styled_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::script(ctx)?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    server.run(&["set", "-p", "-t", "%1", "window-style", "fg=blue,bg=#102030"])?;
    let line = format!("clear; {}", script.display());
    Run::new(ctx, &server, "run")
        .at(3.0, &["send-keys", "-t", "%0", "-l", &line])
        .at(3.0, &["send-keys", "-t", "%0", "Enter"])
        .at(3.1, &["send-keys", "-t", "%1", "-l", &line])
        .at(3.1, &["send-keys", "-t", "%1", "Enter"])
        .dump_at(6.0)
        .run()?;
    expect("unstyled pane", &server.capture("%0")?, theme::BACKGROUND, theme::FOREGROUND)?;
    expect("styled pane", &server.capture("%1")?, Rgb(0x10, 0x20, 0x30), theme::BLUE)
}
