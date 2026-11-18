//! Where a new tab starts, apart from GPUI: the directory a launch asked
//! for, the active tab's, or its profile's own.

use std::path::PathBuf;

use crate::remote::Host;

/// The directory a new tab on some host starts in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewTabCwd {
    /// The one the launch asked for (`-d`, a restored tab's, the forwarded
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
            connect_at_launch: false,
        };
        let container =
            RemoteTarget::Container { engine: "docker".into(), container: "box".into(), user: None, connect_at_launch: false };
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

    #[test]
    fn without_a_request_a_pin_wins_over_the_active_tab() {
        for host in hosts() {
            assert_eq!(new_tab_cwd(None, &host, false), NewTabCwd::FollowActive);
            assert_eq!(new_tab_cwd(None, &host, true), NewTabCwd::ProfileDefault);
        }
    }
}
