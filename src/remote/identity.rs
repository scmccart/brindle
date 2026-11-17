//! Which session a terminal server's message is about, decoded once where
//! the client reads it.
//!
//! Session ids restart at 1 in a fresh terminal server core, so after a core
//! started with no holder surviving, an id alone can name another session.
//! The key, a random value the core gives each session, tells them apart
//! ([`SessionKey`]). A listing's entry is decoded into a [`ListedSession`],
//! and [`ListedSession::is`] is the one rule for "the same session". The
//! wire keeps its own forms (an empty key, a 0x0 size); nothing past this
//! module looks at them.

use crate::remote::protocol::SessionId;
use crate::remote::protocol::client::SessionInfo;

/// A session's key: the random value its core gave it, which tells it
/// apart from another core's session with the same id. Never empty: a core
/// that predates keys sends none, which is `Option::None` where a key is
/// kept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionKey(String);

impl SessionKey {
    /// The key the wire carries (`SPAWNED`'s, a listing's): `None` for the
    /// empty string a core that predates keys sends.
    pub fn from_wire(key: &str) -> Option<Self> {
        (!key.is_empty()).then(|| Self(key.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key as the state file and the wire write it.
    pub fn into_string(self) -> String {
        self.0
    }
}

/// One session of a core's listing (`SESSIONS`), with its key and size
/// decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSession {
    pub session: SessionId,
    /// `None` from a core that predates keys.
    pub key: Option<SessionKey>,
    /// A client has it.
    pub attached: bool,
    /// Its columns and rows; `None` from a core that predates sizes, which
    /// lists 0 for both.
    pub size: Option<(u16, u16)>,
}

impl ListedSession {
    pub fn of(info: &SessionInfo) -> Self {
        Self {
            session: info.session,
            key: SessionKey::from_wire(&info.key),
            attached: info.attached,
            size: (info.cols > 0 && info.rows > 0).then_some((info.cols, info.rows)),
        }
    }

    /// Whether this is the session `session` whose key is `key`. The ids
    /// must be equal, and then:
    /// - with a key, only a listing with that key is it: a listing without
    ///   one comes from a core that didn't give us ours;
    /// - without one (it came from a core that predates keys), any listing
    ///   of the id is, since there is nothing better to compare.
    pub fn is(&self, session: SessionId, key: Option<&SessionKey>) -> bool {
        self.session == session
            && match key {
                Some(ours) => self.key.as_ref() == Some(ours),
                None => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> SessionKey {
        SessionKey::from_wire(text).expect("a key")
    }

    fn listed(session: SessionId, key: &str) -> ListedSession {
        ListedSession::of(&SessionInfo { session, key: key.into(), ..Default::default() })
    }

    #[test]
    fn an_empty_wire_key_is_no_key() {
        assert_eq!(SessionKey::from_wire(""), None);
        let key = SessionKey::from_wire("3f9a").expect("a key");
        assert_eq!(key.as_str(), "3f9a");
        assert_eq!(key.into_string(), "3f9a");
    }

    #[test]
    fn a_listed_session_decodes_its_key_and_size_once() {
        let info = SessionInfo { session: 4, key: "k4".into(), attached: true, cols: 132, rows: 43, ..Default::default() };
        assert_eq!(
            ListedSession::of(&info),
            ListedSession { session: 4, key: Some(key("k4")), attached: true, size: Some((132, 43)) }
        );
        // A core that predates keys and sizes.
        let old = SessionInfo { session: 4, ..Default::default() };
        assert_eq!(ListedSession::of(&old), ListedSession { session: 4, key: None, attached: false, size: None });
        for (cols, rows) in [(0, 0), (80, 0), (0, 24)] {
            let info = SessionInfo { session: 4, cols, rows, ..Default::default() };
            assert_eq!(ListedSession::of(&info).size, None, "{cols}x{rows}");
        }
    }

    #[test]
    fn a_keyed_session_is_only_the_listed_one_with_its_key() {
        let ours = key("k5");
        // (listing, same session?)
        let table = [
            (listed(5, "k5"), true),
            (listed(5, "another"), false),
            (listed(5, ""), false),
            (listed(6, "k5"), false),
        ];
        for (listing, same) in table {
            assert_eq!(listing.is(5, Some(&ours)), same, "{listing:?}");
        }
    }

    #[test]
    fn a_keyless_session_matches_any_listing_of_its_id() {
        for listing in [listed(5, "k5"), listed(5, "another"), listed(5, "")] {
            assert!(listing.is(5, None), "{listing:?}");
        }
        assert!(!listed(6, "").is(5, None));
        assert!(!listed(6, "k6").is(5, None));
    }
}
