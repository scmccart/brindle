//! What a pipe session's spawn and hang-up decide (H1 version 3's
//! `SPAWN_PIPES`; design decisions 2 and 3 of the OpenSpec change
//! `agents-sessions`), beside the platform seams that do them
//! (`{unix,windows}/piped.rs`). Pure and of no platform, so it builds and is
//! tested on every platform with Windows' values as plain numbers and its
//! paths as strings (`fcis R7`, `ports R7`).
//!
//! - **Creating the program on Windows:** its creation flags
//!   ([`CREATION_FLAGS`]: no console window, a process group of its own, a
//!   Unicode environment, an extended startup info), the startup flag that
//!   hands it the pipes ([`STARTUP_FLAGS`]), the handle list that lets it
//!   inherit only them ([`ChildEnds::handle_list`]), and its job's limit
//!   ([`JOB_LIMITS`]: the job's processes end when the holder's handle to
//!   it closes).
//! - **The program, its command line and its environment** ([`launch`]),
//!   through `conpty::cmdline` as a ConPTY session's: the environment
//!   exactly as given, the program resolved in that environment's `PATH`
//!   (`resolve_program`), quoted by `launch_line`. A bare name that is a
//!   batch file isn't found; [`batch_hint`] says so and shows the
//!   `cmd /c …` form to use.
//! - **Hanging it up** ([`HangUp`]): close its stdin, ACP's polite end;
//!   after [`GRACEFUL_SHUTDOWN`] end its group or job; after another, kill
//!   what remains.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use crate::conpty::cmdline::{self, Resolved};
use crate::pty::GRACEFUL_SHUTDOWN;

/// `CREATE_NEW_PROCESS_GROUP`.
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// `CREATE_UNICODE_ENVIRONMENT`.
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
/// `EXTENDED_STARTUPINFO_PRESENT`.
pub const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
/// `CREATE_NO_WINDOW`.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `STARTF_USESTDHANDLES`.
pub const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

/// What a pipe session's program is created with on Windows: no console
/// (so no window, and no pseudoconsole), a process group of its own (so a
/// console's Ctrl+C never reaches it), the environment block as UTF-16, and
/// the extended startup info that carries the handle list. Not
/// `CREATE_BREAKAWAY_FROM_JOB`: it runs in the job it is given.
pub const CREATION_FLAGS: u32 = CREATE_NO_WINDOW
    | CREATE_NEW_PROCESS_GROUP
    | CREATE_UNICODE_ENVIRONMENT
    | EXTENDED_STARTUPINFO_PRESENT;

/// The startup info's flags: its standard handles are the pipes' ends.
pub const STARTUP_FLAGS: u32 = STARTF_USESTDHANDLES;

/// The limit on each pipe session's job: closing the holder's handle to
/// it ends what still runs in it, so an agent dies with its holder.
pub const JOB_LIMITS: u32 = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

/// The program's ends of its three pipes, the ones it inherits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildEnds<H> {
    pub input: H,
    pub output: H,
    pub error: H,
}

impl<H: Copy + PartialEq> ChildEnds<H> {
    /// What `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` names: these and nothing
    /// else, each once, in the order stdin, stdout, stderr. Every other
    /// inheritable handle of the holder stays out of the program.
    pub fn handle_list(&self) -> Vec<H> {
        let mut list = Vec::with_capacity(3);
        for handle in [self.input, self.output, self.error] {
            if !list.contains(&handle) {
                list.push(handle);
            }
        }
        list
    }
}

/// How a pipe session's program starts on Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The file `CreateProcessW` starts, found in the environment's `PATH`.
    pub application: String,
    /// Its command line ([`cmdline::launch_line`]).
    pub line: String,
    /// Its whole environment: the spawn's, exactly.
    pub environment: Vec<(String, String)>,
}

/// Why a pipe session's program can't start; its text is the
/// `SPAWN_FAILED` message the client shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// A bare name no directory of the `PATH` has.
    NotFound { program: String },
    /// A bare name that is a batch file on the `PATH` ([`batch_hint`]).
    BatchFile { hint: String },
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { program } => {
                write!(f, "{program} is in no directory of the session's PATH")
            }
            Self::BatchFile { hint } => f.write_str(hint),
        }
    }
}

/// How `command` with `args` starts with the environment `env` (the
/// program's whole environment, decision 2) on Windows: the file it is,
/// looked up in that environment's `PATH` by [`cmdline::resolve_program`]'s
/// rules (`exists` says whether a path is a file), its command line, and
/// the environment as given. When a bare name isn't found but is a batch
/// file on the `PATH`, the error says how to run it.
pub fn launch(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    exists: impl Fn(&str) -> bool,
) -> Result<Launch, LaunchError> {
    let environment = cmdline::environment(env, true, Vec::new());
    let path = cmdline::variable(&environment, "PATH");
    let application = match cmdline::resolve_program(command, path, &exists) {
        Resolved::Found(file) => file,
        Resolved::NotFound => {
            return Err(match batch_hint(command, args, path, &exists) {
                Some(hint) => LaunchError::BatchFile { hint },
                None => LaunchError::NotFound {
                    program: command.to_owned(),
                },
            });
        }
    };
    Ok(Launch {
        application,
        line: cmdline::launch_line(command, args),
        environment,
    })
}

/// The extensions of the batch files `cmd` would run for a bare name, in
/// `PATHEXT`'s default order.
const BATCH_EXTENSIONS: [&str; 2] = [".bat", ".cmd"];

/// When `name` with `args`, a name without a directory that
/// [`cmdline::resolve_program`] didn't find, is a batch file in a
/// directory of `path_var` (`npx` is `npx.cmd`): the message that says so
/// and shows the `cmd /c` form to run it with. `None` for a name with a
/// directory, or one no directory has a `.bat` or `.cmd` of. Batch files
/// aren't run implicitly (decision 3): that would bring cmd's quoting
/// rules into every argument without the user seeing it.
pub fn batch_hint(
    name: &str,
    args: &[String],
    path_var: &str,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    if name.is_empty() || name.contains(['\\', '/', ':']) {
        return None;
    }
    let file = cmdline::search_dirs(path_var).iter().find_map(|dir| {
        BATCH_EXTENSIONS
            .iter()
            .map(|extension| cmdline::in_dir(dir, &format!("{name}{extension}")))
            .find(|file| exists(file))
    })?;
    let form = cmdline::command_line(name, args);
    Some(format!(
        "{name} is a batch file ({file}); run it as `cmd /c {form}`"
    ))
}

/// A step of a pipe session's hang-up, which its platform seam takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HangUpStep {
    /// Close the program's stdin: ACP's polite end ("close stdin,
    /// terminate subprocess").
    CloseInput,
    /// End its process group (Linux: `SIGHUP`) or job (Windows:
    /// `TerminateJobObject`, which ends every process in it).
    EndGroup,
    /// Kill what is left of the group (Linux: `SIGKILL` while it has
    /// members, the children that ignored `SIGHUP` included). On Windows
    /// the job left nothing.
    KillRemaining,
}

/// When each step comes, in graces after the hang-up.
const STEPS: [(u32, HangUpStep); 3] = [
    (0, HangUpStep::CloseInput),
    (1, HangUpStep::EndGroup),
    (2, HangUpStep::KillRemaining),
];

/// A pipe session's hang-up in progress (`HANGUP`, `STOP`, a `kill`
/// session's owner gone): its stdin is closed at once, its group or job
/// ended after [`GRACEFUL_SHUTDOWN`], and what remains killed after
/// another. The steps come whether or not the program has exited, since
/// the processes it started may not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HangUp {
    started: Instant,
    /// How many of [`STEPS`] were taken.
    taken: usize,
}

impl HangUp {
    /// A hang-up starting at `now`; its first step is due at once.
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            taken: 0,
        }
    }

    /// The steps due at `now` not yet taken, in order; each is returned
    /// once.
    pub fn due(&mut self, now: Instant) -> Vec<HangUpStep> {
        let mut due = Vec::new();
        while let Some(&(graces, step)) = STEPS.get(self.taken) {
            if now < self.started + GRACEFUL_SHUTDOWN * graces {
                break;
            }
            due.push(step);
            self.taken += 1;
        }
        due
    }

    /// When the next step is due; `None` once every step was taken.
    pub fn next_at(&self) -> Option<Instant> {
        STEPS
            .get(self.taken)
            .map(|&(graces, _)| self.started + GRACEFUL_SHUTDOWN * graces)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    /// A file system of `files`, compared without case as Windows does.
    fn files<'a>(files: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |path| files.iter().any(|file| file.eq_ignore_ascii_case(path))
    }

    // --- creation ----------------------------------------------------------

    #[test]
    fn a_pipe_program_has_no_window_its_own_group_a_unicode_environment_and_a_handle_list() {
        assert_eq!(CREATION_FLAGS, 0x0808_0600);
        assert_eq!(STARTUP_FLAGS, 0x100);
        assert_eq!(JOB_LIMITS, 0x2000);
    }

    #[test]
    fn the_program_inherits_only_its_three_pipe_ends_each_once() {
        let ends = ChildEnds {
            input: 10,
            output: 11,
            error: 12,
        };
        assert_eq!(ends.handle_list(), [10, 11, 12]);
        let shared = ChildEnds {
            input: 10,
            output: 11,
            error: 11,
        };
        assert_eq!(shared.handle_list(), [10, 11]);
    }

    #[cfg(windows)]
    #[test]
    fn the_numbers_are_windows_own() {
        // The crate, not the sibling `platform::windows` seam.
        use ::windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE as KILL;
        use ::windows::Win32::System::Threading::{
            CREATE_NEW_PROCESS_GROUP as GROUP, CREATE_NO_WINDOW as NO_WINDOW,
            CREATE_UNICODE_ENVIRONMENT as UNICODE, EXTENDED_STARTUPINFO_PRESENT as EXTENDED,
            STARTF_USESTDHANDLES as STD,
        };
        assert_eq!(CREATE_NEW_PROCESS_GROUP, GROUP.0);
        assert_eq!(CREATE_UNICODE_ENVIRONMENT, UNICODE.0);
        assert_eq!(EXTENDED_STARTUPINFO_PRESENT, EXTENDED.0);
        assert_eq!(CREATE_NO_WINDOW, NO_WINDOW.0);
        assert_eq!(STARTF_USESTDHANDLES, STD.0);
        assert_eq!(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, KILL.0);
    }

    // --- the program and its environment ------------------------------------

    #[test]
    fn a_bare_name_is_found_in_the_given_environments_path_and_quoted() {
        let given = env(&[("Path", r"C:\tools;C:\agents"), ("HOME", r"C:\Users\u")]);
        let launch = launch(
            "agent",
            &args(&["--acp", "a b"]),
            &given,
            files(&[r"C:\agents\agent.exe"]),
        )
        .expect("found");
        assert_eq!(launch.application, r"C:\agents\agent.exe");
        assert_eq!(launch.line, r#"agent --acp "a b""#);
        // Exactly the spawn's environment, nothing of the holder's.
        assert_eq!(
            launch.environment,
            [
                ("HOME".to_string(), r"C:\Users\u".to_string()),
                ("Path".to_string(), r"C:\tools;C:\agents".to_string()),
            ]
        );
    }

    #[test]
    fn a_name_with_a_directory_is_the_file_it_names() {
        let launch = launch(
            r"C:\Program Files\Agent\agent",
            &[],
            &env(&[]),
            files(&[r"C:\Program Files\Agent\agent.exe"]),
        )
        .expect("found");
        assert_eq!(launch.application, r"C:\Program Files\Agent\agent.exe");
        assert_eq!(launch.line, r#""C:\Program Files\Agent\agent""#);
    }

    #[test]
    fn cmd_with_s_gets_its_arguments_as_written() {
        let launch = launch(
            "cmd",
            &args(&["/S", "/C", r#""npx" -y "@scope/agent""#]),
            &env(&[("PATH", r"C:\Windows\System32")]),
            files(&[r"C:\Windows\System32\cmd.exe"]),
        )
        .expect("found");
        assert_eq!(launch.line, r#"cmd /S /C "npx" -y "@scope/agent""#);
    }

    #[test]
    fn a_bare_name_nowhere_on_the_path_is_not_found_naming_it() {
        let error = launch("agent", &[], &env(&[("PATH", r"C:\tools")]), files(&[]))
            .expect_err("not found");
        assert_eq!(
            error,
            LaunchError::NotFound {
                program: "agent".into()
            }
        );
        assert_eq!(
            error.to_string(),
            "agent is in no directory of the session's PATH"
        );
    }

    #[test]
    fn a_bare_batch_file_fails_with_the_hint_and_its_cmd_form() {
        let error = launch(
            "npx",
            &args(&["-y", "@zed-industries/claude-code-acp"]),
            &env(&[("PATH", r"C:\tools;C:\Program Files\nodejs\")]),
            files(&[r"C:\Program Files\nodejs\npx.cmd"]),
        )
        .expect_err("a batch file isn't started");
        assert_eq!(
            error.to_string(),
            r"npx is a batch file (C:\Program Files\nodejs\npx.cmd); run it as `cmd /c npx -y @zed-industries/claude-code-acp`"
        );
    }

    // --- the batch-file hint -------------------------------------------------

    #[test]
    fn the_hint_names_the_first_batch_file_in_path_order() {
        let path = r"C:\first;C:\second";
        let exists = files(&[r"C:\second\npx.bat", r"C:\first\npx.cmd"]);
        let hint = batch_hint("npx", &[], path, exists).expect("a hint");
        assert_eq!(
            hint,
            r"npx is a batch file (C:\first\npx.cmd); run it as `cmd /c npx`"
        );
        // Within a directory, `.bat` comes before `.cmd`, as in PATHEXT.
        let both = files(&[r"C:\first\npx.cmd", r"C:\first\npx.bat"]);
        assert!(
            batch_hint("npx", &[], path, both).is_some_and(|h| h.contains(r"C:\first\npx.bat"))
        );
    }

    #[test]
    fn the_hint_quotes_arguments_in_its_cmd_form() {
        let hint = batch_hint(
            "npx",
            &args(&["--cwd", r"C:\my project"]),
            r"C:\nodejs",
            files(&[r"C:\nodejs\npx.cmd"]),
        )
        .expect("a hint");
        assert!(
            hint.ends_with(r#"run it as `cmd /c npx --cwd "C:\my project"`"#),
            "{hint}"
        );
    }

    #[test]
    fn there_is_no_hint_for_a_name_with_a_directory_or_without_a_batch_file() {
        let exists = files(&[r"C:\nodejs\npx.cmd"]);
        assert_eq!(
            batch_hint(r"C:\nodejs\npx", &[], r"C:\nodejs", &exists),
            None
        );
        assert_eq!(batch_hint("nodejs/npx", &[], r"C:\nodejs", &exists), None);
        assert_eq!(batch_hint("", &[], r"C:\nodejs", &exists), None);
        assert_eq!(batch_hint("npm", &[], r"C:\nodejs", &exists), None);
    }

    #[test]
    fn the_hint_looks_only_in_rooted_directories_of_the_path() {
        let looked = RefCell::new(Vec::new());
        let exists = |path: &str| {
            looked.borrow_mut().push(path.to_string());
            false
        };
        assert_eq!(
            batch_hint("npx", &[], r".;bin;;C:\a;\\server\share", exists),
            None
        );
        assert_eq!(
            *looked.borrow(),
            [
                r"C:\a\npx.bat",
                r"C:\a\npx.cmd",
                r"\\server\share\npx.bat",
                r"\\server\share\npx.cmd",
            ]
        );
    }

    // --- hanging up -----------------------------------------------------------

    #[test]
    fn a_hang_up_closes_stdin_at_once() {
        let t0 = Instant::now();
        let mut hang_up = HangUp::new(t0);
        assert_eq!(hang_up.next_at(), Some(t0));
        assert_eq!(hang_up.due(t0), [HangUpStep::CloseInput]);
        assert_eq!(hang_up.due(t0), [], "each step comes once");
    }

    #[test]
    fn after_the_grace_the_group_is_ended_and_after_another_what_remains_killed() {
        let t0 = Instant::now();
        let mut hang_up = HangUp::new(t0);
        hang_up.due(t0);
        assert_eq!(hang_up.next_at(), Some(t0 + GRACEFUL_SHUTDOWN));
        assert_eq!(
            hang_up.due(t0 + GRACEFUL_SHUTDOWN - Duration::from_millis(1)),
            []
        );
        assert_eq!(hang_up.due(t0 + GRACEFUL_SHUTDOWN), [HangUpStep::EndGroup]);
        assert_eq!(hang_up.next_at(), Some(t0 + GRACEFUL_SHUTDOWN * 2));
        assert_eq!(
            hang_up.due(t0 + GRACEFUL_SHUTDOWN * 2),
            [HangUpStep::KillRemaining]
        );
        assert_eq!(hang_up.next_at(), None);
        assert_eq!(hang_up.due(t0 + GRACEFUL_SHUTDOWN * 10), []);
    }

    #[test]
    fn a_late_look_takes_every_step_due_in_order() {
        let t0 = Instant::now();
        let mut hang_up = HangUp::new(t0);
        assert_eq!(
            hang_up.due(t0 + GRACEFUL_SHUTDOWN * 5),
            [
                HangUpStep::CloseInput,
                HangUpStep::EndGroup,
                HangUpStep::KillRemaining
            ]
        );
    }
}
