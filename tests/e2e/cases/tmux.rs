//! tmux behaviour Brindle relies on, seen over a raw control connection.
//! These need no display. A failure here means tmux changed, and the
//! matching Brindle code (named in each case) should be revisited.

use crate::query;
use crate::tmux::{ControlClient, TmuxServer};
use crate::{Ctx, Outcome, Result, ensure};

fn attach(ctx: &Ctx) -> Result<(TmuxServer, ControlClient)> {
    let server = TmuxServer::new(100, 30)?;
    let client = ControlClient::attach(&server, 100, 30)?;
    ctx.sleep(0.3);
    Ok((server, client))
}

fn check(ctx: &Ctx, client: &ControlClient, result: Result) -> Result<Outcome> {
    ctx.save("transcript.txt", client.transcript());
    result.map(|()| Outcome::Pass)
}

/// `%subscription-changed` lines: `name $s @w idx %p : value`, with a
/// trailing space when the value is empty (`protocol.rs` parses this).
pub fn subscription_format(ctx: &Ctx) -> Result<Outcome> {
    let (server, mut client) = attach(ctx)?;
    let result = (|| {
        server.run(&["set", "-p", "-t", "%0", "pane-border-format", ""])?;
        let mark = client.mark();
        client.send("refresh-client -B 'geom:%*:#{pane_left} #{pane_top} #{pane_width} #{pane_height}'")?;
        client.send("refresh-client -B 'title:%*:#{T:pane-border-format}'")?;
        let geom = client.wait_for(ctx, mark, 3.0, |l| l.starts_with("%subscription-changed geom "));
        let geom = geom.ok_or("no geometry subscription line within 3 s")?;
        ensure!(
            regex_like(&geom, "%subscription-changed geom $", " @", " %0 : 0 0 100 30"),
            "unexpected geometry line {geom:?}"
        );
        let title = client.wait_for(ctx, mark, 3.0, |l| l.starts_with("%subscription-changed title "));
        let title = title.ok_or("no title subscription line within 3 s")?;
        ensure!(title.ends_with(" : "), "an empty value should leave a trailing space: {title:?}");
        Ok(())
    })();
    check(ctx, &client, result)
}

/// The line starts with `start`, contains `middle` and ends with `end`.
fn regex_like(line: &str, start: &str, middle: &str, end: &str) -> bool {
    line.starts_with(start) && line.contains(middle) && line.ends_with(end)
}

/// Setting `pane-border-status` changes pane geometry with no
/// `%layout-change`; Brindle learns it from its geometry subscription.
pub fn border_status_no_layout_change(ctx: &Ctx) -> Result<Outcome> {
    let (server, mut client) = attach(ctx)?;
    let result = (|| {
        client.send("refresh-client -B 'geom:%*:#{pane_top} #{pane_height}'")?;
        client.wait_for(ctx, 0, 3.0, |l| l.contains("geom") && l.ends_with(" : 0 30")).ok_or("no initial geometry")?;
        let mark = client.mark();
        server.run(&["set", "-w", "-t", "e2e", "pane-border-status", "top"])?;
        client.wait_for(ctx, mark, 3.0, |l| l.contains("geom") && l.ends_with(" : 1 29")).ok_or(
            "the geometry subscription didn't report the reserved row within 3 s",
        )?;
        let layout = client.lines_since(mark).into_iter().find(|l| l.starts_with("%layout-change"));
        ensure!(layout.is_none(), "tmux now sends a layout change for border status: {layout:?}");
        Ok(())
    })();
    check(ctx, &client, result)
}

/// `rotate-window` moves panes with no `%layout-change`; Brindle detects
/// it from a reported geometry that doesn't fit the old cell.
pub fn rotate_no_layout_change(ctx: &Ctx) -> Result<Outcome> {
    let (server, mut client) = attach(ctx)?;
    let result = (|| {
        server.run(&["split-window", "-v", "-d", "-t", "%0"])?;
        client.send("refresh-client -B 'geom:%*:#{pane_top}'")?;
        client.wait_for(ctx, 0, 3.0, |l| l.contains("geom") && l.contains(" %1 : ")).ok_or("no initial geometry")?;
        let mark = client.mark();
        server.run(&["rotate-window", "-t", "e2e"])?;
        client.wait_for(ctx, mark, 3.0, |l| l.contains("geom") && l.ends_with(" %1 : 0")).ok_or(
            "the geometry subscription didn't report the rotated pane within 3 s",
        )?;
        let layout = client.lines_since(mark).into_iter().find(|l| l.starts_with("%layout-change"));
        ensure!(layout.is_none(), "tmux now sends a layout change for rotate-window: {layout:?}");
        Ok(())
    })();
    check(ctx, &client, result)
}

/// A control client's color report answers OSC 11 even over the pane's
/// `window-style` (Brindle reports each pane's effective colors).
pub fn color_report_precedence(ctx: &Ctx) -> Result<Outcome> {
    let (server, mut client) = attach(ctx)?;
    let result = (|| {
        let script = query::script(ctx)?;
        server.run(&["set", "-p", "-t", "%0", "window-style", "bg=#102030"])?;
        client.send(r#"refresh-client -r "%0:\033]11;rgb:1111/2222/3333\033\\""#)?;
        ctx.sleep(0.3);
        server.type_line("%0", &format!("clear; {}", script.display()))?;
        ctx.sleep(2.5);
        let (bg, _) = query::replies(&server.capture("%0")?);
        ensure!(bg == "rgb:1111/2222/3333", "OSC 11 answered {bg:?}, not the reported color");
        Ok(())
    })();
    check(ctx, &client, result)
}

/// Brindle's subscriptions, which it must remove before leaving a session
/// (tmux 3.6 can crash otherwise; see `unsubscribe_commands`).
const SUBSCRIPTIONS: [&str; 5] =
    ["brindle-geometry", "brindle-title", "brindle-border-status", "brindle-pane-style", "brindle-border-style"];

/// From a Brindle debug log: every subscription was removed after it was
/// registered and before `last`, the command that leaves the session.
fn removed_before(log: &str, last: &str) -> Result {
    let sent: Vec<&str> = log.lines().filter_map(|l| l.split_once("tmux <- ").map(|(_, c)| c)).collect();
    let end = sent.iter().rposition(|c| c.starts_with(last)).ok_or(format!("Brindle never sent {last:?}"))?;
    for name in SUBSCRIPTIONS {
        let on = sent.iter().position(|c| c.starts_with(&format!("refresh-client -B \"{name}:")));
        let off = sent.iter().rposition(|c| *c == format!("refresh-client -B \"{name}\""));
        let on = on.ok_or(format!("{name} was never subscribed"))?;
        let off = off.ok_or(format!("{name} was never removed"))?;
        ensure!(on < off && off < end, "{name}: subscribed at {on}, removed at {off}, {last:?} at {end}");
    }
    Ok(())
}

fn log(ctx: &Ctx, label: &str) -> Result<String> {
    std::fs::read_to_string(ctx.dir.join(format!("{label}.log"))).map_err(|e| format!("{label}.log: {e}"))
}

/// Subscriptions are removed before Brindle quits, detaches or ends the
/// session, and the server survives repeated quits.
pub fn subscriptions_removed(ctx: &Ctx) -> Result<Outcome> {
    use crate::brindle::Run;
    const DEBUG: (&str, &str) = ("RUST_LOG", "brindle::tmux=debug");

    let server = TmuxServer::new(113, 33)?;
    Run::new(ctx, &server, "quit").env(DEBUG.0, DEBUG.1).dump_at(3.0).run()?;
    removed_before(&log(ctx, "quit")?, "detach-client").map_err(|e| format!("quit: {e}"))?;
    server.run(&["has-session", "-t", "e2e"]).map_err(|e| format!("after quitting: {e}"))?;

    Run::new(ctx, &server, "detach").env(DEBUG.0, DEBUG.1).action("tmux_detach").dump_at(4.0).run()?;
    removed_before(&log(ctx, "detach")?, "detach-client").map_err(|e| format!("detach: {e}"))?;
    server.run(&["has-session", "-t", "e2e"]).map_err(|e| format!("after detaching: {e}"))?;

    for i in 0..10 {
        Run::new(ctx, &server, "repeat").dump_at(2.0).run()?;
        server.run(&["has-session", "-t", "e2e"]).map_err(|e| format!("after quit {}: {e}", i + 1))?;
    }

    // Closing the only window's tab ends the session (and the server).
    Run::new(ctx, &server, "close").env(DEBUG.0, DEBUG.1).action("close_tab").dump_at(4.0).run()?;
    removed_before(&log(ctx, "close")?, "kill-window").map_err(|e| format!("closing the last tab: {e}"))?;
    Ok(Outcome::Pass)
}
