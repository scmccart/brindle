//! Which environment a program started on a remote endpoint gets
//! (OpenSpec change `remote-login-environment`, design decisions 1 and 5).
//!
//! The relay reports its endpoint's environment once per link
//! (`RELAY_HOST`): its own, which is sshd's or the container engine's with
//! the passwd basics, and, over SSH, the login shell's or why it couldn't
//! be read. [`EndpointEnv::from_host`] takes that at the boundary. For each
//! message the writer sends, [`start_of`] says whether it starts a program,
//! and [`merged`] lays the message's own environment over the base that
//! program gets: the login shell's for a program Brindle execs, the
//! endpoint's own for a session that runs the login shell itself. When the
//! login shell's environment wasn't read, a program that would have used it
//! gets the endpoint's own, with a [`Fallback`] that says so on it.
//!
//! Pure: the writer passes the host it got and the message.

use std::collections::BTreeMap;

use crate::remote::protocol::client::{AgentRestart, Spawn, SpawnAgent, ToCore};
use crate::remote::protocol::relay::{LoginEnv, RelayHost};

/// What a message starts on the endpoint, which decides its base
/// environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// A session with no command: the endpoint's login shell, whose own
    /// startup files build on the endpoint's environment.
    LoginShell,
    /// A program Brindle execs directly: a terminal's command, an agent, a
    /// login zone's program, an agent started again.
    Program,
}

/// What `message` starts on the endpoint, if anything. Every `ToCore`
/// variant is named, with no `_ =>` arm (`railway R3`, `types R2`): a new
/// message fails to compile here until someone decides whether it starts a
/// program.
pub fn start_of(message: &ToCore) -> Option<Start> {
    match message {
        ToCore::Spawn(Spawn { command: None, .. }) => Some(Start::LoginShell),
        ToCore::Spawn(Spawn { command: Some(_), .. }) => Some(Start::Program),
        ToCore::SpawnAgent(_) | ToCore::AgentRestart(_) => Some(Start::Program),
        ToCore::Hello(_)
        | ToCore::Data(_)
        | ToCore::Resize(_)
        | ToCore::Close(_)
        | ToCore::Attach(_)
        | ToCore::Detach(_)
        | ToCore::List(_)
        | ToCore::Kill(_)
        | ToCore::Status(_)
        | ToCore::Stop(_)
        | ToCore::StepDown(_)
        | ToCore::Reparent(_)
        | ToCore::SetLayout(_)
        | ToCore::AgentPrompt(_)
        | ToCore::AgentCancel(_)
        | ToCore::AgentPermission(_)
        | ToCore::AgentUnqueue(_)
        | ToCore::AgentResend(_)
        | ToCore::AgentLogin(_)
        | ToCore::AgentFetch(_) => None,
    }
}

/// What the relay said of the login shell's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    /// The relay wasn't asked (a container's), or predates the field.
    NotAsked,
    Captured(BTreeMap<String, String>),
    /// Why it wasn't read.
    Failed(String),
}

/// An endpoint's environments, from its relay's `RELAY_HOST` (`types R6`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointEnv {
    /// The relay's own environment: sshd's, or the engine's `exec`, with
    /// the passwd basics.
    pub exec: BTreeMap<String, String>,
    pub login: Login,
}

impl EndpointEnv {
    pub fn from_host(host: &RelayHost) -> Self {
        let login = match &host.login {
            None => Login::NotAsked,
            Some(LoginEnv::Captured(env)) => Login::Captured(env.clone()),
            Some(LoginEnv::Failed(reason)) => Login::Failed(reason.clone()),
        };
        Self { exec: host.env.clone(), login }
    }
}

/// A program got the endpoint's own environment because the login shell's
/// wasn't read: what its terminal or zone says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    /// The relay's reason ("it took longer than 10 s").
    pub reason: String,
}

impl Fallback {
    /// The words a terminal's dim line and an agent zone's note show, for
    /// the endpoint `endpoint`. The terminal's line passes them through the
    /// notice line's control filter, since the reason may quote what a
    /// startup file printed.
    pub fn text(&self, endpoint: &str) -> String {
        format!(
            "the login shell's environment on {endpoint} wasn't read ({}), so this program has the SSH session's environment",
            self.reason
        )
    }
}

/// A start's complete environment, and the fallback to show when it is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub env: BTreeMap<String, String>,
    pub fallback: Option<Fallback>,
}

/// The environment `start` gets on `endpoint`: its base (decision 5's
/// table) with `own`, the request's environment, over it. Names are
/// compared as given, since endpoints are Linux whatever this machine is.
///
/// | Endpoint | `LoginShell` | `Program` |
/// | --- | --- | --- |
/// | `Captured(env)` | `exec` | `env` |
/// | `Failed(reason)` | `exec` | `exec`, with a fallback |
/// | `NotAsked` | `exec` | `exec` |
pub fn merged(endpoint: &EndpointEnv, start: Start, own: &BTreeMap<String, String>) -> Merged {
    let (base, fallback) = match (start, &endpoint.login) {
        (Start::LoginShell, Login::NotAsked | Login::Captured(_) | Login::Failed(_)) => (&endpoint.exec, None),
        (Start::Program, Login::Captured(env)) => (env, None),
        (Start::Program, Login::Failed(reason)) => (&endpoint.exec, Some(Fallback { reason: reason.clone() })),
        (Start::Program, Login::NotAsked) => (&endpoint.exec, None),
    };
    let mut env = base.clone();
    env.extend(own.iter().map(|(name, value)| (name.clone(), value.clone())));
    Merged { env, fallback }
}

/// `message` with its environment merged for `endpoint`, and the fallback
/// to show, when it starts a program ([`start_of`]); `None` for every other
/// message, which goes out as it is.
pub fn over_endpoint(message: &ToCore, endpoint: &EndpointEnv) -> Option<(ToCore, Option<Fallback>)> {
    let start = start_of(message)?;
    let (message, fallback) = match message {
        ToCore::Spawn(spawn) => {
            let Merged { env, fallback } = merged(endpoint, start, &spawn.env);
            (ToCore::Spawn(Spawn { env, ..spawn.clone() }), fallback)
        }
        ToCore::SpawnAgent(spawn) => {
            let Merged { env, fallback } = merged(endpoint, start, &spawn.env);
            (ToCore::SpawnAgent(SpawnAgent { env, ..spawn.clone() }), fallback)
        }
        ToCore::AgentRestart(restart) => {
            let Merged { env, fallback } = merged(endpoint, start, &restart.env);
            (ToCore::AgentRestart(AgentRestart { env, ..restart.clone() }), fallback)
        }
        ToCore::Hello(_)
        | ToCore::Data(_)
        | ToCore::Resize(_)
        | ToCore::Close(_)
        | ToCore::Attach(_)
        | ToCore::Detach(_)
        | ToCore::List(_)
        | ToCore::Kill(_)
        | ToCore::Status(_)
        | ToCore::Stop(_)
        | ToCore::StepDown(_)
        | ToCore::Reparent(_)
        | ToCore::SetLayout(_)
        | ToCore::AgentPrompt(_)
        | ToCore::AgentCancel(_)
        | ToCore::AgentPermission(_)
        | ToCore::AgentUnqueue(_)
        | ToCore::AgentResend(_)
        | ToCore::AgentLogin(_)
        | ToCore::AgentFetch(_) => return None,
    };
    Some((message, fallback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::spawn::{OpenedBy, agent_spawn_request};
    use crate::config::{Config, Profile};
    use crate::remote::Host;
    use crate::remote::protocol::client::{
        AgentFetch, AgentLogin, AgentPermission, AgentPrompt, AgentRequest, Attach, Data, Detach, Hello, Kill, KillTarget, Policy,
        Reparent, Request, Resize, SessionRef, SetLayout,
    };

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
    }

    fn endpoint(login: Login) -> EndpointEnv {
        EndpointEnv {
            exec: vars(&[("HOME", "/home/u"), ("PATH", "/usr/bin"), ("SSH_ONLY", "1")]),
            login,
        }
    }

    fn captured() -> Login {
        Login::Captured(vars(&[
            ("HOME", "/home/u"),
            ("PATH", "/home/u/.local/bin:/usr/bin"),
            ("EDITOR", "vim"),
            ("MODEL", "large"),
        ]))
    }

    fn spawn(command: Option<&str>) -> Spawn {
        Spawn {
            req: 1,
            command: command.map(String::from),
            args: Vec::new(),
            cwd: None,
            env: vars(&[("FOO", "1")]),
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
            scrollback: 1000,
            policy: Policy::Detach,
            profile: None,
        }
    }

    fn spawn_agent(env: BTreeMap<String, String>) -> SpawnAgent {
        SpawnAgent {
            req: 2,
            command: "agy".into(),
            args: vec!["acp".into()],
            cwd: None,
            env,
            policy: Policy::Detach,
            profile: Some("Agy".into()),
            conversation: None,
        }
    }

    fn restart(env: BTreeMap<String, String>) -> AgentRestart {
        AgentRestart { req: 3, session: 7, command: "agy".into(), args: vec!["acp".into()], env }
    }

    /// Every `ToCore` variant, one each.
    fn every_message() -> Vec<(ToCore, Option<Start>)> {
        let session = SessionRef { session: 1 };
        let request = Request { req: 1 };
        let (session, request) = (|| session.clone(), || request.clone());
        vec![
            (ToCore::Hello(Hello::default()), None),
            (ToCore::Spawn(spawn(None)), Some(Start::LoginShell)),
            (ToCore::Spawn(spawn(Some("htop"))), Some(Start::Program)),
            (ToCore::Data(Data { session: 1, bytes: b"x".to_vec() }), None),
            (ToCore::Resize(Resize { session: 1, cols: 80, rows: 24, cell_w: 8, cell_h: 16 }), None),
            (ToCore::Close(session()), None),
            (ToCore::Attach(Attach { req: 1, session: 1, cols: 80, rows: 24, page: None }), None),
            (ToCore::Detach(Detach { session: 1, layout: None }), None),
            (ToCore::List(request()), None),
            (ToCore::Kill(Kill { req: 1, target: KillTarget::All }), None),
            (ToCore::Status(request()), None),
            (ToCore::Stop(request()), None),
            (ToCore::StepDown(request()), None),
            (ToCore::Reparent(Reparent { session: 1, hwnd: 0 }), None),
            (ToCore::SetLayout(SetLayout::default()), None),
            (ToCore::SpawnAgent(spawn_agent(BTreeMap::new())), Some(Start::Program)),
            (ToCore::AgentPrompt(AgentPrompt { req: 1, session: 1, text: "hi".into() }), None),
            (ToCore::AgentCancel(session()), None),
            (ToCore::AgentPermission(AgentPermission { req: 1, session: 1, entry: 1, option: None }), None),
            (ToCore::AgentUnqueue(AgentRequest { req: 1, session: 1 }), None),
            (ToCore::AgentResend(AgentRequest { req: 1, session: 1 }), None),
            (ToCore::AgentRestart(restart(BTreeMap::new())), Some(Start::Program)),
            (ToCore::AgentLogin(AgentLogin { req: 1, session: 1, method: "m".into() }), None),
            (ToCore::AgentFetch(AgentFetch { req: 1, session: 1, before: 9, max_bytes: 1024 }), None),
        ]
    }

    #[test]
    fn only_the_three_starts_start_a_program() {
        let endpoint = endpoint(captured());
        for (message, start) in every_message() {
            assert_eq!(start_of(&message), start, "{message:?}");
            // Exactly the starts are re-encoded.
            assert_eq!(over_endpoint(&message, &endpoint).is_some(), start.is_some(), "{message:?}");
        }
    }

    #[test]
    fn each_start_gets_the_base_decision_5_gives_it() {
        let own = BTreeMap::new();
        let login = |start| merged(&endpoint(captured()), start, &own);
        // Captured: a program gets the login environment; a login shell the
        // SSH session's, which its own startup files build on.
        assert_eq!(login(Start::Program).env.get("EDITOR").map(String::as_str), Some("vim"));
        assert!(!login(Start::Program).env.contains_key("SSH_ONLY"));
        assert_eq!(login(Start::LoginShell).env, endpoint(captured()).exec);
        assert_eq!(login(Start::Program).fallback, None);
        assert_eq!(login(Start::LoginShell).fallback, None);

        // Failed: both get the SSH session's; only a program says so.
        let failed = endpoint(Login::Failed("it took longer than 10 s".into()));
        let program = merged(&failed, Start::Program, &own);
        assert_eq!(program.env, failed.exec);
        assert_eq!(program.fallback, Some(Fallback { reason: "it took longer than 10 s".into() }));
        let shell = merged(&failed, Start::LoginShell, &own);
        assert_eq!((shell.env, shell.fallback), (failed.exec.clone(), None));

        // Not asked (a container): both get the exec environment, silently.
        let container = endpoint(Login::NotAsked);
        for start in [Start::Program, Start::LoginShell] {
            assert_eq!(merged(&container, start, &own), Merged { env: container.exec.clone(), fallback: None });
        }
    }

    #[test]
    fn the_requests_environment_wins_over_the_login_one() {
        let profile = Profile {
            name: "A8".into(),
            env: vars(&[("MODEL", "small"), ("PATH", "/opt/bin")]),
            ..Default::default()
        };
        let own: BTreeMap<String, String> = crate::terminal::child_env(&profile).into_iter().collect();
        let mut endpoint = endpoint(captured());
        if let Login::Captured(env) = &mut endpoint.login {
            env.insert("TERM".into(), "dumb".into());
        }
        let env = merged(&endpoint, Start::Program, &own).env;
        assert_eq!(env["MODEL"], "small");
        // A profile's value replaces the variable; nothing is expanded.
        assert_eq!(env["PATH"], "/opt/bin");
        assert_eq!(env["EDITOR"], "vim");
        // Brindle's terminal variables win over the login shell's.
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["TERM_PROGRAM"], "Brindle");
    }

    /// Moved from `relay::relay_env`: the endpoint's own environment is
    /// under a terminal's, and nothing of Brindle's own environment is in
    /// it.
    #[test]
    fn the_endpoints_environment_is_under_the_profiles() {
        let endpoint = EndpointEnv {
            exec: vars(&[("LANG", "en_GB.UTF-8"), ("HOME", "/home/ubuntu"), ("TERM", "dumb"), ("path", "/lower")]),
            login: Login::NotAsked,
        };
        let profile = Profile {
            name: "A8".into(),
            env: vars(&[("FOO", "1"), ("PATH", "/usr/bin")]),
            ..Default::default()
        };
        let own: BTreeMap<String, String> = crate::terminal::child_env(&profile).into_iter().collect();
        let env = merged(&endpoint, Start::LoginShell, &own).env;
        assert_eq!(env["LANG"], "en_GB.UTF-8");
        assert_eq!(env["HOME"], "/home/ubuntu");
        assert_eq!(env["FOO"], "1");
        // Brindle's terminal variables win over a tty-less login's.
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["COLORTERM"], "truecolor");
        assert_eq!(env["TERM_PROGRAM"], "Brindle");
        // Linux names: `PATH` doesn't replace `path`, even on Windows.
        assert_eq!((env["PATH"].as_str(), env["path"].as_str()), ("/usr/bin", "/lower"));
        // Nothing of Brindle's own environment.
        let keys: Vec<&str> = env.keys().map(String::as_str).collect();
        assert_eq!(keys, ["COLORTERM", "FOO", "HOME", "LANG", "PATH", "TERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "path"]);
    }

    #[test]
    fn the_messages_are_re_encoded_with_the_merged_environment() {
        let endpoint = endpoint(captured());
        let Some((ToCore::Spawn(merged_spawn), None)) = over_endpoint(&ToCore::Spawn(spawn(Some("htop"))), &endpoint)
        else {
            panic!("a spawn with a command is re-encoded");
        };
        assert_eq!(merged_spawn.env["EDITOR"], "vim");
        assert_eq!(merged_spawn.env["FOO"], "1");
        assert_eq!(merged_spawn.command.as_deref(), Some("htop"));
        let Some((ToCore::Spawn(shell), None)) = over_endpoint(&ToCore::Spawn(spawn(None)), &endpoint) else {
            panic!("a spawn with no command is re-encoded");
        };
        assert_eq!(shell.env["SSH_ONLY"], "1");
        assert!(!shell.env.contains_key("EDITOR"));
        let Some((ToCore::AgentRestart(again), None)) =
            over_endpoint(&ToCore::AgentRestart(restart(vars(&[("MODEL", "small")]))), &endpoint)
        else {
            panic!("a restart is re-encoded");
        };
        assert_eq!((again.env["HOME"].as_str(), again.env["MODEL"].as_str()), ("/home/u", "small"));
        assert_eq!((again.req, again.session, again.command.as_str()), (3, 7, "agy"));

        let failed = EndpointEnv { login: Login::Failed("why".into()), ..endpoint.clone() };
        let (_, fallback) = over_endpoint(&ToCore::SpawnAgent(spawn_agent(BTreeMap::new())), &failed).unwrap();
        assert_eq!(fallback, Some(Fallback { reason: "why".into() }));
    }

    #[test]
    fn the_host_frame_becomes_the_endpoints_environments() {
        let host = |login| RelayHost { env: vars(&[("HOME", "/root")]), login, ..Default::default() };
        assert_eq!(EndpointEnv::from_host(&host(None)), EndpointEnv { exec: vars(&[("HOME", "/root")]), login: Login::NotAsked });
        assert_eq!(
            EndpointEnv::from_host(&host(Some(LoginEnv::Captured(vars(&[("A", "1")]))))).login,
            Login::Captured(vars(&[("A", "1")]))
        );
        assert_eq!(EndpointEnv::from_host(&host(Some(LoginEnv::Failed("x".into())))).login, Login::Failed("x".into()));
    }

    #[test]
    fn a_fallback_names_the_endpoint_and_why_and_stays_one_line() {
        let fallback = Fallback { reason: "it took longer than 10 s".into() };
        assert_eq!(
            fallback.text("A8"),
            "the login shell's environment on A8 wasn't read (it took longer than 10 s), so this program has the SSH session's environment"
        );
        // A reason quoting a startup file's output can't steer the terminal.
        let hostile = Fallback { reason: "the login shell exited with status 1 before printing it: \x1b]0;pwned\x07x\r\ny".into() };
        let line = crate::terminal::tab_note::line(&hostile.text("A8"));
        assert!(!line.contains("\x1b]0") && !line.contains('\x07'), "{line:?}");
        assert!(line.contains("x; y"), "{line:?}");
    }

    /// `remote-login-environment` 3.3: for one remote agent profile and one
    /// endpoint, a new zone's `SPAWN_AGENT` and a restart's `AGENT_RESTART`
    /// go out with the same environment.
    #[test]
    fn a_restart_gets_the_environment_a_new_start_gets() {
        let text = "[[profiles]]\nname = \"Agy\"\ntype = \"agent\"\ncommand = \"agy\"\nargs = [\"acp\"]\n\
                    env = { MODEL = \"small\" }\nremote = { type = \"ssh\", host = \"A8\" }\n";
        let profile = Config::parse(text).unwrap().config.profiles.remove(0);
        let host = Host::Remote(profile.remote.clone().expect("a remote profile"));
        let request = agent_spawn_request(&profile, &host, None, &OpenedBy::Window, Vec::new(), None, false);
        let again = request.clone().restart();
        for login in [captured(), Login::Failed("why".into()), Login::NotAsked] {
            let endpoint = endpoint(login);
            let spawned = over_endpoint(&ToCore::SpawnAgent(spawn_agent(request.env.clone())), &endpoint).unwrap();
            let restarted = over_endpoint(&ToCore::AgentRestart(restart(again.env.clone())), &endpoint).unwrap();
            let (ToCore::SpawnAgent(spawned_message), ToCore::AgentRestart(restarted_message)) = (&spawned.0, &restarted.0)
            else {
                panic!("both are re-encoded as themselves");
            };
            assert_eq!(spawned_message.env, restarted_message.env);
            assert_eq!(spawned_message.env["MODEL"], "small");
            assert_eq!(spawned_message.env["HOME"], "/home/u");
            assert_eq!(spawned.1, restarted.1);
        }
    }
}
