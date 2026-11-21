//! Mouse pointer shapes: xterm's OSC 22, with kitty's stack and queries
//! (<https://sw.kovidgoyal.net/kitty/pointer-shapes/>), so a program that
//! uses the mouse can say what the spot under it does.
//!
//! [`parse`] reads one `OSC 22` body (after `22;`) into an [`Op`], and
//! [`Stacks::apply`] folds it into a terminal's two stacks, one per screen,
//! returning the reply a query gets. The rules are kitty's (its
//! `set_pointer_shape` and `change_pointer_shape`): a set applies each
//! listed name in turn, replacing the top; a push pushes each in turn; an
//! empty name sets "no shape"; a pop drops the top; a name Brindle doesn't
//! show is skipped, and a query answers each listed item in turn.
//!
//! The lifetime is the client's `Terminal`: a full reset, the program's end
//! and a replay's start empty both stacks. The server keeps none, so after
//! an attach the pointer is Brindle's own until the program sets one again.

/// Entries per stack; a push onto a full stack drops the bottom one.
pub const STACK_DEPTH: usize = 16;

/// The CSS pointer names Brindle shows: the ones GPUI has a platform
/// pointer for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Default,
    Text,
    VerticalText,
    Pointer,
    Crosshair,
    Grab,
    Grabbing,
    NotAllowed,
    NoDrop,
    Alias,
    Copy,
    ContextMenu,
    EwResize,
    NsResize,
    EResize,
    WResize,
    NResize,
    SResize,
    NwseResize,
    NeswResize,
    ColResize,
    RowResize,
}

impl Shape {
    /// Every shape, in the spec's order.
    pub const ALL: [Shape; 22] = [
        Shape::Default,
        Shape::Text,
        Shape::VerticalText,
        Shape::Pointer,
        Shape::Crosshair,
        Shape::Grab,
        Shape::Grabbing,
        Shape::NotAllowed,
        Shape::NoDrop,
        Shape::Alias,
        Shape::Copy,
        Shape::ContextMenu,
        Shape::EwResize,
        Shape::NsResize,
        Shape::EResize,
        Shape::WResize,
        Shape::NResize,
        Shape::SResize,
        Shape::NwseResize,
        Shape::NeswResize,
        Shape::ColResize,
        Shape::RowResize,
    ];

    /// The shape's CSS name, as sequences and replies spell it.
    pub fn name(self) -> &'static str {
        match self {
            Shape::Default => "default",
            Shape::Text => "text",
            Shape::VerticalText => "vertical-text",
            Shape::Pointer => "pointer",
            Shape::Crosshair => "crosshair",
            Shape::Grab => "grab",
            Shape::Grabbing => "grabbing",
            Shape::NotAllowed => "not-allowed",
            Shape::NoDrop => "no-drop",
            Shape::Alias => "alias",
            Shape::Copy => "copy",
            Shape::ContextMenu => "context-menu",
            Shape::EwResize => "ew-resize",
            Shape::NsResize => "ns-resize",
            Shape::EResize => "e-resize",
            Shape::WResize => "w-resize",
            Shape::NResize => "n-resize",
            Shape::SResize => "s-resize",
            Shape::NwseResize => "nwse-resize",
            Shape::NeswResize => "nesw-resize",
            Shape::ColResize => "col-resize",
            Shape::RowResize => "row-resize",
        }
    }

    /// The shape named `name`, or `None` for a name Brindle doesn't show
    /// (another CSS name, a typo, or bytes that are no name at all).
    pub fn from_name(name: &[u8]) -> Option<Shape> {
        Shape::ALL.into_iter().find(|shape| shape.name().as_bytes() == name)
    }
}

/// What one sequence does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Replace the top (push onto an empty stack) with this entry: a shape,
    /// or `None`, "no shape", which shows Brindle's own pointer.
    Set(Option<Shape>),
    /// Push these, in order (1 to [`STACK_DEPTH`] of them).
    Push(Vec<Shape>),
    /// Drop the top, if any.
    Pop,
    /// Answer each item, in order.
    Query(Vec<Ask>),
}

/// One item of a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// A name: whether Brindle shows it.
    Supported(bool),
    /// `__current__`: the top of the active screen's stack.
    Current,
    /// `__default__`: the shape used when no program sets one.
    Default,
    /// `__grabbed__`: the shape used while the program reports the mouse.
    Grabbed,
}

/// The screen a sequence arrived on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Primary,
    Alternate,
}

impl Screen {
    /// The alternate screen when `alternate` (the term's `ALT_SCREEN`
    /// mode), else the primary one.
    pub fn of(alternate: bool) -> Screen {
        if alternate { Screen::Alternate } else { Screen::Primary }
    }

    pub fn name(self) -> &'static str {
        match self {
            Screen::Primary => "primary",
            Screen::Alternate => "alternate",
        }
    }
}

/// Reads an `OSC 22` body: an optional `=`, `>`, `<` or `?`, then a
/// comma-separated list of names. `None` when it does nothing: a set or
/// push naming no shape Brindle shows (a set's empty name counts: it sets
/// "no shape").
pub fn parse(body: &[u8]) -> Option<Op> {
    let (kind, names) = match body.split_first() {
        Some((&kind @ (b'=' | b'>' | b'<' | b'?'), rest)) => (kind, rest),
        Some(_) | None => (b'=', body),
    };
    let names = names.split(|&b| b == b',');
    match kind {
        b'<' => Some(Op::Pop),
        b'?' => Some(Op::Query(names.map(ask).collect())),
        b'>' => {
            let shapes: Vec<Shape> = names.filter_map(Shape::from_name).collect();
            // More than a stack holds would only push the first ones off
            // its bottom.
            let keep = &shapes[shapes.len().saturating_sub(STACK_DEPTH)..];
            (!keep.is_empty()).then(|| Op::Push(keep.to_vec()))
        }
        // Each set replaces the top, so only the last one counts.
        _ => names
            .rev()
            .find_map(|name| if name.is_empty() { Some(None) } else { Shape::from_name(name).map(Some) })
            .map(Op::Set),
    }
}

fn ask(item: &[u8]) -> Ask {
    match item {
        b"__current__" => Ask::Current,
        b"__default__" => Ask::Default,
        b"__grabbed__" => Ask::Grabbed,
        name => Ask::Supported(Shape::from_name(name).is_some()),
    }
}

/// A terminal's pointer stacks, one per screen, bottom first. An entry is a
/// shape or `None`, the "no shape" an empty set leaves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stacks {
    primary: Vec<Option<Shape>>,
    alternate: Vec<Option<Shape>>,
}

impl Stacks {
    fn stack(&self, screen: Screen) -> &Vec<Option<Shape>> {
        match screen {
            Screen::Primary => &self.primary,
            Screen::Alternate => &self.alternate,
        }
    }

    fn stack_mut(&mut self, screen: Screen) -> &mut Vec<Option<Shape>> {
        match screen {
            Screen::Primary => &mut self.primary,
            Screen::Alternate => &mut self.alternate,
        }
    }

    /// Applies `op`, which arrived on `screen`; a query returns its whole
    /// reply sequence, `ESC ] 22 ; <answers> ESC \`.
    pub fn apply(&mut self, screen: Screen, op: Op) -> Option<String> {
        match op {
            Op::Set(entry) => {
                let stack = self.stack_mut(screen);
                match stack.last_mut() {
                    Some(top) => *top = entry,
                    None => stack.push(entry),
                }
                None
            }
            Op::Push(shapes) => {
                let stack = self.stack_mut(screen);
                for shape in shapes {
                    if stack.len() >= STACK_DEPTH {
                        stack.remove(0);
                    }
                    stack.push(Some(shape));
                }
                None
            }
            Op::Pop => {
                self.stack_mut(screen).pop();
                None
            }
            Op::Query(asks) => {
                let answers: Vec<&str> = asks.iter().map(|ask| self.answer(screen, *ask)).collect();
                Some(format!("\x1b]22;{}\x1b\\", answers.join(",")))
            }
        }
    }

    fn answer(&self, screen: Screen, ask: Ask) -> &'static str {
        match ask {
            Ask::Supported(true) => "1",
            Ask::Supported(false) => "0",
            Ask::Current => self.top(screen).map_or("0", Shape::name),
            // Brindle's own: the text beam, and the arrow while the
            // program reports the mouse.
            Ask::Default => Shape::Text.name(),
            Ask::Grabbed => Shape::Default.name(),
        }
    }

    /// The program's shape on `screen`: the top of its stack, or `None`
    /// when it is empty or "no shape" is on top (Brindle's own then).
    pub fn top(&self, screen: Screen) -> Option<Shape> {
        self.stack(screen).last().copied().flatten()
    }

    /// How many entries `screen`'s stack holds.
    #[cfg(test)]
    pub(crate) fn depth(&self, screen: Screen) -> usize {
        self.stack(screen).len()
    }

    /// Empties both stacks; true when either had an entry.
    pub fn clear(&mut self) -> bool {
        let had = !self.primary.is_empty() || !self.alternate.is_empty();
        self.primary.clear();
        self.alternate.clear();
        had
    }

    /// The `--dump-screen-after` line, `--- pointer shape=NAME
    /// screen=primary|alternate`, for the program's shape on `screen`;
    /// empty when it has none.
    pub fn describe(&self, screen: Screen) -> String {
        match self.top(screen) {
            Some(shape) => format!("--- pointer shape={} screen={}\n", shape.name(), screen.name()),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use Screen::{Alternate, Primary};

    /// Applies each body on `screen`, and returns the replies.
    fn feed(stacks: &mut Stacks, screen: Screen, bodies: &[&str]) -> Vec<String> {
        bodies
            .iter()
            .filter_map(|body| parse(body.as_bytes()))
            .filter_map(|op| stacks.apply(screen, op))
            .collect()
    }

    fn after(bodies: &[&str]) -> Stacks {
        let mut stacks = Stacks::default();
        assert!(feed(&mut stacks, Primary, bodies).is_empty());
        stacks
    }

    #[test]
    fn every_shape_round_trips_through_its_name() {
        let mut names: Vec<&str> = Shape::ALL.iter().map(|shape| shape.name()).collect();
        for shape in Shape::ALL {
            assert_eq!(Shape::from_name(shape.name().as_bytes()), Some(shape));
            assert!(shape.name().bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-')));
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Shape::ALL.len(), "names are unique");
        // The spec's list, in its order.
        let spec = "default text vertical-text pointer crosshair grab grabbing not-allowed no-drop alias copy \
                    context-menu ew-resize ns-resize e-resize w-resize n-resize s-resize nwse-resize nesw-resize \
                    col-resize row-resize";
        let listed: Vec<&str> = Shape::ALL.iter().map(|shape| shape.name()).collect();
        assert_eq!(listed, spec.split(' ').collect::<Vec<_>>());
    }

    #[test]
    fn names_brindle_does_not_show_are_unknown() {
        for name in ["wait", "progress", "help", "cell", "move", "zoom-in", "zoom-out", "ne-resize", "sw-resize"] {
            assert_eq!(Shape::from_name(name.as_bytes()), None, "{name}");
        }
        // kitty's X11 aliases, other cases, other characters.
        for name in ["hand", "xterm", "left_ptr", "Pointer", "pointer ", "poin\u{fc}ter", "", "pointer\0"] {
            assert_eq!(Shape::from_name(name.as_bytes()), None, "{name:?}");
        }
    }

    #[test]
    fn push_and_pop() {
        // The spec's scenario: `wait` and `progress` are skipped.
        let stacks = after(&[">pointer", ">wait,progress,default", "<"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Pointer));
        // Several names push in turn, the last on top.
        let mut stacks = after(&[">pointer,crosshair"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Crosshair));
        feed(&mut stacks, Primary, &["<"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Pointer));
        // A pop's names are ignored, and popping an empty stack leaves it.
        let stacks = after(&[">grab", "<grab,pointer", "<", "<"]);
        assert_eq!(stacks, Stacks::default());
        // An empty push does nothing.
        assert_eq!(parse(b">"), None);
        assert_eq!(parse(b">zoom-in,,wait"), None);
    }

    #[test]
    fn a_set_replaces_the_top_with_its_last_known_name() {
        let mut stacks = after(&["pointer,crosshair"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Crosshair));
        // It pushed onto the empty stack once: one pop empties it.
        feed(&mut stacks, Primary, &["<"]);
        assert_eq!(stacks, Stacks::default());
        // `=` is the same, and a set replaces only the top.
        let stacks = after(&[">grab", ">pointer", "=copy"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Copy));
        let mut stacks = stacks;
        feed(&mut stacks, Primary, &["<"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Grab));
    }

    #[test]
    fn the_fallback_list_and_a_list_with_nothing_shown() {
        assert_eq!(after(&["zoom-in,pointer"]).top(Primary), Some(Shape::Pointer));
        // Nothing Brindle shows: no find, and the stack stays.
        assert_eq!(parse(b"zoom-in"), None);
        assert_eq!(after(&["crosshair", "zoom-in"]).top(Primary), Some(Shape::Crosshair));
        // A name with other characters is skipped like any unknown name.
        assert_eq!(after(&["crosshair", "no such,ew-resize"]).top(Primary), Some(Shape::EwResize));
        assert_eq!(after(&["crosshair", "\u{e9}t\u{e9}"]).top(Primary), Some(Shape::Crosshair));
    }

    #[test]
    fn an_empty_body_shows_brindles_own_and_keeps_the_entries_below() {
        assert_eq!(parse(b""), Some(Op::Set(None)));
        assert_eq!(parse(b"="), Some(Op::Set(None)));
        // The spec's reset.
        let stacks = after(&[">crosshair", ""]);
        assert_eq!(stacks.top(Primary), None);
        assert_eq!(stacks.describe(Primary), "");
        // The entry below shows again after a pop.
        let stacks = after(&[">pointer", ">crosshair", "", "<"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Pointer));
        // On an empty stack it pushes "no shape", which one pop removes.
        let mut stacks = after(&[""]);
        assert_ne!(stacks, Stacks::default());
        feed(&mut stacks, Primary, &["<"]);
        assert_eq!(stacks, Stacks::default());
        // A trailing empty name in a set is "no shape" too, as in kitty.
        assert_eq!(parse(b"pointer,"), Some(Op::Set(None)));
        assert_eq!(parse(b",pointer"), Some(Op::Set(Some(Shape::Pointer))));
    }

    #[test]
    fn seventeen_pushes_drop_the_bottom_entry() {
        let mut stacks = Stacks::default();
        let pushes: Vec<String> = (0..17).map(|i| format!(">{}", Shape::ALL[i].name())).collect();
        let pushes: Vec<&str> = pushes.iter().map(String::as_str).collect();
        feed(&mut stacks, Primary, &pushes);
        assert_eq!(stacks.top(Primary), Some(Shape::ALL[16]));
        feed(&mut stacks, Primary, &["<"; 15]);
        // The first push went off the bottom: the second is last.
        assert_eq!(stacks.top(Primary), Some(Shape::ALL[1]));
        feed(&mut stacks, Primary, &["<"]);
        assert_eq!(stacks, Stacks::default());
        assert_eq!(stacks.top(Primary), None);
        // One long push list keeps its last 16, the same as 17 pushes.
        let list = Shape::ALL[..17].iter().map(|shape| shape.name()).collect::<Vec<_>>().join(",");
        match parse(format!(">{list}").as_bytes()) {
            Some(Op::Push(shapes)) => assert_eq!(shapes, Shape::ALL[1..17]),
            other => panic!("not a push: {other:?}"),
        }
    }

    #[test]
    fn each_screen_keeps_its_own_stack() {
        let mut stacks = Stacks::default();
        feed(&mut stacks, Primary, &[">pointer"]);
        feed(&mut stacks, Alternate, &[">crosshair"]);
        assert_eq!(stacks.top(Alternate), Some(Shape::Crosshair));
        assert_eq!(stacks.top(Primary), Some(Shape::Pointer));
        assert_eq!(stacks.describe(Primary), "--- pointer shape=pointer screen=primary\n");
        assert_eq!(stacks.describe(Alternate), "--- pointer shape=crosshair screen=alternate\n");
        // A pop on one leaves the other.
        feed(&mut stacks, Alternate, &["<"]);
        assert_eq!(stacks.top(Alternate), None);
        assert_eq!(stacks.top(Primary), Some(Shape::Pointer));
        // A clear empties both, once.
        feed(&mut stacks, Alternate, &[">grab"]);
        assert!(stacks.clear());
        assert_eq!(stacks, Stacks::default());
        assert!(!stacks.clear());
    }

    #[test]
    fn queries_reply_byte_for_byte() {
        let mut stacks = Stacks::default();
        let reply = |stacks: &mut Stacks, screen, body: &str| feed(stacks, screen, &[body]);
        assert_eq!(reply(&mut stacks, Primary, "?pointer,crosshair,no-such-name,wait"), ["\x1b]22;1,1,0,0\x1b\\"]);
        assert_eq!(reply(&mut stacks, Primary, "?__current__"), ["\x1b]22;0\x1b\\"]);
        assert_eq!(reply(&mut stacks, Primary, "?__default__"), ["\x1b]22;text\x1b\\"]);
        assert_eq!(reply(&mut stacks, Primary, "?__grabbed__"), ["\x1b]22;default\x1b\\"]);
        // Mixed, in order.
        assert_eq!(reply(&mut stacks, Primary, "?__current__,pointer,__default__"), ["\x1b]22;0,1,text\x1b\\"]);
        // The spec's current, on the screen it was asked on.
        feed(&mut stacks, Primary, &[">pointer", ">grab"]);
        assert_eq!(reply(&mut stacks, Primary, "?__current__"), ["\x1b]22;grab\x1b\\"]);
        assert_eq!(reply(&mut stacks, Alternate, "?__current__"), ["\x1b]22;0\x1b\\"]);
        // "No shape" on top reads as none.
        feed(&mut stacks, Primary, &[""]);
        assert_eq!(reply(&mut stacks, Primary, "?__current__"), ["\x1b]22;0\x1b\\"]);
        // An empty query, empty items and other characters answer 0.
        assert_eq!(reply(&mut stacks, Primary, "?"), ["\x1b]22;0\x1b\\"]);
        assert_eq!(reply(&mut stacks, Primary, "?,text,"), ["\x1b]22;0,1,0\x1b\\"]);
        assert_eq!(reply(&mut stacks, Primary, "?\u{1b}x,__CURRENT__"), ["\x1b]22;0,0\x1b\\"]);
        // A query changes no stack.
        assert_eq!(stacks.top(Primary), None);
        feed(&mut stacks, Primary, &["<"]);
        assert_eq!(stacks.top(Primary), Some(Shape::Pointer));
    }

    #[test]
    fn parse_never_panics_and_queries_answer_every_item() {
        // Every first byte, and bodies built from the bytes that matter.
        for first in 0..=255u8 {
            parse(&[first]);
            parse(&[first, b',', first]);
        }
        let pieces: [&[u8]; 9] = [b"=", b">", b"<", b"?", b",", b"pointer", b"__current__", b"\xff", b""];
        for a in pieces {
            for b in pieces {
                for c in pieces {
                    let body = [a, b, c].concat();
                    if let Some(Op::Query(asks)) = parse(&body) {
                        let count = body[1..].split(|&byte| byte == b',').count();
                        assert_eq!(asks.len(), count, "{body:?}");
                    }
                }
            }
        }
    }
}
