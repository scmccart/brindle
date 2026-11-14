//! Which terminal server a session runs in: this machine's, or a remote
//! endpoint's (an SSH host or a container), reached through a relay. Tabs
//! and zones keep the one they opened on, so it is built on every platform.

use std::hash::{Hash, Hasher};

use crate::config::{Profile, RemoteTarget};

/// An endpoint the process keeps one connection to (`remote::local::shared`).
///
/// Remote endpoints are equal when their targets name the same endpoint:
/// an SSH target's host, user, port, identity file and SSH arguments, or a
/// container target's engine, container and user. Anything else a target
/// may carry (such as whether it connects at launch) isn't part of that.
#[derive(Clone, Debug)]
pub enum Host {
    Local,
    Remote(RemoteTarget),
}

/// What tells one endpoint from another.
#[derive(PartialEq, Eq, Hash)]
enum Identity<'a> {
    Local,
    Ssh {
        host: &'a str,
        user: Option<&'a str>,
        port: Option<u16>,
        identity_file: Option<&'a str>,
        ssh_args: &'a [String],
    },
    Container {
        engine: &'a str,
        container: &'a str,
        user: Option<&'a str>,
    },
}

impl Host {
    /// The endpoint a profile's sessions run on.
    pub fn of(profile: &Profile) -> Self {
        profile.remote.clone().map_or(Self::Local, Self::Remote)
    }

    /// The remote target, for a remote endpoint.
    pub fn target(&self) -> Option<&RemoteTarget> {
        match self {
            Self::Local => None,
            Self::Remote(target) => Some(target),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    fn identity(&self) -> Identity<'_> {
        // Every field is named, so a field added to a target has to be
        // placed here: part of the identity, or not.
        match self {
            Self::Local => Identity::Local,
            Self::Remote(RemoteTarget::Ssh { host, user, port, identity_file, ssh_args }) => Identity::Ssh {
                host,
                user: user.as_deref(),
                port: *port,
                identity_file: identity_file.as_deref(),
                ssh_args,
            },
            Self::Remote(RemoteTarget::Container { engine, container, user }) => {
                Identity::Container { engine, container, user: user.as_deref() }
            }
        }
    }

    /// The endpoint as messages name it: `dev@A8:2222`, `A8`, `devbox`.
    pub fn name(&self) -> String {
        match self {
            Self::Local => "this machine".to_string(),
            Self::Remote(RemoteTarget::Ssh { host, user, port, .. }) => {
                let user = user.as_ref().map(|user| format!("{user}@")).unwrap_or_default();
                let port = port.map(|port| format!(":{port}")).unwrap_or_default();
                format!("{user}{host}{port}")
            }
            Self::Remote(RemoteTarget::Container { container, user, .. }) => match user {
                Some(user) => format!("{user}@{container}"),
                None => container.clone(),
            },
        }
    }
}

impl PartialEq for Host {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Host {}

impl Hash for Host {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn ssh(user: Option<&str>, port: Option<u16>) -> RemoteTarget {
        RemoteTarget::Ssh {
            host: "A8".into(),
            user: user.map(Into::into),
            port,
            identity_file: Some("~/.ssh/a8".into()),
            ssh_args: vec!["-o".into(), "BatchMode=yes".into()],
        }
    }

    fn profile(name: &str, remote: Option<RemoteTarget>) -> Profile {
        Profile { name: name.into(), remote, ..Default::default() }
    }

    #[test]
    fn profiles_with_the_same_target_share_a_host() {
        // Different profiles (names, commands, themes) on one endpoint.
        let mut htop = profile("htop on A8", Some(ssh(Some("dev"), Some(22))));
        htop.command = Some("htop".into());
        let shell = profile("A8", Some(ssh(Some("dev"), Some(22))));
        assert_eq!(Host::of(&htop), Host::of(&shell));
        let hosts: HashSet<Host> = [Host::of(&htop), Host::of(&shell)].into();
        assert_eq!(hosts.len(), 1);
        assert_eq!(Host::of(&profile("Shell", None)), Host::Local);
        assert_ne!(Host::of(&shell), Host::Local);
    }

    #[test]
    fn another_user_port_or_option_is_another_host() {
        let base = Host::Remote(ssh(Some("dev"), Some(22)));
        let others = [
            Host::Remote(ssh(Some("root"), Some(22))),
            Host::Remote(ssh(None, Some(22))),
            Host::Remote(ssh(Some("dev"), Some(2222))),
            Host::Remote(ssh(Some("dev"), None)),
            Host::Remote(RemoteTarget::Ssh {
                host: "A8".into(),
                user: Some("dev".into()),
                port: Some(22),
                identity_file: Some("~/.ssh/a8".into()),
                ssh_args: Vec::new(),
            }),
            Host::Remote(RemoteTarget::Ssh {
                host: "A8".into(),
                user: Some("dev".into()),
                port: Some(22),
                identity_file: None,
                ssh_args: vec!["-o".into(), "BatchMode=yes".into()],
            }),
        ];
        let mut hosts: HashSet<Host> = HashSet::from([base.clone()]);
        for other in others {
            assert_ne!(base, other, "{other:?}");
            assert!(hosts.insert(other));
        }
    }

    #[test]
    fn containers_are_told_apart_by_engine_container_and_user() {
        let container = |engine: &str, container: &str, user: Option<&str>| {
            Host::Remote(RemoteTarget::Container {
                engine: engine.into(),
                container: container.into(),
                user: user.map(Into::into),
            })
        };
        let base = container("docker", "devbox", None);
        assert_eq!(base, container("docker", "devbox", None));
        assert_ne!(base, container("podman", "devbox", None));
        assert_ne!(base, container("docker", "other", None));
        assert_ne!(base, container("docker", "devbox", Some("1000")));
        // An SSH host named like the container is another endpoint.
        let ssh = Host::Remote(RemoteTarget::Ssh {
            host: "devbox".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
        });
        assert_ne!(base, ssh);
    }

    #[test]
    fn hosts_are_named_for_messages() {
        assert_eq!(Host::Remote(ssh(Some("dev"), Some(2222))).name(), "dev@A8:2222");
        assert_eq!(Host::Remote(ssh(None, None)).name(), "A8");
        let container = RemoteTarget::Container { engine: "docker".into(), container: "devbox".into(), user: None };
        assert_eq!(Host::Remote(container).name(), "devbox");
    }
}
