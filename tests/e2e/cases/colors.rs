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

/// Replies to palette queries (OSC 4), which Brindle answers itself.
pub fn palette(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::palette_script(ctx)?;
    Run::new(ctx, &server, "run")
        .send(&format!("clear; {} 4 196\\r", script.display()))
        .action("tmux_split_right")
        .send(&format!("clear; {} 1\\r", script.display()))
        .dump_at(8.0)
        .run()?;
    let first = server.capture("%0")?;
    query::expect_palette("after attach", &first, 4, &theme::BLUE.report())?;
    query::expect_palette("after attach", &first, 196, &Rgb(0xff, 0, 0).report())?;
    ensure!(server.active_pane()? == "%1", "the split didn't become the active pane");
    query::expect_palette("in a new pane", &server.capture("%1")?, 1, &theme::RED.report())?;
    Ok(Outcome::Pass)
}

pub fn palette_reload(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::palette_script(ctx)?;
    let out = Run::new(ctx, &server, "run")
        .theme_at(1.5, "e2e-alt")
        .send("clear\\r")
        .send("clear\\r")
        .send("clear\\r")
        .send(&format!("clear; {} 4\\r", script.display()))
        .dump_at(8.0)
        .run()?;
    query::expect_palette("after a reload", &out.dump.screen, 4, &theme::ALT_BLUE.report())?;
    Ok(Outcome::Pass)
}

/// tmux answers indexes in the pane's palette (set by a program, or by
/// `pane-colours`); Brindle must not answer them too.
pub fn palette_tmux_answers(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::palette_script(ctx)?;
    server.run(&["set", "-p", "-t", "%0", "pane-colours[1]", "#445566"])?;
    let line = format!(r"clear; printf '\033]4;5;rgb:ff/00/00\a'; {} 1 5 4", script.display());
    Run::new(ctx, &server, "run")
        .at(3.0, &["send-keys", "-t", "%0", "-l", &line])
        .at(3.0, &["send-keys", "-t", "%0", "Enter"])
        .dump_at(7.5)
        .run()?;
    let screen = server.capture("%0")?;
    query::expect_palette("pane-colours", &screen, 1, &Rgb(0x44, 0x55, 0x66).report())?;
    query::expect_palette("set by the program", &screen, 5, &Rgb(0xff, 0, 0).report())?;
    query::expect_palette("from the theme", &screen, 4, &theme::BLUE.report())?;
    Ok(Outcome::Pass)
}

/// With a terminal client attached, tmux asks its terminal (here `script`,
/// which never answers), so Brindle stays silent.
pub fn palette_tty_client(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let script = query::palette_script(ctx)?;
    let attach = format!("tmux -L {} -f /dev/null attach -t e2e", server.socket);
    // tmux won't attach a terminal client without TERM, which CI doesn't set.
    let mut tty = std::process::Command::new("script")
        .args(["-qfc", &attach, "/dev/null"])
        .env("TERM", "xterm-256color")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("script: {e}"))?;
    let result = (|| {
        ctx.sleep(1.0);
        let clients = server.run(&["list-clients", "-t", "e2e", "-F", "#{client_control_mode}"])?;
        ensure!(clients.lines().any(|l| l == "0"), "the terminal client didn't attach: {clients:?}");
        let line = format!("clear; {} 4", script.display());
        Run::new(ctx, &server, "run")
            .at(3.0, &["send-keys", "-t", "%0", "-l", &line])
            .at(3.0, &["send-keys", "-t", "%0", "Enter"])
            .dump_at(6.5)
            .run()?;
        let screen = server.capture("%0")?;
        ensure!(screen.iter().any(|l| l.starts_with("p4=")), "the query script didn't finish: {screen:?}");
        let (first, extra) = query::palette_reply(&screen, 4);
        ensure!(first.is_empty() && extra.is_empty(), "a reply arrived with a terminal client attached: {first:?} {extra:?}");
        Ok(Outcome::Pass)
    })();
    tty.kill().ok();
    tty.wait().ok();
    result
}
