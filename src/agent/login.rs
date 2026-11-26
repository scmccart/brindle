//! What a login method offers and what a terminal login runs (OpenSpec
//! change `agents-login`, design decision 9): the profile's command with
//! the method's arguments appended, the method's environment over the
//! profile's minus the variables that change how programs are found,
//! loaded or trusted ([`DENIED_ENV`]), and the commands a login is
//! withheld from because they parse their arguments again.
//!
//! A whole pure module (`fcis R2`, `R3`; `PURE_MODULES`): the zone passes
//! the profile's command and whether the endpoint's variable names ignore
//! case.

use std::collections::BTreeMap;

use crate::agent::transcript::{LoginKind, LoginMethod};

/// The environment variables a terminal login never sets, whatever the
/// agent asks: exact names, `PREFIX*` and `*SUFFIX`. Compared ignoring
/// case where the endpoint's names do (Windows); `*_PROXY` in any case
/// everywhere, as programs read `http_proxy` as well as `HTTP_PROXY`.
pub const DENIED_ENV: [&str; 21] = [
    "PATH",
    "PATHEXT",
    "LD_*",
    "DYLD_*",
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHON*",
    "RUBYOPT",
    "RUBYLIB",
    "PERL5OPT",
    "PERL5LIB",
    "*_PROXY",
    "SSL_CERT_*",
    "NODE_EXTRA_CA_CERTS",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "GIT_SSH*",
    "BASH_ENV",
    "ENV",
    "ZDOTDIR",
    "COMSPEC",
];

/// Whether `name` matches `pattern` ([`DENIED_ENV`]'s form).
fn matches(pattern: &str, name: &str, ignore_case: bool) -> bool {
    let (pattern, name) = if ignore_case || pattern == "*_PROXY" {
        (pattern.to_ascii_uppercase(), name.to_ascii_uppercase())
    } else {
        (pattern.to_string(), name.to_string())
    };
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        name == pattern
    }
}

/// Whether a terminal login leaves variable `name` out.
pub fn denied(name: &str, ignore_case: bool) -> bool {
    DENIED_ENV.iter().any(|pattern| matches(pattern, name, ignore_case))
}

/// The agent profile's command, which a terminal login runs again with the
/// method's arguments, and whether its endpoint's variable names ignore
/// case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
    pub ignore_case: bool,
}

/// What a terminal login runs, in a terminal zone beside the agent zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLogin {
    /// The profile's program.
    pub command: String,
    /// The profile's arguments, then the method's.
    pub args: Vec<String>,
    /// The method's arguments, as its button shows them.
    pub appended: Vec<String>,
    /// The method's variables that are set, over the profile's.
    pub env: BTreeMap<String, String>,
    /// The method's variables left out ([`DENIED_ENV`]), sorted.
    pub dropped: Vec<String>,
}

impl TerminalLogin {
    /// What the method's button says before it is chosen: the arguments,
    /// each variable set, then the ones not applied.
    pub fn shown(&self) -> String {
        let mut parts = Vec::new();
        if !self.appended.is_empty() {
            parts.push(format!("Runs with {}", self.appended.join(" ")));
        }
        if !self.env.is_empty() {
            parts.push(format!("sets {}", self.env_text()));
        }
        if !self.dropped.is_empty() {
            parts.push(format!("not applied: {}", self.dropped.join(", ")));
        }
        parts.join("; ")
    }

    /// The variables set, `K=V` comma-separated (the dump's `env=`).
    pub fn env_text(&self) -> String {
        self.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
    }
}

/// Why a terminal method isn't offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withheld {
    pub why: String,
}

/// What a login method offers in the zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodOffer {
    /// The agent logs in itself (`AGENT_LOGIN`).
    Agent,
    /// Brindle runs it in a terminal zone.
    Terminal(TerminalLogin),
    /// A terminal method this profile's command can't run, and why.
    Withheld(Withheld),
}

impl MethodOffer {
    pub fn usable(&self) -> bool {
        match self {
            Self::Agent | Self::Terminal(_) => true,
            Self::Withheld(_) => false,
        }
    }
}

/// The program's name as a word, from a path in either platform's form
/// (text, not `Path`, which splits only on `/` on Linux), lowercased.
fn program_name(program: &str) -> String {
    program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase()
}

/// Why `command` can't run a terminal login, if it can't: it parses its
/// arguments again, so the method's would be read as something else
/// (`cmd /c`, a batch file), or it runs them elsewhere (`ssh`).
pub fn reparses(command: &AgentCommand) -> Option<Withheld> {
    let name = program_name(&command.program);
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    let why = if name.ends_with(".bat") || name.ends_with(".cmd") {
        "the login can't be run through a batch file".to_string()
    } else if stem == "cmd" {
        let switch = command.args.iter().map(|a| a.to_ascii_lowercase()).find(|a| a == "/c" || a == "/k")?;
        format!("the login can't be run through `cmd {switch}`")
    } else if stem == "ssh" {
        "the login can't be run through `ssh`".to_string()
    } else {
        return None;
    };
    Some(Withheld { why })
}

/// What a terminal method with `args` and `env` runs for `command`.
pub fn terminal_login(
    command: &AgentCommand,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<TerminalLogin, Withheld> {
    if let Some(withheld) = reparses(command) {
        return Err(withheld);
    }
    let (dropped, applied): (Vec<_>, Vec<_>) = env.iter().partition(|(name, _)| denied(name, command.ignore_case));
    Ok(TerminalLogin {
        command: command.program.clone(),
        args: command.args.iter().chain(args).cloned().collect(),
        appended: args.to_vec(),
        env: applied.into_iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        dropped: dropped.into_iter().map(|(k, _)| k.clone()).collect(),
    })
}

/// What `method` offers for `command`.
pub fn offer(command: &AgentCommand, method: &LoginMethod) -> MethodOffer {
    match &method.kind {
        LoginKind::Agent => MethodOffer::Agent,
        LoginKind::Terminal { args, env } => match terminal_login(command, args, env) {
            Ok(login) => MethodOffer::Terminal(login),
            Err(withheld) => MethodOffer::Withheld(withheld),
        },
    }
}

/// The profile's environment `base` with a login's `applied` variables
/// over it; a name `base` has in another case is replaced when names
/// ignore case.
pub fn overlay(
    base: &BTreeMap<String, String>,
    applied: &BTreeMap<String, String>,
    ignore_case: bool,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = base
        .iter()
        .filter(|(name, _)| !(ignore_case && applied.keys().any(|own| own.eq_ignore_ascii_case(name))))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    env.extend(applied.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str, args: &[&str]) -> AgentCommand {
        AgentCommand { program: program.into(), args: args.iter().map(|a| a.to_string()).collect(), ignore_case: false }
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn every_denied_name_and_pattern_is_left_out() {
        let names = [
            "PATH",
            "PATHEXT",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "NODE_OPTIONS",
            "NODE_PATH",
            "PYTHONPATH",
            "PYTHONSTARTUP",
            "RUBYOPT",
            "RUBYLIB",
            "PERL5OPT",
            "PERL5LIB",
            "HTTPS_PROXY",
            "http_proxy",
            "No_Proxy",
            "ALL_PROXY",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NODE_EXTRA_CA_CERTS",
            "REQUESTS_CA_BUNDLE",
            "CURL_CA_BUNDLE",
            "GIT_SSH",
            "GIT_SSH_COMMAND",
            "BASH_ENV",
            "ENV",
            "ZDOTDIR",
            "COMSPEC",
        ];
        for name in names {
            assert!(denied(name, false), "{name}");
            assert!(denied(name, true), "{name}");
        }
        for name in ["LOGIN_MODE", "HOME", "PATHS", "MY_PATH", "ENVIRONMENT", "NODE_ENV", "SSL_KEY", "LANG"] {
            assert!(!denied(name, false), "{name}");
            assert!(!denied(name, true), "{name}");
        }
    }

    /// Windows' names ignore case, so `Path` and `ld_preload` are denied
    /// there; elsewhere a lowercase `path` is another variable. `*_PROXY`
    /// is denied in any case everywhere.
    #[test]
    fn case_is_ignored_where_the_endpoint_ignores_it() {
        for name in ["Path", "pathext", "ld_preload", "PythonHome", "ComSpec", "git_ssh_command"] {
            assert!(denied(name, true), "{name}");
            assert!(!denied(name, false), "{name}");
        }
        assert!(denied("https_proxy", false));
    }

    #[test]
    fn a_plan_appends_the_methods_arguments_and_sets_its_allowed_variables() {
        let plan = terminal_login(
            &command("agy", &["acp"]),
            &["--login".into()],
            &env(&[("LOGIN_MODE", "device"), ("PATH", "/tmp/evil"), ("LD_PRELOAD", "x.so")]),
        )
        .unwrap();
        assert_eq!(plan.command, "agy");
        assert_eq!(plan.args, ["acp", "--login"]);
        assert_eq!(plan.appended, ["--login"]);
        assert_eq!(plan.env, env(&[("LOGIN_MODE", "device")]));
        assert_eq!(plan.dropped, ["LD_PRELOAD", "PATH"]);
        assert_eq!(plan.env_text(), "LOGIN_MODE=device");
        assert_eq!(plan.shown(), "Runs with --login; sets LOGIN_MODE=device; not applied: LD_PRELOAD, PATH");
        let bare = terminal_login(&command("agy", &[]), &[], &BTreeMap::new()).unwrap();
        assert_eq!(bare.shown(), "");
    }

    #[test]
    fn commands_that_reparse_their_arguments_are_withheld() {
        let withheld = |program: &str, args: &[&str]| reparses(&command(program, args)).map(|w| w.why);
        assert_eq!(withheld("cmd", &["/c", "npx", "agent"]).as_deref(), Some("the login can't be run through `cmd /c`"));
        assert_eq!(withheld("cmd.exe", &["/k", "agent"]).as_deref(), Some("the login can't be run through `cmd /k`"));
        assert_eq!(withheld("C:\\Windows\\System32\\CMD.EXE", &["/C", "a"]).as_deref(), Some("the login can't be run through `cmd /c`"));
        assert_eq!(withheld("x.CMD", &[]).as_deref(), Some("the login can't be run through a batch file"));
        assert_eq!(withheld("C:/tools/agent.bat", &["--acp"]).as_deref(), Some("the login can't be run through a batch file"));
        assert_eq!(withheld("ssh", &["host", "agent"]).as_deref(), Some("the login can't be run through `ssh`"));
        assert_eq!(withheld("C:/tools/ssh.exe", &["host"]).as_deref(), Some("the login can't be run through `ssh`"));
        assert_eq!(withheld("/usr/bin/ssh", &[]).as_deref(), Some("the login can't be run through `ssh`"));
        // `cmd` without a switch that runs a command, and programs merely
        // named like them, are run as they are.
        assert_eq!(withheld("cmd", &[]), None);
        assert_eq!(withheld("/opt/cmdline/agy", &["acp"]), None);
        assert_eq!(withheld("sshd-agent", &[]), None);
        assert_eq!(withheld("npx", &["agent"]), None);
    }

    /// An SSH *host* profile runs its own command on the host (Brindle's
    /// spawn passes argv through the endpoint's helper): only the command
    /// decides, so it isn't withheld.
    #[test]
    fn an_ssh_host_profile_is_not_withheld() {
        let method = LoginMethod {
            id: "cli-login".into(),
            name: "Log in".into(),
            description: None,
            kind: LoginKind::Terminal { args: vec!["login".into()], env: BTreeMap::new() },
        };
        let MethodOffer::Terminal(plan) = offer(&command("agy", &["acp"]), &method) else { panic!("offered") };
        assert_eq!(plan.args, ["acp", "login"]);
    }

    #[test]
    fn each_kind_offers_what_it_is() {
        let agent = LoginMethod { id: "a".into(), name: "A".into(), description: None, kind: LoginKind::Agent };
        assert_eq!(offer(&command("cmd", &["/c", "x"]), &agent), MethodOffer::Agent);
        let terminal = LoginMethod {
            kind: LoginKind::Terminal { args: vec!["login".into()], env: BTreeMap::new() },
            ..agent.clone()
        };
        let withheld = offer(&command("cmd", &["/c", "x"]), &terminal);
        assert!(!withheld.usable());
        assert_eq!(withheld, MethodOffer::Withheld(Withheld { why: "the login can't be run through `cmd /c`".into() }));
        assert!(offer(&command("x", &[]), &terminal).usable());
        assert!(MethodOffer::Agent.usable());
    }

    #[test]
    fn the_overlay_sets_the_logins_variables_over_the_profiles() {
        let base = env(&[("Path", "C:/bin"), ("LOGIN_MODE", "web"), ("HOME", "/h")]);
        let applied = env(&[("login_mode", "device")]);
        assert_eq!(overlay(&base, &applied, true), env(&[("Path", "C:/bin"), ("HOME", "/h"), ("login_mode", "device")]));
        assert_eq!(
            overlay(&base, &applied, false),
            env(&[("Path", "C:/bin"), ("LOGIN_MODE", "web"), ("HOME", "/h"), ("login_mode", "device")])
        );
    }
}
