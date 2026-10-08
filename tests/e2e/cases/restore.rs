//! Session restore: a plain launch reopens the windows and tabs recorded in
//! the state file, with fallbacks for what changed meanwhile, and tmux
//! control-mode sessions are attached once without changing tmux's
//! current window.

use std::path::{Path, PathBuf};

use super::server;
use crate::brindle::{Run, state_dir};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

fn state_text(ctx: &Ctx) -> Result<String> {
    std::fs::read_to_string(state_dir(ctx).join("session.toml")).map_err(|e| format!("reading the state: {e}"))
}

fn write_state(ctx: &Ctx, text: &str) -> Result {
    let dir = state_dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("session.toml"), text).map_err(|e| e.to_string())
}

fn dir(ctx: &Ctx, name: &str) -> Result<PathBuf> {
    let dir = ctx.dir.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn local(profile: &str, cwd: &Path) -> String {
    format!("[[windows.tabs]]\nkind = \"local\"\nprofile = \"{profile}\"\ncwd = \"{}\"\n", cwd.display())
}

fn tmux(session: &str) -> String {
    format!("[[windows.tabs]]\nkind = \"tmux\"\nprofile = \"tmux\"\nsession = \"{session}\"\n")
}

fn state(active: usize, tabs: &[String]) -> String {
    format!("version = 1\n\n[[windows]]\nwidth = 1000.0\nheight = 660.0\nactive = {active}\n\n{}", tabs.join("\n"))
}

fn titles(tabs: &[(String, bool)]) -> Vec<String> {
    tabs.iter().map(|(t, active)| if *active { format!("{t}*") } else { t.clone() }).collect()
}

/// Two local tabs in different directories, the first active: quitting
/// records them, and the next plain launch brings them back.
pub fn local_tabs(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let (a, b) = (dir(ctx, "dir a")?, dir(ctx, "dir-b")?);
    let quote = |p: &Path| format!("'{}'", p.display());
    Run::new(ctx, &server, "record")
        .restore()
        .default_profile("Shell")
        .send(&format!("cd {}\r", quote(&a)))
        .action("new_tab")
        .send(&format!("cd {}\r", quote(&b)))
        .action("prev_tab")
        .dump_at(6.0)
        .run()?;
    let recorded = state_text(ctx)?;
    ctx.save("recorded.toml", &recorded);
    for want in [
        format!("cwd = \"{}\"", a.display()),
        format!("cwd = \"{}\"", b.display()),
        "active = 0".to_string(),
        "width = 1000.0".to_string(),
        "height = 660.0".to_string(),
    ] {
        ensure!(recorded.contains(&want), "the state lacks {want:?}:\n{recorded}");
    }
    ensure!(recorded.find(&a.display().to_string()) < recorded.find(&b.display().to_string()), "tab order:\n{recorded}");

    let out = Run::new(ctx, &server, "restore").restore().default_profile("Shell").send("pwd\r").dump_at(4.0).run()?;
    ensure!(titles(&out.dump.tabs) == ["bash*", "bash"], "restored tabs {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| *l == a.display().to_string()), "pwd in the active tab: {:?}", out.dump.screen);
    // The size comes back as it was: the window doesn't grow per restore.
    let again = state_text(ctx)?;
    ensure!(again.contains("width = 1000.0") && again.contains("height = 660.0"), "size after restore:\n{again}");
    Ok(Outcome::Pass)
}

/// A missing profile and a deleted directory fall back to the default
/// profile in the home directory; bad state files and `--no-restore` give
/// a normal launch.
pub fn fallbacks(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let home = dir(ctx, "home")?;
    let gone = ctx.dir.join("deleted");
    write_state(ctx, &state(0, &[local("Removed", &gone), local("Shell", &ctx.dir)]))?;
    let out = Run::new(ctx, &server, "fallback")
        .restore()
        .default_profile("Shell")
        .env("HOME", &home.display().to_string())
        .send("pwd\r")
        .dump_at(4.0)
        .run()?;
    ensure!(titles(&out.dump.tabs) == ["bash*", "bash"], "restored tabs {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| *l == home.display().to_string()), "pwd: {:?}", out.dump.screen);

    for (label, text, log) in [
        ("corrupt", "this is [not a state file", "not a valid state file"),
        ("version", "version = 99\n[[windows]]\n", "unknown format version 99"),
    ] {
        write_state(ctx, text)?;
        let out = Run::new(ctx, &server, label).restore().default_profile("Shell").dump_at(3.0).run()?;
        ensure!(out.dump.tabs.len() == 1, "{label}: tabs {:?}", out.dump.tabs);
        let log_text = std::fs::read_to_string(ctx.dir.join(format!("{label}.log"))).unwrap_or_default();
        ensure!(log_text.contains(log), "{label}: the log doesn't say {log:?}");
    }

    write_state(ctx, &state(0, &[local("Shell", &ctx.dir), local("Shell", &ctx.dir)]))?;
    let out = Run::new(ctx, &server, "no-restore").restore().default_profile("Shell").arg("--no-restore").dump_at(3.0).run()?;
    ensure!(out.dump.tabs.len() == 1, "--no-restore: tabs {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

/// Runs a plain restore launch whose steps close its only window, so it
/// quits without ever dumping, and waits for it to exit.
fn run_until_window_closes(ctx: &Ctx, server: &TmuxServer, label: &str, steps: &[&str]) -> Result {
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config_with(&server.socket, "e2e", "Shell", "restore_session = true\n"))
        .map_err(|e| e.to_string())?;
    let log = std::fs::File::create(ctx.dir.join(format!("{label}.log"))).map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(&ctx.bin)
        .args(steps)
        // Only reached if the window doesn't close.
        .args(["--dump-screen-after", &ctx.secs(20.0).to_string()])
        .env("BRINDLE_CONFIG", &config)
        .env("BRINDLE_STATE_DIR", state_dir(ctx))
        .env("DISPLAY", &ctx.display)
        .env_remove("WAYLAND_DISPLAY")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(ctx.secs(12.0));
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            return Err(format!("{label}: Brindle didn't quit when its window closed"));
        }
        ctx.sleep(0.1);
    }
    Ok(())
}

/// The only tab exiting closes the window and quits: nothing is left to
/// restore, and the next launch is a normal one without complaints.
pub fn forget_on_exit(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    write_state(ctx, &state(0, &[local("Shell", &ctx.dir)]))?;
    run_until_window_closes(ctx, &server, "exit", &["--send", "exit\\r"])?;
    let text = state_text(ctx)?;
    ctx.save("after-exit.toml", &text);
    ensure!(!text.contains("[[windows"), "a window is still recorded:\n{text}");

    let out = Run::new(ctx, &server, "next").restore().default_profile("Shell").dump_at(3.0).run()?;
    ensure!(out.dump.tabs.len() == 1, "next launch: tabs {:?}", out.dump.tabs);
    let log = std::fs::read_to_string(ctx.dir.join("next.log")).unwrap_or_default();
    ensure!(!log.contains("ignoring session state"), "an empty state is reported as bad:\n{log}");
    Ok(Outcome::Pass)
}

/// Closing the last window while it still has tabs (the close button, or
/// close_window) quits and keeps the tabs for the next launch.
pub fn close_window(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let kept = dir(ctx, "kept")?;
    // cd at ~1.5 s, recorded by the 2 s poll, window closed at ~2.5 s.
    let cd = format!("cd '{}'\\r", kept.display());
    run_until_window_closes(ctx, &server, "close", &["--send", &cd, "--action", "close_window"])?;
    let text = state_text(ctx)?;
    ctx.save("after-close.toml", &text);
    ensure!(text.matches("kind = \"local\"").count() == 1, "recorded tabs:\n{text}");
    ensure!(text.contains(&format!("cwd = \"{}\"", kept.display())), "recorded directory:\n{text}");

    let out = Run::new(ctx, &server, "reopen").restore().default_profile("Shell").send("pwd\r").dump_at(4.0).run()?;
    ensure!(out.dump.tabs.len() == 1, "reopened tabs {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| *l == kept.display().to_string()), "pwd: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// A second Brindle started while one runs neither restores nor records.
pub fn second_instance(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let recorded = state(1, &[local("Shell", &ctx.dir), local("Shell", &ctx.dir)]);
    write_state(ctx, &recorded)?;
    let (first, second) = std::thread::scope(|scope| {
        let first =
            scope.spawn(|| Run::new(ctx, &server, "first").restore().default_profile("Shell").dump_at(7.0).run());
        ctx.sleep(2.0);
        let second = Run::new(ctx, &server, "second").restore().default_profile("Shell").dump_at(3.0).run();
        (first.join().unwrap_or_else(|_| Err("first run panicked".into())), second)
    });
    let (first, second) = (first?, second?);
    ensure!(titles(&first.dump.tabs) == ["bash", "bash*"], "first: {:?}", first.dump.tabs);
    ensure!(second.dump.tabs.len() == 1, "second: {:?}", second.dump.tabs);
    let log = std::fs::read_to_string(ctx.dir.join("second.log")).unwrap_or_default();
    ensure!(log.contains("another Brindle owns the session"), "the second run doesn't say why it didn't restore");
    // The first process's two tabs are what's recorded.
    let text = state_text(ctx)?;
    ensure!(text.matches("kind = \"local\"").count() == 2, "recorded:\n{text}");
    Ok(Outcome::Pass)
}

/// Windows 0, 1 and 2 named w0..w2, window 2 current and window 0 the last
/// window.
fn three_windows() -> Result<TmuxServer> {
    let server = server()?;
    server.run(&["rename-window", "-t", "e2e:0", "w0"])?;
    server.run(&["new-window", "-t", "e2e:1", "-n", "w1"])?;
    server.run(&["new-window", "-t", "e2e:2", "-n", "w2"])?;
    server.run(&["set", "-g", "automatic-rename", "off"])?;
    server.run(&["select-window", "-t", "e2e:0"])?;
    server.run(&["select-window", "-t", "e2e:2"])?;
    Ok(server)
}

/// Each window's index, current flag and last-window flag.
fn selection(server: &TmuxServer) -> Result<String> {
    server.run(&["list-windows", "-t", "e2e", "-F", "#{window_index}:#{window_active}#{window_last_flag}"])
}

/// A local tab followed by a control-mode session with three windows.
pub fn tmux_session(ctx: &Ctx) -> Result<Outcome> {
    let server = three_windows()?;
    let before = selection(&server)?;

    // The local tab was active: it stays active once the session attaches.
    write_state(ctx, &state(0, &[local("Shell", &ctx.dir), tmux("e2e")]))?;
    let out = Run::new(ctx, &server, "local-active").restore().dump_at(5.0).run()?;
    ensure!(titles(&out.dump.tabs) == ["bash*", "w0", "w1", "w2"], "tabs {:?}", out.dump.tabs);
    ensure!(selection(&server)? == before, "tmux's selection changed: {before:?} -> {:?}", selection(&server)?);
    let recorded = state_text(ctx)?;
    ctx.save("local-active.toml", &recorded);
    ensure!(recorded.matches("kind = \"tmux\"").count() == 1, "one entry for the session:\n{recorded}");
    ensure!(recorded.contains("active = 0"), "active after restore:\n{recorded}");

    // The session first and active: its tabs go to the front, and tmux's
    // current window is the active tab.
    write_state(ctx, &state(0, &[tmux("e2e"), local("Shell", &ctx.dir)]))?;
    let out = Run::new(ctx, &server, "tmux-active").restore().dump_at(5.0).run()?;
    ensure!(titles(&out.dump.tabs) == ["w0", "w1", "w2*", "bash"], "tabs {:?}", out.dump.tabs);
    ensure!(selection(&server)? == before, "tmux's selection changed: {before:?} -> {:?}", selection(&server)?);
    let recorded = state_text(ctx)?;
    ctx.save("tmux-active.toml", &recorded);
    ensure!(recorded.find("kind = \"tmux\"") < recorded.find("kind = \"local\""), "order:\n{recorded}");

    // A session that no longer exists is created again.
    write_state(ctx, &state(0, &[tmux("e2e-gone")]))?;
    let out = Run::new(ctx, &server, "recreated").restore().dump_at(5.0).run()?;
    ensure!(out.dump.tabs.len() == 1 && out.dump.tabs[0].1, "tabs {:?}", out.dump.tabs);
    server.run(&["has-session", "-t", "e2e-gone"])?;
    Ok(Outcome::Pass)
}
