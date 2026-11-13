//! What Brindle asks the terminal server to run on Windows: the default
//! program, the config editor's command and the merged environment, plus
//! program names and console titles. The holder builds the command line
//! itself (`remote-agent/src/conpty/cmdline.rs`).
//!
//! Pure, so they are tested on every platform.

/// `cmd.exe /D /S /C "<editor> "<path>""`: the editor command (from
/// `%VISUAL%`, else `%EDITOR%`, else Notepad) run by cmd so values like
/// `code -w` work, with `path` quoted. `editor` is used as written, so it
/// can quote a program path with spaces itself.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_editor_command(visual: Option<&str>, editor: Option<&str>, path: &str) -> (String, Vec<String>) {
    let editor = [visual, editor].into_iter().flatten().map(str::trim).find(|e| !e.is_empty()).unwrap_or("notepad.exe");
    let tail = format!("\"{editor} \"{path}\"\"");
    ("cmd.exe".into(), vec!["/D".into(), "/S".into(), "/C".into(), tail])
}

/// A Windows program's name without its directory or extension:
/// `C:\Windows\System32\CMD.EXE` is `CMD`. Both `\` and `/` separate
/// directories, on any platform.
pub fn program_stem(program: &str) -> &str {
    let name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// Whether a Windows title is the console's rather than the program's: the
/// console host starts every program with the program's full path as its
/// title, and cmd, while it runs a command, appends ` - <command>` to the
/// title it started with (`C:\WINDOWS\system32\cmd.exe - python`, or pwsh's
/// path when started from pwsh). Neither names what's running.
pub fn is_console_title(title: &str) -> bool {
    let bytes = title.as_bytes();
    let drive = bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
    if !drive && !title.starts_with(r"\\") {
        return false;
    }
    // The path ends at the first `.exe` that ends a word. Lowercasing ASCII
    // keeps byte offsets.
    let lower = title.to_ascii_lowercase();
    let end = lower.match_indices(".exe").map(|(at, _)| at + 4).find(|&end| matches!(bytes.get(end), None | Some(b' ')));
    end.is_some_and(|end| title[end..].is_empty() || title[end..].starts_with(" - "))
}

/// The program a Windows tab runs when its profile names none: `pwsh.exe`
/// in one of `path_dirs` (the `PATH` entries), then `powershell.exe`, then
/// `comspec` (`%COMSPEC%`), then `cmd.exe`. PowerShell starts without its
/// logo.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_default_program(
    path_dirs: impl IntoIterator<Item = std::path::PathBuf>,
    comspec: Option<&str>,
) -> (String, Vec<String>) {
    let dirs: Vec<std::path::PathBuf> = path_dirs.into_iter().collect();
    for exe in ["pwsh.exe", "powershell.exe"] {
        if let Some(found) = dirs.iter().map(|dir| dir.join(exe)).find(|path| path.is_file()) {
            return (found.display().to_string(), vec!["-NoLogo".to_string()]);
        }
    }
    let program = comspec.filter(|c| !c.trim().is_empty()).unwrap_or("cmd.exe");
    (program.to_string(), Vec::new())
}

/// The variables for a child: `overrides` (the profile's, which win) and
/// then `inherited`, with names compared case-insensitively as Windows does,
/// sorted case-insensitively as `CreateProcessW` requires. Each name keeps
/// the case of the entry that won.
pub fn merge_environment<V>(inherited: Vec<(String, V)>, overrides: Vec<(String, V)>) -> Vec<(String, V)> {
    let mut merged: Vec<(String, V)> = Vec::new();
    for (name, value) in overrides.into_iter().chain(inherited) {
        if !merged.iter().any(|(seen, _)| seen.eq_ignore_ascii_case(&name)) {
            merged.push((name, value));
        }
    }
    merged.sort_by_cached_key(|(name, _)| name.to_uppercase());
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_runs_through_cmd_with_the_path_quoted() {
        let path = r"C:\Users\me\App Data\brindle\config.toml";
        // The server passes the tail after `/S` as written.
        let cmd = |tail: &str| ("cmd.exe".to_string(), ["/D", "/S", "/C", tail].map(String::from).to_vec());
        assert_eq!(
            windows_editor_command(None, Some("code -w"), path),
            cmd(r#""code -w "C:\Users\me\App Data\brindle\config.toml"""#)
        );
        assert_eq!(
            windows_editor_command(Some(r#""C:\Program Files\Vim\gvim.exe""#), Some("code -w"), path),
            cmd(r#"""C:\Program Files\Vim\gvim.exe" "C:\Users\me\App Data\brindle\config.toml"""#)
        );
        assert_eq!(windows_editor_command(Some(" "), None, r"C:\c.toml"), cmd(r#""notepad.exe "C:\c.toml"""#));
    }

    #[test]
    fn program_stems() {
        assert_eq!(program_stem(r"C:\Windows\System32\CMD.EXE"), "CMD");
        assert_eq!(program_stem("C:/tools/wsl.exe"), "wsl");
        assert_eq!(program_stem("wsl"), "wsl");
        assert_eq!(program_stem(".hidden"), ".hidden");
    }

    #[test]
    fn console_default_titles() {
        // The console host's default, the program's path, and cmd's title
        // while it runs a command, built on whatever title it started with.
        for console in [
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\WINDOWS\SYSTEM32\CMD.EXE",
            r"\\server\tools\make.exe",
            r"C:\WINDOWS\system32\cmd.exe - python",
            r"C:\Program Files\PowerShell\7\pwsh.exe - python",
            "C:/tools/cmd.exe - ping -n 3 x",
        ] {
            assert!(is_console_title(console), "{console}");
        }
        // Anything else is the program's own.
        for own in [
            "vim",
            r"Administrator: C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\src - nvim",
            r"C:\tools\app.exe.config",
            r"C:\tools\app.exe --flag",
            "c:",
        ] {
            assert!(!is_console_title(own), "{own}");
        }
    }

    fn bin_dirs(name: &str, exes: &[&[&str]]) -> Vec<std::path::PathBuf> {
        let root = std::env::temp_dir().join(format!("brindle-default-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        exes.iter()
            .enumerate()
            .map(|(i, files)| {
                let dir = root.join(i.to_string());
                std::fs::create_dir_all(&dir).unwrap();
                for file in *files {
                    std::fs::write(dir.join(file), b"").unwrap();
                }
                dir
            })
            .collect()
    }

    #[test]
    fn default_program_prefers_pwsh_then_powershell() {
        let dirs = bin_dirs("both", &[&["powershell.exe"], &["pwsh.exe"]]);
        let (program, args) = windows_default_program(dirs.clone(), Some("C:\\Windows\\system32\\cmd.exe"));
        assert_eq!(program, dirs[1].join("pwsh.exe").display().to_string());
        assert_eq!(args, ["-NoLogo"]);

        let dirs = bin_dirs("ps51", &[&["notepad.exe"], &["powershell.exe"]]);
        let (program, args) = windows_default_program(dirs.clone(), None);
        assert_eq!(program, dirs[1].join("powershell.exe").display().to_string());
        assert_eq!(args, ["-NoLogo"]);
    }

    #[test]
    fn default_program_falls_back_to_comspec_then_cmd() {
        let dirs = bin_dirs("none", &[&["notepad.exe"]]);
        assert_eq!(windows_default_program(dirs.clone(), Some("C:\\tcc\\tcc.exe")), ("C:\\tcc\\tcc.exe".to_string(), vec![]));
        assert_eq!(windows_default_program(dirs.clone(), Some(" ")), ("cmd.exe".to_string(), vec![]));
        assert_eq!(windows_default_program(dirs, None), ("cmd.exe".to_string(), vec![]));
        // A directory named like the program doesn't count.
        let dirs = bin_dirs("dir-named", &[&[]]);
        std::fs::create_dir_all(dirs[0].join("pwsh.exe")).unwrap();
        assert_eq!(windows_default_program(dirs, None).0, "cmd.exe");
    }

    #[test]
    fn profile_entries_win_whatever_their_case() {
        let inherited = vec![("Path".to_string(), "C:\\Windows"), ("TEMP".to_string(), "C:\\t")];
        let overrides = vec![("path".to_string(), "C:\\tools")];
        assert_eq!(merge_environment(inherited, overrides), [("path".to_string(), "C:\\tools"), ("TEMP".to_string(), "C:\\t")]);
    }

    #[test]
    fn environment_is_sorted_case_insensitively() {
        let inherited = vec![
            ("windir".to_string(), 1),
            ("ALLUSERSPROFILE".to_string(), 2),
            ("=C:".to_string(), 3),
            ("Path".to_string(), 4),
        ];
        let names: Vec<String> = merge_environment(inherited, vec![("TERM".to_string(), 5)]).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["=C:", "ALLUSERSPROFILE", "Path", "TERM", "windir"]);
    }
}
