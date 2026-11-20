//! Whether the directory a local program last reported still exists, kept
//! apart from GPUI and from the file system.
//!
//! Where Brindle can't read another program's directory (Windows), a
//! tab's directory is the one its program reported (OSC 7, OSC 9;9), if
//! it exists. Checking can take as long as the path's drive takes to
//! answer, so it never happens on the UI thread: the terminal tells this
//! model what is reported ([`DirCheck::report`]), runs the check it asks
//! for on the background executor, and gives the answer back
//! ([`DirCheck::finished`]). Readers only read the model
//! ([`DirCheck::existing`]).
//!
//! - **One check at a time.** A report that arrives while a check runs is
//!   checked after it, and only the latest is kept.
//! - **A change, not a repeat.** A directory reported again is not checked
//!   again: what was found of it stands until another is reported. A new
//!   tab or split that starts in it checks it once more, on the thread
//!   that starts its session (`spawn_request`).

use std::path::{Path, PathBuf};

/// What is known of the directory a program last reported.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Checked {
    /// It was a directory when it was checked.
    Exists(PathBuf),
    /// It wasn't one, or nothing is reported.
    Missing,
    /// Its check hasn't come back yet.
    Pending,
}

/// The directory a program last reported, and what is known of it.
#[derive(Debug)]
pub struct DirCheck {
    /// The directory last reported: the one `checked` is about.
    reported: Option<PathBuf>,
    checked: Checked,
    /// The directory a check is running on, until its answer comes back.
    checking: Option<PathBuf>,
}

impl Default for DirCheck {
    fn default() -> Self {
        Self { reported: None, checked: Checked::Missing, checking: None }
    }
}

impl DirCheck {
    /// The program reports `dir` now (`None`: nothing). The directory to
    /// check, if a check should start: a directory that wasn't the one
    /// reported before, while no other check runs.
    pub fn report(&mut self, dir: Option<&Path>) -> Option<PathBuf> {
        if dir == self.reported.as_deref() {
            return None;
        }
        self.reported = dir.map(Path::to_path_buf);
        let Some(dir) = dir else {
            self.checked = Checked::Missing;
            return None;
        };
        self.checked = Checked::Pending;
        self.start(dir)
    }

    /// The check of `dir` came back: `is_dir` is whether it is a directory.
    /// The next directory to check, if another was reported meanwhile. An
    /// answer about a directory no check runs on is dropped.
    pub fn finished(&mut self, dir: &Path, is_dir: bool) -> Option<PathBuf> {
        if self.checking.as_deref() != Some(dir) {
            return None;
        }
        self.checking = None;
        match self.reported.clone() {
            Some(reported) if reported == dir => {
                self.checked = if is_dir { Checked::Exists(reported) } else { Checked::Missing };
                None
            }
            // Another directory is reported now: the answer is about one
            // that no longer matters, and the latest is checked.
            Some(reported) => self.start(&reported),
            None => None,
        }
    }

    /// Starts a check of `dir`, unless one runs.
    fn start(&mut self, dir: &Path) -> Option<PathBuf> {
        if self.checking.is_some() {
            return None;
        }
        self.checking = Some(dir.to_path_buf());
        Some(dir.to_path_buf())
    }

    /// The reported directory, if it was one when it was checked: `None`
    /// while its check is pending, when it is missing and when nothing is
    /// reported.
    pub fn existing(&self) -> Option<&Path> {
        match &self.checked {
            Checked::Exists(dir) => Some(dir),
            Checked::Missing | Checked::Pending => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    #[test]
    fn nothing_is_reported_at_first() {
        let check = DirCheck::default();
        assert_eq!(check.checked, Checked::Missing);
        assert_eq!(check.existing(), None);
    }

    #[test]
    fn a_reported_directory_is_pending_until_its_check_comes_back() {
        let mut check = DirCheck::default();
        let src = path(r"C:\src");
        assert_eq!(check.report(Some(&src)), Some(src.clone()));
        assert_eq!(check.checked, Checked::Pending);
        assert_eq!(check.existing(), None, "nothing is given while the check runs");
        assert_eq!(check.finished(&src, true), None);
        assert_eq!(check.checked, Checked::Exists(src.clone()));
        assert_eq!(check.existing(), Some(src.as_path()));
    }

    #[test]
    fn a_missing_directory_is_not_given() {
        let mut check = DirCheck::default();
        let gone = path(r"Z:\gone");
        assert_eq!(check.report(Some(&gone)), Some(gone.clone()));
        assert_eq!(check.finished(&gone, false), None);
        assert_eq!(check.checked, Checked::Missing);
        assert_eq!(check.existing(), None);
        // The next directory replaces it.
        let src = path(r"C:\src");
        assert_eq!(check.report(Some(&src)), Some(src.clone()));
        assert_eq!(check.finished(&src, true), None);
        assert_eq!(check.existing(), Some(src.as_path()));
    }

    #[test]
    fn a_repeat_report_starts_no_other_check() {
        let mut check = DirCheck::default();
        let src = path(r"C:\src");
        assert_eq!(check.report(Some(&src)), Some(src.clone()));
        // While its check runs.
        assert_eq!(check.report(Some(&src)), None);
        assert_eq!(check.checked, Checked::Pending);
        assert_eq!(check.finished(&src, true), None);
        // And once it is known: what was found stands.
        assert_eq!(check.report(Some(&src)), None);
        assert_eq!(check.checked, Checked::Exists(src.clone()));
        // A missing one isn't checked again either.
        let gone = path(r"Z:\gone");
        assert_eq!(check.report(Some(&gone)), Some(gone.clone()));
        assert_eq!(check.finished(&gone, false), None);
        assert_eq!(check.report(Some(&gone)), None);
        assert_eq!(check.checked, Checked::Missing);
    }

    #[test]
    fn a_change_during_a_check_is_checked_after_it_and_only_the_latest() {
        let mut check = DirCheck::default();
        let (one, two, three) = (path(r"C:\one"), path(r"C:\two"), path(r"C:\three"));
        assert_eq!(check.report(Some(&one)), Some(one.clone()));
        // Two more reports while the first check runs: no second one yet.
        assert_eq!(check.report(Some(&two)), None);
        assert_eq!(check.report(Some(&three)), None);
        assert_eq!(check.checked, Checked::Pending);
        // The first answer is about a directory that is no longer the
        // reported one: it is dropped, and the latest is checked.
        assert_eq!(check.finished(&one, true), Some(three.clone()));
        assert_eq!(check.checked, Checked::Pending);
        assert_eq!(check.existing(), None);
        assert_eq!(check.finished(&three, true), None);
        assert_eq!(check.existing(), Some(three.as_path()));
    }

    #[test]
    fn a_report_of_the_directory_being_checked_takes_its_answer() {
        let mut check = DirCheck::default();
        let (one, two) = (path(r"C:\one"), path(r"C:\two"));
        assert_eq!(check.report(Some(&one)), Some(one.clone()));
        assert_eq!(check.report(Some(&two)), None);
        // Back to the one whose check still runs: its answer counts.
        assert_eq!(check.report(Some(&one)), None);
        assert_eq!(check.finished(&one, true), None);
        assert_eq!(check.existing(), Some(one.as_path()));
    }

    #[test]
    fn a_known_directory_is_pending_again_once_another_is_reported() {
        let mut check = DirCheck::default();
        let (one, two) = (path(r"C:\one"), path(r"C:\two"));
        assert_eq!(check.report(Some(&one)), Some(one.clone()));
        assert_eq!(check.finished(&one, true), None);
        assert_eq!(check.report(Some(&two)), Some(two.clone()));
        assert_eq!(check.existing(), None, "the old directory isn't given for the new report");
        assert_eq!(check.finished(&two, false), None);
        assert_eq!(check.checked, Checked::Missing);
    }

    #[test]
    fn no_report_is_missing_and_ends_what_was_known() {
        let mut check = DirCheck::default();
        let src = path(r"C:\src");
        assert_eq!(check.report(Some(&src)), Some(src.clone()));
        assert_eq!(check.finished(&src, true), None);
        assert_eq!(check.report(None), None);
        assert_eq!(check.checked, Checked::Missing);
        assert_eq!(check.report(None), None);
        // During a check: its answer is dropped, and nothing else is checked.
        assert_eq!(check.report(Some(&src)), Some(src.clone()));
        assert_eq!(check.report(None), None);
        assert_eq!(check.finished(&src, true), None);
        assert_eq!(check.checked, Checked::Missing);
        assert_eq!(check.existing(), None);
        // A later report of it is checked again.
        assert_eq!(check.report(Some(&src)), Some(src.clone()));
    }

    #[test]
    fn an_answer_no_check_asked_for_is_dropped() {
        let mut check = DirCheck::default();
        let (one, two) = (path(r"C:\one"), path(r"C:\two"));
        assert_eq!(check.finished(&one, true), None);
        assert_eq!(check.existing(), None);
        assert_eq!(check.report(Some(&one)), Some(one.clone()));
        // An answer about another directory than the one being checked.
        assert_eq!(check.finished(&two, true), None);
        assert_eq!(check.checked, Checked::Pending);
        assert_eq!(check.finished(&one, true), None);
        assert_eq!(check.existing(), Some(one.as_path()));
        // And a second answer for a check that already came back.
        assert_eq!(check.finished(&one, false), None);
        assert_eq!(check.existing(), Some(one.as_path()));
    }
}
