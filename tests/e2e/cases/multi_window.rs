//! Two Brindles beside each other (the second a debug run, which runs on
//! its own) stay independent: what is done in one doesn't happen in the
//! other.

use crate::brindle::{Output, Run, listed_sessions};
use crate::{Ctx, Outcome, Result, ensure};

/// Runs `first`, then `second` once the case's server lists an attached
/// session, which only `first` can have made: a listed session means the
/// first Brindle has started, and it takes `session.lock` before GPUI
/// starts, so the second runs beside it. Both runs must use the same
/// config: they share the case's `config.toml`.
pub(super) fn two_processes<'a>(ctx: &Ctx, first: Run<'a>, second: Run<'a>) -> Result<(Output, Output)> {
    let attached = || Ok(listed_sessions(ctx)?.iter().any(|&(_, attached)| attached).then_some(()));
    two_processes_after(ctx, "the first Brindle's session listed", attached, first, second)
}

/// [`two_processes`], starting `second` once `ready` gives a value (polled
/// within a stretched 30 s), for a first run that must get further than
/// its first session before the second starts.
pub(super) fn two_processes_after<'a>(
    ctx: &Ctx,
    what: &str,
    mut ready: impl FnMut() -> Result<Option<()>>,
    first: Run<'a>,
    second: Run<'a>,
) -> Result<(Output, Output)> {
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| first.run());
        let started = ctx.scale().poll(30.0, what, || {
            if first.is_finished() {
                return Ok(Some(false));
            }
            Ok(ready()?.map(|()| true))
        });
        let second = match started {
            Ok(true) => second.run(),
            Ok(false) => Err(format!("the first run ended before {what}")),
            Err(why) => Err(why),
        };
        (first.join().unwrap_or_else(|_| Err("first run panicked".into())), second)
    });
    Ok((first?, second?))
}

/// Two Brindles on local shells: tabs opened, typed into and closed in the
/// second leave the first as it was.
pub fn second_process_local(ctx: &Ctx) -> Result<Outcome> {
    let first = Run::new(ctx, "first").dump_at(9.0);
    let second = Run::new(ctx, "second")
        .action("new_tab_with_profile_1")
        .send("echo from-second\r")
        .action("new_tab_with_profile_1")
        .dump_at(4.5);
    let (first, second) = two_processes(ctx, first, second)?;
    ensure!(second.dump.tabs.len() == 3, "second tabs: {:?}", second.dump.tabs);
    ensure!(first.dump.tabs.len() == 1, "first tabs: {:?}", first.dump.tabs);
    ensure!(!first.dump.screen.concat().contains("from-second"), "first screen: {:?}", first.dump.screen);
    Ok(Outcome::Pass)
}
