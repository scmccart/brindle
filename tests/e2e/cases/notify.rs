//! Desktop notifications, read from the `BRINDLE_NOTIFY_LOG` file instead
//! of the desktop: one `source<TAB>title<TAB>body` line each. kitty's OSC 99
//! lines add tab-separated flags (`i=ID`, `replaces`, `buttons=A|B`,
//! `silent`, `urgency=…`, `expiry=…`), and a program's close is
//! `close<TAB>osc99<TAB>i=ID`. OSC 99's replies are read back by the shell
//! byte for byte.

use super::Gate;
use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

fn notifications(ctx: &Ctx) -> Vec<String> {
    let text = std::fs::read_to_string(ctx.dir.join("notify.log")).unwrap_or_default();
    text.lines().map(str::to_string).collect()
}

/// A plain `$ ` prompt with no shell integration.
const PLAIN: &str = "PS1='$ '; clear\\r";

/// A shell `printf` of `OSC 99 ; <body> ST` for each body, in order.
/// Bodies are printf formats: `\342\200\250` is U+2028.
fn osc99(bodies: &[&str]) -> String {
    let sequences: String = bodies.iter().map(|body| format!(r"\033]99;{body}\033\\")).collect();
    format!("printf '{sequences}'")
}

/// Runs `command`, reads the reply up to its ST's `\` (giving up after 3 s,
/// stretched like the case's other waits), and prints `LABEL:` and the bytes
/// before that `\` in hex, then `;`, so the line shows the ESCs too.
/// `--send` turns `\n` into a newline, so the hex is cleaned with
/// `[:space:]`. The command line spells the label `LABEL"":`, so a
/// `--wait-for "LABEL:"` waits for the reply, not for the typed command
/// (the next step's text would go to the `read`).
fn read_reply(ctx: &Ctx, label: &str, command: &str) -> String {
    format!(
        r#"{command}; read -rs -t {} -d '\\' r && echo "{label}"":$(printf %s "$r" | od -An -tx1 | tr -d '[:space:]');"\r"#,
        ctx.secs(3.0)
    )
}

/// The hex `read_reply` prints for the reply `ESC ] 99 ; <reply> ESC \`.
fn hex_reply(label: &str, reply: &str) -> String {
    let bytes = [b"\x1b]99;".as_slice(), reply.as_bytes(), b"\x1b"].concat();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{label}:{hex};")
}

/// The screen as one string: a long reply wraps onto several rows.
fn screen_text(out: &Output) -> String {
    out.dump.screen.iter().map(|line| line.trim_end()).collect()
}

fn expect_reply(out: &Output, label: &str, reply: &str, what: &str) -> Result {
    let want = hex_reply(label, reply);
    ensure!(out.dump.wait_timeouts.is_empty(), "{what}: a wait gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    ensure!(
        screen_text(out).contains(&want),
        "{what}: the reply should read {reply:?} ({want}); screen {:?}",
        out.dump.screen
    );
    Ok(())
}

/// A run in a local tab that logs notifications, with `when = "always"`
/// (window focus doesn't matter) and `extra` added to `[notifications]`.
fn osc99_run<'a>(ctx: &'a Ctx, label: &'static str, extra: &str) -> Run<'a> {
    let log = ctx.dir.join(format!("{label}-notify.log"));
    std::fs::remove_file(&log).ok();
    Run::new(ctx, label)
        .profile("Shell")
        .config(&format!("\n[notifications]\nwhen = \"always\"\n{extra}"))
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(PLAIN)
}

/// The OSC 99 lines `osc99_run` logged for `label`.
fn logged(ctx: &Ctx, label: &str) -> Vec<String> {
    let text = std::fs::read_to_string(ctx.dir.join(format!("{label}-notify.log"))).unwrap_or_default();
    text.lines().map(str::to_string).collect()
}

/// Whether `line` is the OSC 99 notification titled `title` (before the
/// tab's title, which follows the foreground process), with `body` and
/// exactly `flags`.
fn is_osc99(line: &str, title: &str, body: &str, flags: &[&str]) -> bool {
    let fields: Vec<&str> = line.split('\t').collect();
    fields.len() == 3 + flags.len()
        && fields[0] == "osc99"
        && fields[1].starts_with(&format!("{title} — "))
        && fields[2] == body
        && fields[3..] == *flags
}

fn expect_osc99(got: &[String], at: usize, title: &str, body: &str, flags: &[&str]) -> Result {
    ensure!(
        got.get(at).is_some_and(|line| is_osc99(line, title, body, flags)),
        "notification {at} should be {title:?} — …, {body:?}, flags {flags:?}; the log has {got:?}"
    );
    Ok(())
}

/// A notification in three chunks (title, then the body in two), one in
/// base64 cut mid-quad, and one with `o=invisible` from the active tab,
/// which isn't shown.
pub fn osc99_chunks(ctx: &Ctx) -> Result<Outcome> {
    let chunks = osc99(&[
        "i=b:d=0;Build",
        "i=b:d=0:p=body;3 failed",
        "i=b:p=body;, 9 passed",
        "i=s:d=0:e=1;SGV",
        "i=s:e=1;sbG8=",
        "o=invisible;hidden",
    ]);
    osc99_run(ctx, "chunks", "").send(&format!("{chunks}\\r")).dump_at(4.0).run()?;
    let got = logged(ctx, "chunks");
    ensure!(got.len() == 2, "two notifications expected: {got:?}");
    expect_osc99(&got, 0, "Build", "3 failed, 9 passed", &["i=b"])?;
    expect_osc99(&got, 1, "Hello", "", &["i=s"])?;
    Ok(Outcome::Pass)
}

/// An update with the same id replaces the open notification (the log
/// marks it), an unnamed one never does, and the program's `p=close` closes
/// it (a `close` line).
pub fn osc99_update(ctx: &Ctx) -> Result<Outcome> {
    let update = osc99(&["i=job;Running", "i=job;Failed", ";Unnamed", "i=job:p=close;", "i=gone:p=close;"]);
    osc99_run(ctx, "update", "").send(&format!("{update}\\r")).dump_at(4.0).run()?;
    let got = logged(ctx, "update");
    ensure!(got.len() == 4, "three notifications and a close expected: {got:?}");
    expect_osc99(&got, 0, "Running", "", &["i=job"])?;
    expect_osc99(&got, 1, "Failed", "", &["i=job", "replaces"])?;
    expect_osc99(&got, 2, "Unnamed", "", &[])?;
    ensure!(got[3] == "close\tosc99\ti=job", "the close line, then nothing for an unknown id: {got:?}");
    Ok(Outcome::Pass)
}

/// The capability query's reply, byte for byte, on this tab's PTY: on
/// Linux with urgency, expiry and buttons (the log sink answers as the
/// D-Bus sink does before the server has been asked), on Windows without
/// urgency and expiry.
pub fn osc99_query(ctx: &Ctx) -> Result<Outcome> {
    let out = osc99_run(ctx, "query", "")
        .send(&read_reply(ctx, "reply", &osc99(&["i=q:p=?;"])))
        .wait_for("reply:")
        .dump_at(5.0)
        .run()?;
    let caps = if cfg!(windows) {
        "a=focus,report:o=always,unfocused,invisible:p=title,body,?,close,alive,buttons:c=1:s=system,silent"
    } else {
        "a=focus,report:o=always,unfocused,invisible:u=0,1,2:p=title,body,?,close,alive,buttons:c=1:w=1:s=system,silent"
    };
    expect_reply(&out, "reply", &format!("i=q:p=?;{caps}"), "the query")?;
    ensure!(logged(ctx, "query").is_empty(), "a query notifies nothing: {:?}", logged(ctx, "query"));
    Ok(Outcome::Pass)
}

/// `p=alive` lists the terminal's open notifications, oldest first; the
/// program's close of one raised with `c=1` writes its close report and
/// takes it off the list.
pub fn osc99_alive(ctx: &Ctx) -> Result<Outcome> {
    let out = osc99_run(ctx, "alive", "")
        .send(&format!("{}\\r", osc99(&["i=a:c=1;First", "i=b;Second"])))
        .send(&read_reply(ctx, "one", &osc99(&["i=x:p=alive;"])))
        .wait_for("one:")
        .send(&read_reply(ctx, "two", &osc99(&["i=a:p=close;"])))
        .wait_for("two:")
        .send(&read_reply(ctx, "three", &osc99(&["i=x:p=alive;"])))
        .wait_for("three:")
        .dump_at(8.0)
        .run()?;
    expect_reply(&out, "one", "i=x:p=alive;a,b", "both open")?;
    expect_reply(&out, "two", "i=a:p=close;", "the close report")?;
    expect_reply(&out, "three", "i=x:p=alive;b", "after the close")?;
    let got = logged(ctx, "alive");
    ensure!(got.len() == 3 && got[2] == "close\tosc99\ti=a", "two notifications, then a's close: {got:?}");
    Ok(Outcome::Pass)
}

/// With `osc99 = false`, nothing is shown and the query gets no reply:
/// the next thing the shell reads is the answer to the device attributes
/// request sent after it (kitty's way to tell a terminal without OSC 99),
/// read up to its final `c`.
pub fn osc99_off(ctx: &Ctx) -> Result<Outcome> {
    let command = format!(r"{}; printf '\033[c'", osc99(&[";hidden", "i=q:p=?;"]));
    let read = format!(
        r#"{command}; read -rs -t {} -d c r && echo "first"":$(printf %s "$r" | od -An -tx1 | tr -d '[:space:]')"\r"#,
        ctx.secs(3.0)
    );
    let out = osc99_run(ctx, "off", "osc99 = false\n").send(&read).wait_for("first:").dump_at(5.0).run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "a wait gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    // `ESC [ ?`: the device attributes reply, with no `ESC ] 99` before it.
    let first = out.dump.screen.iter().find_map(|line| line.strip_prefix("first:")).unwrap_or_default();
    ensure!(first.starts_with("1b5b3f"), "the first reply should be DA1's, not OSC 99's: {first:?}");
    ensure!(logged(ctx, "off").is_empty(), "nothing notifies: {:?}", logged(ctx, "off"));
    Ok(Outcome::Pass)
}

/// Buttons, split on U+2028, are sent with the notification: at most four,
/// the fifth dropped, an empty label skipped; an update without
/// `p=buttons` has none.
pub fn osc99_buttons(ctx: &Ctx) -> Result<Outcome> {
    let sep = r"\342\200\250";
    let labels = ["Allow", "Deny", "", "Three", "Four", "Five"].join(sep);
    let buttons = format!("i=p:a=report:d=0:p=buttons;{labels}");
    let chunks = osc99(&[&buttons, "i=p;Run cargo test?", "i=p;Ran"]);
    osc99_run(ctx, "buttons", "").send(&format!("{chunks}\\r")).dump_at(4.0).run()?;
    let got = logged(ctx, "buttons");
    ensure!(got.len() == 2, "a notification and its update expected: {got:?}");
    expect_osc99(&got, 0, "Run cargo test?", "", &["i=p", "buttons=Allow|Deny|Three|Four"])?;
    expect_osc99(&got, 1, "Ran", "", &["i=p", "replaces"])?;
    Ok(Outcome::Pass)
}

/// `s=silent` (base64 `c2lsZW50`) is sent silent, a named sound isn't, and
/// on Linux `u=2` is sent critical.
pub fn osc99_silent(ctx: &Ctx) -> Result<Outcome> {
    let chunks = osc99(&["s=c2lsZW50:u=2;Synced", "s=d2FybmluZw==;Warned"]);
    osc99_run(ctx, "silent", "").send(&format!("{chunks}\\r")).dump_at(4.0).run()?;
    let got = logged(ctx, "silent");
    ensure!(got.len() == 2, "two notifications expected: {got:?}");
    let silent: &[&str] = if cfg!(windows) { &["silent"] } else { &["silent", "urgency=critical"] };
    expect_osc99(&got, 0, "Synced", "", silent)?;
    expect_osc99(&got, 1, "Warned", "", &[])?;
    Ok(Outcome::Pass)
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

/// Notifications sent right before Brindle quits are in the log once it has
/// exited: the only tab's `-e` program sends two and exits, so its tab and
/// the window close, and Brindle quits with no dump. The log's writer
/// thread is flushed in the quit path, so this checks no timing.
pub fn log_at_exit(ctx: &Ctx) -> Result<Outcome> {
    let log = ctx.dir.join("notify.log");
    Run::new(ctx, "run")
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .expect_exit()
        .dump_at(20.0)
        .command(&["sh", "-c", r"printf '\033]9;one\a\033]777;notify;t;two\a'"])
        .run()?;
    let got = notifications(ctx);
    ensure!(
        got.len() == 2
            && got[0].starts_with("osc9\t")
            && got[0].ends_with("\tone")
            && got[1].starts_with("osc777\tt")
            && got[1].ends_with("\ttwo"),
        "notifications {got:?}, expected the OSC 9 for `one`, then the OSC 777 for `two`"
    );
    Ok(Outcome::Pass)
}

/// A bell storm and an OSC 777 from a tab that isn't active (the default
/// `when = "unfocused"`): one bell notification and one OSC 777, both
/// naming the tab. The command waits until a second tab has opened in
/// front of it: the new tab's own command opens its [`Gate`]. The tab title
/// follows the foreground process, so it isn't pinned down.
pub fn background_tab(ctx: &Ctx) -> Result<Outcome> {
    let log = ctx.dir.join("notify.log");
    let gate = Gate::new(ctx, "go");
    let storm = format!(
        r"{}; for i in $(seq 20); do printf '\a'; done; printf '\033]777;notify;Build;done\a'\r",
        gate.wait()
    );
    let out = Run::new(ctx, "run")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(&storm)
        .action("new_tab")
        .send(&gate.open())
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
