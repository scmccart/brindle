//! `Message` and `Bubble`: AI Elements' message, one turn's text in a
//! bubble.
//!
//! The person's bubble ([`From::User`]) sits at the end of the row in the
//! muted colours; the other side's ([`From::Assistant`]) at the start, on
//! the card with a border. The owner passes the text (or any element) as
//! children; a text keeps its line breaks and wraps long lines, as gpui
//! lays text out.
//!
//! A long message can be drawn as several items of a list (a
//! [`MessageScroller`](crate::components::MessageScroller) re-lays out only
//! the item that changed), each a [`BubblePart`]: the first rounds only its
//! top corners and the last only its bottom ones, the middle ones neither,
//! and only the outer edges are bordered and padded, so the parts read as
//! one bubble.
//!
//! Actions (a copy button, say) go under the bubble of a whole message or
//! its last part, shown while the pointer is over the message (AI
//! Elements' `MessageActions` under a `group-hover`). They are drawn only
//! then, so they take no keyboard focus: the owner gives each its keyboard
//! path (a palette action).

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, prelude::FluentBuilder, relative, rems,
};

use crate::theme::{ActiveTheme, Tokens};

/// Whose turn a message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum From {
    /// The person at the keyboard: at the end, muted.
    User,
    /// The other side: at the start, on the card.
    Assistant,
}

/// Which part of a bubble an item draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BubblePart {
    /// The whole bubble.
    #[default]
    Whole,
    First,
    Middle,
    Last,
}

/// Which outer edges a part draws: its rounded corners, border and padding
/// at the top and at the bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BubbleEdges {
    pub top: bool,
    pub bottom: bool,
}

impl BubblePart {
    pub fn edges(self) -> BubbleEdges {
        match self {
            BubblePart::Whole => BubbleEdges { top: true, bottom: true },
            BubblePart::First => BubbleEdges { top: true, bottom: false },
            BubblePart::Middle => BubbleEdges { top: false, bottom: false },
            BubblePart::Last => BubbleEdges { top: false, bottom: true },
        }
    }
}

/// A bubble's colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleStyle {
    pub background: gpui::Hsla,
    pub foreground: gpui::Hsla,
    /// The card's border, for the other side's bubble.
    pub border: Option<gpui::Hsla>,
}

/// The person's bubble in `bg-muted text-foreground`, the other side's on
/// `bg-card text-card-foreground border`.
pub fn bubble_style(from: From, t: &Tokens) -> BubbleStyle {
    match from {
        From::User => BubbleStyle { background: t.muted, foreground: t.foreground, border: None },
        From::Assistant => BubbleStyle { background: t.card, foreground: t.card_foreground, border: Some(t.border) },
    }
}

/// A bubble, or a part of one. Its children are the text.
#[derive(IntoElement)]
pub struct Bubble {
    from: From,
    part: BubblePart,
    children: Vec<AnyElement>,
}

impl Bubble {
    pub fn new(from: From) -> Self {
        Self { from, part: BubblePart::Whole, children: Vec::new() }
    }

    pub fn part(mut self, part: BubblePart) -> Self {
        self.part = part;
        self
    }
}

impl ParentElement for Bubble {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Bubble {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let style = bubble_style(self.from, t);
        let edges = self.part.edges();
        let radius = t.radius_lg();
        // rounded-lg px-4 py-3 text-sm, its outer edges only.
        let mut bubble = div()
            .flex()
            .flex_col()
            .min_w_0()
            .px_4()
            .when(edges.top, |d| d.pt_3())
            .when(edges.bottom, |d| d.pb_3())
            .text_size(rems(0.875))
            .bg(style.background)
            .text_color(style.foreground)
            .when(edges.top, |d| d.rounded_t(radius))
            .when(edges.bottom, |d| d.rounded_b(radius))
            .children(self.children);
        if let Some(border) = style.border {
            bubble = bubble
                .border_color(border)
                .border_l_1()
                .border_r_1()
                .when(edges.top, |d| d.border_t_1())
                .when(edges.bottom, |d| d.border_b_1());
        }
        bubble
    }
}

/// The group a message's hover actions follow.
const GROUP: &str = "brindle-ui-message";

/// One message: a [`Bubble`] aligned to its side, at most four fifths of
/// the row wide, and the actions under it while the pointer is over it.
#[derive(IntoElement)]
pub struct Message {
    id: ElementId,
    from: From,
    part: BubblePart,
    children: Vec<AnyElement>,
    actions: Vec<AnyElement>,
}

impl Message {
    /// `id` is unique among the messages of one list.
    pub fn new(id: impl Into<ElementId>, from: From) -> Self {
        Self { id: id.into(), from, part: BubblePart::Whole, children: Vec::new(), actions: Vec::new() }
    }

    /// A message of plain text.
    pub fn text(id: impl Into<ElementId>, from: From, text: impl Into<SharedString>) -> Self {
        Self::new(id, from).child(text.into())
    }

    pub fn part(mut self, part: BubblePart) -> Self {
        self.part = part;
        self
    }

    /// An action shown under the bubble while the pointer is over the
    /// message (on a whole message or its last part).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl ParentElement for Message {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Message {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let edges = self.part.edges();
        let user = self.from == From::User;
        let actions = (edges.bottom && !self.actions.is_empty()).then(|| {
            div()
                .flex()
                .gap_1()
                .pt_1()
                .when(user, |d| d.justify_end())
                .invisible()
                .group_hover(GROUP, |s| s.visible())
                .children(self.actions)
        });
        // The row: is-user justify-end, is-assistant justify-start; py-2
        // between messages, none between the parts of one.
        div()
            .id(self.id)
            .group(GROUP)
            .flex()
            .w_full()
            .when(user, |d| d.justify_end())
            .when(edges.top, |d| d.pt_2())
            .when(edges.bottom, |d| d.pb_2())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .max_w(relative(0.8))
                    .min_w_0()
                    .when(user, |d| d.items_end())
                    .child(Bubble::new(self.from).part(self.part).children(self.children))
                    .children(actions),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Render, TestAppContext, px};

    use super::*;
    use crate::theme::Tokens;
    use crate::test_support;

    #[test]
    fn parts_draw_only_their_outer_edges() {
        assert_eq!(BubblePart::Whole.edges(), BubbleEdges { top: true, bottom: true });
        assert_eq!(BubblePart::First.edges(), BubbleEdges { top: true, bottom: false });
        assert_eq!(BubblePart::Middle.edges(), BubbleEdges { top: false, bottom: false });
        assert_eq!(BubblePart::Last.edges(), BubbleEdges { top: false, bottom: true });
    }

    #[test]
    fn the_person_is_muted_and_the_agent_on_the_card() {
        let t = Tokens::neutral_dark();
        assert_eq!(bubble_style(From::User, &t), BubbleStyle { background: t.muted, foreground: t.foreground, border: None });
        let agent = bubble_style(From::Assistant, &t);
        assert_eq!((agent.background, agent.foreground, agent.border), (t.card, t.card_foreground, Some(t.border)));
    }

    struct Host;

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let text = |selector: &'static str, text: &'static str| {
                div().debug_selector(move || selector.into()).child(text)
            };
            div()
                .w(px(400.))
                .flex()
                .flex_col()
                .child(Message::new("u", From::User).child(text("user", "hi")))
                .child(Message::new("a", From::Assistant).child(text("agent", "hello\nthere")))
        }
    }

    /// The spec's "Two sides": the person's bubble ends at the row's end,
    /// the agent's starts at its start (inside their px-4 padding and the
    /// agent's border), and the agent's text keeps its line break.
    #[test]
    fn the_two_sides_sit_at_either_end() {
        let mut app: TestAppContext = test_support::app();
        let (_, cx) = app.add_window_view(|_, _| Host);
        cx.run_until_parked();
        let user = cx.debug_bounds("user").expect("drawn");
        let agent = cx.debug_bounds("agent").expect("drawn");
        assert_eq!(user.right(), px(400. - 16.));
        assert!(user.left() > px(200.), "{user:?}");
        assert_eq!(agent.left(), px(1. + 16.));
        // Two lines (laid out to the half pixel).
        assert!((agent.size.height - user.size.height * 2.).abs() <= px(0.5), "{agent:?} {user:?}");
    }

    /// A message whose action records each time it is drawn.
    struct WithAction {
        drawn: std::rc::Rc<std::cell::Cell<bool>>,
    }

    impl Render for WithAction {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let drawn = self.drawn.clone();
            div().w(px(400.)).child(
                Message::text("a", From::Assistant, "hello")
                    .action(gpui::canvas(|_, _, _| {}, move |_, _, _, _| drawn.set(true)).size(px(12.))),
            )
        }
    }

    /// Actions show while the pointer is over the message, and only then.
    #[test]
    fn actions_show_on_hover() {
        let mut app: TestAppContext = test_support::app();
        let drawn = std::rc::Rc::new(std::cell::Cell::new(false));
        let flag = drawn.clone();
        let (_, cx) = app.add_window_view(move |_, _| WithAction { drawn: flag });
        let shown_at =|y: f32, cx: &mut gpui::VisualTestContext| {
            drawn.set(false);
            cx.simulate_mouse_move(gpui::point(px(20.), px(y)), None, gpui::Modifiers::none());
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            drawn.get()
        };
        assert!(!shown_at(300., cx));
        assert!(shown_at(20., cx));
        assert!(!shown_at(300., cx));
    }
}
