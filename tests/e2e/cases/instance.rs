//! One Brindle per state directory: a later launch without debug options
//! hands its request to the running one and exits, and the running one
//! opens it in its window.

use std::path::PathBuf;
use std::process::Command;

use crate::brindle::{Run, state_dir};
use crate::common::server::{Isolation, isolate};
use crate::{Ctx, Outcome, Result, Scale, ensure};

/// A later launch from `cwd`, sharing the case's state directory.
#[derive(Clone)]
struct Launcher {
    bin: PathBuf,
    state: PathBuf,
    config: PathBuf,
    server_dir: PathBuf,
    /// The case's display, so a launch that fails to hand itself over opens
    /// there, never on the user's desktop.
    display: String,
    log: PathBuf,
    scale: Scale,
}

impl Launcher {
    fn of(ctx: &Ctx) -> Self {
        Self {
            bin: ctx.bin.clone(),
            state: state_dir(ctx),
            config: ctx.dir.join("config.toml"),
            server_dir: ctx.server_dir.clone(),
            display: ctx.display.clone(),
            log: ctx.dir.join("launches.log"),
            scale: ctx.scale(),
        }
    }

    /// Runs `brindle ARGS` in `cwd` and waits for it to exit, which it must
    /// do with status 0 within 8 s (stretched). It passes no debug option,
    /// so it hands itself to the running Brindle.
    fn launch(&self, cwd: &std::path::Path, args: &[&str]) -> Result {
        let log = std::fs::OpenOptions::new().create(true).append(true).open(&self.log).map_err(|e| e.to_string())?;
        let mut command = Command::new(&self.bin);
        isolate(
            &mut command,
            &Isolation {
                config: &self.config,
                state_dir: &self.state,
                server_dir: &self.server_dir,
                display: &self.display,
            },
        );
        let mut child = command
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .spawn()
            .map_err(|e| format!("launching {args:?}: {e}"))?;
        let exited = self.scale.poll(8.0, &format!("brindle {args:?} exiting"), || child.try_wait().map_err(|e| e.to_string()));
        match exited {
            Ok(status) => {
                ensure!(status.success(), "brindle {args:?} exited with {status}");
                Ok(())
            }
            Err(why) => {
                child.kill().ok();
                child.wait().ok();
                Err(format!("{why}: it opened a Brindle of its own?"))
            }
        }
    }
}

/// A plain launch beside the running Brindle exits and opens nothing; an
/// `-e` launch from a shell in another directory exits, and its command
/// runs there, in a new active tab of the running Brindle.
pub fn second_launch(ctx: &Ctx) -> Result<Outcome> {
    let launcher = Launcher::of(ctx);
    let elsewhere = ctx.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).map_err(|e| e.to_string())?;
    let cwd = elsewhere.clone();
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .default_profile("Shell")
        .send("echo first-tab\r")
        .call_at(2.0, move |_| {
            launcher.launch(&cwd, &[])?;
            launcher.launch(&cwd, &["-e", "sh", "-c", "echo forwarded-$((40+2)) in $(pwd); exec sleep 100"])
        })
        .wait_for("forwarded-42")
        .dump_at(10.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs: {:?}", out.dump.tabs);
    let shown = out.dump.screen.concat();
    ensure!(shown.contains(&format!("forwarded-42 in {}", elsewhere.display())), "screen: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// `brindle --attach ID` beside the running Brindle opens that detached
/// session there.
pub fn attach(ctx: &Ctx) -> Result<Outcome> {
    Run::new(ctx, "first").keep_detached().profile("Shell").send("echo attach-$((1+1))\r").wait_for("attach-2").dump_at(4.0).run()?;
    let helper = ctx.helper.clone();
    let server_dir = ctx.server_dir.clone();
    let listed = Command::new(&helper)
        .arg("list")
        .env("BRINDLE_SERVER_DIR", &server_dir)
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&listed.stdout).into_owned();
    let id = text
        .lines()
        .find(|l| l.split('\t').nth(2) == Some("detached"))
        .and_then(|l| l.split('\t').next())
        .ok_or_else(|| format!("no detached session: {text:?}"))?
        .to_string();
    let launcher = Launcher::of(ctx);
    let cwd = ctx.dir.clone();
    let out = Run::new(ctx, "run")
        .keep_detached()
        .profile("Shell")
        .default_profile("Shell")
        .call_at(2.0, move |_| launcher.launch(&cwd, &["--attach", &id]))
        .wait_for("attach-2")
        .dump_at(8.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| l.contains("attach-2")), "screen: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}
