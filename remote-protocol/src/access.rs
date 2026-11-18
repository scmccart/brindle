//! Who may use a Windows terminal server's pipes: the rule both ends apply
//! to the process at the other end ([`peer_allowed`], and the server's
//! [`admit_client`]), the security descriptors the pipes and the server
//! directory are created with, and what a DACL read back from one must
//! hold ([`Dacl::allows_only`]).
//!
//! Everything here works on values (a SID string, an integrity RID, the
//! entries of a DACL), so it is tested on every platform; `crate::win`
//! reads those values from tokens and security descriptors.

use std::fmt;
use std::io;

/// A token's mandatory integrity level, as the last sub-authority (RID) of
/// its label SID (`S-1-16-<rid>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Integrity(pub u32);

impl Integrity {
    pub const UNTRUSTED: Self = Self(0x0000);
    pub const LOW: Self = Self(0x1000);
    pub const MEDIUM: Self = Self(0x2000);
    pub const HIGH: Self = Self(0x3000);
    pub const SYSTEM: Self = Self(0x4000);

    /// High integrity or above: an elevated process.
    pub fn is_elevated(self) -> bool {
        self >= Self::HIGH
    }
}

impl fmt::Display for Integrity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match *self {
            Self::UNTRUSTED => "untrusted",
            Self::LOW => "low",
            Self::MEDIUM => "medium",
            Self::HIGH => "high",
            Self::SYSTEM => "system",
            Self(rid) => return write!(f, "{rid:#06x}"),
        };
        f.write_str(name)
    }
}

/// Who a process runs as: its token's user and integrity level.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Principal {
    /// The token user's SID in string form (`S-1-5-21-…`).
    pub sid: String,
    pub integrity: Integrity,
}

impl Principal {
    pub fn is_elevated(&self) -> bool {
        self.integrity.is_elevated()
    }
}

/// Which end of a pipe is checking the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A core or holder checking a client that connected.
    Server,
    /// A client checking the server whose pipe it opened.
    Client,
}

/// Why the process at the other end of a pipe is not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// It runs as another user.
    OtherUser { own: String, peer: String },
    /// A client of lower integrity than the server.
    LowerIntegrity { own: Integrity, peer: Integrity },
    /// A server at another integrity level than the client.
    OtherIntegrity { own: Integrity, peer: Integrity },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OtherUser { own, peer } => {
                write!(f, "it runs as another user ({peer}, not {own})")
            }
            Self::LowerIntegrity { own, peer } => write!(
                f,
                "the client runs at {peer} integrity, below this server's {own}"
            ),
            Self::OtherIntegrity { own, peer } => write!(
                f,
                "the server runs at {peer} integrity, and this process at {own}"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

impl From<Refusal> for io::Error {
    fn from(refusal: Refusal) -> Self {
        io::Error::new(io::ErrorKind::PermissionDenied, refusal)
    }
}

/// Whether `own`, on `side` of a pipe, may use the `peer` at its other end.
/// The user must be the same. A server refuses a client of lower integrity,
/// and a client refuses a server at any other integrity, so no process uses
/// a server running at another level.
pub fn peer_allowed(own: &Principal, peer: &Principal, side: Side) -> Result<(), Refusal> {
    if own.sid != peer.sid {
        return Err(Refusal::OtherUser {
            own: own.sid.clone(),
            peer: peer.sid.clone(),
        });
    }
    let (own, peer) = (own.integrity, peer.integrity);
    match side {
        Side::Server if peer < own => Err(Refusal::LowerIntegrity { own, peer }),
        Side::Client if peer != own => Err(Refusal::OtherIntegrity { own, peer }),
        _ => Ok(()),
    }
}

/// The server's check of a client that connected, before anything is read:
/// accepted only when it runs as `own`'s user and not below its integrity
/// level. `client` is who it runs as, or why that can't be told, which
/// refuses it too. Every refusal is `PermissionDenied`.
pub fn admit_client(own: &Principal, client: io::Result<Principal>) -> io::Result<()> {
    let refused = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
    let client = client.map_err(|error| {
        refused(format!(
            "refused a connection whose user can't be told: {error}"
        ))
    })?;
    peer_allowed(own, &client, Side::Server)
        .map_err(|refusal| refused(format!("refused a connection: {refusal}")))
}

/// SYSTEM's SID, the one entry beside the user's in the server's DACLs.
pub const SYSTEM_SID: &str = "S-1-5-18";

/// One access-allowed entry of a DACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    /// The trustee, as a SID string.
    pub sid: String,
    /// The access mask.
    pub mask: u32,
    /// The entry's flags (inheritance).
    pub flags: u8,
}

/// An object's discretionary ACL, as read back from it (`crate::win`'s
/// `Dacl::of_handle` and `Dacl::of_path` on Windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dacl {
    /// Whether the DACL is protected from inheriting its parent's entries.
    pub protected: bool,
    /// The access-allowed entries, in order.
    pub allowed: Vec<Ace>,
    /// How many entries are of another kind (deny, and so on).
    pub others: usize,
}

impl Dacl {
    /// Whether the DACL is protected and allows exactly `sids`, each once,
    /// with no other entries.
    pub fn allows_only(&self, sids: &[&str]) -> bool {
        let mut found: Vec<&str> = self.allowed.iter().map(|ace| ace.sid.as_str()).collect();
        let mut wanted = sids.to_vec();
        found.sort_unstable();
        wanted.sort_unstable();
        self.protected && self.others == 0 && found == wanted
    }
}

/// The SDDL a server pipe is created with: a protected DACL for the user and
/// SYSTEM, and a no-write-up mandatory label at the creator's level, since a
/// process can't label above its own (error 1314).
pub fn pipe_sddl(user_sid: &str, elevated: bool) -> String {
    let label = if elevated { "HI" } else { "ME" };
    format!("D:P(A;;GA;;;{user_sid})(A;;GA;;;SY)S:(ML;;NW;;;{label})")
}

/// The SDDL the server directory is created with: a protected DACL for the
/// user and SYSTEM, inherited by what is created inside. It grants `FA`
/// (all file access) rather than `GA`: a generic right on an inheritable
/// entry is stored as two entries, one for the directory and one to inherit.
pub fn dir_sddl(user_sid: &str) -> String {
    format!("D:P(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVELS: [Integrity; 6] = [
        Integrity::UNTRUSTED,
        Integrity::LOW,
        Integrity::MEDIUM,
        Integrity(0x2100),
        Integrity::HIGH,
        Integrity::SYSTEM,
    ];
    const SIDS: [&str; 2] = ["S-1-5-21-1-2-3-1001", "S-1-5-21-1-2-3-1002"];

    fn principal(sid: &str, integrity: Integrity) -> Principal {
        Principal {
            sid: sid.to_string(),
            integrity,
        }
    }

    #[test]
    fn every_pair_of_principals_gets_the_expected_answer() {
        for own_sid in SIDS {
            for peer_sid in SIDS {
                for own_level in LEVELS {
                    for peer_level in LEVELS {
                        let own = principal(own_sid, own_level);
                        let peer = principal(peer_sid, peer_level);
                        let case = format!("{own:?} / {peer:?}");

                        let server = peer_allowed(&own, &peer, Side::Server);
                        let client = peer_allowed(&own, &peer, Side::Client);
                        if own_sid != peer_sid {
                            // Another user is refused first, whatever the levels.
                            let other = Refusal::OtherUser {
                                own: own_sid.to_string(),
                                peer: peer_sid.to_string(),
                            };
                            assert_eq!(server, Err(other.clone()), "{case}");
                            assert_eq!(client, Err(other), "{case}");
                            continue;
                        }
                        let levels = (own_level, peer_level);
                        let lower = Refusal::LowerIntegrity {
                            own: levels.0,
                            peer: levels.1,
                        };
                        let other = Refusal::OtherIntegrity {
                            own: levels.0,
                            peer: levels.1,
                        };
                        let expected = if peer_level < own_level {
                            Err(lower)
                        } else {
                            Ok(())
                        };
                        assert_eq!(server, expected, "{case}");
                        let expected = if peer_level != own_level {
                            Err(other)
                        } else {
                            Ok(())
                        };
                        assert_eq!(client, expected, "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn an_elevated_pair_never_meets_an_unelevated_one() {
        // Whichever end is elevated, one of the two checks refuses, so an
        // elevated server is never driven by a medium client and an elevated
        // client never uses a medium server.
        let medium = principal(SIDS[0], Integrity::MEDIUM);
        let high = principal(SIDS[0], Integrity::HIGH);
        // Medium client, elevated server: both ends refuse.
        assert!(peer_allowed(&high, &medium, Side::Server).is_err());
        assert!(peer_allowed(&medium, &high, Side::Client).is_err());
        // Elevated client, medium server: the server would take it, and the
        // client refuses.
        assert!(peer_allowed(&medium, &high, Side::Server).is_ok());
        assert!(peer_allowed(&high, &medium, Side::Client).is_err());
    }

    #[test]
    fn refusals_name_the_reason() {
        let own = principal(SIDS[0], Integrity::MEDIUM);
        let message =
            |peer: &Principal, side| peer_allowed(&own, peer, side).unwrap_err().to_string();
        let other_user = message(&principal(SIDS[1], Integrity::MEDIUM), Side::Client);
        assert!(other_user.contains("another user"), "{other_user}");
        assert!(other_user.contains(SIDS[0]) && other_user.contains(SIDS[1]));
        let lower = message(&principal(SIDS[0], Integrity::LOW), Side::Server);
        assert_eq!(
            lower,
            "the client runs at low integrity, below this server's medium"
        );
        let other = message(&principal(SIDS[0], Integrity::HIGH), Side::Client);
        assert_eq!(
            other,
            "the server runs at high integrity, and this process at medium"
        );

        let error = io::Error::from(
            peer_allowed(&own, &principal(SIDS[1], Integrity::LOW), Side::Server).unwrap_err(),
        );
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("another user"));
    }

    #[test]
    fn integrity_levels_order_and_print() {
        assert!(LEVELS.windows(2).all(|pair| pair[0] < pair[1]));
        let elevated: Vec<bool> = LEVELS.iter().map(|level| level.is_elevated()).collect();
        assert_eq!(elevated, [false, false, false, false, true, true]);
        let names: Vec<String> = LEVELS.iter().map(Integrity::to_string).collect();
        assert_eq!(
            names,
            ["untrusted", "low", "medium", "0x2100", "high", "system"]
        );
    }

    #[test]
    fn the_client_must_be_the_same_user_at_no_lower_integrity() {
        let own = principal(SIDS[0], Integrity::MEDIUM);
        admit_client(&own, Ok(own.clone())).unwrap();
        // A client above the server is the server's to accept; the client
        // itself refuses a server at another level.
        admit_client(&own, Ok(principal(SIDS[0], Integrity::HIGH))).unwrap();

        let other_user =
            admit_client(&own, Ok(principal(SIDS[1], Integrity::MEDIUM))).unwrap_err();
        assert_eq!(other_user.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            other_user.to_string(),
            "refused a connection: it runs as another user \
             (S-1-5-21-1-2-3-1002, not S-1-5-21-1-2-3-1001)"
        );

        let lower = admit_client(&own, Ok(principal(SIDS[0], Integrity::LOW))).unwrap_err();
        assert_eq!(lower.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            lower.to_string(),
            "refused a connection: the client runs at low integrity, below this server's medium"
        );
        let elevated = principal(SIDS[0], Integrity::HIGH);
        assert!(admit_client(&elevated, Ok(own.clone())).is_err());

        // A token that can't be opened is refused, whatever the reason.
        let unreadable = admit_client(
            &own,
            Err(io::Error::other("process 4: can't open it: access denied")),
        )
        .unwrap_err();
        assert_eq!(unreadable.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            unreadable.to_string(),
            "refused a connection whose user can't be told: process 4: can't open it: access denied"
        );
        let gone = admit_client(&own, Err(io::ErrorKind::NotFound.into())).unwrap_err();
        assert_eq!(gone.kind(), io::ErrorKind::PermissionDenied);
    }

    fn ace(sid: &str) -> Ace {
        Ace {
            sid: sid.to_string(),
            mask: 0x1f_01ff,
            flags: 0,
        }
    }

    fn dacl(protected: bool, sids: &[&str], others: usize) -> Dacl {
        Dacl {
            protected,
            allowed: sids.iter().map(|sid| ace(sid)).collect(),
            others,
        }
    }

    #[test]
    fn a_dacl_allows_only_the_named_sids() {
        let wanted = [SIDS[0], SYSTEM_SID];
        // In any order.
        assert!(dacl(true, &[SYSTEM_SID, SIDS[0]], 0).allows_only(&wanted));
        assert!(dacl(true, &[SIDS[0], SYSTEM_SID], 0).allows_only(&wanted));

        let refused = [
            ("unprotected", dacl(false, &[SIDS[0], SYSTEM_SID], 0)),
            ("another kind of entry", dacl(true, &[SIDS[0], SYSTEM_SID], 1)),
            ("an extra SID", dacl(true, &[SIDS[0], SYSTEM_SID, SIDS[1]], 0)),
            ("a missing SID", dacl(true, &[SIDS[0]], 0)),
            ("a duplicated SID", dacl(true, &[SIDS[0], SIDS[0], SYSTEM_SID], 0)),
        ];
        for (case, dacl) in refused {
            assert!(!dacl.allows_only(&wanted), "{case}: {dacl:?}");
        }
    }

    #[test]
    fn sddl_strings_are_pinned() {
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001", false),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)(A;;GA;;;SY)S:(ML;;NW;;;ME)"
        );
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001", true),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)(A;;GA;;;SY)S:(ML;;NW;;;HI)"
        );
        assert_eq!(
            dir_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;OICI;FA;;;S-1-5-21-1-2-3-1001)(A;OICI;FA;;;SY)"
        );
    }
}
