//! What making sure of the Windows server directory does with what it
//! found there: use it, protect it, create it, or refuse it.
//!
//! Pure, so it is tested on every platform. The Windows `dir` reads the
//! path's metadata, creates and protects the directory, and words the
//! errors.

/// What is at the directory's path, as its metadata says (a link is not
/// followed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Dir,
    /// A file, or a link: a symlink or a junction is not a directory here.
    NotDir,
    Missing,
    /// Its metadata can't be read.
    Unreadable,
}

/// What to do with a directory that exists already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protect {
    /// Set its DACL again: it is the default directory.
    Existing,
    /// Use it as it is: a directory someone named is theirs to protect.
    AsFound,
}

/// What the first look at the path leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum First {
    Use,
    Protect,
    Create,
    Refuse,
    FailRead,
}

/// What the look after creating the directory leads to. It can't ask to
/// create again or to protect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterCreate {
    Use,
    Refuse,
    FailCreate,
}

/// What to do with what was `found` at the server directory's path.
pub fn first(found: Found, protect: Protect) -> First {
    match (found, protect) {
        (Found::Dir, Protect::Existing) => First::Protect,
        (Found::Dir, Protect::AsFound) => First::Use,
        (Found::NotDir, _) => First::Refuse,
        (Found::Missing, _) => First::Create,
        (Found::Unreadable, _) => First::FailRead,
    }
}

/// What to do with what was `found` once the directory was created:
/// whoever created it, it must be a directory now.
pub fn after_create(found: Found) -> AfterCreate {
    match found {
        Found::Dir => AfterCreate::Use,
        Found::NotDir => AfterCreate::Refuse,
        Found::Missing | Found::Unreadable => AfterCreate::FailCreate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_directory_is_used_or_protected_as_asked() {
        assert_eq!(first(Found::Dir, Protect::AsFound), First::Use);
        assert_eq!(first(Found::Dir, Protect::Existing), First::Protect);
    }

    #[test]
    fn anything_but_a_directory_is_refused() {
        // A file, a symlink or a junction, protected or not.
        for protect in [Protect::AsFound, Protect::Existing] {
            assert_eq!(first(Found::NotDir, protect), First::Refuse);
        }
    }

    #[test]
    fn a_missing_directory_is_created() {
        for protect in [Protect::AsFound, Protect::Existing] {
            assert_eq!(first(Found::Missing, protect), First::Create);
        }
    }

    #[test]
    fn an_unreadable_path_fails_the_read() {
        for protect in [Protect::AsFound, Protect::Existing] {
            assert_eq!(first(Found::Unreadable, protect), First::FailRead);
        }
    }

    #[test]
    fn after_creating_only_a_directory_is_used() {
        assert_eq!(after_create(Found::Dir), AfterCreate::Use);
        // Someone else put something else there meanwhile.
        assert_eq!(after_create(Found::NotDir), AfterCreate::Refuse);
        // Gone again, or unreadable: the create failed.
        assert_eq!(after_create(Found::Missing), AfterCreate::FailCreate);
        assert_eq!(after_create(Found::Unreadable), AfterCreate::FailCreate);
    }
}
