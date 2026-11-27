//! What an agent session's start asks the terminal server for
//! (`SPAWN_AGENT`): the profile's command and arguments, the directory it
//! starts in, and its environment (design.md decision 2).
//!
//! Pure: the caller reads Brindle's environment, its home directory and
//! the launching command's directory, and passes them in.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::agent::transcript::Conversation;
use crate::config::{Config, Profile, ProfileKind};
use crate::remote::Host;
use crate::remote::protocol::client::{AgentConversation, SpawnStart};

/// The variables that describe a terminal, which an agent (on plain pipes,
/// with no grid) doesn't get from Brindle: those Brindle sets for its
/// terminals and any it inherited. Brindle sets no shell-integration
/// variables of its own (the scripts are sourced from the user's rc files
/// and read none), so there are none of those to drop.
pub const TERMINAL_ONLY: [&str; 4] = ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"];

/// What opened the agent tab, which decides the directory a profile with
/// no `cwd` starts in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenedBy {
    /// `brindle agent`, run in this directory (`None` when it couldn't be
    /// read).
    AgentCommand(Option<PathBuf>),
    /// The profile menu, the picker, a shortcut, ctrl-shift-t, a split or
    /// the default profile at launch: no launching shell of its own.
    Window,
}

/// An agent session's start, as `SPAWN_AGENT` sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpawnRequest {
    /// The profile's command (an agent profile always has one: the config
    /// is refused without it).
    pub command: String,
    pub args: Vec<String>,
    /// Where the agent starts: `None` leaves it to the server (the home
    /// directory there).
    pub cwd: Option<String>,
    /// The agent's complete environment; the server adds nothing.
    pub env: BTreeMap<String, String>,
    /// The profile's name, which the session's listing carries.
    pub profile: String,
    /// A conversation for the agent to continue instead of starting one
    /// (`SPAWN_AGENT`'s `conversation`): a continue in a new session.
    pub conversation: Option<AgentConversation>,
    /// `Some(Choose)`: the agent is initialized and then waits for a
    /// conversation to be chosen (`SPAWN_AGENT`'s `start`, OpenSpec change
    /// `agents-session-list` decision 1). Never with `conversation`.
    pub start: Option<SpawnStart>,
}

impl AgentSpawnRequest {
    /// This start choosing: initialized, it waits for `AGENT_CHOOSE`
    /// instead of starting a conversation.
    pub fn choosing(self) -> Self {
        Self { start: Some(SpawnStart::Choose), conversation: None, ..self }
    }

    /// This start continuing the listed conversation `id` of directory
    /// `cwd` (a choice from a list a running zone's agent gave), in that
    /// directory, as [`Self::continuing`] does a zone's own.
    pub fn continuing_listed(self, id: &str, cwd: &str) -> Self {
        Self {
            cwd: Some(cwd.to_string()),
            conversation: Some(AgentConversation { id: id.to_string(), cwd: cwd.to_string() }),
            start: None,
            ..self
        }
    }

    /// This start continuing `conversation` (an `Offer::Respawn`), in the
    /// conversation's directory, which ACP ties it to (the server runs it
    /// there whatever `cwd` says).
    pub fn continuing(self, conversation: &Conversation) -> Self {
        Self {
            cwd: Some(conversation.cwd.clone()),
            conversation: Some(AgentConversation { id: conversation.id.clone(), cwd: conversation.cwd.clone() }),
            ..self
        }
    }

    /// What `AGENT_RESTART` sends for this start (an `Offer::Restart`): the
    /// command, its arguments and its environment. The session keeps its
    /// directory, the conversation's.
    pub fn restart(self) -> AgentRestartRequest {
        AgentRestartRequest { command: self.command, args: self.args, env: self.env }
    }
}

/// An ended agent's restart, as `AGENT_RESTART` sends it: built from the
/// profile as a new zone's start is ([`agent_spawn_request`], then
/// [`AgentSpawnRequest::restart`]), so the environment rules are one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRestartRequest {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// The agent profile a continue runs: the one named `name` (the profile
/// the zone's session was started with), while it is still an agent
/// profile. `None` when it is gone or is a terminal profile now.
pub fn continuing_profile<'a>(name: &str, config: &'a Config) -> Option<&'a Profile> {
    config.profile(name).filter(|profile| profile.kind() == ProfileKind::Agent)
}

/// Where an agent of `profile` starts on `host`: `asked` (`-d`) wins, then
/// the profile's `cwd` (with `~` as `home` on this machine; as written on
/// a remote endpoint, whose server expands it), then, for `brindle agent`
/// on this machine, the directory it ran in, else `home`. On a remote
/// endpoint the launching command's directory and this machine's home mean
/// nothing, so with neither `-d` nor a `cwd` the server starts the agent in
/// the home directory there.
pub fn directory(profile: &Profile, host: &Host, asked: Option<PathBuf>, opened: &OpenedBy, home: Option<&Path>) -> Option<PathBuf> {
    let profile_dir =
        profile.cwd.as_deref().filter(|dir| !dir.is_empty()).and_then(|_| profile.working_directory(home));
    let fallback = || match (host.is_local(), opened) {
        (true, OpenedBy::AgentCommand(Some(dir))) => Some(dir.clone()),
        (true, OpenedBy::AgentCommand(None) | OpenedBy::Window) => home.map(Path::to_path_buf),
        (false, OpenedBy::AgentCommand(_) | OpenedBy::Window) => None,
    };
    asked.or(profile_dir).or_else(fallback)
}

/// The environment an agent of `profile` runs with. On this machine: as a
/// terminal's (`vars`, Brindle's own, with the profile's `env` over them,
/// names compared case-insensitively when `ignore_case`, as on Windows),
/// minus the [`TERMINAL_ONLY`] variables Brindle set or inherited. On a
/// remote endpoint: the profile's `env`, over the endpoint's own. A
/// variable the profile's `env` sets itself is kept, whatever its name.
fn environment(
    profile: &Profile,
    host: &Host,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    ignore_case: bool,
) -> BTreeMap<String, String> {
    if !host.is_local() {
        return profile.env.clone();
    }
    let terminal_only = |name: &OsString| {
        let name = name.to_string_lossy();
        TERMINAL_ONLY.iter().any(|own| if ignore_case { own.eq_ignore_ascii_case(&name) } else { **own == *name })
    };
    let inherited: Vec<(OsString, OsString)> = vars.into_iter().filter(|(name, _)| !terminal_only(name)).collect();
    let set: HashMap<String, String> = profile.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    crate::terminal::merged_env(inherited, set, ignore_case)
}

/// The `SPAWN_AGENT` request for an agent of `profile` on `host`, opened by
/// `opened` with `asked` as its `-d`; `vars`, `home` and `ignore_case` are
/// this machine's (see [`directory`] and [`environment`]).
pub fn agent_spawn_request(
    profile: &Profile,
    host: &Host,
    asked: Option<PathBuf>,
    opened: &OpenedBy,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    home: Option<&Path>,
    ignore_case: bool,
) -> AgentSpawnRequest {
    AgentSpawnRequest {
        command: profile.command.clone().unwrap_or_default(),
        args: profile.args.clone(),
        cwd: directory(profile, host, asked, opened, home).map(|dir| dir.to_string_lossy().into_owned()),
        env: environment(profile, host, vars, ignore_case),
        profile: profile.name.clone(),
        conversation: None,
        start: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RemoteTarget};

    fn agent(extra: &str) -> Profile {
        let text = format!("[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = \"fake-acp\"\nargs = [\"--acp\"]\n{extra}");
        Config::parse(&text).unwrap().config.profiles.remove(0)
    }

    fn ssh() -> Host {
        let profile = agent("remote = { type = \"ssh\", host = \"A8\" }\n");
        let target: RemoteTarget = profile.remote.clone().unwrap();
        Host::Remote(target)
    }

    fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect()
    }

    const HOME: &str = "/home/u";

    fn request(profile: &Profile, host: &Host, asked: Option<&str>, opened: &OpenedBy) -> AgentSpawnRequest {
        let own = vars(&[("PATH", "/usr/bin"), ("TERM", "xterm-256color"), ("LANG", "C.UTF-8")]);
        agent_spawn_request(profile, host, asked.map(PathBuf::from), opened, own, Some(Path::new(HOME)), false)
    }

    #[test]
    fn the_request_runs_the_profiles_command_and_arguments() {
        let request = request(&agent(""), &Host::Local, None, &OpenedBy::Window);
        assert_eq!((request.command.as_str(), request.args.as_slice()), ("fake-acp", &["--acp".to_string()][..]));
        assert_eq!(request.profile, "Fake");
    }

    #[test]
    fn each_terminal_variable_is_removed() {
        for name in TERMINAL_ONLY {
            let own = vars(&[(name, "x"), ("PATH", "/usr/bin")]);
            let request = agent_spawn_request(&agent(""), &Host::Local, None, &OpenedBy::Window, own, None, false);
            assert_eq!(request.env.get(name), None, "{name}");
            assert_eq!(request.env.get("PATH").map(String::as_str), Some("/usr/bin"), "{name}");
        }
    }

    /// Brindle's environment under the profile's, as a terminal's, minus
    /// the terminal's variables; the startup tokens go, as for terminals.
    #[test]
    fn the_environment_is_brindles_under_the_profiles() {
        let own = vars(&[
            ("PATH", "/usr/bin"),
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("MODEL", "big"),
            ("XDG_ACTIVATION_TOKEN", "tok"),
        ]);
        let profile = agent("env = { MODEL = \"small\", EXTRA = \"1\" }\n");
        let request = agent_spawn_request(&profile, &Host::Local, None, &OpenedBy::Window, own, None, false);
        let env: Vec<(&str, &str)> = request.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env, [("EXTRA", "1"), ("MODEL", "small"), ("PATH", "/usr/bin")]);
    }

    /// A variable the profile sets itself is the user's choice, kept even
    /// when it is one of a terminal's.
    #[test]
    fn a_terminal_variable_the_profile_sets_is_kept() {
        let own = vars(&[("TERM", "xterm-256color")]);
        let profile = agent("env = { TERM = \"dumb\" }\n");
        let request = agent_spawn_request(&profile, &Host::Local, None, &OpenedBy::Window, own, None, false);
        assert_eq!(request.env.get("TERM").map(String::as_str), Some("dumb"));
    }

    /// On Windows names are one variable whatever their case: an inherited
    /// `Term` goes, and a profile's `path` replaces Brindle's `Path` under
    /// the profile's spelling.
    #[test]
    fn the_windows_merge_ignores_case() {
        let own = vars(&[("Path", r"C:\Windows"), ("Term", "x"), ("colorterm", "y"), ("TEMP", r"C:\t")]);
        let profile = agent("env = { path = 'C:\\tools' }\n");
        let request = agent_spawn_request(&profile, &Host::Local, None, &OpenedBy::Window, own.clone(), None, true);
        let env: Vec<(&str, &str)> = request.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env, [("TEMP", r"C:\t"), ("path", r"C:\tools")]);
        let request = agent_spawn_request(&profile, &Host::Local, None, &OpenedBy::Window, own, None, false);
        assert!(request.env.contains_key("Term"), "case counts elsewhere: {:?}", request.env);
    }

    #[test]
    fn a_remote_agent_gets_the_profiles_environment_only() {
        let profile = agent("env = { MODEL = \"small\" }\nremote = { type = \"ssh\", host = \"A8\" }\n");
        let request = request(&profile, &ssh(), None, &OpenedBy::Window);
        assert_eq!(request.env, BTreeMap::from([("MODEL".to_string(), "small".to_string())]));
    }

    /// `-d`, then the profile's `cwd`, then for `brindle agent` the
    /// directory it ran in, else home.
    #[test]
    fn the_directory_is_asked_then_the_profiles_then_the_commands_then_home() {
        let here = OpenedBy::AgentCommand(Some(PathBuf::from("/home/u/src/brindle")));
        let with_cwd = agent("cwd = \"/srv\"\n");
        let plain = agent("");
        let dir = |profile: &Profile, asked: Option<&str>, opened: &OpenedBy| request(profile, &Host::Local, asked, opened).cwd;
        assert_eq!(dir(&with_cwd, Some("/tmp"), &here).as_deref(), Some("/tmp"), "-d wins");
        assert_eq!(dir(&with_cwd, None, &here).as_deref(), Some("/srv"), "then the profile's cwd");
        assert_eq!(dir(&plain, None, &here).as_deref(), Some("/home/u/src/brindle"), "then where brindle agent ran");
        assert_eq!(dir(&plain, None, &OpenedBy::Window).as_deref(), Some(HOME), "else home");
        assert_eq!(dir(&plain, None, &OpenedBy::AgentCommand(None)).as_deref(), Some(HOME));
        assert_eq!(dir(&with_cwd, None, &OpenedBy::Window).as_deref(), Some("/srv"));
        assert_eq!(dir(&agent("cwd = \"\"\n"), None, &OpenedBy::Window).as_deref(), Some(HOME), "an empty cwd is none");
        let nowhere = agent_spawn_request(&plain, &Host::Local, None, &OpenedBy::Window, Vec::new(), None, false);
        assert_eq!(nowhere.cwd, None, "no home: the server's own");
    }

    #[test]
    fn a_tilde_in_the_profiles_cwd_is_home_here_and_left_to_a_remote_server() {
        let local = agent("cwd = \"~/src\"\n");
        // Joined onto this machine's home, so with its separator (`\` on Windows).
        let joined = Path::new(HOME).join("src").to_string_lossy().into_owned();
        assert_eq!(request(&local, &Host::Local, None, &OpenedBy::Window).cwd, Some(joined));
        let tilde = agent("cwd = \"~\"\n");
        assert_eq!(request(&tilde, &Host::Local, None, &OpenedBy::Window).cwd.as_deref(), Some(HOME));
        let remote = agent("cwd = \"~/src\"\nremote = { type = \"ssh\", host = \"A8\" }\n");
        assert_eq!(request(&remote, &ssh(), None, &OpenedBy::Window).cwd.as_deref(), Some("~/src"));
    }

    /// The launching command's directory and this machine's home mean
    /// nothing on another machine: the server there starts in its home.
    #[test]
    fn a_remote_agent_without_a_directory_leaves_it_to_its_server() {
        let remote = agent("remote = { type = \"ssh\", host = \"A8\" }\n");
        let here = OpenedBy::AgentCommand(Some(PathBuf::from("/home/u/src")));
        assert_eq!(request(&remote, &ssh(), None, &here).cwd, None);
        assert_eq!(request(&remote, &ssh(), None, &OpenedBy::Window).cwd, None);
        assert_eq!(request(&remote, &ssh(), Some("/srv"), &here).cwd.as_deref(), Some("/srv"), "-d is sent as given");
    }

    fn conversation() -> Conversation {
        Conversation {
            id: "sess-1".into(),
            cwd: "/home/u/src".into(),
            continuation: crate::agent::transcript::Continuation::Resume,
            title: None,
        }
    }

    /// A restart is a new zone's start without its directory: the same
    /// command, arguments and environment, terminal variables dropped and
    /// the profile's over Brindle's, here and on a remote endpoint.
    #[test]
    fn a_restart_takes_a_new_zones_command_arguments_and_environment() {
        let own = vars(&[("PATH", "/usr/bin"), ("TERM", "xterm-256color"), ("MODEL", "big")]);
        let profile = agent("env = { MODEL = \"small\" }\ncwd = \"/srv\"\n");
        for host in [Host::Local, ssh()] {
            let start = agent_spawn_request(&profile, &host, None, &OpenedBy::Window, own.clone(), None, false);
            let restart = start.clone().restart();
            assert_eq!(restart, AgentRestartRequest { command: start.command, args: start.args, env: start.env.clone() });
            assert_eq!(restart.env.get("TERM"), None, "{host:?}");
            assert_eq!(restart.env.get("MODEL").map(String::as_str), Some("small"), "{host:?}");
        }
    }

    /// A continue in a new session names the conversation and runs in its
    /// directory, whatever the profile's says.
    #[test]
    fn a_continuing_start_names_the_conversation_and_runs_in_its_directory() {
        let profile = agent("cwd = \"/srv\"\n");
        let start = request(&profile, &Host::Local, None, &OpenedBy::Window);
        assert_eq!(start.conversation, None, "a new zone's start continues nothing");
        let continuing = start.clone().continuing(&conversation());
        assert_eq!(continuing.cwd.as_deref(), Some("/home/u/src"));
        assert_eq!(continuing.conversation, Some(AgentConversation { id: "sess-1".into(), cwd: "/home/u/src".into() }));
        assert_eq!((continuing.command, continuing.args, continuing.env), (start.command, start.args, start.env));
    }

    /// A choosing start keeps the directory and asks to choose; a listed
    /// conversation continued in a new session names it and runs in its
    /// directory, choosing nothing (`agents-session-list` decisions 1, 5).
    #[test]
    fn a_choosing_start_and_a_listed_conversations_continue() {
        let profile = agent("cwd = \"/srv\"\n");
        let start = request(&profile, &Host::Local, None, &OpenedBy::Window);
        assert_eq!(start.start, None, "a new zone's start starts a conversation");
        let choosing = start.clone().choosing();
        assert_eq!(choosing.start, Some(SpawnStart::Choose));
        assert_eq!((choosing.cwd.as_deref(), choosing.conversation.as_ref()), (Some("/srv"), None));
        let listed = choosing.continuing_listed("s7", "/home/u/api");
        assert_eq!(listed.start, None, "a continue never also chooses");
        assert_eq!(listed.cwd.as_deref(), Some("/home/u/api"));
        assert_eq!(listed.conversation, Some(AgentConversation { id: "s7".into(), cwd: "/home/u/api".into() }));
    }

    /// The profile a continue runs is found by the session's profile name,
    /// ignoring case as lookups do, and only while it is an agent profile.
    #[test]
    fn a_continue_runs_the_sessions_agent_profile_only() {
        let config = Config::parse(
            "[[profiles]]\nname = \"Shell\"\n\n[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = \"fake-acp\"\n",
        )
        .unwrap()
        .config;
        assert_eq!(continuing_profile("Fake", &config).map(|p| p.name.as_str()), Some("Fake"));
        assert_eq!(continuing_profile("fake", &config).map(|p| p.name.as_str()), Some("Fake"));
        assert!(continuing_profile("Gone", &config).is_none(), "deleted");
        assert!(continuing_profile("Shell", &config).is_none(), "a terminal profile now");
    }
}
