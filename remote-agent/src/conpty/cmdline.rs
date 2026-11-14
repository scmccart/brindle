//! Windows command lines and environment blocks for `CreateProcessW`.
//!
//! Pure, so they are tested on every platform; only the Windows
//! pseudoconsole uses them.

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
    if program_stem(program).eq_ignore_ascii_case("cmd")
        && args.iter().any(|a| a.eq_ignore_ascii_case("/S"))
    {
        command_line_raw(program, &args.join(" "))
    } else {
        command_line(program, args)
    }
}

/// The environment block for `vars`: `name=value` entries in UTF-16, each
/// NUL-terminated, sorted by name case-insensitively as `CreateProcessW`
/// requires, and ended by one more NUL. The variables are used as given:
/// the caller has already merged names that differ only in case.
pub fn environment_block<'a>(vars: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<u16> {
    let mut vars: Vec<(&str, &str)> = vars.into_iter().collect();
    vars.sort_by_cached_key(|(name, _)| name.to_uppercase());
    let mut block = Vec::new();
    for (name, value) in vars {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        // An empty block is still two NULs.
        block.push(0);
    }
    block.push(0);
    block
}

/// A Windows program's name without its directory or extension:
/// `C:\Windows\System32\CMD.EXE` is `CMD`. Both `\` and `/` separate
/// directories, on any platform.
fn program_stem(program: &str) -> &str {
    let name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// `CreateProcessW` reads the program token itself: quotes delimit it and
/// backslashes are literal (a path can't contain `"`).
fn program_token(program: &str) -> String {
    if program.contains([' ', '\t']) {
        format!("\"{program}\"")
    } else {
        program.to_string()
    }
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
        assert_eq!(
            command_line("pwsh.exe", &["-NoLogo".into(), "-NoExit".into()]),
            "pwsh.exe -NoLogo -NoExit"
        );
    }

    #[test]
    fn spaces_quotes_and_backslashes_round_trip() {
        round_trip(&["-Command", "Set-Location 'C:\\My Files'"]);
        round_trip(&[
            "",
            "a b",
            "tab\there",
            "say \"hi\"",
            r#"\"#,
            r#"C:\dir\"#,
            r#"C:\with space\"#,
        ]);
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
            &[
                "",
                "a b",
                "tab\there",
                "say \"hi\"",
                r#"\"#,
                r#"C:\dir\"#,
                r#"C:\with space\"#,
            ],
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
            let parsed: Vec<String> = (0..count as usize)
                .map(|i| unsafe { (*argv.add(i)).to_string() }.unwrap())
                .collect();
            // SAFETY: the array CommandLineToArgvW allocated, freed once.
            unsafe { LocalFree(Some(HLOCAL(argv.cast()))) };
            assert_eq!(parsed[0], r#"C:\Program Files\p.exe"#);
            assert_eq!(parsed[1..], owned[..], "{line}");
        }
    }

    #[test]
    fn trailing_backslash_is_doubled_inside_quotes() {
        assert_eq!(
            command_line("p", &[r#"C:\with space\"#.into()]),
            r#"p "C:\with space\\""#
        );
    }

    #[test]
    fn program_with_spaces_is_quoted_verbatim() {
        let line = command_line(
            r#"C:\Program Files\PowerShell\7\pwsh.exe"#,
            &["-NoLogo".into()],
        );
        assert_eq!(
            line,
            r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#
        );
    }

    #[test]
    fn raw_tail_is_passed_as_written() {
        let line = command_line_raw("cmd.exe", r#"/D /S /C "code -w "C:\a b\config.toml"""#);
        assert_eq!(line, r#"cmd.exe /D /S /C "code -w "C:\a b\config.toml"""#);
        assert_eq!(command_line_raw("cmd.exe", ""), "cmd.exe");
    }

    #[test]
    fn only_cmd_with_s_is_passed_raw() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            launch_line("cmd.exe", &args(&["/C", "dir C:\\a b"])),
            r#"cmd.exe /C "dir C:\a b""#
        );
        assert_eq!(
            launch_line(r"C:\Windows\System32\CMD.EXE", &args(&["/s", "/c", "\"x\""])),
            r#"C:\Windows\System32\CMD.EXE /s /c "x""#
        );
        assert_eq!(
            launch_line("C:/tools/cmd.exe", &args(&["/D", "/S", "/C", "\"code -w \"C:\\a b\\c.toml\"\""])),
            r#"C:/tools/cmd.exe /D /S /C "code -w "C:\a b\c.toml"""#
        );
        assert_eq!(
            launch_line("pwsh.exe", &args(&["/S", "a b"])),
            r#"pwsh.exe /S "a b""#
        );
    }

    fn entries(block: &[u16]) -> Vec<String> {
        assert_eq!(block.last(), Some(&0), "the block ends with a NUL");
        let body = &block[..block.len() - 1];
        if body == [0] {
            return Vec::new();
        }
        assert_eq!(body.last(), Some(&0), "each entry ends with a NUL");
        body[..body.len() - 1]
            .split(|unit| *unit == 0)
            .map(|entry| String::from_utf16(entry).unwrap())
            .collect()
    }

    #[test]
    fn environment_is_sorted_case_insensitively() {
        let block = environment_block([
            ("windir", "C:\\Windows"),
            ("ALLUSERSPROFILE", "C:\\ProgramData"),
            ("=C:", "C:\\src"),
            ("Path", "C:\\tools;C:\\Windows"),
            ("TERM", "xterm-256color"),
        ]);
        assert_eq!(
            entries(&block),
            [
                "=C:=C:\\src",
                "ALLUSERSPROFILE=C:\\ProgramData",
                "Path=C:\\tools;C:\\Windows",
                "TERM=xterm-256color",
                "windir=C:\\Windows",
            ]
        );
    }

    #[test]
    fn environment_values_keep_non_ascii_text() {
        let block = environment_block([("NAME", "caf\u{e9} \u{1f600}")]);
        assert_eq!(entries(&block), ["NAME=caf\u{e9} \u{1f600}"]);
    }

    #[test]
    fn an_empty_environment_is_two_nuls() {
        assert_eq!(environment_block([]), [0, 0]);
    }
}
