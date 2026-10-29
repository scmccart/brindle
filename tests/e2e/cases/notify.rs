//! Desktop notifications, read from the `BRINDLE_NOTIFY_LOG` file instead
//! of the desktop: one `source<TAB>title<TAB>body` line each.

use super::server;
use crate::brindle::Run;
use crate::tmux::ControlClient;
use crate::{Ctx, Outcome, Result, ensure};

fn notifications(ctx: &Ctx) -> Vec<String> {
    let text = std::fs::read_to_string(ctx.dir.join("notify.log")).unwrap_or_default();
    text.lines().map(str::to_string).collect()
}

/// tmux sends no `%bell` to control clients: bells, OSC 9, OSC 777 and
/// OSC 7501 from a pane arrive raw in `%output`, also for a window that
/// isn't current. `Terminal::feed` relies on this to find them in tmux
/// panes.
pub fn tmux_osc_in_output(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let mut client = ControlClient::attach(&server, 100, 30)?;
    ctx.sleep(0.3);
    let result = (|| {
        server.run(&["new-window", "-d", "-t", "e2e", "-n", "bg"])?;
        let pane = server.display("e2e:bg", "#{pane_id}")?;
        ctx.sleep(0.5);
        let mark = client.mark();
        server.type_line(&pane, r"printf '\033]777;notify;T;B\a\033]9;Hi\a\a\033]7501;state=idle\033\\'")?;
        // tmux may put the command's line ending in the same %output line.
        let prefix = format!("%output {pane} ");
        let want = r"\033]777;notify;T;B\007\033]9;Hi\007\007\033]7501;state=idle\033\134";
        let line = client.wait_for(ctx, mark, 3.0, |l| l.starts_with(&prefix) && l.contains(want));
        ensure!(line.is_some(), "no %output with the raw sequences from {pane} within 3 s");
        Ok(())
    })();
    client.send("detach-client").ok();
    ctx.save("transcript.txt", client.transcript());
    result.map(|()| Outcome::Pass)
}

/// OSC 777 and OSC 9 in a local tab notify (`when = "always"`, so window
/// focus doesn't matter); ConEmu's `9;4` progress and `9;9` cwd don't.
pub fn osc_local(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let log = ctx.dir.join("notify.log");
    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(r"printf '\033]777;notify;Build;done; 0 errors\a\033]9;4;1;50\a\033]9;9;/tmp\a\033]9;Ping\a'\r")
        .dump_at(4.0)
        .run()?;
    let title = out.dump.tabs.iter().find(|(_, active)| *active).map(|(t, _)| t.clone()).unwrap_or_default();
    let got = notifications(ctx);
    let want = [format!("osc777\tBuild — {title}\tdone; 0 errors"), format!("osc9\t{title}\tPing")];
    ensure!(got == want, "notifications {got:?}, expected {want:?}");
    Ok(Outcome::Pass)
}

/// A bell storm and an OSC 777 in a tmux window whose tab isn't active
/// (the default `when = "unfocused"`): one bell notification, one OSC 777,
/// both naming the window.
pub fn tmux_background(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["new-window", "-d", "-t", "e2e", "-n", "bg"])?;
    let log = ctx.dir.join("notify.log");
    let storm = r"for i in $(seq 20); do printf '\a'; done; printf '\033]777;notify;Build;done\a'";
    let out = Run::new(ctx, &server, "run")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .at(2.5, &["send-keys", "-t", "e2e:bg", "-l", storm])
        .at(2.6, &["send-keys", "-t", "e2e:bg", "Enter"])
        .dump_at(5.0)
        .run()?;
    ensure!(
        out.dump.tabs.iter().any(|(t, active)| t == "bg" && !active),
        "expected an inactive tab \"bg\": {:?}",
        out.dump.tabs
    );
    let mut got = notifications(ctx);
    got.sort();
    let want = ["bell\tbg\tBell", "osc777\tBuild — bg\tdone"];
    ensure!(got == want, "notifications {got:?}, expected {want:?}");
    Ok(Outcome::Pass)
}
