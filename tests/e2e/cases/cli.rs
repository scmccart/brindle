//! The command line: an option without its value, or a value Brindle can't
//! use, is a usage error before any window opens.

use std::process::{Command, Stdio};

use crate::brindle::state_dir;
use crate::common::server::{Isolation, isolate};
use crate::{Ctx, Outcome, Result, ensure};

/// `brindle --list-fault-switches` and `brindle-remote --list-fault-switches`
/// exit 0 and print only `BRINDLE_TEST_` names (none in a release build),
/// opening no window: the runs get no display. This checks the contract the
/// runner's probe relies on, whichever build is under test.
pub fn list_fault_switches(ctx: &Ctx) -> Result<Outcome> {
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config("e2e")).map_err(|e| format!("writing the config: {e}"))?;
    for (label, binary) in [("brindle", &ctx.bin), ("helper", &ctx.helper)] {
        let mut command = Command::new(binary);
        isolate(
            &mut command,
            &Isolation { config: &config, state_dir: &state_dir(ctx), server_dir: &ctx.server_dir, display: &ctx.display },
        );
        // A window would need a display: none is given.
        command.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        let stdout = ctx.dir.join(format!("{label}.out"));
        let file = std::fs::File::create(&stdout).map_err(|e| format!("creating {}: {e}", stdout.display()))?;
        let mut child = command
            .arg("--list-fault-switches")
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("starting {}: {e}", binary.display()))?;
        let exited = ctx.scale().poll(10.0, &format!("{label} --list-fault-switches exiting"), || child.try_wait().map_err(|e| e.to_string()));
        let status = match exited {
            Ok(status) => status,
            Err(why) => {
                child.kill().ok();
                child.wait().ok();
                return Err(why);
            }
        };
        ensure!(status.code() == Some(0), "{} --list-fault-switches exited with {status}", binary.display());
        let text = std::fs::read_to_string(&stdout).map_err(|e| format!("reading {}: {e}", stdout.display()))?;
        for line in text.lines() {
            ensure!(line.starts_with("BRINDLE_TEST_"), "{label} printed {line:?}, which isn't a BRINDLE_TEST_ name");
        }
    }
    Ok(Outcome::Pass)
}

/// `-p` without a name, a misspelled `--action` and `--dump-screen-after`
/// without a number each exit with status 2, naming what was wrong, and
/// open no window: the runs pass no display that could show one, and each
/// exits within the limit, long before a dump would be due.
pub fn usage_errors(ctx: &Ctx) -> Result<Outcome> {
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config("e2e")).map_err(|e| format!("writing the config: {e}"))?;
    let cases: [(&str, &[&str], &str); 4] = [
        ("profile", &["-p"], "-p needs"),
        ("right-click", &["--right-click", "3", "--dump-screen-after", "30"], "--right-click: expected COL,ROW in \"3\""),
        ("action", &["--action", "split_zone_rihgt", "--dump-screen-after", "30"], "\"split_zone_rihgt\""),
        ("dump", &["--dump-screen-after", "soon"], "--dump-screen-after needs"),
    ];
    for (label, args, names) in cases {
        let stderr = ctx.dir.join(format!("{label}.log"));
        let log = std::fs::File::create(&stderr).map_err(|e| format!("creating {}: {e}", stderr.display()))?;
        let mut command = Command::new(&ctx.bin);
        isolate(
            &mut command,
            &Isolation { config: &config, state_dir: &state_dir(ctx), server_dir: &ctx.server_dir, display: &ctx.display },
        );
        // A window would need a display: none is given.
        command.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        let mut child = command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .map_err(|e| format!("starting {}: {e}", ctx.bin.display()))?;
        let exited = ctx.scale().poll(10.0, &format!("brindle {args:?} exiting"), || child.try_wait().map_err(|e| e.to_string()));
        let status = match exited {
            Ok(status) => status,
            Err(why) => {
                child.kill().ok();
                child.wait().ok();
                return Err(why);
            }
        };
        let text = std::fs::read_to_string(&stderr).map_err(|e| format!("reading {}: {e}", stderr.display()))?;
        ensure!(status.code() == Some(2), "brindle {args:?} exited with {status}; see {label}.log");
        let first = text.lines().next().unwrap_or_default();
        ensure!(first.starts_with("brindle: ") && first.contains(names), "brindle {args:?} said {first:?}, not {names:?}");
    }
    Ok(Outcome::Pass)
}
