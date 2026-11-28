//! Where a new tab starts, apart from GPUI: the directory a launch asked
//! for, the active tab's, or its profile's own; and what it runs, by its
//! profile's kind.

use std::path::PathBuf;

use crate::FirstPrompt;
use crate::agent::model::LaunchChoice;
use crate::agent::spawn::OpenedBy;
use crate::config::ProfileKind;
use crate::remote::Host;

/// What a launch asks of its tab beyond its profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchWith {
    /// The profile as it is: what the window opens (the profile menu, the
    /// picker, a shortcut, ctrl-shift-t, the default tab at launch, a
    /// restore's).
    Profile,
    /// `brindle terminal -e`: this program and its arguments instead of
    /// the profile's.
    Command(String, Vec<String>),
    /// `brindle agent`, run in `launched_in` (`None` when that couldn't be
    /// read), with its first prompt, and `-c` or `-r` as `choose` (OpenSpec
    /// change `agents-session-list` decision 8).
    Agent { launched_in: Option<PathBuf>, first: Option<FirstPrompt>, choose: Option<LaunchChoice> },
}

/// What a launch's tab runs (design.md decision 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// A terminal tab, running `command` or the profile's own.
    Terminal { command: Option<(String, Vec<String>)> },
    /// An agent tab, opened as `opened` says, with its first prompt, its
    /// agent started choosing for `choose` when given.
    Agent { opened: OpenedBy, first: Option<FirstPrompt>, choose: Option<LaunchChoice> },
}

/// What a launch opens for a profile of `kind`: the profile's kind decides
/// the tab's. A launch's own profile check (`profile_for`) already matched
/// the kinds, so what the other kind asked (a command for an agent, a
/// first prompt for a terminal) only comes with a profile whose kind a
/// reload changed in between, and is dropped.
pub fn launch_plan(kind: ProfileKind, with: LaunchWith) -> LaunchPlan {
    match (kind, with) {
        (ProfileKind::Terminal, LaunchWith::Command(program, args)) => {
            LaunchPlan::Terminal { command: Some((program, args)) }
        }
        (ProfileKind::Terminal, LaunchWith::Profile | LaunchWith::Agent { .. }) => LaunchPlan::Terminal { command: None },
        (ProfileKind::Agent, LaunchWith::Agent { launched_in, first, choose }) => {
            LaunchPlan::Agent { opened: OpenedBy::AgentCommand(launched_in), first, choose }
        }
        (ProfileKind::Agent, LaunchWith::Profile | LaunchWith::Command(..)) => {
            LaunchPlan::Agent { opened: OpenedBy::Window, first: None, choose: None }
        }
    }
}

/// The directory an agent tab on `host` is asked to start in: the one the
/// launch gave, when it means something there ([`new_tab_cwd`]). An agent
/// tab never follows the active tab's directory: without one, its profile's
/// `cwd`, the launching command's directory or the home directory decide
/// (`agent::spawn::directory`).
pub fn agent_cwd(requested: Option<PathBuf>, host: &Host) -> Option<PathBuf> {
    match new_tab_cwd(requested, host, true) {
        NewTabCwd::Given(cwd) => Some(cwd),
        NewTabCwd::FollowActive | NewTabCwd::ProfileDefault => None,
    }
}

/// The directory a new tab on some host starts in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewTabCwd {
    /// The one the launch asked for (`brindle terminal -d`, a restored
    /// tab's, the forwarded
    /// working directory).
    Given(PathBuf),
    /// The active tab's current directory, which the window looks up only
    /// then, and only when the active tab runs on the same endpoint.
    FollowActive,
    /// The profile's own: its `cwd`, else the session's default.
    ProfileDefault,
}

/// The directory a new tab on `host` starts in, given the `requested` one
/// and whether its profile `pins` one (its `cwd`). A remote endpoint takes a
/// requested directory only when it means something there: an absolute
/// POSIX path, `~` or `~/…`; this machine takes any. Without a usable
/// request, the tab follows the active tab's directory unless its profile
/// pins one.
pub fn new_tab_cwd(requested: Option<PathBuf>, host: &Host, pins: bool) -> NewTabCwd {
    let usable = requested.filter(|cwd| match host {
        Host::Local => true,
        Host::Remote(_) => {
            let cwd = cwd.to_string_lossy();
            cwd.starts_with('/') || cwd == "~" || cwd.starts_with("~/")
        }
    });
    match (usable, pins) {
        (Some(cwd), _) => NewTabCwd::Given(cwd),
        (None, false) => NewTabCwd::FollowActive,
        (None, true) => NewTabCwd::ProfileDefault,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteTarget;

    /// `Workspace::launch_on`'s choice before this function: the request
    /// filtered for the host, else the active tab's directory unless the
    /// profile pins one (`active` stands for what that lookup gives).
    fn before(requested: Option<PathBuf>, host: &Host, pins: bool, active: Option<PathBuf>) -> Option<PathBuf> {
        let requested_cwd = requested.filter(|cwd| {
            if host.is_local() {
                return true;
            }
            let cwd = cwd.to_string_lossy();
            cwd.starts_with('/') || cwd == "~" || cwd.starts_with("~/")
        });
        requested_cwd.or(if !pins { active } else { None })
    }

    fn hosts() -> [Host; 3] {
        let ssh = RemoteTarget::Ssh {
            host: "A8".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false, attachments_dir: None,
        };
        let container =
            RemoteTarget::Container { engine: "docker".into(), container: "box".into(), user: None, connect_at_launch: false, attachments_dir: None };
        [Host::Local, Host::Remote(ssh), Host::Remote(container)]
    }

    /// Over every host kind, request and pin: what the window then does
    /// with the answer is what it did before.
    #[test]
    fn the_directory_is_the_one_it_was() {
        let requests = [None, Some("/srv/app"), Some("~"), Some("~/src"), Some("src"), Some(r"C:\src"), Some("~other")];
        let active = PathBuf::from("/active");
        for host in hosts() {
            for requested in requests.map(|r| r.map(PathBuf::from)) {
                for pins in [false, true] {
                    let applied = match new_tab_cwd(requested.clone(), &host, pins) {
                        NewTabCwd::Given(cwd) => Some(cwd),
                        NewTabCwd::FollowActive => Some(active.clone()),
                        NewTabCwd::ProfileDefault => None,
                    };
                    let expected = before(requested.clone(), &host, pins, Some(active.clone()));
                    assert_eq!(applied, expected, "{} {requested:?} pins={pins}", host.name());
                }
            }
        }
    }

    #[test]
    fn a_remote_endpoint_takes_only_paths_that_mean_something_there() {
        let [local, ssh, container] = hosts();
        let given = |cwd: &str| NewTabCwd::Given(PathBuf::from(cwd));
        for host in [&ssh, &container] {
            assert_eq!(new_tab_cwd(Some("/srv".into()), host, true), given("/srv"));
            assert_eq!(new_tab_cwd(Some("~".into()), host, true), given("~"));
            assert_eq!(new_tab_cwd(Some("~/src".into()), host, true), given("~/src"));
            assert_eq!(new_tab_cwd(Some("src".into()), host, false), NewTabCwd::FollowActive);
            assert_eq!(new_tab_cwd(Some(r"C:\src".into()), host, true), NewTabCwd::ProfileDefault);
            assert_eq!(new_tab_cwd(Some("~other".into()), host, true), NewTabCwd::ProfileDefault);
        }
        assert_eq!(new_tab_cwd(Some("src".into()), &local, true), given("src"));
        assert_eq!(new_tab_cwd(Some(r"C:\src".into()), &local, false), given(r"C:\src"));
    }

    /// The profile's kind decides the tab's: a terminal profile keeps a
    /// command and drops an agent launch's extras; an agent profile is
    /// opened by its command (with the launch's directory and prompt) or
    /// by the window.
    #[test]
    fn the_profile_kind_decides_what_a_launch_runs() {
        let first = FirstPrompt::new("hi".into()).ok();
        let here = Some(PathBuf::from("/work"));
        let agent = LaunchWith::Agent { launched_in: here.clone(), first: first.clone(), choose: Some(LaunchChoice::Last) };
        let command = LaunchWith::Command("htop".into(), vec!["-d".into()]);
        let terminal = |command: Option<(&str, &[&str])>| LaunchPlan::Terminal {
            command: command.map(|(p, a)| (p.to_string(), a.iter().map(|a| a.to_string()).collect())),
        };
        assert_eq!(launch_plan(ProfileKind::Terminal, LaunchWith::Profile), terminal(None));
        assert_eq!(launch_plan(ProfileKind::Terminal, command.clone()), terminal(Some(("htop", &["-d"]))));
        assert_eq!(launch_plan(ProfileKind::Terminal, agent.clone()), terminal(None));
        assert_eq!(
            launch_plan(ProfileKind::Agent, agent),
            LaunchPlan::Agent { opened: OpenedBy::AgentCommand(here), first, choose: Some(LaunchChoice::Last) }
        );
        let window = LaunchPlan::Agent { opened: OpenedBy::Window, first: None, choose: None };
        assert_eq!(launch_plan(ProfileKind::Agent, LaunchWith::Profile), window);
        assert_eq!(launch_plan(ProfileKind::Agent, command), window);
    }

    /// An agent tab takes a launch's directory where it means something,
    /// and never the active tab's.
    #[test]
    fn an_agent_tab_takes_only_the_directory_it_was_given() {
        let [local, ssh, _] = hosts();
        assert_eq!(agent_cwd(None, &local), None);
        assert_eq!(agent_cwd(Some("/srv".into()), &local), Some("/srv".into()));
        assert_eq!(agent_cwd(Some("/srv".into()), &ssh), Some("/srv".into()));
        assert_eq!(agent_cwd(Some("src".into()), &ssh), None);
    }

    #[test]
    fn without_a_request_a_pin_wins_over_the_active_tab() {
        for host in hosts() {
            assert_eq!(new_tab_cwd(None, &host, false), NewTabCwd::FollowActive);
            assert_eq!(new_tab_cwd(None, &host, true), NewTabCwd::ProfileDefault);
        }
    }
}
