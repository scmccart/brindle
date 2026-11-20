//! Windows command lines and environment blocks for `CreateProcessW`.
//!
//! Pure, so they are tested on every platform; only the Windows
//! pseudoconsole uses them.

use std::collections::BTreeMap;

/// The program a session runs: its `command`, else the command interpreter
/// its environment names (`COMSPEC`, in any case, when not empty), else
/// `cmd.exe`. Brindle always names the program.
pub fn program<'a>(command: Option<&'a str>, env: &'a BTreeMap<String, String>) -> &'a str {
    command
        .or_else(|| {
            env.iter()
                .find(|(name, value)| name.eq_ignore_ascii_case("COMSPEC") && !value.is_empty())
                .map(|(_, value)| value.as_str())
        })
        .unwrap_or("cmd.exe")
}

/// A session's environment: `given` when it is `exact`, else `own` (this
/// process's) without the names `given` sets, whatever their case, followed
/// by `given`.
pub fn environment(
    given: &BTreeMap<String, String>,
    exact: bool,
    own: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let given_vars = given
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()));
    if exact {
        return given_vars.collect();
    }
    let set = |name: &str| given.keys().any(|set| set.eq_ignore_ascii_case(name));
    own.into_iter()
        .filter(|(name, _)| !set(name))
        .chain(given_vars)
        .collect()
}

/// The value of the variable `name` in the environment `env`, whatever the
/// case of its name, as Windows reads names; empty when it isn't set. The
/// holder reads the `PATH` a program is looked up in ([`resolve_program`])
/// from the environment the program gets.
pub fn variable<'a>(env: &'a [(String, String)], name: &str) -> &'a str {
    env.iter()
        .find(|(set, _)| set.eq_ignore_ascii_case(name))
        .map_or("", |(_, value)| value.as_str())
}

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

/// Where the program a session names is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The file to start, complete with its extension: what
    /// `CreateProcessW` takes as its application name, which it starts as
    /// it is given, with no search and no extension added.
    Found(String),
    /// A name without a directory that no directory of `PATH` has.
    NotFound,
}

/// Where the program `name` is, so the holder can hand `CreateProcessW`
/// the file itself instead of a name to look for. Left to find a name
/// without a directory, `CreateProcessW` looks first in the directory of
/// the image that called it and then in its current directory. Both are
/// the helper copy's directory for a server role, so a file put there
/// under a program's name would be started in the program's place.
///
/// - **A name with a directory** (a `\`, a `/` or a drive's `:`) isn't
///   searched for: it names the file. One that doesn't end in `.exe` is
///   taken with `.exe` added when that file exists, and as it is
///   otherwise, which is how `CreateProcessW` reads such a name itself.
/// - **A name without one** is looked for in the directories of
///   `path_var` (`PATH`), in their order, and nowhere else: not in the
///   image's directory, not in the current one, not in the system's
///   unless `PATH` lists it. A directory `PATH` doesn't spell from a drive
///   or a share (`.`, `bin`, an empty entry) is skipped, since it would be
///   read against the current directory. A name with no extension is tried
///   with `.exe`, then `.com`, in each directory, and with nothing else
///   `PATHEXT` lists: those are what `CreateProcessW` would have started
///   for it. A batch file is never found for a bare name, since one is
///   started through `cmd /c`, which looks its name up again by its own
///   rules.
///
/// `exists` says whether a path is a file. Paths are text, joined with
/// `\`, so the same code runs and is tested on every platform.
pub fn resolve_program(
    name: &str,
    path_var: &str,
    exists: impl Fn(&str) -> bool,
) -> Resolved {
    if name.is_empty() {
        return Resolved::NotFound;
    }
    if name.contains(['\\', '/', ':']) {
        let with_exe = format!("{name}.exe");
        let named = !has_suffix(name, ".exe") && exists(&with_exe);
        return Resolved::Found(if named { with_exe } else { name.to_string() });
    }
    let names: Vec<String> = if name.contains('.') {
        vec![name.to_string()]
    } else {
        EXTENSIONS
            .iter()
            .map(|extension| format!("{name}{extension}"))
            .collect()
    };
    path_dirs(path_var)
        .iter()
        .filter(|dir| is_rooted(dir))
        .flat_map(|dir| names.iter().map(move |name| in_dir(dir, name)))
        .find(|path| exists(path))
        .map_or(Resolved::NotFound, Resolved::Found)
}

/// Whether `name` ends with `suffix`, whatever the case of its ASCII
/// letters.
fn has_suffix(name: &str, suffix: &str) -> bool {
    name.len()
        .checked_sub(suffix.len())
        .and_then(|at| name.get(at..))
        .is_some_and(|end| end.eq_ignore_ascii_case(suffix))
}

/// The extensions a name with none is tried with, in this order within
/// each directory of `PATH`: the programs `CreateProcessW` starts itself.
const EXTENSIONS: [&str; 2] = [".exe", ".com"];

/// The directories of a `PATH`: `;` between them, except inside double
/// quotes, which are dropped.
fn path_dirs(path_var: &str) -> Vec<String> {
    let mut dirs = vec![String::new()];
    let mut quoted = false;
    for c in path_var.chars() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => dirs.push(String::new()),
            _ => {
                if let Some(dir) = dirs.last_mut() {
                    dir.push(c);
                }
            }
        }
    }
    dirs
}

/// Whether `dir` is spelled from a drive (`C:\tools`) or from a share
/// (`\\server\tools`), so that it means the same directory whatever the
/// current one is.
fn is_rooted(dir: &str) -> bool {
    let separator = |byte: &u8| matches!(byte, b'\\' | b'/');
    match dir.as_bytes() {
        [drive, b':', third, ..] => drive.is_ascii_alphabetic() && separator(third),
        [first, second, ..] => separator(first) && separator(second),
        _ => false,
    }
}

/// `name` in the directory `dir`.
fn in_dir(dir: &str, name: &str) -> String {
    if dir.ends_with(['\\', '/']) {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

/// The environment block, shared with Brindle: it lives in
/// `brindle-remote-protocol`, which both binaries depend on.
pub use brindle_remote_protocol::environment::environment_block;

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

    // ---- finding the program ----------------------------------------------------

    use std::cell::RefCell;

    const PATH: &str = r"C:\Windows\System32;C:\Windows;C:\Program Files\PowerShell\7";

    /// A probe over the files that exist (compared without case, as
    /// Windows names are), which records every path it is asked about.
    fn files<'a>(
        existing: &'a [&'a str],
        asked: &'a RefCell<Vec<String>>,
    ) -> impl Fn(&str) -> bool + 'a {
        move |path| {
            asked.borrow_mut().push(path.to_string());
            existing.iter().any(|file| file.eq_ignore_ascii_case(path))
        }
    }

    /// [`resolve_program`] of `name` over [`PATH`], when only `existing`
    /// exist.
    fn resolve(name: &str, existing: &[&str]) -> Resolved {
        resolve_program(name, PATH, files(existing, &RefCell::default()))
    }

    fn found(path: &str) -> Resolved {
        Resolved::Found(path.to_string())
    }

    #[test]
    fn a_bare_name_is_found_in_path_in_its_order() {
        let pwsh = r"C:\Program Files\PowerShell\7\pwsh.EXE";
        assert_eq!(resolve("pwsh", &[pwsh]), found(r"C:\Program Files\PowerShell\7\pwsh.exe"));
        // The first directory of `PATH` that has it wins.
        let (system, windows) = (r"C:\Windows\System32\cmd.exe", r"C:\Windows\cmd.exe");
        assert_eq!(resolve("cmd.exe", &[windows, system]), found(system));
        assert_eq!(resolve("cmd.exe", &[windows]), found(windows));
        // Whatever the case of the name: Windows names have none.
        assert_eq!(resolve("CMD.EXE", &[system]), found(r"C:\Windows\System32\CMD.EXE"));
    }

    #[test]
    fn a_name_with_no_extension_tries_exe_then_com_in_each_directory() {
        // `.exe` before `.com`, within a directory.
        let (com, exe) = (r"C:\Windows\System32\more.COM", r"C:\Windows\System32\more.EXE");
        assert_eq!(resolve("more", &[com, exe]), found(r"C:\Windows\System32\more.exe"));
        assert_eq!(resolve("more", &[com]), found(r"C:\Windows\System32\more.com"));
        // The directories in `PATH`'s order, before the extensions.
        let later_exe = r"C:\Windows\tool.EXE";
        let earlier_com = r"C:\Windows\System32\tool.COM";
        assert_eq!(resolve("tool", &[later_exe, earlier_com]), found(r"C:\Windows\System32\tool.com"));
        // Every candidate was asked about in that order.
        let asked = RefCell::default();
        let resolved = resolve_program("x", r"C:\a;C:\b", files(&[], &asked));
        assert_eq!(resolved, Resolved::NotFound);
        assert_eq!(*asked.borrow(), [r"C:\a\x.exe", r"C:\a\x.com", r"C:\b\x.exe", r"C:\b\x.com"]);
    }

    /// A batch file runs through `cmd /c`, which would look its name up
    /// again: a bare name never finds one, nor any script `PATHEXT` lists.
    #[test]
    fn a_bare_name_never_finds_a_batch_file_or_a_script() {
        let scripts = [
            r"C:\Windows\System32\tool.cmd",
            r"C:\Windows\System32\tool.bat",
            r"C:\Windows\tool.js",
            r"C:\Windows\tool.ps1",
        ];
        assert_eq!(resolve("tool", &scripts), Resolved::NotFound);
        // A later directory's program is found past them.
        let program = r"C:\Program Files\PowerShell\7\tool.exe";
        let mut with_program = scripts.to_vec();
        with_program.push(program);
        assert_eq!(resolve("tool", &with_program), found(program));
    }

    #[test]
    fn a_name_with_an_extension_is_looked_for_as_it_is() {
        let asked = RefCell::default();
        let existing = [r"C:\Windows\run.cmd.EXE", r"C:\Windows\run.cmd"];
        assert_eq!(
            resolve_program("run.cmd", PATH, files(&existing, &asked)),
            found(r"C:\Windows\run.cmd")
        );
        assert_eq!(*asked.borrow(), [r"C:\Windows\System32\run.cmd", r"C:\Windows\run.cmd"]);
        // A dot anywhere counts: no extension is added to `python3.12`.
        assert_eq!(resolve("python3.12", &[r"C:\Windows\python3.12.EXE"]), Resolved::NotFound);
    }

    /// The point of resolving here: nothing is ever looked for in the
    /// image's directory or the current one, where `CreateProcessW` would
    /// look first.
    #[test]
    fn only_the_directories_of_path_are_searched() {
        let asked = RefCell::default();
        // What a search of the current directory would find.
        let planted = ["cmd.exe", r".\cmd.exe", "cmd.EXE", r"bin\cmd.exe", r"C:cmd.exe", r"\tools\cmd.exe"];
        let path = r".;;bin;C:tools;\tools;C:\Windows\System32;..";
        let resolved = resolve_program("cmd.exe", path, files(&planted, &asked));
        assert_eq!(resolved, Resolved::NotFound);
        // Only the one directory spelled from a drive was looked in.
        assert_eq!(*asked.borrow(), [r"C:\Windows\System32\cmd.exe"]);

        // With no `PATH` at all, nothing is searched.
        let asked = RefCell::default();
        assert_eq!(resolve_program("cmd", "", files(&planted, &asked)), Resolved::NotFound);
        assert_eq!(*asked.borrow(), Vec::<String>::new());
    }

    #[test]
    fn a_directory_of_path_is_spelled_from_a_drive_or_a_share() {
        for dir in [r"C:\tools", "c:/tools", r"\\server\tools", "//server/tools", r"\\?\C:\tools", r"Z:\"] {
            assert!(is_rooted(dir), "{dir}");
        }
        for dir in ["", ".", "..", "bin", r"bin\tools", "C:", "C:tools", r"\tools", "/tools", r"1:\tools", r"é:\tools"] {
            assert!(!is_rooted(dir), "{dir:?}");
        }
        // A share's directory is searched like a drive's.
        let tool = r"\\server\tools\tool.exe";
        assert_eq!(resolve_program("tool", r"\\server\tools", files(&[tool], &RefCell::default())), found(tool));
    }

    #[test]
    fn a_directory_of_path_may_be_quoted_and_end_with_a_separator() {
        assert_eq!(path_dirs(r#"C:\a;"C:\b;c";C:\d"#), [r"C:\a", r"C:\b;c", r"C:\d"]);
        assert_eq!(path_dirs(r#""C:\Program Files"\x;;C:\y"#), [r"C:\Program Files\x", "", r"C:\y"]);
        assert_eq!(path_dirs(""), [""]);
        let tool = r"C:\b;c\tool.exe";
        assert_eq!(resolve_program("tool", r#"C:\a;"C:\b;c""#, files(&[tool], &RefCell::default())), found(tool));
        // The separator a directory ends with isn't doubled.
        let asked = RefCell::default();
        resolve_program("x.exe", r"C:\a\;C:/b/;C:\", files(&[], &asked));
        assert_eq!(*asked.borrow(), [r"C:\a\x.exe", "C:/b/x.exe", r"C:\x.exe"]);
    }

    #[test]
    fn a_name_with_a_directory_is_not_searched_for() {
        let asked = RefCell::default();
        // Where a search would find another file of the same name.
        let elsewhere = [r"C:\Windows\System32\pwsh.exe", r"C:\Windows\System32\tool.exe"];
        let named = [
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\tools\PWSH.EXE",
            "C:/tools/pwsh.exe",
            r".\pwsh.exe",
            r"bin\pwsh.exe",
            r"..\pwsh.exe",
            r"\\server\tools\pwsh.exe",
            "C:pwsh.exe",
        ];
        for name in named {
            assert_eq!(resolve_program(name, PATH, files(&elsewhere, &asked)), found(name), "{name}");
        }
        // A name that ends in `.exe` is the file: nothing is asked.
        assert_eq!(*asked.borrow(), Vec::<String>::new());
    }

    #[test]
    fn a_name_with_a_directory_and_no_exe_takes_it_when_that_file_exists() {
        let asked = RefCell::default();
        let existing = [r"C:\msys64\usr\bin\bash.exe", r"C:\tools\run.cmd"];
        let resolve = |name: &str| resolve_program(name, PATH, files(&existing, &asked));
        assert_eq!(resolve(r"C:\msys64\usr\bin\bash"), found(r"C:\msys64\usr\bin\bash.exe"));
        // Another extension stays, since no `run.cmd.exe` exists.
        assert_eq!(resolve(r"C:\tools\run.cmd"), found(r"C:\tools\run.cmd"));
        // Neither exists: the name as given, for `CreateProcessW` to refuse.
        assert_eq!(resolve(r"C:\tools\missing"), found(r"C:\tools\missing"));
        // Only the name with `.exe` was asked about, never `PATH`.
        assert_eq!(
            *asked.borrow(),
            [r"C:\msys64\usr\bin\bash.exe", r"C:\tools\run.cmd.exe", r"C:\tools\missing.exe"]
        );
    }

    #[test]
    fn a_program_that_path_does_not_have_is_not_found() {
        assert_eq!(resolve("no-such-program", &[]), Resolved::NotFound);
        assert_eq!(resolve("no-such-program.exe", &[r"C:\Elsewhere\no-such-program.exe"]), Resolved::NotFound);
        assert_eq!(resolve("", &[r"C:\Windows\System32\.EXE"]), Resolved::NotFound);
    }

    #[test]
    fn an_exe_suffix_is_told_whatever_its_case() {
        for name in ["a.exe", "A.EXE", r"C:\x\pwsh.Exe", ".exe"] {
            assert!(has_suffix(name, ".exe"), "{name}");
        }
        for name in ["a", "exe", "a.ex", "a.exe.bak", "", "é"] {
            assert!(!has_suffix(name, ".exe"), "{name:?}");
        }
    }

    fn vars(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn map(list: &[(&str, &str)]) -> BTreeMap<String, String> {
        vars(list).into_iter().collect()
    }

    #[test]
    fn the_program_is_the_command_else_a_nonempty_comspec_else_cmd() {
        let mut env = BTreeMap::new();
        assert_eq!(program(Some("pwsh.exe"), &env), "pwsh.exe");
        assert_eq!(program(None, &env), "cmd.exe");
        // An empty one counts as unset.
        env.insert("COMSPEC".to_string(), String::new());
        assert_eq!(program(None, &env), "cmd.exe");
        env.insert(
            "ComSpec".to_string(),
            r"C:\Windows\System32\cmd.exe".to_string(),
        );
        assert_eq!(program(None, &env), r"C:\Windows\System32\cmd.exe");
        // The command wins over it.
        assert_eq!(program(Some("pwsh.exe"), &env), "pwsh.exe");
    }

    #[test]
    fn an_exact_environment_is_the_given_one() {
        let given = map(&[("Path", r"C:\tools"), ("TERM", "xterm-256color")]);
        let own = vars(&[("PATH", r"C:\Windows"), ("USERNAME", "me")]);
        assert_eq!(
            environment(&given, true, own),
            vars(&[("Path", r"C:\tools"), ("TERM", "xterm-256color")])
        );
    }

    #[test]
    fn given_names_replace_own_names_of_any_case() {
        let given = map(&[("Path", r"C:\tools"), ("TERM", "xterm-256color")]);
        let own = vars(&[
            ("USERNAME", "me"),
            ("PATH", r"C:\Windows"),
            ("term", "dumb"),
            ("windir", r"C:\Windows"),
        ]);
        // This process's own first, in its order, then the given ones.
        assert_eq!(
            environment(&given, false, own),
            vars(&[
                ("USERNAME", "me"),
                ("windir", r"C:\Windows"),
                ("Path", r"C:\tools"),
                ("TERM", "xterm-256color"),
            ])
        );
        assert_eq!(
            environment(&BTreeMap::new(), false, vars(&[("A", "1")])),
            vars(&[("A", "1")])
        );
    }

    #[test]
    fn a_variable_is_read_by_its_name_in_any_case() {
        let env = vars(&[("Path", r"C:\tools"), ("PATHEXT", ".COM;.EXE"), ("Empty", "")]);
        assert_eq!(variable(&env, "PATH"), r"C:\tools");
        assert_eq!(variable(&env, "path"), r"C:\tools");
        assert_eq!(variable(&env, "PathExt"), ".COM;.EXE");
        assert_eq!(variable(&env, "EMPTY"), "");
        // An unset one reads as empty, which searches nothing.
        assert_eq!(variable(&env, "COMSPEC"), "");
        assert_eq!(resolve_program("cmd.exe", variable(&env, "COMSPEC"), |_| true), Resolved::NotFound);
    }
}
