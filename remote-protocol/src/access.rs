//! Who may use a Windows terminal server's pipes: the rule both ends apply
//! to the process at the other end ([`peer_allowed`], and the server's
//! [`admit_client`]), the security descriptors the pipes and the server
//! directory are created with, and what a DACL read back from one must
//! hold ([`Dacl::allows_only`]). An elevated process's directories get a
//! descriptor of their own ([`elevated_dir_sddl`]), which one read back
//! must match exactly ([`matches_elevated`]).
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
        Side::Server | Side::Client => Ok(()),
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

/// Administrators' SID (`BA` in SDDL): the owner of an elevated process's
/// directories, and with SYSTEM the only one their DACL allows.
pub const ADMINISTRATORS_SID: &str = "S-1-5-32-544";

/// `FILE_ALL_ACCESS` (`FA` in SDDL): every right on a file or a directory.
pub const FILE_ALL_ACCESS: u32 = 0x001f_01ff;

/// `OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE` (`OICI` in SDDL): the files
/// and directories made inside inherit the entry, and nothing else is set,
/// so the entry applies to the directory itself and came from no parent.
pub const INHERITED_BY_ALL: u8 = 0x03;

/// `SYSTEM_MANDATORY_LABEL_NO_WRITE_UP` (`NW` in SDDL): a process below the
/// label's level can't write to the object.
pub const NO_WRITE_UP: u32 = 0x1;

/// The SDDL the directories of an elevated process are created with: its
/// server directory, its helper copies and its ConPTY files.
///
/// - **Owner:** Administrators, so the user's unelevated processes, which
///   hold that group only to be denied by it, can't change the DACL as its
///   owner.
/// - **DACL:** protected, full access for Administrators and SYSTEM and
///   for nobody else, inherited by what is made inside. There is no entry
///   for the user's own SID, which an unelevated process of the same user
///   has too.
/// - **Label:** high, no write-up, inherited likewise.
///
/// So an unelevated process of the same user can neither write nor read
/// in the directory. [`matches_elevated`] says whether a directory read
/// back has exactly this.
pub fn elevated_dir_sddl() -> &'static str {
    "O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)S:(ML;OICI;NW;;;HI)"
}

/// An object's mandatory label, as read back from its SACL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Label {
    /// Its level: the last sub-authority of the label's SID.
    pub integrity: Integrity,
    /// Its policy mask.
    pub mask: u32,
    /// The entry's flags (inheritance).
    pub flags: u8,
}

/// What protects a directory, as read back from it (`crate::win`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSecurity {
    /// The owner, as a SID string.
    pub owner: String,
    pub dacl: Dacl,
    /// The mandatory label; `None` when the directory has none, which
    /// Windows treats as medium.
    pub label: Option<Label>,
}

/// Whether `found` is exactly what [`elevated_dir_sddl`] gives a directory:
/// owned by Administrators; a protected DACL of two entries, Administrators
/// and SYSTEM, each with full access and inherited by everything inside;
/// and a high, no-write-up label inherited likewise.
///
/// Anything else is a directory an elevated process didn't make that way,
/// and may hold what an unelevated process put there: an entry for another
/// SID, another owner, a lower label or none, a narrower inheritance.
pub fn matches_elevated(found: &DirSecurity) -> bool {
    matches_owned(found, ADMINISTRATORS_SID, Integrity::HIGH)
}

/// Whether `found` is exactly the descriptor [`elevated_dir_sddl`]'s form
/// gives with `owner` in Administrators' place and a `level` label: owned
/// by `owner`; a protected DACL of two entries, `owner` and SYSTEM, each
/// with full access and inherited by everything inside; and a no-write-up
/// label at `level`, inherited likewise. [`matches_elevated`] is its
/// Administrators and high case; the seam's tests use their own user's at
/// medium, which an unelevated process can make.
pub fn matches_owned(found: &DirSecurity, owner: &str, level: Integrity) -> bool {
    let allows_fully = |sid: &str| {
        found
            .dacl
            .allowed
            .iter()
            .any(|ace| ace.sid == sid && ace.mask == FILE_ALL_ACCESS && ace.flags == INHERITED_BY_ALL)
    };
    let label = Label {
        integrity: level,
        mask: NO_WRITE_UP,
        flags: INHERITED_BY_ALL,
    };
    found.owner == owner
        && found.dacl.allows_only(&[owner, SYSTEM_SID])
        && allows_fully(owner)
        && allows_fully(SYSTEM_SID)
        && found.label == Some(label)
}

/// What a role found of a lock or log it opened in an elevated server
/// directory, through a handle that doesn't follow links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// A junction, a symbolic link or another reparse point.
    pub reparse_point: bool,
    /// How many names the file has (`nNumberOfLinks`).
    pub links: u32,
    /// Its owner, as a SID string.
    pub owner: String,
}

/// Why a role refuses a lock or log in an elevated server directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRefusal {
    /// It is a link, which would send the role's writes elsewhere.
    Link,
    /// It has other names (hard links): it may be a file outside.
    Links(u32),
    /// Another owner made it, such as an unelevated process through a
    /// handle to the directory opened before the directory was protected.
    Owner(String),
}

impl std::fmt::Display for FileRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Link => write!(f, "it is a link"),
            Self::Links(links) => write!(f, "it has {links} names (hard links)"),
            Self::Owner(owner) => write!(f, "it is owned by {owner}, not by Administrators"),
        }
    }
}

/// Whether a role uses a lock or log it opened in an elevated server
/// directory: only a plain file of one name owned by `owner`
/// (Administrators, whom an elevated process's files belong to).
///
/// Protecting the directory stops new entries from unelevated processes,
/// but not from one that opened the directory before: what it planted is
/// refused here, file by file.
pub fn check_file(facts: &FileFacts, owner: &str) -> Result<(), FileRefusal> {
    if facts.reparse_point {
        return Err(FileRefusal::Link);
    }
    if facts.links != 1 {
        return Err(FileRefusal::Links(facts.links));
    }
    if facts.owner != owner {
        return Err(FileRefusal::Owner(facts.owner.clone()));
    }
    Ok(())
}

/// The descriptor a file kept in a directory made with `dir_sddl` is given
/// when the directory is protected again in place: the same owner, entries
/// and label, without the inheritance flags only a directory takes.
pub fn contained_file_sddl(dir_sddl: &str) -> String {
    dir_sddl.replace("OICI;", ";")
}

/// What a protected directory holds, which says what becomes of one found
/// with another descriptor ([`protected_step`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contents {
    /// Files that are run or loaded (the helper copies, the ConPTY files):
    /// nothing in a directory someone else could write can be trusted, so
    /// it is set aside and made fresh.
    Executables,
    /// Locks and logs, nothing that is run (`run-elevated`): it is
    /// protected again where it is, since an older build's elevated core
    /// may still have its sessions there.
    State,
}

/// What is at a protected directory's path, as the seam found it through
/// a handle that doesn't follow links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// Nothing.
    Missing,
    /// A junction, a symbolic link or another reparse point: the link
    /// itself, never what it points to.
    Link,
    /// A file, not a directory.
    NotDir,
    /// A directory whose descriptor is the protected one.
    Matches,
    /// A directory with any other descriptor, or one that can't be read.
    Other,
}

/// What [`protected_step`] says to do with what was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Hold it: it is the protected directory.
    Use,
    /// Make it with the protected descriptor.
    Create,
    /// Rename what is there aside ([`crate::server_dir::untrusted_name`]),
    /// without following it, then make the directory fresh.
    SetAside,
    /// Put the protected descriptor on the directory where it is, after
    /// deleting every link inside it.
    Reprotect,
    /// Give up: the directory this process just made doesn't read back as
    /// made, so making it again wouldn't help.
    Fail,
}

/// What to do with what was found at a protected directory's path.
/// `created` says whether this process made what is there, in which case
/// only the protected directory itself will do.
///
/// A link or a file is set aside whatever the contents: a link was never
/// a directory an older build kept sessions in, and following one would
/// put the files wherever it points. A directory with another descriptor
/// is set aside when it holds what is run, and protected again in place
/// when it holds state ([`Contents`]).
pub fn protected_step(found: Found, contents: Contents, created: bool) -> Step {
    match (found, created) {
        (Found::Matches, _) => Step::Use,
        (Found::Missing, false) => Step::Create,
        (Found::Missing | Found::Link | Found::NotDir | Found::Other, true) => Step::Fail,
        (Found::Link | Found::NotDir, false) => Step::SetAside,
        (Found::Other, false) => match contents {
            Contents::Executables => Step::SetAside,
            Contents::State => Step::Reprotect,
        },
    }
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
        assert_eq!(
            elevated_dir_sddl(),
            "O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)S:(ML;OICI;NW;;;HI)"
        );
    }

    /// One entry of the elevated directories' DACL.
    fn full(sid: &str) -> Ace {
        Ace {
            sid: sid.to_string(),
            mask: FILE_ALL_ACCESS,
            flags: INHERITED_BY_ALL,
        }
    }

    const HIGH_LABEL: Label = Label {
        integrity: Integrity::HIGH,
        mask: NO_WRITE_UP,
        flags: INHERITED_BY_ALL,
    };

    /// What a directory made with [`elevated_dir_sddl`] reads back as:
    /// `O:BA`, `D:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)`, `S:(ML;OICI;NW;;;HI)`.
    fn elevated() -> DirSecurity {
        DirSecurity {
            owner: ADMINISTRATORS_SID.to_string(),
            dacl: Dacl {
                protected: true,
                allowed: vec![full(ADMINISTRATORS_SID), full(SYSTEM_SID)],
                others: 0,
            },
            label: Some(HIGH_LABEL),
        }
    }

    /// [`elevated`] with one thing changed.
    fn elevated_but(change: impl FnOnce(&mut DirSecurity)) -> DirSecurity {
        let mut found = elevated();
        change(&mut found);
        found
    }

    #[test]
    fn the_elevated_descriptors_values_are_what_its_sddl_names() {
        // `BA`, `FA`, `OICI`, `NW` and `HI`, as Windows defines them.
        assert_eq!(ADMINISTRATORS_SID, "S-1-5-32-544");
        assert_eq!(FILE_ALL_ACCESS, 0x1f_01ff);
        assert_eq!(INHERITED_BY_ALL, 0x1 | 0x2);
        assert_eq!(NO_WRITE_UP, 1);
        assert_eq!(Integrity::HIGH, Integrity(0x3000));
    }

    #[test]
    fn the_exact_elevated_descriptor_matches() {
        assert!(matches_elevated(&elevated()));
        // The two entries in either order.
        assert!(matches_elevated(&elevated_but(|found| found.dacl.allowed.reverse())));
    }

    #[test]
    fn any_other_descriptor_is_not_the_elevated_one() {
        let user = SIDS[0];
        let label = |change: fn(&mut Label)| {
            elevated_but(|found| {
                let mut label = HIGH_LABEL;
                change(&mut label);
                found.label = Some(label);
            })
        };
        let entry = |change: fn(&mut Ace)| elevated_but(|found| change(&mut found.dacl.allowed[0]));
        let others = [
            // What an unelevated process of the same user could use.
            ("an entry for the user's SID", elevated_but(|found| found.dacl.allowed.push(full(user)))),
            ("the user's SID in place of Administrators", elevated_but(|found| found.dacl.allowed[0] = full(user))),
            ("owned by the user", elevated_but(|found| found.owner = user.to_string())),
            ("owned by SYSTEM", elevated_but(|found| found.owner = SYSTEM_SID.to_string())),
            ("no label", elevated_but(|found| found.label = None)),
            ("a medium label", label(|label| label.integrity = Integrity::MEDIUM)),
            ("a low label", label(|label| label.integrity = Integrity::LOW)),
            // And what the SDDL simply doesn't give.
            ("a system label", label(|label| label.integrity = Integrity::SYSTEM)),
            ("a label that lets a lower process write", label(|label| label.mask = 0)),
            ("a label that also stops reading", label(|label| label.mask = 0x3)),
            ("a label nothing inherits", label(|label| label.flags = 0)),
            ("a label only directories inherit", label(|label| label.flags = 0x2)),
            ("an unprotected DACL", elevated_but(|found| found.dacl.protected = false)),
            ("an entry of another kind", elevated_but(|found| found.dacl.others = 1)),
            ("less than full access", entry(|ace| ace.mask = 0x12_00a9)),
            ("more than file access", entry(|ace| ace.mask = 0x1000_0000)),
            ("an entry nothing inherits", entry(|ace| ace.flags = 0)),
            ("an entry only files inherit", entry(|ace| ace.flags = 0x1)),
            ("an entry that skips the directory itself", entry(|ace| ace.flags = 0x3 | 0x8)),
            ("an entry inherited from the parent", entry(|ace| ace.flags = 0x3 | 0x10)),
            ("Administrators alone", elevated_but(|found| found.dacl.allowed.truncate(1))),
            ("SYSTEM alone", elevated_but(|found| drop(found.dacl.allowed.remove(0)))),
            ("Administrators twice", elevated_but(|found| found.dacl.allowed[1] = full(ADMINISTRATORS_SID))),
            ("no entry at all", elevated_but(|found| found.dacl.allowed.clear())),
        ];
        for (case, found) in others {
            assert!(!matches_elevated(&found), "{case}: {found:?}");
        }
    }

    /// The server directory's own descriptor, for the user and SYSTEM, is
    /// not the elevated one.
    #[test]
    fn the_unelevated_server_directory_is_not_the_elevated_one() {
        let user = SIDS[0];
        let found = DirSecurity {
            owner: user.to_string(),
            dacl: Dacl {
                protected: true,
                allowed: vec![full(user), full(SYSTEM_SID)],
                others: 0,
            },
            label: None,
        };
        assert!(!matches_elevated(&found));
        // With a medium label it is the user's own protected form, which
        // the seam's unelevated tests make.
        let labelled = DirSecurity {
            label: Some(Label {
                integrity: Integrity::MEDIUM,
                ..HIGH_LABEL
            }),
            ..found
        };
        assert!(matches_owned(&labelled, user, Integrity::MEDIUM));
        assert!(!matches_owned(&labelled, user, Integrity::HIGH));
        assert!(!matches_owned(&labelled, SIDS[1], Integrity::MEDIUM));
        assert!(!matches_elevated(&labelled));
    }

    fn facts(reparse_point: bool, links: u32, owner: &str) -> FileFacts {
        FileFacts {
            reparse_point,
            links,
            owner: owner.to_string(),
        }
    }

    /// An elevated role's lock or log is a plain file of one name that an
    /// elevated process made; anything else is refused with its reason.
    #[test]
    fn an_elevated_role_uses_only_a_plain_file_of_its_own() {
        assert_eq!(check_file(&facts(false, 1, ADMINISTRATORS_SID), ADMINISTRATORS_SID), Ok(()));
        let user = SIDS[0];
        assert_eq!(
            check_file(&facts(true, 1, ADMINISTRATORS_SID), ADMINISTRATORS_SID),
            Err(FileRefusal::Link)
        );
        assert_eq!(
            check_file(&facts(false, 2, ADMINISTRATORS_SID), ADMINISTRATORS_SID),
            Err(FileRefusal::Links(2))
        );
        assert_eq!(
            check_file(&facts(false, 1, user), ADMINISTRATORS_SID),
            Err(FileRefusal::Owner(user.to_string()))
        );
        // A link is refused as one, whatever else is true of it.
        assert_eq!(check_file(&facts(true, 3, user), ADMINISTRATORS_SID), Err(FileRefusal::Link));
        assert_eq!(check_file(&facts(false, 2, user), ADMINISTRATORS_SID), Err(FileRefusal::Links(2)));
        assert_eq!(FileRefusal::Link.to_string(), "it is a link");
        assert_eq!(FileRefusal::Links(2).to_string(), "it has 2 names (hard links)");
        assert_eq!(
            FileRefusal::Owner(user.to_string()).to_string(),
            format!("it is owned by {user}, not by Administrators")
        );
    }

    #[test]
    fn a_file_kept_inside_takes_the_directory_s_descriptor_without_inheritance() {
        assert_eq!(
            contained_file_sddl(elevated_dir_sddl()),
            "O:BAD:P(A;;FA;;;BA)(A;;FA;;;SY)S:(ML;;NW;;;HI)"
        );
    }

    #[test]
    fn the_protected_directory_is_used_and_a_missing_one_made() {
        for contents in [Contents::Executables, Contents::State] {
            for created in [false, true] {
                assert_eq!(protected_step(Found::Matches, contents, created), Step::Use);
            }
            assert_eq!(protected_step(Found::Missing, contents, false), Step::Create);
        }
    }

    /// A link or a file in the directory's place is renamed aside, never
    /// followed, whatever the directory holds.
    #[test]
    fn a_link_or_a_file_is_set_aside_whatever_the_contents() {
        for contents in [Contents::Executables, Contents::State] {
            for found in [Found::Link, Found::NotDir] {
                assert_eq!(protected_step(found, contents, false), Step::SetAside, "{found:?}");
            }
        }
    }

    /// Another descriptor: what is run can't be trusted and is set aside,
    /// while state an older build's sessions use is protected in place.
    #[test]
    fn a_directory_with_another_descriptor_is_set_aside_or_reprotected() {
        assert_eq!(protected_step(Found::Other, Contents::Executables, false), Step::SetAside);
        assert_eq!(protected_step(Found::Other, Contents::State, false), Step::Reprotect);
    }

    /// What this process just made must read back as made: anything else
    /// is a failure, not another round of setting aside.
    #[test]
    fn a_directory_just_made_is_used_only_as_made() {
        for contents in [Contents::Executables, Contents::State] {
            for found in [Found::Missing, Found::Link, Found::NotDir, Found::Other] {
                assert_eq!(protected_step(found, contents, true), Step::Fail, "{found:?}");
            }
        }
    }
}
