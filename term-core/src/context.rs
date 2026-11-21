//! Program contexts: the UAPI group's hierarchical context signalling
//! (UAPI.15, OSC 3008), which systemd 257 and later send when output moves
//! into another context: `run0` (elevation), `systemd-nspawn` and
//! `machinectl` (containers and VMs), ssh logins, and others.
//!
//! [`parse`] reads one `OSC 3008` payload (the body after `3008;`) into a
//! [`Message`], [`Contexts::apply`] folds it into a terminal's stack, and
//! [`Contexts::notable`] picks the one its tab and zone header show as a
//! [`Badge`]. Only the active chain is ever open: a start nests under the
//! active context, and an update or end of a context ends everything after
//! it, so the stack is a `Vec` (index 0 outermost, the last active).
//!
//! The lifetime is the callers', the client's `Terminal` and the server's
//! [`Emulator`](crate::emulator::Emulator): only the program's exit drops
//! the stack (UAPI's hangup reset). A full or soft reset, a new prompt and
//! the alternate screen leave it.

/// The most contexts a terminal keeps; a start past it is ignored.
pub const MAX_DEPTH: usize = 32;
/// The longest id, in bytes after decoding.
pub const MAX_ID: usize = 64;
/// The longest field value, in bytes after decoding.
pub const MAX_VALUE: usize = 255;
/// The longest badge label, in characters, `…` included.
pub const MAX_LABEL_CHARS: usize = 24;

/// A context's id: 1 to [`MAX_ID`] bytes in 0x20–0x7E, decoded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContextId(String);

impl ContextId {
    /// The id in `raw` (escapes decoded), or `None` when it isn't one.
    fn parse(raw: &[u8]) -> Option<Self> {
        let bytes = decode(raw)?;
        let printable = bytes.iter().all(|b| (0x20..=0x7e).contains(b));
        if bytes.is_empty() || bytes.len() > MAX_ID || !printable {
            return None;
        }
        // Printable ASCII is UTF-8.
        String::from_utf8(bytes).ok().map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A field value: decoded, valid UTF-8, at most [`MAX_VALUE`] bytes and
/// without control characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Text(String);

impl Text {
    /// The value in `raw` (escapes decoded), or `None` when it isn't one.
    fn parse(raw: &[u8]) -> Option<Self> {
        let bytes = decode(raw)?;
        if bytes.len() > MAX_VALUE {
            return None;
        }
        let text = String::from_utf8(bytes).ok()?;
        (!text.chars().any(char::is_control)).then_some(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A context's `type`, from UAPI.15's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Service,
    Session,
    Shell,
    Command,
    Vm,
    Container,
    Elevate,
    Chpriv,
    Subcontext,
    Remote,
    Boot,
    App,
}

impl Kind {
    const ALL: [Kind; 12] = [
        Kind::Service,
        Kind::Session,
        Kind::Shell,
        Kind::Command,
        Kind::Vm,
        Kind::Container,
        Kind::Elevate,
        Kind::Chpriv,
        Kind::Subcontext,
        Kind::Remote,
        Kind::Boot,
        Kind::App,
    ];

    /// Its name on the wire and in the dump.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Service => "service",
            Kind::Session => "session",
            Kind::Shell => "shell",
            Kind::Command => "command",
            Kind::Vm => "vm",
            Kind::Container => "container",
            Kind::Elevate => "elevate",
            Kind::Chpriv => "chpriv",
            Kind::Subcontext => "subcontext",
            Kind::Remote => "remote",
            Kind::Boot => "boot",
            Kind::App => "app",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == text)
    }
}

/// The fields of a start that Brindle keeps. The rest (`machineid`,
/// `bootid`, `pid`, `cmdline`, …) are parsed past and dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Fields {
    /// `None` when absent or outside UAPI.15's list.
    pub kind: Option<Kind>,
    pub user: Option<Text>,
    pub hostname: Option<Text>,
    pub targetuser: Option<Text>,
    pub targethost: Option<Text>,
    pub container: Option<Text>,
    pub vm: Option<Text>,
    pub cwd: Option<Text>,
    pub comm: Option<Text>,
}

impl Fields {
    /// The kept text fields by their wire names, in the order the replay
    /// writes them.
    fn texts(&self) -> [(&'static str, Option<&Text>); 8] {
        [
            ("user", self.user.as_ref()),
            ("hostname", self.hostname.as_ref()),
            ("targetuser", self.targetuser.as_ref()),
            ("targethost", self.targethost.as_ref()),
            ("container", self.container.as_ref()),
            ("vm", self.vm.as_ref()),
            ("cwd", self.cwd.as_ref()),
            ("comm", self.comm.as_ref()),
        ]
    }

    /// Takes `name=value` if Brindle keeps that field and the value is
    /// well formed; anything else is dropped.
    fn take(&mut self, name: &[u8], value: &[u8]) {
        let slot = match name {
            b"type" => {
                // An unknown type is no type: the context stays, untyped.
                self.kind = Text::parse(value).and_then(|text| Kind::parse(text.as_str()));
                return;
            }
            b"user" => &mut self.user,
            b"hostname" => &mut self.hostname,
            b"targetuser" => &mut self.targetuser,
            b"targethost" => &mut self.targethost,
            b"container" => &mut self.container,
            b"vm" => &mut self.vm,
            b"cwd" => &mut self.cwd,
            b"comm" => &mut self.comm,
            _ => return,
        };
        if let Some(text) = Text::parse(value) {
            *slot = Some(text);
        }
    }

    /// The badge this context would give, if its kind is notable.
    fn badge(&self) -> Option<Badge> {
        let text = |field: &Option<Text>| field.as_ref().map(|t| t.as_str().to_owned());
        let (kind, label) = match self.kind? {
            Kind::Elevate | Kind::Chpriv => {
                (BadgeKind::Privileged, text(&self.targetuser).unwrap_or_else(|| "elevated".into()))
            }
            Kind::Remote => {
                let label = match (&self.targetuser, &self.targethost) {
                    (Some(user), Some(host)) => format!("{}@{}", user.as_str(), host.as_str()),
                    (None, Some(host)) => host.as_str().to_owned(),
                    (Some(_) | None, None) => "remote".into(),
                };
                (BadgeKind::Elsewhere, label)
            }
            Kind::Container => (BadgeKind::Elsewhere, text(&self.container).unwrap_or_else(|| "container".into())),
            Kind::Vm => (BadgeKind::Elsewhere, text(&self.vm).unwrap_or_else(|| "vm".into())),
            Kind::Service
            | Kind::Session
            | Kind::Shell
            | Kind::Command
            | Kind::Subcontext
            | Kind::Boot
            | Kind::App => return None,
        };
        Some(Badge { kind, label: shorten(&label) })
    }
}

/// One `OSC 3008` sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Start(ContextId, Fields),
    End(ContextId),
}

/// The message in an `OSC 3008` payload: `start=<id>` or `end=<id>` first,
/// then `;`-separated `name=value` fields in any order. `None` when it
/// doesn't start so or its id isn't one; a field that is unknown or
/// malformed is dropped and the rest kept.
pub fn parse(payload: &[u8]) -> Option<Message> {
    let mut fields = payload.split(|&b| b == b';');
    let first = fields.next()?;
    let (name, raw_id) = split_field(first)?;
    let id = ContextId::parse(raw_id)?;
    match name {
        b"start" => {
            let mut kept = Fields::default();
            for field in fields {
                if let Some((name, value)) = split_field(field) {
                    kept.take(name, value);
                }
            }
            Some(Message::Start(id, kept))
        }
        // An end's fields (`exit=`, `status=`, `signal=`) aren't used.
        b"end" => Some(Message::End(id)),
        _ => None,
    }
}

/// `name=value` at its first `=`.
fn split_field(field: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = field.iter().position(|&b| b == b'=')?;
    Some((&field[..at], &field[at + 1..]))
}

/// `raw` with each `\xHH` decoded to its byte; `None` for a `\` that
/// doesn't start one.
fn decode(raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut bytes = raw.iter();
    while let Some(&byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        if bytes.next() != Some(&b'x') {
            return None;
        }
        let high = hex(*bytes.next()?)?;
        let low = hex(*bytes.next()?)?;
        out.push(high << 4 | low);
    }
    Some(out)
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).and_then(|digit| u8::try_from(digit).ok())
}

/// `text` for a payload: `;`, `\` and bytes outside 0x20–0x7E as `\xHH`,
/// so [`decode`] gives `text` back.
fn encode(text: &str, out: &mut String) {
    for &byte in text.as_bytes() {
        if byte == b';' || byte == b'\\' || !(0x20..=0x7e).contains(&byte) {
            out.push_str(&format!("\\x{byte:02x}"));
        } else {
            out.push(char::from(byte));
        }
    }
}

/// `label`, cut to [`MAX_LABEL_CHARS`] with `…` last when it is longer.
fn shorten(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL_CHARS {
        return label.to_owned();
    }
    let mut out: String = label.chars().take(MAX_LABEL_CHARS - 1).collect();
    out.push('…');
    out
}

/// How a badge is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BadgeKind {
    /// An elevation or privilege change (`elevate`, `chpriv`): red.
    Privileged,
    /// Another host, a container or a VM: muted.
    Elsewhere,
}

/// What a tab face or zone header shows for a terminal's notable context.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Badge {
    kind: BadgeKind,
    label: String,
}

impl Badge {
    pub fn kind(&self) -> BadgeKind {
        self.kind
    }

    /// At most [`MAX_LABEL_CHARS`] characters.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// `kind=privileged|elsewhere label="…"`, as the dump prints it.
    pub fn describe(&self) -> String {
        let kind = match self.kind {
            BadgeKind::Privileged => "privileged",
            BadgeKind::Elsewhere => "elsewhere",
        };
        format!("kind={kind} label={:?}", self.label)
    }
}

/// A terminal's open contexts, outermost first; the last is active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contexts {
    stack: Vec<(ContextId, Fields)>,
}

impl Contexts {
    /// Applies `message`; true when the stack changed. A start of a new id
    /// nests it under the active context (ignored past [`MAX_DEPTH`]); a
    /// start of a known id replaces its fields, ends its subcontexts and
    /// makes it active; an end ends that context and its subcontexts, and
    /// one for an unknown id is ignored.
    pub fn apply(&mut self, message: Message) -> bool {
        match message {
            Message::Start(id, fields) => match self.position(&id) {
                Some(at) => {
                    let changed = self.stack.len() != at + 1 || self.stack[at].1 != fields;
                    self.stack.truncate(at + 1);
                    self.stack[at].1 = fields;
                    changed
                }
                None if self.stack.len() < MAX_DEPTH => {
                    self.stack.push((id, fields));
                    true
                }
                None => false,
            },
            Message::End(id) => match self.position(&id) {
                Some(at) => {
                    self.stack.truncate(at);
                    true
                }
                None => false,
            },
        }
    }

    /// Drops every context (the program exited); true when there were any.
    pub fn clear(&mut self) -> bool {
        let had = !self.stack.is_empty();
        self.stack.clear();
        had
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// The open contexts, outermost first.
    pub fn iter(&self) -> impl Iterator<Item = (&ContextId, &Fields)> {
        self.stack.iter().map(|(id, fields)| (id, fields))
    }

    /// The badge of the innermost context of a notable kind (`elevate`,
    /// `chpriv`, `remote`, `container`, `vm`), if any.
    pub fn notable(&self) -> Option<Badge> {
        self.stack.iter().rev().find_map(|(_, fields)| fields.badge())
    }

    /// One `OSC 3008` payload per open context, outermost first: fed in
    /// order to [`parse`] and [`Contexts::apply`] from none, they give this
    /// stack back.
    pub fn encode(&self) -> Vec<String> {
        self.stack
            .iter()
            .map(|(id, fields)| {
                let mut out = String::from("start=");
                encode(id.as_str(), &mut out);
                if let Some(kind) = fields.kind {
                    out.push_str(";type=");
                    out.push_str(kind.name());
                }
                for (name, text) in fields.texts() {
                    if let Some(text) = text {
                        out.push(';');
                        out.push_str(name);
                        out.push('=');
                        encode(text.as_str(), &mut out);
                    }
                }
                out
            })
            .collect()
    }

    /// The `--dump-screen-after` lines, `--- context id="…" type=…[
    /// label="…"]` per open context, outermost first; `type=none` for an
    /// untyped one, and `label` for one of a notable kind.
    pub fn describe(&self) -> String {
        self.stack
            .iter()
            .map(|(id, fields)| {
                let kind = fields.kind.map_or("none", Kind::name);
                let label = fields.badge().map(|badge| format!(" label={:?}", badge.label)).unwrap_or_default();
                format!("--- context id={:?} type={kind}{label}\n", id.as_str())
            })
            .collect()
    }

    fn position(&self, id: &ContextId) -> Option<usize> {
        self.stack.iter().position(|(open, _)| open == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stack after each payload in turn, from none.
    fn after(payloads: &[&str]) -> Contexts {
        let mut contexts = Contexts::default();
        for payload in payloads {
            if let Some(message) = parse(payload.as_bytes()) {
                contexts.apply(message);
            }
        }
        contexts
    }

    fn ids(contexts: &Contexts) -> Vec<&str> {
        contexts.iter().map(|(id, _)| id.as_str()).collect()
    }

    fn active(contexts: &Contexts) -> Option<(&str, &Fields)> {
        contexts.iter().last().map(|(id, fields)| (id.as_str(), fields))
    }

    fn text(value: &str) -> Option<Text> {
        Some(Text(value.into()))
    }

    #[test]
    fn run0_starts_an_elevation() {
        let contexts = after(&["start=a1;type=elevate;user=me;targetuser=root"]);
        let (id, fields) = active(&contexts).unwrap();
        assert_eq!(id, "a1");
        assert_eq!(fields.kind, Some(Kind::Elevate));
        assert_eq!(fields.user, text("me"));
        assert_eq!(fields.targetuser, text("root"));
    }

    #[test]
    fn start_or_end_must_come_first() {
        assert_eq!(parse(b"type=elevate;start=a1"), None);
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"start"), None);
        assert_eq!(parse(b"begin=a1"), None);
        assert_eq!(parse(b"end=a1;exit=failure;status=1"), Some(Message::End(ContextId("a1".into()))));
        assert!(after(&["type=elevate;start=a1"]).is_empty());
    }

    #[test]
    fn escapes_decode_in_ids_and_values() {
        let contexts = after(&["start=c1;type=container;container=web\\x3bdb"]);
        assert_eq!(active(&contexts).unwrap().1.container, text("web;db"));
        let contexts = after(&["start=a\\x5cb\\x3B;hostname=h\\xc3\\xa9"]);
        let (id, fields) = active(&contexts).unwrap();
        assert_eq!(id, "a\\b;");
        assert_eq!(fields.hostname, text("hé"));
    }

    #[test]
    fn bad_ids_are_ignored() {
        let long = format!("start={}", "x".repeat(65));
        let longest = format!("start={}", "x".repeat(64));
        for payload in ["start=", "start=a\\", "start=a\\x4", "start=a\\y41", "start=a\\x0a", "start=caf\\xc3\\xa9", &long] {
            assert_eq!(parse(payload.as_bytes()), None, "{payload:?}");
        }
        assert!(matches!(parse(longest.as_bytes()), Some(Message::Start(..))));
        // An escape counts as the one byte it decodes to.
        let escaped = format!("start={}", "\\x41".repeat(64));
        assert!(matches!(parse(escaped.as_bytes()), Some(Message::Start(..))));
    }

    #[test]
    fn a_bad_type_leaves_the_context_untyped() {
        let contexts = after(&["start=x;type=spaceship;hostname=h"]);
        let (id, fields) = active(&contexts).unwrap();
        assert_eq!(id, "x");
        assert_eq!(fields.kind, None);
        assert_eq!(fields.hostname, text("h"));
    }

    #[test]
    fn malformed_and_unknown_fields_are_dropped_and_the_rest_kept() {
        let long = "v".repeat(256);
        let longest = "v".repeat(255);
        let payload = format!(
            "start=x;type=vm;vm={long};hostname={longest};user=a\\x07b;cwd=\\xff;comm=bad\\q;\
             machineid=abc;pid=12;=;novalue;;targethost=host"
        );
        let contexts = after(&[&payload]);
        let fields = active(&contexts).unwrap().1;
        assert_eq!(fields.kind, Some(Kind::Vm));
        assert_eq!((&fields.vm, &fields.user, &fields.cwd, &fields.comm), (&None, &None, &None, &None));
        assert_eq!(fields.hostname, text(&longest));
        assert_eq!(fields.targethost, text("host"));
        // DEL and C1 controls count as controls.
        let fields = &after(&["start=x;user=a\\x7fb;hostname=a\\xc2\\x85b"]).stack[0].1;
        assert_eq!((&fields.user, &fields.hostname), (&None, &None));
    }

    #[test]
    fn every_type_in_uapi_15_parses() {
        for kind in Kind::ALL {
            let contexts = after(&[&format!("start=x;type={}", kind.name())]);
            assert_eq!(active(&contexts).unwrap().1.kind, Some(kind));
        }
    }

    #[test]
    fn a_new_start_nests_under_the_active_context() {
        let contexts = after(&["start=a;type=session", "start=b;type=shell", "start=c;type=command"]);
        assert_eq!(ids(&contexts), ["a", "b", "c"]);
    }

    #[test]
    fn ending_a_context_ends_its_subcontexts() {
        let contexts = after(&["start=a;type=elevate", "start=b;type=container", "end=a"]);
        assert!(contexts.is_empty());
        let contexts = after(&["start=a", "start=b", "start=c", "end=b"]);
        assert_eq!(ids(&contexts), ["a"]);
    }

    #[test]
    fn an_unknown_end_is_ignored() {
        let mut contexts = after(&["start=a;type=elevate"]);
        assert!(!contexts.apply(parse(b"end=zzz").unwrap()));
        assert_eq!(ids(&contexts), ["a"]);
    }

    #[test]
    fn starting_a_known_id_updates_it_and_ends_its_subcontexts() {
        let contexts = after(&["start=a;type=shell", "start=b;type=command", "start=a;type=elevate;targetuser=root"]);
        assert_eq!(ids(&contexts), ["a"]);
        let fields = active(&contexts).unwrap().1;
        assert_eq!((fields.kind, &fields.targetuser), (Some(Kind::Elevate), &text("root")));
        // The same start again changes nothing.
        let mut contexts = after(&["start=a;type=shell"]);
        assert!(!contexts.apply(parse(b"start=a;type=shell").unwrap()));
        assert!(contexts.apply(parse(b"start=a;type=shell;cwd=/").unwrap()));
    }

    #[test]
    fn past_32_contexts_a_start_is_ignored() {
        let starts: Vec<String> = (1..=33).map(|n| format!("start=c{n}")).collect();
        let mut contexts = after(&starts.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(contexts.iter().count(), MAX_DEPTH);
        assert_eq!(active(&contexts).unwrap().0, "c32");
        assert!(!contexts.apply(parse(b"start=c34").unwrap()));
        assert!(!contexts.apply(parse(b"end=c33").unwrap()), "the 33rd was never recorded");
        assert!(contexts.apply(parse(b"end=c32").unwrap()));
        assert_eq!(active(&contexts).unwrap().0, "c31");
        // An update of a known one is not a new start.
        let mut full = after(&starts[..32].iter().map(String::as_str).collect::<Vec<_>>());
        assert!(full.apply(parse(b"start=c32;type=shell").unwrap()));
    }

    #[test]
    fn clear_drops_everything() {
        let mut contexts = after(&["start=a", "start=b"]);
        assert!(contexts.clear());
        assert!(contexts.is_empty());
        assert!(!contexts.clear());
    }

    fn badge(payloads: &[&str]) -> Option<(BadgeKind, String)> {
        after(payloads).notable().map(|badge| (badge.kind(), badge.label().to_owned()))
    }

    #[test]
    fn the_notable_context_is_the_innermost_notable_kind() {
        use BadgeKind::{Elsewhere, Privileged};
        assert_eq!(badge(&["start=c;type=container;container=web", "start=s;type=shell"]), Some((Elsewhere, "web".into())));
        assert_eq!(badge(&["start=a;type=session", "start=b;type=shell"]), None);
        assert_eq!(badge(&[]), None);
        assert_eq!(
            badge(&["start=c;type=container;container=web", "start=e;type=elevate;targetuser=root"]),
            Some((Privileged, "root".into()))
        );
        assert_eq!(
            badge(&["start=e;type=elevate;targetuser=root", "start=c;type=container;container=web", "start=s;type=shell"]),
            Some((Elsewhere, "web".into()))
        );
        assert_eq!(badge(&["start=x;type=spaceship;container=web"]), None, "an untyped context is not notable");
    }

    #[test]
    fn labels_follow_the_kind() {
        use BadgeKind::{Elsewhere, Privileged};
        assert_eq!(badge(&["start=e;type=elevate"]), Some((Privileged, "elevated".into())));
        assert_eq!(badge(&["start=e;type=chpriv;targetuser=nobody"]), Some((Privileged, "nobody".into())));
        assert_eq!(badge(&["start=e;type=chpriv;user=me"]), Some((Privileged, "elevated".into())));
        assert_eq!(badge(&["start=r;type=remote;targetuser=me;targethost=box"]), Some((Elsewhere, "me@box".into())));
        assert_eq!(badge(&["start=r;type=remote;targethost=box"]), Some((Elsewhere, "box".into())));
        assert_eq!(badge(&["start=r;type=remote;targetuser=me;hostname=here"]), Some((Elsewhere, "remote".into())));
        assert_eq!(badge(&["start=c;type=container"]), Some((Elsewhere, "container".into())));
        assert_eq!(badge(&["start=v;type=vm;vm=win11"]), Some((Elsewhere, "win11".into())));
        assert_eq!(badge(&["start=v;type=vm"]), Some((Elsewhere, "vm".into())));
    }

    #[test]
    fn long_labels_end_in_an_ellipsis_at_24_characters() {
        let exact = "a".repeat(24);
        assert_eq!(badge(&[&format!("start=c;type=container;container={exact}")]).unwrap().1, exact);
        let label = badge(&["start=c;type=container;container=a-very-long-container-name-indeed"]).unwrap().1;
        assert_eq!(label, "a-very-long-container-n…");
        assert_eq!(label.chars().count(), 24);
        // Characters, not bytes.
        let wide = "é".repeat(30);
        let label = badge(&[&format!("start=c;type=container;container={wide}")]).unwrap().1;
        assert_eq!(label, format!("{}…", "é".repeat(23)));
    }

    #[test]
    fn an_encoded_stack_parses_back_from_none() {
        let contexts = after(&[
            "start=a\\x3bb\\x5c;type=elevate;user=me;targetuser=root",
            "start=c 1;type=container;container=web\\x3bdb;hostname=h\\xc3\\xa9;cwd=/srv/x=y;comm=bash",
            "start=r;targethost=box;vm=v;targetuser=u",
        ]);
        assert_eq!(contexts.iter().count(), 3);
        let payloads = contexts.encode();
        assert!(payloads.iter().all(|p| p.bytes().all(|b| (0x20..=0x7e).contains(&b))), "{payloads:?}");
        assert_eq!(payloads[0], "start=a\\x3bb\\x5c;type=elevate;user=me;targetuser=root");
        let back = after(&payloads.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(back, contexts);
        assert!(Contexts::default().encode().is_empty());
    }

    #[test]
    fn describe_prints_one_line_per_context() {
        let contexts = after(&["start=s;type=session", "start=e;type=elevate;targetuser=root", "start=x\"y"]);
        assert_eq!(
            contexts.describe(),
            "--- context id=\"s\" type=session\n--- context id=\"e\" type=elevate label=\"root\"\n\
             --- context id=\"x\\\"y\" type=none\n"
        );
        assert_eq!(Contexts::default().describe(), "");
        assert_eq!(contexts.notable().unwrap().describe(), "kind=privileged label=\"root\"");
        let remote = after(&["start=r;type=remote;targethost=box"]).notable().unwrap();
        assert_eq!(remote.describe(), "kind=elsewhere label=\"box\"");
    }
}
