//! The detached terminal server sessions a launch attaches (Linux and
//! Windows): listed
//! once before any window opens, then taken by the restored zones whose
//! recorded session they are, and the rest opened as tabs of the first
//! window, oldest first.

use crate::remote::protocol::client::SessionInfo;
use crate::session::ServerSession;

/// A GPUI global: what is left to attach. Empty for `--no-restore`
/// launches, for a Brindle that doesn't own the session state, and where
/// no terminal server runs (macOS).
#[derive(Default)]
pub struct Detached {
    sessions: Vec<SessionInfo>,
}

impl gpui::Global for Detached {}

impl Detached {
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        Self { sessions }
    }

    /// The listed session a restored zone recorded: same id and same key
    /// (an empty key never matches, so a reused id is never taken for it).
    pub fn take(&mut self, recorded: &ServerSession) -> Option<SessionInfo> {
        let ix = self.position(recorded)?;
        Some(self.sessions.remove(ix))
    }

    /// Whether [`Detached::take`] would find `recorded`.
    pub fn has(&self, recorded: &ServerSession) -> bool {
        self.position(recorded).is_some()
    }

    fn position(&self, recorded: &ServerSession) -> Option<usize> {
        self.sessions.iter().position(|s| s.session == recorded.id && !s.key.is_empty() && s.key == recorded.key)
    }

    /// Everything not taken, in start order.
    pub fn take_rest(&mut self) -> Vec<SessionInfo> {
        let mut rest = std::mem::take(&mut self.sessions);
        rest.sort_by_key(|s| (s.started, s.session));
        rest
    }
}

/// The session `brindle --attach ID` opens, from the launch's listing of
/// every session, or the message its tab shows instead.
pub fn attach_target(id: u32, sessions: &[SessionInfo]) -> Result<SessionInfo, String> {
    match sessions.iter().find(|s| s.session == id) {
        Some(s) if s.attached => Err(format!("session {id} is shown by another Brindle")),
        Some(s) => Ok(s.clone()),
        None => Err(format!("the terminal server has no session {id}")),
    }
}

/// What a session's tab shows until its title arrives, and in errors.
pub fn label(info: &SessionInfo) -> String {
    if !info.title.is_empty() {
        return info.title.clone();
    }
    let program = info.command.split_whitespace().next().unwrap_or_default();
    std::path::Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("session {}", info.session))
}

/// How long ago `started` (unix seconds) was at `now`, shortly ("42s",
/// "5m", "3h", "2d"); `None` when the core didn't say.
pub fn age(started: u64, now: u64) -> Option<String> {
    if started == 0 {
        return None;
    }
    let secs = now.saturating_sub(started);
    Some(match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_are_short() {
        assert_eq!(age(0, 100), None);
        assert_eq!(age(100, 142).as_deref(), Some("42s"));
        assert_eq!(age(100, 100 + 5 * 60 + 7).as_deref(), Some("5m"));
        assert_eq!(age(100, 100 + 3 * 3600).as_deref(), Some("3h"));
        assert_eq!(age(100, 100 + 2 * 86400 + 1).as_deref(), Some("2d"));
        // A clock that went back reads as just now.
        assert_eq!(age(200, 100).as_deref(), Some("0s"));
    }

    fn info(session: u32, key: &str, started: u64) -> SessionInfo {
        SessionInfo { session, key: key.into(), started, ..Default::default() }
    }

    #[test]
    fn a_zone_takes_only_its_own_session_and_the_rest_come_oldest_first() {
        let mut detached = Detached::new(vec![info(3, "c", 30), info(1, "a", 10), info(2, "", 5), info(4, "d", 10)]);
        // Same id, another key: a session that reused the id.
        assert!(detached.take(&ServerSession { id: 1, key: "old".into() }).is_none());
        // A core that predates keys: never matched.
        assert!(detached.take(&ServerSession { id: 2, key: String::new() }).is_none());
        assert_eq!(detached.take(&ServerSession { id: 3, key: "c".into() }).map(|s| s.session), Some(3));
        assert!(detached.take(&ServerSession { id: 3, key: "c".into() }).is_none(), "taken once");
        let rest: Vec<u32> = detached.take_rest().iter().map(|s| s.session).collect();
        assert_eq!(rest, [2, 1, 4]);
        assert!(detached.take_rest().is_empty());
    }

    #[test]
    fn attach_finds_a_detached_session_or_says_why_not() {
        let mut shown = info(2, "b", 0);
        shown.attached = true;
        let sessions = [info(1, "a", 0), shown];
        assert_eq!(attach_target(1, &sessions).map(|s| s.key), Ok("a".into()));
        assert_eq!(attach_target(2, &sessions), Err("session 2 is shown by another Brindle".into()));
        assert_eq!(attach_target(99, &sessions), Err("the terminal server has no session 99".into()));
        assert_eq!(attach_target(1, &[]), Err("the terminal server has no session 1".into()));
    }

    #[test]
    fn labels_prefer_the_title_then_the_program() {
        let mut s = info(5, "k", 0);
        assert_eq!(label(&s), "session 5");
        s.command = "/usr/bin/htop".into();
        assert_eq!(label(&s), "htop");
        s.command = "bash --norc --noprofile".into();
        assert_eq!(label(&s), "bash");
        s.title = "vim notes.md".into();
        assert_eq!(label(&s), "vim notes.md");
    }
}
