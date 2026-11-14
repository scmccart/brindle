//! Two Brindles beside each other (the second a debug run, which runs on
//! its own) stay independent: what is done in one doesn't happen in the
//! other.

use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// Runs `first`, then `second` once `first` has had `gap` seconds to
/// start, as two Brindle processes. Both runs must use the same config:
/// they share the case's `config.toml`.
pub(super) fn two_processes<'a>(ctx: &Ctx, gap: f64, first: Run<'a>, second: Run<'a>) -> Result<(Output, Output)> {
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| first.run());
        ctx.sleep(gap);
        let second = second.run();
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
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    ensure!(second.dump.tabs.len() == 3, "second tabs: {:?}", second.dump.tabs);
    ensure!(first.dump.tabs.len() == 1, "first tabs: {:?}", first.dump.tabs);
    ensure!(!first.dump.screen.concat().contains("from-second"), "first screen: {:?}", first.dump.screen);
    Ok(Outcome::Pass)
}
