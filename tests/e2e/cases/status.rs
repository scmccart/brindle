//! Program status (OSC 7501): the records in the dump, the notifications
//! (read from `BRINDLE_NOTIFY_LOG`, as in `notify.rs`), and the feature
//! query's reply.

use super::server;
use crate::brindle::Run;
use crate::{Ctx, Outcome, Result, ensure};

fn notifications(ctx: &Ctx) -> Vec<String> {
    let text = std::fs::read_to_string(ctx.dir.join("notify.log")).unwrap_or_default();
    text.lines().map(str::to_string).collect()
}

/// `osc7501<TAB><title> — <tab title><TAB><body>`; the tab title follows
/// the foreground process, so it isn't pinned down.
fn expect_notification(got: &[String], title: &str, body: &str) -> Result {
    let prefix = format!("osc7501\t{title} — ");
    let suffix = format!("\t{body}");
    ensure!(
        got.iter().any(|l| l.starts_with(&prefix) && l.ends_with(&suffix)),
        "no notification {prefix:?}…{suffix:?} in {got:?}"
    );
    Ok(())
}

/// A plain prompt with Brindle's bash integration, so prompts send OSC 133 A.
fn setup(ctx: &Ctx) -> String {
    format!("PS1='$ '; eval \"$({} --shell-integration bash)\"; clear\\r", ctx.bin.display())
}

/// Two runs in a local tab (`when = "always"`, so window focus doesn't
/// matter). First: the shell echoes the reply to `OSC 7501 ; ?`, and a
/// program that is still running leaves its `working` child record and its
/// `blocked` root record in the dump, the latter notifying. Second: a
/// `done` record outlives the program and the prompt that follows it.
pub fn local(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let log = ctx.dir.join("notify.log");
    // `--send` turns `\n` into a newline, so the reply is printed with echo.
    let query = r#"printf '\033]7501;?\033\\'; read -rs -t 3 -d '\\' r; echo "reply:$r" | tr -d '\033'\r"#;
    let working = r"\033]7501;state=working:id=a:progress=40:app=cargo\033\\";
    let blocked = r"\033]7501;state=blocked:kind=permission:app=claude-code:msg=UnVuIGNhcmdvIHRlc3Q/\033\\";
    let out = Run::new(ctx, &server, "blocked")
        .profile("Shell")
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(&setup(ctx))
        .send(query)
        .send(&format!("printf '{working}{blocked}'; sleep 30\r"))
        .dump_at(6.0)
        .run()?;
    ensure!(
        out.dump.screen.iter().any(|l| l == "reply:]7501;?"),
        "the query's reply should be echoed as `reply:]7501;?`; screen {:?}",
        out.dump.screen
    );
    let want = [
        "state=blocked kind=permission app=\"claude-code\" msg=\"Run cargo test?\"",
        "state=working id=\"a\" progress=40 app=\"cargo\"",
    ];
    ensure!(out.dump.status == want, "status {:?}, expected {want:?}", out.dump.status);
    let got = notifications(ctx);
    ensure!(got.len() == 1, "one notification expected, got {got:?}");
    expect_notification(&got, "claude-code", "Run cargo test?")?;

    std::fs::remove_file(&log).ok();
    let done = r"\033]7501;state=done:app=brew:msg=VXBncmFkZWQgMTIgcGFja2FnZXM=\033\\";
    let out = Run::new(ctx, &server, "done")
        .profile("Shell")
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(&setup(ctx))
        .send(&format!("printf '{working}{done}'\r"))
        .dump_at(5.0)
        .run()?;
    ensure!(
        out.dump.screen.last().map(String::as_str) == Some("$"),
        "the prompt should have returned after printf; screen {:?}",
        out.dump.screen
    );
    let want = ["state=done app=\"brew\" msg=\"Upgraded 12 packages\""];
    ensure!(out.dump.status == want, "status after the prompt {:?}, expected {want:?}", out.dump.status);
    let got = notifications(ctx);
    ensure!(got.len() == 1, "one notification expected, got {got:?}");
    expect_notification(&got, "brew", "Upgraded 12 packages")?;
    Ok(Outcome::Pass)
}

/// A `blocked` report without a message from a pane in a tmux window whose
/// tab isn't active (the default `when = "unfocused"`): the notification
/// names the window and says what kind of wait it is.
pub fn tmux_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["new-window", "-d", "-t", "e2e", "-n", "bg"])?;
    let log = ctx.dir.join("notify.log");
    let report = r"printf '\033]7501;state=blocked:kind=auth:app=brew\033\\'; sleep 30";
    let out = Run::new(ctx, &server, "run")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .at(2.5, &["send-keys", "-t", "e2e:bg", "-l", report])
        .at(2.6, &["send-keys", "-t", "e2e:bg", "Enter"])
        .dump_at(5.0)
        .run()?;
    ensure!(
        out.dump.tabs.iter().any(|(t, active)| t == "bg" && !active),
        "expected an inactive tab \"bg\": {:?}",
        out.dump.tabs
    );
    ensure!(out.dump.status.is_empty(), "the active window has no records: {:?}", out.dump.status);
    let got = notifications(ctx);
    let want = ["osc7501\tbrew — bg\tWaiting for a login"];
    ensure!(got == want, "notifications {got:?}, expected {want:?}");
    Ok(Outcome::Pass)
}
