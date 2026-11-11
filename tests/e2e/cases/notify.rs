//! Desktop notifications, read from the `BRINDLE_NOTIFY_LOG` file instead
//! of the desktop: one `source<TAB>title<TAB>body` line each.

use crate::brindle::Run;
use crate::{Ctx, Outcome, Result, ensure};

fn notifications(ctx: &Ctx) -> Vec<String> {
    let text = std::fs::read_to_string(ctx.dir.join("notify.log")).unwrap_or_default();
    text.lines().map(str::to_string).collect()
}

/// OSC 777 and OSC 9 in a local tab notify (`when = "always"`, so window
/// focus doesn't matter); ConEmu's `9;4` progress and `9;9` cwd don't.
pub fn osc_local(ctx: &Ctx) -> Result<Outcome> {
    let log = ctx.dir.join("notify.log");
    let out = Run::new(ctx, "run")
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

/// A bell storm and an OSC 777 from a tab that isn't active (the default
/// `when = "unfocused"`): one bell notification and one OSC 777, both
/// naming the tab. The command waits until a second tab has opened in
/// front of it; the tab title follows the foreground process, so it isn't
/// pinned down.
pub fn background_tab(ctx: &Ctx) -> Result<Outcome> {
    let log = ctx.dir.join("notify.log");
    let storm = r"sleep 3; for i in $(seq 20); do printf '\a'; done; printf '\033]777;notify;Build;done\a'\r";
    let out = Run::new(ctx, "run")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(storm)
        .action("new_tab")
        .dump_at(7.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "expected the second tab active: {:?}", out.dump.tabs);
    let mut got = notifications(ctx);
    got.sort();
    ensure!(
        got.len() == 2
            && got[0].starts_with("bell\t")
            && got[0].ends_with("\tBell")
            && got[1].starts_with("osc777\tBuild — ")
            && got[1].ends_with("\tdone"),
        "notifications {got:?}, expected one bell and one OSC 777"
    );
    Ok(Outcome::Pass)
}
