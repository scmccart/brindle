//! The window's transient notices, apart from GPUI: what the config line
//! says about something the user asked for and Brindle didn't do (a link it
//! didn't open), until the user dismisses it or [`NOTICE_LIFETIME`] has
//! passed. The config's own errors and findings aren't here: they show
//! until the file is fixed.

use std::time::{Duration, Instant};

/// How long a notice shows.
pub const NOTICE_LIFETIME: Duration = Duration::from_secs(10);

/// How many notices show at once: a newer one drops the oldest.
const NOTICE_CAP: usize = 3;

/// A notice's identity within its [`Notices`]. A text shown again keeps
/// its notice's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeId(u64);

#[derive(Debug)]
struct Notice {
    id: NoticeId,
    text: String,
    /// When it goes.
    until: Instant,
}

/// The window's transient notices, oldest first.
#[derive(Debug, Default)]
pub struct Notices {
    shown: Vec<Notice>,
    next: u64,
}

impl Notices {
    /// Shows `text` from `now` for [`NOTICE_LIFETIME`], as the newest
    /// notice. A notice already showing that text takes its place, so a
    /// text never shows twice: it keeps its id and shows for the lifetime
    /// again. Past [`NOTICE_CAP`] the oldest goes.
    pub fn push(&mut self, text: String, now: Instant) -> NoticeId {
        let text = clean(&text);
        let until = now + NOTICE_LIFETIME;
        let id = match self.shown.iter().position(|notice| notice.text == text) {
            Some(ix) => self.shown.remove(ix).id,
            None => {
                self.next += 1;
                NoticeId(self.next)
            }
        };
        self.shown.push(Notice { id, text, until });
        let over = self.shown.len().saturating_sub(NOTICE_CAP);
        self.shown.drain(..over);
        id
    }

    /// Drops the notices whose time is up at `now`, and says when the next
    /// of those left goes (`None`: none is left).
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.shown.retain(|notice| notice.until > now);
        self.shown.iter().map(|notice| notice.until).min()
    }

    /// Drops every notice (the config line's dismiss button,
    /// `dismiss_notices`).
    pub fn dismiss_all(&mut self) {
        self.shown.clear();
    }

    /// The notices' texts, oldest first, as the config line shows them.
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        self.shown.iter().map(|notice| notice.text.as_str())
    }

    /// Whether no notice shows.
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }
}

/// The most characters a notice shows; a longer one ends in `…`.
pub const NOTICE_MAX_CHARS: usize = 200;

/// What a notice shows of `text`, which may hold a program's bytes (a link
/// it printed): its lines joined with `; `, without control characters or
/// the invisible format characters that reorder or hide text (bidi
/// embeddings, overrides and isolates, zero-width marks), and cut to
/// [`NOTICE_MAX_CHARS`], so a link can't pass for another notice.
pub fn clean(text: &str) -> String {
    let hidden = |c: char| {
        c.is_control()
            || matches!(
                c,
                '\u{00AD}'
                    | '\u{061C}'
                    | '\u{180E}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2069}'
                    | '\u{FEFF}'
            )
    };
    let joined = text
        .split(['\n', '\r'])
        .map(|part| part.chars().filter(|c| !hidden(*c)).collect::<String>())
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if joined.chars().count() <= NOTICE_MAX_CHARS {
        return joined;
    }
    let mut cut: String = joined.chars().take(NOTICE_MAX_CHARS - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program's link can't hide text, reorder it or run past one line.
    #[test]
    fn a_notice_shows_no_hidden_or_reordering_characters_and_is_cut() {
        assert_eq!(
            clean("could not open\nhttps://a\u{202E}gpj.exe\u{1b}[2m\u{200B}"),
            "could not open; https://agpj.exe[2m"
        );
        let shown = clean(&"x".repeat(500));
        assert_eq!(shown.chars().count(), NOTICE_MAX_CHARS);
        assert!(shown.ends_with('…'));
        let mut notices = Notices::default();
        notices.push("a\u{2066}b".to_string(), Instant::now());
        assert_eq!(texts(&notices), ["ab"]);
    }

    fn texts(notices: &Notices) -> Vec<&str> {
        notices.texts().collect()
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_notice_shows_until_its_lifetime_has_passed() {
        let start = Instant::now();
        let mut notices = Notices::default();
        assert!(notices.is_empty());
        notices.push("couldn't open a".into(), start);
        assert_eq!(texts(&notices), ["couldn't open a"]);
        assert_eq!(notices.expire(start), Some(start + NOTICE_LIFETIME));
        assert_eq!(notices.expire(start + NOTICE_LIFETIME - Duration::from_millis(1)), Some(start + NOTICE_LIFETIME));
        assert_eq!(texts(&notices), ["couldn't open a"], "still within its lifetime");
        assert_eq!(notices.expire(start + NOTICE_LIFETIME), None, "gone at its deadline");
        assert!(notices.is_empty());
    }

    /// The deadline returned is the earliest of those left, and each
    /// notice goes at its own.
    #[test]
    fn expire_drops_only_the_notices_due_and_gives_the_next_deadline() {
        let start = Instant::now();
        let mut notices = Notices::default();
        notices.push("a".into(), start);
        notices.push("b".into(), start + secs(3));
        notices.push("c".into(), start + secs(6));
        assert_eq!(notices.expire(start + secs(1)), Some(start + NOTICE_LIFETIME));
        assert_eq!(notices.expire(start + NOTICE_LIFETIME), Some(start + secs(3) + NOTICE_LIFETIME));
        assert_eq!(texts(&notices), ["b", "c"]);
        assert_eq!(notices.expire(start + secs(3) + NOTICE_LIFETIME), Some(start + secs(6) + NOTICE_LIFETIME));
        assert_eq!(texts(&notices), ["c"]);
        assert_eq!(notices.expire(start + secs(60)), None);
        assert_eq!(texts(&notices), Vec::<&str>::new());
    }

    /// The same text again shows once, as the newest, with its id and a
    /// fresh lifetime.
    #[test]
    fn a_repeated_text_replaces_the_earlier_notice() {
        let start = Instant::now();
        let mut notices = Notices::default();
        let a = notices.push("a".into(), start);
        let b = notices.push("b".into(), start + secs(1));
        assert_ne!(a, b);
        let again = notices.push("a".into(), start + secs(5));
        assert_eq!(again, a, "the same notice");
        assert_eq!(texts(&notices), ["b", "a"], "once, and now the newest");
        // `b` goes at its deadline; `a` lives on from its second showing.
        assert_eq!(notices.expire(start + secs(1) + NOTICE_LIFETIME), Some(start + secs(5) + NOTICE_LIFETIME));
        assert_eq!(texts(&notices), ["a"]);
        let c = notices.push("c".into(), start + secs(12));
        assert!(c != a && c != b, "a new text gets a new id");
    }

    /// The spec's scenario: the same refusal twice, then three others,
    /// leaves the three latest.
    #[test]
    fn at_most_three_show_and_the_oldest_goes() {
        let start = Instant::now();
        let mut notices = Notices::default();
        for (n, text) in ["first", "first", "second", "third"].into_iter().enumerate() {
            notices.push(text.into(), start + secs(n as u64));
        }
        assert_eq!(texts(&notices), ["first", "second", "third"]);
        notices.push("fourth".into(), start + secs(4));
        assert_eq!(texts(&notices), ["second", "third", "fourth"]);
        // The dropped notice's deadline is gone with it.
        assert_eq!(notices.expire(start + secs(4)), Some(start + secs(2) + NOTICE_LIFETIME));
    }

    #[test]
    fn dismiss_all_drops_every_notice() {
        let start = Instant::now();
        let mut notices = Notices::default();
        notices.push("a".into(), start);
        notices.push("b".into(), start);
        notices.dismiss_all();
        assert!(notices.is_empty());
        assert_eq!(notices.expire(start), None, "no deadline left");
        // Ids aren't reused after a dismissal.
        let a = notices.push("a".into(), start);
        let b = notices.push("b".into(), start);
        notices.dismiss_all();
        assert_ne!(notices.push("a".into(), start), a);
        assert_ne!(notices.push("c".into(), start), b);
    }
}
