//! Windows command lines and environment blocks for `CreateProcessW`.
//!
//! Pure, so they are tested on every platform; only the Windows PTY uses
//! them.

/// The command line for `program` and `args`, each argument quoted so that
/// the MSVC runtime's `CommandLineToArgvW` rules give it back as written.
pub fn command_line(program: &str, args: &[String]) -> String {
    let mut line = program_token(program);
    for arg in args {
        line.push(' ');
        push_arg(&mut line, arg);
    }
    line
}

/// The command line for `program` followed by `tail` exactly as written,
/// for programs like `cmd.exe /S /C` that parse their own command line and
/// don't understand backslash escapes.
pub fn command_line_raw(program: &str, tail: &str) -> String {
    let mut line = program_token(program);
    if !tail.is_empty() {
        line.push(' ');
        line.push_str(tail);
    }
    line
}

/// The command line for a launch: [`command_line`], except that `cmd.exe`
/// given `/S` gets its arguments as written ([`command_line_raw`]), since in
/// that mode cmd takes the rest of its line literally, minus the outer
/// quotes, and never reads backslash escapes.
pub fn launch_line(program: &str, args: &[String]) -> String {
    if program_stem(program).eq_ignore_ascii_case("cmd") && args.iter().any(|a| a.eq_ignore_ascii_case("/S")) {
        command_line_raw(program, &args.join(" "))
    } else {
        command_line(program, args)
    }
}

/// `cmd.exe /D /S /C "<editor> "<path>""`: the editor command (from
/// `%VISUAL%`, else `%EDITOR%`, else Notepad) run by cmd so values like
/// `code -w` work, with `path` quoted. `editor` is used as written, so it
/// can quote a program path with spaces itself.
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

/// A program's own title as a tab shows it on Windows, where the console
/// host starts every program with the program's full path as its title.
/// `None` for that path alone, which the program didn't choose. A title that
/// starts with it, like cmd's `C:\WINDOWS\system32\cmd.exe - python`, keeps
/// the program's name without directory or extension: `cmd - python`.
pub fn console_title(title: &str) -> Option<String> {
    let own = || Some(title.to_string());
    let bytes = title.as_bytes();
    let drive = bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
    if !drive && !title.starts_with(r"\\") {
        return own();
    }
    // The path ends at the first `.exe` that ends a word. Lowercasing ASCII
    // keeps byte offsets.
    let lower = title.to_ascii_lowercase();
    let Some(end) = lower.match_indices(".exe").map(|(at, _)| at + 4).find(|&end| matches!(bytes.get(end), None | Some(b' ')))
    else {
        return own();
    };
    let rest = &title[end..];
    if rest.is_empty() {
        None
    } else if rest.starts_with(" - ") {
        Some(format!("{}{rest}", program_stem(&title[..end])))
    } else {
        own()
    }
}

/// `CreateProcessW` reads the program token itself: quotes delimit it and
/// backslashes are literal (a path can't contain `"`).
fn program_token(program: &str) -> String {
    if program.contains([' ', '\t']) { format!("\"{program}\"") } else { program.to_string() }
}

fn push_arg(line: &mut String, arg: &str) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        line.push_str(arg);
        return;
    }
    line.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            // Backslashes before a quote are escaped, and so is the quote.
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(c);
                backslashes = 0;
            }
        }
    }
    // Doubled, so the closing quote isn't escaped.
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

/// The program a Windows tab runs when its profile names none: `pwsh.exe`
/// in one of `path_dirs` (the `PATH` entries), then `powershell.exe`, then
/// `comspec` (`%COMSPEC%`), then `cmd.exe`. PowerShell starts without its
/// logo.
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

    /// What the MSVC runtime gives the program for `line`: the rules
    /// `CommandLineToArgvW` documents, after the program token.
    fn argv(line: &str) -> Vec<String> {
        let mut chars = line.chars().peekable();
        // The program token: quotes delimit it, nothing is escaped.
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                quoted = !quoted;
            } else if !quoted && (c == ' ' || c == '\t') {
                break;
            }
            chars.next();
        }
        let mut args = Vec::new();
        loop {
            while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                chars.next();
            }
            if chars.peek().is_none() {
                return args;
            }
            let (mut arg, mut quoted, mut backslashes) = (String::new(), false, 0usize);
            while let Some(c) = chars.next() {
                match c {
                    '\\' => backslashes += 1,
                    '"' => {
                        arg.extend(std::iter::repeat_n('\\', backslashes / 2));
                        if backslashes % 2 == 1 {
                            arg.push('"');
                        } else if quoted && chars.peek() == Some(&'"') {
                            chars.next();
                            arg.push('"');
                        } else {
                            quoted = !quoted;
                        }
                        backslashes = 0;
                    }
                    ' ' | '\t' if !quoted => {
                        arg.extend(std::iter::repeat_n('\\', backslashes));
                        backslashes = 0;
                        break;
                    }
                    c => {
                        arg.extend(std::iter::repeat_n('\\', backslashes));
                        backslashes = 0;
                        arg.push(c);
                    }
                }
            }
            arg.extend(std::iter::repeat_n('\\', backslashes));
            args.push(arg);
        }
    }

    fn round_trip(args: &[&str]) {
        let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let line = command_line("prog.exe", &owned);
        assert_eq!(argv(&line), owned, "{line}");
    }

    #[test]
    fn plain_arguments_are_not_quoted() {
        assert_eq!(command_line("pwsh.exe", &["-NoLogo".into(), "-NoExit".into()]), "pwsh.exe -NoLogo -NoExit");
    }

    #[test]
    fn spaces_quotes_and_backslashes_round_trip() {
        round_trip(&["-Command", "Set-Location 'C:\\My Files'"]);
        round_trip(&["", "a b", "tab\there", "say \"hi\"", r#"\"#, r#"C:\dir\"#, r#"C:\with space\"#]);
        round_trip(&[r#"a\\b"#, r#"a\"b"#, r#"\\"quoted\\""#, "new\nline"]);
    }

    /// The same cases through the real parser.
    #[cfg(windows)]
    #[test]
    fn windows_parses_them_back() {
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::UI::Shell::CommandLineToArgvW;
        use windows::core::PCWSTR;

        let cases: &[&[&str]] = &[
            &["-Command", "Set-Location 'C:\\My Files'"],
            &["", "a b", "tab\there", "say \"hi\"", r#"\"#, r#"C:\dir\"#, r#"C:\with space\"#],
            &[r#"a\\b"#, r#"a\"b"#, r#"\\"quoted\\""#, "new\nline"],
        ];
        for args in cases {
            let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let line = command_line(r#"C:\Program Files\p.exe"#, &owned);
            let wide: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
            let mut count = 0;
            // SAFETY: `wide` is NUL-terminated; the array is freed below.
            let argv = unsafe { CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut count) };
            assert!(!argv.is_null());
            // SAFETY: CommandLineToArgvW returned `count` valid strings.
            let parsed: Vec<String> =
                (0..count as usize).map(|i| unsafe { (*argv.add(i)).to_string() }.unwrap()).collect();
            unsafe { LocalFree(Some(HLOCAL(argv.cast()))) };
            assert_eq!(parsed[0], r#"C:\Program Files\p.exe"#);
            assert_eq!(parsed[1..], owned[..], "{line}");
        }
    }

    #[test]
    fn trailing_backslash_is_doubled_inside_quotes() {
        assert_eq!(command_line("p", &[r#"C:\with space\"#.into()]), r#"p "C:\with space\\""#);
    }

    #[test]
    fn program_with_spaces_is_quoted_verbatim() {
        let line = command_line(r#"C:\Program Files\PowerShell\7\pwsh.exe"#, &["-NoLogo".into()]);
        assert_eq!(line, r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#);
    }

    #[test]
    fn raw_tail_is_passed_as_written() {
        let line = command_line_raw("cmd.exe", r#"/D /S /C "code -w "C:\a b\config.toml"""#);
        assert_eq!(line, r#"cmd.exe /D /S /C "code -w "C:\a b\config.toml"""#);
        assert_eq!(command_line_raw("cmd.exe", ""), "cmd.exe");
    }

    #[test]
    fn editor_runs_through_cmd_with_the_path_quoted() {
        let path = r"C:\Users\me\App Data\brindle\config.toml";
        let (program, args) = windows_editor_command(None, Some("code -w"), path);
        assert_eq!(launch_line(&program, &args), r#"cmd.exe /D /S /C "code -w "C:\Users\me\App Data\brindle\config.toml"""#);
        let (program, args) = windows_editor_command(Some(r#""C:\Program Files\Vim\gvim.exe""#), Some("code -w"), path);
        assert_eq!(
            launch_line(&program, &args),
            r#"cmd.exe /D /S /C ""C:\Program Files\Vim\gvim.exe" "C:\Users\me\App Data\brindle\config.toml"""#
        );
        let (program, args) = windows_editor_command(Some(" "), None, r"C:\c.toml");
        assert_eq!(launch_line(&program, &args), r#"cmd.exe /D /S /C "notepad.exe "C:\c.toml"""#);
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
        let shown = |title| console_title(title);
        // The console host's default: the program's path, which is no title.
        assert_eq!(shown(r"C:\Program Files\PowerShell\7\pwsh.exe"), None);
        assert_eq!(shown(r"C:\WINDOWS\SYSTEM32\CMD.EXE"), None);
        assert_eq!(shown(r"\\server\tools\make.exe"), None);
        // cmd's title while it runs a command keeps the name and the command.
        assert_eq!(shown(r"C:\WINDOWS\system32\cmd.exe - python").as_deref(), Some("cmd - python"));
        assert_eq!(shown("C:/tools/cmd.exe - ping -n 3 x").as_deref(), Some("cmd - ping -n 3 x"));
        // Anything else is the program's own and stays as it is.
        for own in [
            "vim",
            r"Administrator: C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\src - nvim",
            r"C:\tools\app.exe.config",
            r"C:\tools\app.exe --flag",
            "c:",
        ] {
            assert_eq!(shown(own).as_deref(), Some(own), "{own}");
        }
    }

    #[test]
    fn only_cmd_with_s_is_passed_raw() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(launch_line("cmd.exe", &args(&["/C", "dir C:\\a b"])), r#"cmd.exe /C "dir C:\a b""#);
        assert_eq!(launch_line(r"C:\Windows\System32\CMD.EXE", &args(&["/s", "/c", "\"x\""])), r#"C:\Windows\System32\CMD.EXE /s /c "x""#);
        assert_eq!(launch_line("pwsh.exe", &args(&["/S", "a b"])), r#"pwsh.exe /S "a b""#);
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
