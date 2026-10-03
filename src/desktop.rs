//! Registers Brindle with freedesktop launchers: `--install-desktop` writes a
//! desktop entry and the scalable icon into the XDG data directory, and
//! `--uninstall-desktop` removes them.
//!
//! Linux finds an app's icon through its desktop entry and the icon theme,
//! never through the executable, so this is what puts Brindle in launchers and
//! gives its windows an icon. Both files are embedded so that a plain
//! `cargo install` can register itself. Runs before GPUI starts, so it works
//! without a display.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow};

const ICON: &[u8] = include_bytes!("../assets/brindle.svg");

/// Must match the `app_id` passed to GPUI: the compositor matches windows to
/// `<app_id>.desktop`, and X11 docks use `WM_CLASS` (also the app id).
const APP_ID: &str = "brindle";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopCommand {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub command: DesktopCommand,
    pub prefix: Option<PathBuf>,
}

/// Validates the desktop options gathered by the argument parser. `Ok(None)`
/// means no desktop command was given; `Err` is a usage error.
pub fn request(
    install: bool,
    uninstall: bool,
    prefix: Option<PathBuf>,
) -> Result<Option<Request>, &'static str> {
    let command = match (install, uninstall) {
        (true, true) => return Err("--install-desktop and --uninstall-desktop are exclusive"),
        (true, false) => DesktopCommand::Install,
        (false, true) => DesktopCommand::Uninstall,
        (false, false) if prefix.is_some() => {
            return Err("--prefix needs --install-desktop or --uninstall-desktop");
        }
        (false, false) => return Ok(None),
    };
    Ok(Some(Request { command, prefix }))
}

/// Runs a validated request against the real data directory.
pub fn run(request: &Request) -> Result<()> {
    let data = data_dir(request.prefix.as_deref())?;
    match request.command {
        DesktopCommand::Install => {
            let exe = std::env::current_exe()
                .and_then(|exe| exe.canonicalize())
                .context("can't locate the running binary")?;
            install(&data, &exe)
        }
        DesktopCommand::Uninstall => uninstall(&data),
    }
}

/// `<prefix>/share`, or the user's XDG data directory (`$XDG_DATA_HOME`,
/// else `~/.local/share`).
pub fn data_dir(prefix: Option<&Path>) -> Result<PathBuf> {
    match prefix {
        Some(prefix) => Ok(prefix.join("share")),
        None => dirs::data_dir().ok_or_else(|| anyhow!("can't determine the XDG data directory")),
    }
}

pub fn entry_path(data: &Path) -> PathBuf {
    data.join("applications").join(format!("{APP_ID}.desktop"))
}

pub fn icon_path(data: &Path) -> PathBuf {
    data.join("icons/hicolor/scalable/apps").join(format!("{APP_ID}.svg"))
}

pub fn install(data: &Path, exe: &Path) -> Result<()> {
    if exe.to_str().is_none() {
        return Err(anyhow!("binary path {} is not valid UTF-8", exe.display()));
    }
    let entry = entry_path(data);
    write_atomic(&entry, desktop_entry(&quote_exec(exe)).as_bytes())?;
    println!("installed {}", entry.display());
    let icon = icon_path(data);
    write_atomic(&icon, ICON)?;
    println!("installed {}", icon.display());
    refresh_caches(data);
    Ok(())
}

pub fn uninstall(data: &Path) -> Result<()> {
    for path in [entry_path(data), icon_path(data)] {
        match std::fs::remove_file(&path) {
            Ok(()) => println!("removed {}", path.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("can't remove {}", path.display())),
        }
    }
    refresh_caches(data);
    Ok(())
}

/// Writes through a temporary sibling and a rename, so a desktop environment
/// watching the directory never reads a half-written entry.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().expect("install paths have a parent");
    std::fs::create_dir_all(dir).with_context(|| format!("can't create {}", dir.display()))?;
    let name = path.file_name().expect("install paths have a file name").to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, contents)
        .and_then(|()| std::fs::rename(&tmp, path))
        .inspect_err(|_| {
            std::fs::remove_file(&tmp).ok();
        })
        .with_context(|| format!("can't write {}", path.display()))
}

pub fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]
Type=Application
Name=Brindle
GenericName=Terminal
Comment=GPU-accelerated terminal emulator
Exec={exec}
Icon={APP_ID}
Terminal=false
Categories=System;TerminalEmulator;
Keywords=shell;prompt;command;commandline;cmd;terminal;tmux;
StartupWMClass={APP_ID}
StartupNotify=false
"
    )
}

/// Quotes a path for an `Exec` key. The Desktop Entry spec's argument quoting
/// applies first (double quotes when a reserved character is present, with
/// `"` `` ` `` `$` `\` escaped inside), then the general string escaping
/// (`\` becomes `\\`). `%` starts a field code, so it is doubled.
pub fn quote_exec(path: &Path) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let path = path.to_string_lossy().replace('%', "%%");
    let arg = if path.contains(RESERVED) {
        let mut quoted = String::from("\"");
        for c in path.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    } else {
        path
    };
    arg.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

/// Refreshes desktop caches without ever making lookup worse: the icon cache
/// is only rebuilt where one already exists, because a cache that goes stale
/// hides icons other apps install later. Failures are only warnings.
fn refresh_caches(data: &Path) {
    let applications = data.join("applications");
    if applications.is_dir() {
        run_tool("update-desktop-database", &["-q".as_ref(), applications.as_os_str()]);
    }
    let hicolor = data.join("icons/hicolor");
    if hicolor.join("icon-theme.cache").exists() {
        let args = ["-q".as_ref(), "-t".as_ref(), "-f".as_ref(), hicolor.as_os_str()];
        run_tool("gtk-update-icon-cache", &args);
    }
}

fn run_tool(name: &str, args: &[&OsStr]) {
    let Some(tool) = find_on_path(name) else {
        return;
    };
    match Command::new(&tool).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!("{name} exited with {status}"),
        Err(err) => log::warn!("couldn't run {name}: {err}"),
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-desktop-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn plain_path_is_not_quoted() {
        assert_eq!(quote_exec(Path::new("/home/u/.cargo/bin/brindle")), "/home/u/.cargo/bin/brindle");
    }

    #[test]
    fn path_with_space_is_quoted() {
        assert_eq!(quote_exec(Path::new("/opt/my apps/brindle")), "\"/opt/my apps/brindle\"");
    }

    #[test]
    fn quoted_specials_are_escaped_then_string_escaped() {
        // Quoting gives `"/a\"b\$c\\d"`; string escaping then doubles each `\`.
        assert_eq!(quote_exec(Path::new(r#"/a"b$c\d"#)), r#""/a\\"b\\$c\\\\d""#);
    }

    #[test]
    fn percent_is_doubled() {
        assert_eq!(quote_exec(Path::new("/opt/100%/brindle")), "/opt/100%%/brindle");
    }

    #[test]
    fn entry_names_the_app() {
        let entry = desktop_entry("/usr/bin/brindle");
        assert!(entry.starts_with("[Desktop Entry]\n"));
        for line in [
            "Exec=/usr/bin/brindle",
            "Icon=brindle",
            "StartupWMClass=brindle",
            "StartupNotify=false",
            "Terminal=false",
        ] {
            assert!(entry.lines().any(|l| l == line), "missing {line}");
        }
    }

    #[test]
    fn prefix_targets_share() {
        let data = data_dir(Some(Path::new("/usr/local"))).unwrap();
        assert_eq!(entry_path(&data), Path::new("/usr/local/share/applications/brindle.desktop"));
        assert_eq!(
            icon_path(&data),
            Path::new("/usr/local/share/icons/hicolor/scalable/apps/brindle.svg")
        );
    }

    #[test]
    fn no_prefix_uses_xdg_data_dir() {
        assert_eq!(data_dir(None).unwrap(), dirs::data_dir().unwrap());
    }

    #[test]
    fn install_reinstall_uninstall() {
        let data = scratch("cycle");
        let exe = Path::new("/opt/brindle/bin/brindle");
        install(&data, exe).unwrap();
        install(&data, exe).unwrap();
        let entry = std::fs::read_to_string(entry_path(&data)).unwrap();
        assert!(entry.contains("Exec=/opt/brindle/bin/brindle\n"));
        assert_eq!(std::fs::read(icon_path(&data)).unwrap(), ICON);
        // No temp files left behind.
        let leftovers = std::fs::read_dir(data.join("applications"))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);

        uninstall(&data).unwrap();
        assert!(!entry_path(&data).exists());
        assert!(!icon_path(&data).exists());
        uninstall(&data).unwrap();
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn install_never_creates_an_icon_cache() {
        let data = scratch("cache");
        install(&data, Path::new("/usr/bin/brindle")).unwrap();
        assert!(!data.join("icons/hicolor/icon-theme.cache").exists());
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn unwritable_target_names_the_path() {
        let err = install(Path::new("/proc/brindle"), Path::new("/usr/bin/brindle")).unwrap_err();
        assert!(format!("{err:#}").contains("/proc/brindle"), "{err:#}");
    }

    #[test]
    fn request_validation() {
        assert_eq!(request(false, false, None), Ok(None));
        assert_eq!(
            request(true, false, None),
            Ok(Some(Request { command: DesktopCommand::Install, prefix: None }))
        );
        let prefix = Some(PathBuf::from("/usr/local"));
        assert_eq!(
            request(false, true, prefix.clone()),
            Ok(Some(Request { command: DesktopCommand::Uninstall, prefix: prefix.clone() }))
        );
        assert!(request(false, false, prefix).is_err());
        assert!(request(true, true, None).is_err());
    }
}
