//! Reading the user's login shell's environment, for `brindle-remote relay
//! --login-environment`: the decisions, with no IO (`fcis R1`).
//!
//! The relay runs the login shell as an interactive login shell with no
//! terminal ([`invocation`]). The shell `exec`s the hidden `login-env` role,
//! which writes its environment to file descriptor 3 as `NAME=VALUE\0`
//! records and one empty record as the terminator ([`encode_dump`]). The
//! relay reads that pipe up to the terminator ([`parse_dump`]), never to its
//! end, since a program a startup file left running may hold it open, and
//! keeps what the shell exported except the variables the shell set for
//! itself ([`captured`]). Every endpoint's own environment gets the passwd
//! basics where it lacks them ([`with_basics`]).
//!
//! Only the Linux relay calls this, but it builds and is tested everywhere
//! (`fcis R7`).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::BTreeMap;
use std::time::Duration;

/// How long the relay waits for the login shell's environment.
pub const LIMIT: Duration = Duration::from_secs(10);
/// The most a dump may hold before its terminator.
pub const MAX_DUMP: usize = 256 * 1024;
/// The most variables a dump may hold.
pub const MAX_VARS: usize = 4096;
/// How much of the shell's standard error the relay keeps for the reason:
/// the last this many bytes.
pub const STDERR_KEPT: usize = 4096;
/// The longest stderr line a reason quotes, in characters.
const MAX_REASON_LINE: usize = 300;
/// The hidden role the login shell `exec`s.
pub const ROLE: &str = "login-env";
/// The variables a shell sets for itself, which would be wrong in every
/// session: taken from the relay's own environment instead.
const SHELL_OWN: [&str; 4] = ["_", "SHLVL", "PWD", "OLDPWD"];

/// Why a login shell's environment wasn't read (`railway R1`, `R4`: one
/// consumer, which shows [`LoginEnvFailure::text`]; the variants are for the
/// tests and the log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginEnvFailure {
    /// Neither `SHELL` nor the passwd entry names a shell.
    NoShell,
    /// A shell that can't be run as `-l -i -c` (csh, tcsh, nu, …).
    Unsupported { shell: String },
    /// The shell couldn't be started.
    Spawn { shell: String, error: String },
    /// It took longer than the limit.
    TimedOut { after: Duration },
    /// The shell ended, or closed the pipe, before the dump's terminator.
    /// `status` is [`exit_text`]'s; `stderr` the tail it wrote.
    Exited { status: String, stderr: String },
    /// The dump was past [`MAX_DUMP`] or [`MAX_VARS`].
    TooLarge,
}

impl LoginEnvFailure {
    /// What `RELAY_HOST` carries and the fallback line or note shows: the
    /// reason, to follow "wasn't read (…)".
    pub fn text(&self) -> String {
        match self {
            Self::NoShell => "no login shell is set".into(),
            Self::Unsupported { shell } => format!(
                "SHELL is {shell}, which can't be run as an interactive login shell to read it"
            ),
            Self::Spawn { shell, error } => {
                format!("the login shell {shell} couldn't start: {error}")
            }
            Self::TimedOut { after } => format!("it took longer than {}", duration_text(*after)),
            Self::Exited { status, stderr } => match last_line(stderr) {
                Some(line) => {
                    format!("the login shell exited with {status} before printing it: {line}")
                }
                None => format!("the login shell exited with {status} before printing it"),
            },
            Self::TooLarge => format!(
                "it was larger than {} KiB or had more than {MAX_VARS} variables",
                MAX_DUMP / 1024
            ),
        }
    }
}

/// `10 s`, or `300 ms` for a limit that isn't whole seconds (a test's).
fn duration_text(duration: Duration) -> String {
    if duration.subsec_millis() == 0 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

/// The last line of the shell's stderr that says something, cut to
/// [`MAX_REASON_LINE`] characters. The last, not the first: an interactive
/// shell with no terminal starts with job-control warnings, and what made
/// it exit comes after them (an rc's `exec tmux`: "open terminal failed").
fn last_line(stderr: &str) -> Option<String> {
    let line = stderr.lines().map(str::trim).rfind(|line| !line.is_empty())?;
    Some(line.chars().take(MAX_REASON_LINE).collect())
}

/// An exit status's words for [`LoginEnvFailure::Exited`]: `status 1`, or
/// `signal 9` for one a signal ended.
pub fn exit_text(code: Option<i32>, signal: Option<i32>) -> String {
    match (code, signal) {
        (Some(code), _) => format!("status {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "an unknown status".into(),
    }
}

/// The command that runs `shell` as an interactive login shell, `exec`ing
/// `helper`'s `login-env` role: the program first, then its arguments.
///
/// - The POSIX family (`bash`, `zsh`, `ksh`, `mksh`, `oksh`, `sh`, `dash`,
///   `ash`): `-l -i -c 'exec "$0" login-env' HELPER`, the helper's path a
///   positional parameter, never quoted into the script. `busybox` gets
///   `sh` first, since it is a multi-call binary.
/// - `fish`: `-l -i -c "exec 'HELPER' login-env"`, the path in fish's
///   single quotes (only `\\` and `\'` escape), since fish's handling of
///   extra arguments to `-c` differs between versions.
/// - Anything else is `Unsupported`, an empty shell `NoShell`.
///
/// `-i` makes `~/.bashrc` and `~/.zshrc` count: Ubuntu's `~/.bashrc`
/// returns at once in a shell that isn't interactive, and installers (nvm,
/// cargo) append below that guard.
pub fn invocation(shell: &str, helper: &str) -> Result<Vec<String>, LoginEnvFailure> {
    if shell.is_empty() {
        return Err(LoginEnvFailure::NoShell);
    }
    let name = shell.rsplit('/').next().unwrap_or(shell);
    let owned = |items: &[&str]| items.iter().map(|item| item.to_string()).collect::<Vec<_>>();
    let posix_script = format!("exec \"$0\" {ROLE}");
    match name {
        "bash" | "zsh" | "ksh" | "mksh" | "oksh" | "sh" | "dash" | "ash" => {
            Ok(owned(&[shell, "-l", "-i", "-c", &posix_script, helper]))
        }
        "busybox" => Ok(owned(&[shell, "sh", "-l", "-i", "-c", &posix_script, helper])),
        "fish" => {
            let script = format!("exec {} {ROLE}", fish_quoted(helper));
            Ok(owned(&[shell, "-l", "-i", "-c", &script]))
        }
        _ => Err(LoginEnvFailure::Unsupported {
            shell: shell.to_string(),
        }),
    }
}

/// `text` in fish's single quotes, where only `\\` and `\'` escape.
fn fish_quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for character in text.chars() {
        if matches!(character, '\\' | '\'') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
}

/// The `login-env` role's output: one `NAME=VALUE\0` record per variable,
/// then an empty record. A name that is empty or holds `=` or a NUL, and a
/// value that holds a NUL, can't be a record and are left out.
pub fn encode_dump<N, V>(vars: impl IntoIterator<Item = (N, V)>) -> Vec<u8>
where
    N: AsRef<[u8]>,
    V: AsRef<[u8]>,
{
    let mut dump = Vec::new();
    for (name, value) in vars {
        let (name, value) = (name.as_ref(), value.as_ref());
        if name.is_empty() || name.contains(&b'=') || name.contains(&0) || value.contains(&0) {
            continue;
        }
        dump.extend_from_slice(name);
        dump.push(b'=');
        dump.extend_from_slice(value);
        dump.push(0);
    }
    dump.push(0);
    dump
}

/// What the bytes read from the dump pipe so far say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DumpRead {
    /// The terminator arrived: the variables before it.
    Complete(BTreeMap<String, String>),
    /// No terminator yet.
    Incomplete,
    /// Past [`MAX_DUMP`] bytes or [`MAX_VARS`] variables before the
    /// terminator.
    TooLarge,
}

/// Reads a dump, bounded (`railway R5`). Records that aren't UTF-8, have an
/// empty name or no `=` are dropped, as `RELAY_HOST.env` drops variables
/// that aren't UTF-8; a later record of the same name wins. Bytes after the
/// terminator are ignored.
pub fn parse_dump(bytes: &[u8]) -> DumpRead {
    let mut vars = BTreeMap::new();
    let mut records = 0usize;
    let mut start = 0usize;
    while let Some(length) = bytes[start..].iter().position(|&byte| byte == 0) {
        let end = start + length;
        if end >= MAX_DUMP {
            return DumpRead::TooLarge;
        }
        let record = &bytes[start..end];
        if record.is_empty() {
            return DumpRead::Complete(vars);
        }
        records += 1;
        if records > MAX_VARS {
            return DumpRead::TooLarge;
        }
        if let Some((name, value)) = std::str::from_utf8(record)
            .ok()
            .and_then(|record| record.split_once('='))
            .filter(|(name, _)| !name.is_empty())
        {
            vars.insert(name.to_string(), value.to_string());
        }
        start = end + 1;
    }
    if bytes.len() > MAX_DUMP {
        DumpRead::TooLarge
    } else {
        DumpRead::Incomplete
    }
}

/// What a shell that exited (`status`, [`exit_text`]'s words) leaves, once
/// the dump pipe has been drained: the dump when it was complete, else why
/// not, with the tail of its stderr.
pub fn after_exit(
    dump: &[u8],
    status: String,
    stderr: &[u8],
) -> Result<BTreeMap<String, String>, LoginEnvFailure> {
    match parse_dump(dump) {
        DumpRead::Complete(vars) => Ok(vars),
        DumpRead::TooLarge => Err(LoginEnvFailure::TooLarge),
        DumpRead::Incomplete => Err(LoginEnvFailure::Exited {
            status,
            stderr: String::from_utf8_lossy(stderr).into_owned(),
        }),
    }
}

/// Appends `more` to the kept stderr `tail`, keeping its last
/// [`STDERR_KEPT`] bytes.
pub fn keep_tail(tail: &mut Vec<u8>, more: &[u8]) {
    tail.extend_from_slice(more);
    if tail.len() > STDERR_KEPT {
        tail.drain(..tail.len() - STDERR_KEPT);
    }
}

/// The login environment `RELAY_HOST` carries: the dump, except that the
/// variables the shell set for itself (`_`, `SHLVL`, `PWD`, `OLDPWD`) are
/// the relay's own (`own`), or left out where it has none.
pub fn captured(
    own: &BTreeMap<String, String>,
    mut dump: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    for name in SHELL_OWN {
        match own.get(name) {
            Some(value) => dump.insert(name.to_string(), value.clone()),
            None => dump.remove(name),
        };
    }
    dump
}

/// The user's passwd entry, as far as a login's basics need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passwd {
    pub name: String,
    pub home: String,
    pub shell: String,
}

/// `env` with `HOME`, `USER`, `LOGNAME` and `SHELL` filled in from `passwd`
/// where they are unset or empty, as a login would set them (`docker exec`
/// often sets no `HOME` and never a `SHELL`). A value the entry leaves
/// empty fills nothing.
pub fn with_basics(
    mut env: BTreeMap<String, String>,
    passwd: Option<&Passwd>,
) -> BTreeMap<String, String> {
    let Some(passwd) = passwd else {
        return env;
    };
    for (name, value) in [
        ("HOME", &passwd.home),
        ("USER", &passwd.name),
        ("LOGNAME", &passwd.name),
        ("SHELL", &passwd.shell),
    ] {
        let unset = env.get(name).is_none_or(String::is_empty);
        if unset && !value.is_empty() {
            env.insert(name.to_string(), value.clone());
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELPER: &str = "/home/u/.cache/brindle/0.5.0-ab/brindle-remote";

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn the_posix_shells_run_the_helper_as_a_positional_parameter() {
        for shell in [
            "/bin/bash",
            "/usr/bin/zsh",
            "/bin/ksh",
            "/bin/mksh",
            "/usr/bin/oksh",
            "/bin/sh",
            "/bin/dash",
            "/bin/ash",
            "bash",
        ] {
            assert_eq!(
                invocation(shell, HELPER).unwrap(),
                [shell, "-l", "-i", "-c", "exec \"$0\" login-env", HELPER],
                "{shell}"
            );
        }
        assert_eq!(
            invocation("/bin/busybox", HELPER).unwrap(),
            ["/bin/busybox", "sh", "-l", "-i", "-c", "exec \"$0\" login-env", HELPER]
        );
    }

    /// A path with spaces, quotes and backslashes is one argument, never
    /// part of the script.
    #[test]
    fn a_helper_path_is_never_quoted_into_a_posix_script() {
        let helper = "/home/a b/it's \\ \"here\"/brindle-remote";
        let command = invocation("/bin/bash", helper).unwrap();
        assert_eq!(command[4], "exec \"$0\" login-env");
        assert_eq!(command[5], helper);
        assert_eq!(command.len(), 6);
    }

    #[test]
    fn fish_gets_the_path_in_its_single_quotes() {
        assert_eq!(
            invocation("/usr/bin/fish", HELPER).unwrap(),
            [
                "/usr/bin/fish",
                "-l",
                "-i",
                "-c",
                "exec '/home/u/.cache/brindle/0.5.0-ab/brindle-remote' login-env"
            ]
        );
        let command = invocation("fish", "/home/a b/it's \\x/h").unwrap();
        assert_eq!(command[4], r"exec '/home/a b/it\'s \\x/h' login-env");
        assert_eq!(command.len(), 5);
    }

    #[test]
    fn other_shells_are_unsupported_and_none_is_no_shell() {
        for shell in ["/bin/tcsh", "/bin/csh", "/usr/bin/nu", "xonsh", "elvish", "/usr/bin/pwsh", "/opt/weird"] {
            assert_eq!(
                invocation(shell, HELPER),
                Err(LoginEnvFailure::Unsupported {
                    shell: shell.to_string()
                }),
                "{shell}"
            );
        }
        assert_eq!(invocation("", HELPER), Err(LoginEnvFailure::NoShell));
        // A directory named like a shell is still its file name that counts.
        assert!(invocation("/bash/tcsh", HELPER).is_err());
    }

    #[test]
    fn every_failure_says_why() {
        let texts = [
            (LoginEnvFailure::NoShell, "no login shell is set"),
            (
                LoginEnvFailure::Unsupported {
                    shell: "/bin/tcsh".into(),
                },
                "SHELL is /bin/tcsh, which can't be run as an interactive login shell to read it",
            ),
            (
                LoginEnvFailure::Spawn {
                    shell: "/bin/zsh".into(),
                    error: "No such file or directory (os error 2)".into(),
                },
                "the login shell /bin/zsh couldn't start: No such file or directory (os error 2)",
            ),
            (
                LoginEnvFailure::TimedOut { after: LIMIT },
                "it took longer than 10 s",
            ),
            (
                LoginEnvFailure::TimedOut {
                    after: Duration::from_millis(300),
                },
                "it took longer than 300 ms",
            ),
            (
                LoginEnvFailure::Exited {
                    status: "status 1".into(),
                    stderr: "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\n\
                             bash: no job control in this shell\nopen terminal failed: not a terminal\n\n"
                        .into(),
                },
                "the login shell exited with status 1 before printing it: open terminal failed: not a terminal",
            ),
            (
                LoginEnvFailure::Exited {
                    status: "signal 9".into(),
                    stderr: " \n".into(),
                },
                "the login shell exited with signal 9 before printing it",
            ),
            (
                LoginEnvFailure::TooLarge,
                "it was larger than 256 KiB or had more than 4096 variables",
            ),
        ];
        for (failure, text) in texts {
            assert_eq!(failure.text(), text, "{failure:?}");
        }
        let long = LoginEnvFailure::Exited {
            status: "status 2".into(),
            stderr: "x".repeat(5000),
        };
        assert!(long.text().len() < 400);
    }

    #[test]
    fn an_exit_status_is_its_code_or_its_signal() {
        assert_eq!(exit_text(Some(1), None), "status 1");
        assert_eq!(exit_text(Some(0), Some(9)), "status 0");
        assert_eq!(exit_text(None, Some(9)), "signal 9");
        assert_eq!(exit_text(None, None), "an unknown status");
    }

    #[test]
    fn a_dump_round_trips_and_ends_with_an_empty_record() {
        let env = vars(&[("HOME", "/root"), ("PATH", "/a:/b"), ("EMPTY", ""), ("EQ", "a=b")]);
        let dump = encode_dump(env.iter().map(|(n, v)| (n.as_bytes(), v.as_bytes())));
        assert!(dump.ends_with(b"\0\0"));
        assert_eq!(parse_dump(&dump), DumpRead::Complete(env));
        assert_eq!(encode_dump(Vec::<(&[u8], &[u8])>::new()), b"\0");
        assert_eq!(parse_dump(b"\0"), DumpRead::Complete(BTreeMap::new()));
    }

    #[test]
    fn records_that_cant_be_one_are_left_out_of_a_dump() {
        let dump = encode_dump([
            (&b""[..], &b"x"[..]),
            (b"A=B", b"x"),
            (b"N\0UL", b"x"),
            (b"V", b"a\0b"),
            (b"OK", b"1"),
        ]);
        assert_eq!(dump, b"OK=1\0\0");
    }

    #[test]
    fn a_dump_without_its_terminator_is_incomplete() {
        assert_eq!(parse_dump(b""), DumpRead::Incomplete);
        assert_eq!(parse_dump(b"A=1"), DumpRead::Incomplete);
        assert_eq!(parse_dump(b"A=1\0"), DumpRead::Incomplete);
        assert_eq!(parse_dump(b"A=1\0B=2"), DumpRead::Incomplete);
        // Bytes after the terminator don't count.
        assert_eq!(
            parse_dump(b"A=1\0\0B=2\0garbage"),
            DumpRead::Complete(vars(&[("A", "1")]))
        );
    }

    #[test]
    fn bad_records_are_dropped_and_the_last_of_a_name_wins() {
        let dump = b"A=1\0\xff\xfe=x\0B=\xff\0=nameless\0noequals\0A=2\0C=\0\0";
        assert_eq!(
            parse_dump(dump),
            DumpRead::Complete(vars(&[("A", "2"), ("C", "")]))
        );
    }

    #[test]
    fn a_dump_past_either_bound_is_too_large() {
        // Bytes: a value past the bound, with or without its terminator.
        let big = format!("A={}", "x".repeat(MAX_DUMP));
        assert_eq!(parse_dump(big.as_bytes()), DumpRead::TooLarge);
        let mut ended = big.into_bytes();
        ended.extend_from_slice(b"\0\0");
        assert_eq!(parse_dump(&ended), DumpRead::TooLarge);
        // Just under the bound is fine.
        let under = format!("A={}\0\0", "x".repeat(MAX_DUMP - 4));
        assert!(matches!(parse_dump(under.as_bytes()), DumpRead::Complete(_)));
        // Variables: one past the bound.
        let many: Vec<(String, String)> = (0..=MAX_VARS).map(|i| (format!("V{i}"), "1".into())).collect();
        let dump = encode_dump(many.iter().map(|(n, v)| (n.as_bytes(), v.as_bytes())));
        assert_eq!(parse_dump(&dump), DumpRead::TooLarge);
        let dump = encode_dump(many[..MAX_VARS].iter().map(|(n, v)| (n.as_bytes(), v.as_bytes())));
        assert!(matches!(parse_dump(&dump), DumpRead::Complete(env) if env.len() == MAX_VARS));
    }

    #[test]
    fn an_exited_shell_leaves_its_dump_or_its_reason() {
        assert_eq!(
            after_exit(b"A=1\0\0", "status 0".into(), b""),
            Ok(vars(&[("A", "1")]))
        );
        assert_eq!(
            after_exit(b"A=1\0", "status 1".into(), b"oops\n"),
            Err(LoginEnvFailure::Exited {
                status: "status 1".into(),
                stderr: "oops\n".into()
            })
        );
        let big = vec![b'x'; MAX_DUMP + 1];
        assert_eq!(after_exit(&big, "status 0".into(), b""), Err(LoginEnvFailure::TooLarge));
    }

    #[test]
    fn the_stderr_tail_keeps_the_last_bytes() {
        let mut tail = Vec::new();
        keep_tail(&mut tail, b"abc");
        assert_eq!(tail, b"abc");
        keep_tail(&mut tail, &vec![b'x'; STDERR_KEPT]);
        assert_eq!(tail.len(), STDERR_KEPT);
        assert!(tail.iter().all(|&byte| byte == b'x'));
        keep_tail(&mut tail, b"end");
        assert!(tail.ends_with(b"end") && tail.len() == STDERR_KEPT);
    }

    #[test]
    fn the_shells_own_variables_come_from_the_relay() {
        let own = vars(&[("_", "/usr/bin/brindle-remote"), ("PWD", "/home/u"), ("HOME", "/home/u")]);
        let dump = vars(&[
            ("_", "/bin/env"),
            ("SHLVL", "2"),
            ("PWD", "/tmp"),
            ("OLDPWD", "/"),
            ("FOO", "1"),
            ("HOME", "/home/u"),
        ]);
        assert_eq!(
            captured(&own, dump),
            vars(&[("_", "/usr/bin/brindle-remote"), ("PWD", "/home/u"), ("FOO", "1"), ("HOME", "/home/u")])
        );
    }

    #[test]
    fn the_basics_fill_only_what_is_unset_or_empty() {
        let passwd = Passwd {
            name: "dev".into(),
            home: "/home/dev".into(),
            shell: "/bin/bash".into(),
        };
        // Missing: all four filled.
        assert_eq!(
            with_basics(vars(&[("PATH", "/bin")]), Some(&passwd)),
            vars(&[
                ("PATH", "/bin"),
                ("HOME", "/home/dev"),
                ("USER", "dev"),
                ("LOGNAME", "dev"),
                ("SHELL", "/bin/bash"),
            ])
        );
        // Empty: filled; set: kept.
        assert_eq!(
            with_basics(
                vars(&[("HOME", ""), ("USER", "other"), ("LOGNAME", "other"), ("SHELL", "/bin/zsh")]),
                Some(&passwd)
            ),
            vars(&[("HOME", "/home/dev"), ("USER", "other"), ("LOGNAME", "other"), ("SHELL", "/bin/zsh")])
        );
        // No entry: nothing changes.
        assert_eq!(with_basics(vars(&[("HOME", "")]), None), vars(&[("HOME", "")]));
        // An entry's empty field fills nothing.
        let shell_less = Passwd {
            shell: String::new(),
            ..passwd
        };
        assert_eq!(
            with_basics(BTreeMap::new(), Some(&shell_less)),
            vars(&[("HOME", "/home/dev"), ("USER", "dev"), ("LOGNAME", "dev")])
        );
    }
}
