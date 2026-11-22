//! Pointer shapes (xterm's and kitty's `OSC 22`): the query replies, read
//! back by the shell byte for byte, and the program's shape on each screen
//! in the dump (`--- pointer shape=NAME screen=…`). GPUI's pointer can't be
//! captured, so the dump lines are what these check: the program's shape,
//! and the pointer the grid's last painted frame chose (`--- grid-pointer`)
//! through a selection drag. The precedence over links and selections is
//! `terminal::pointer::choose`'s unit tests.

use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// A plain `$ ` prompt with no shell integration.
const PLAIN: &str = "PS1='$ '; clear\\r";

/// A shell command printing `OSC 22 ; <body> ST`.
fn osc22(body: &str) -> String {
    format!(r"printf '\033]22;{body}\033\\'")
}

/// Prints `query`, reads the reply up to its ST's `\` (giving up after 3 s,
/// stretched like the case's other waits), and prints `reply:` and the
/// bytes before that `\` in hex, so the line shows the ESCs too. `--send`
/// turns `\n` into a newline, so the hex is cleaned with `[:space:]`.
fn read_reply(ctx: &Ctx, query: &str) -> String {
    format!(
        r#"{query}; read -rs -t {} -d '\\' r && echo "reply:$(printf %s "$r" | od -An -tx1 | tr -d '[:space:]')"\r"#,
        ctx.secs(3.0)
    )
}

/// The hex `read_reply` prints for the reply `ESC ] 22 ; <answer> ESC \`.
fn hex_reply(answer: &str) -> String {
    let bytes = [b"\x1b]22;".as_slice(), answer.as_bytes(), b"\x1b"].concat();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("reply:{hex}")
}

fn expect_reply(out: &Output, answer: &str, what: &str) -> Result {
    let want = hex_reply(answer);
    ensure!(out.dump.wait_timeouts.is_empty(), "{what}: a wait gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    ensure!(
        out.dump.screen.contains(&want),
        "{what}: the reply should read {answer:?} ({want}); screen {:?}",
        out.dump.screen
    );
    Ok(())
}

/// The spec's support query: `?pointer,crosshair,no-such-name,wait` reads
/// `1,1,0,0` (no `wait`: GPUI has no such pointer), on this tab's PTY.
pub fn query(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "support")
        .profile("Shell")
        .send(PLAIN)
        .send(&read_reply(ctx, &osc22("?pointer,crosshair,no-such-name,wait")))
        .wait_for("reply:")
        .dump_at(5.0)
        .run()?;
    expect_reply(&out, "1,1,0,0", "the support query")?;
    Ok(Outcome::Pass)
}

/// `__current__` after a push of `grab` reads `grab`, and with a mixed
/// query after a pop (an empty stack), `0,1,text,default`.
pub fn current(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "current")
        .profile("Shell")
        .send(PLAIN)
        .send(&read_reply(ctx, &format!("{}; {}", osc22(">grab"), osc22("?__current__"))))
        .wait_for("reply:")
        .dump_at(5.0)
        .run()?;
    expect_reply(&out, "grab", "after a push of grab")?;
    ensure!(out.dump.pointer.as_deref() == Some("shape=grab screen=primary"), "pointer {:?}", out.dump.pointer);

    let mixed = format!("{}; {}; {}", osc22(">grab"), osc22("<"), osc22("?__current__,pointer,__default__,__grabbed__"));
    let out = Run::new(ctx, "mixed")
        .profile("Shell")
        .send(PLAIN)
        .send(&read_reply(ctx, &mixed))
        .wait_for("reply:")
        .dump_at(5.0)
        .run()?;
    expect_reply(&out, "0,1,text,default", "a mixed query on an empty stack")?;
    ensure!(out.dump.pointer.is_none(), "an empty stack has no pointer line: {:?}", out.dump.pointer);
    Ok(Outcome::Pass)
}

/// Each screen keeps its own stack: the shell pushes `pointer`, a program
/// on the alternate screen pushes `crosshair` (the dump shows it there),
/// and once it leaves, the primary screen's `pointer` is back. Then
/// `ESC c` empties both, leaving Brindle's own (no line).
pub fn dump(ctx: &Ctx) -> Result<Outcome> {
    let alternate = format!(r"{}; printf '\033[?1049h'; {}; echo in-''alt; sleep 30", osc22(">pointer"), osc22(">crosshair"));
    let out = Run::new(ctx, "alternate")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{alternate}\\r"))
        .wait_for("in-alt")
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "on the alternate screen: a wait gave up; screen {:?}", out.dump.screen);
    ensure!(
        out.dump.pointer.as_deref() == Some("shape=crosshair screen=alternate"),
        "on the alternate screen: pointer {:?}",
        out.dump.pointer
    );

    let back = format!(
        r"{}; printf '\033[?1049h'; {}; printf '\033[?1049l'; echo back-''home",
        osc22(">pointer"),
        osc22(">crosshair")
    );
    let out = Run::new(ctx, "primary")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{back}\\r"))
        .wait_for("back-home")
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "back on the primary screen: a wait gave up; screen {:?}", out.dump.screen);
    ensure!(
        out.dump.pointer.as_deref() == Some("shape=pointer screen=primary"),
        "back on the primary screen: pointer {:?}",
        out.dump.pointer
    );

    let reset = format!(r"{}; printf '\033c'; echo was-''reset", osc22(">crosshair"));
    let out = Run::new(ctx, "reset")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{reset}\\r"))
        .wait_for("was-reset")
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "after a reset: a wait gave up; screen {:?}", out.dump.screen);
    ensure!(out.dump.pointer.is_none(), "a reset leaves Brindle's own: pointer {:?}", out.dump.pointer);
    Ok(Outcome::Pass)
}

/// A selection drag over a program's crosshair: the frames painted while
/// the left button is held choose the text beam, and the frame after the
/// release chooses the crosshair again. The dump's `--- grid-pointer` is
/// the last painted frame's, so a press or a release that draws no frame
/// shows here as the previous pointer. The release moves nothing, as a real
/// one after a drag: the drag's own frames can't stand in for its frame.
pub fn drag(ctx: &Ctx) -> Result<Outcome> {
    let crosshair = format!(r"{}; echo pointer-''set\r", osc22(">crosshair"));
    let held = Run::new(ctx, "held")
        .profile("Shell")
        .send(PLAIN)
        .send(&crosshair)
        .wait_for("pointer-set")
        .press("0,0")
        .dump_at(6.5)
        .run()?;
    ensure!(held.dump.wait_timeouts.is_empty(), "while held: a wait gave up; screen {:?}", held.dump.screen);
    ensure!(
        held.dump.pointer.as_deref() == Some("shape=crosshair screen=primary"),
        "while held: the program's pointer {:?}",
        held.dump.pointer
    );
    ensure!(held.dump.grid_pointer.as_deref() == Some("text"), "while held: grid pointer {:?}", held.dump.grid_pointer);

    let released = Run::new(ctx, "released")
        .profile("Shell")
        .send(PLAIN)
        .send(&crosshair)
        .wait_for("pointer-set")
        .press("0,0")
        .hover("4,0")
        .release("4,0")
        .dump_at(8.5)
        .run()?;
    ensure!(released.dump.wait_timeouts.is_empty(), "after the release: a wait gave up; screen {:?}", released.dump.screen);
    ensure!(released.dump.selection.is_some(), "the drag selected nothing; screen {:?}", released.dump.screen);
    ensure!(
        released.dump.grid_pointer.as_deref() == Some("crosshair"),
        "after the release: grid pointer {:?}",
        released.dump.grid_pointer
    );
    Ok(Outcome::Pass)
}
