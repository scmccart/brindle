//! What the components' `TestAppContext` tests share: an app set up as an
//! app using the crate sets itself up.

use gpui::TestAppContext;

use crate::theme::{Tokens, UiTheme};

/// The debug selector of a tooltip drawn in `tokens` (`Tooltip::tokens`),
/// which names the set by its background.
pub(crate) fn tooltip_in(tokens: &Tokens) -> &'static str {
    Box::leak(format!("tooltip-in {:?}", tokens.background).into_boxed_str())
}

/// A test app with the crate set up: `init`, the default bindings and the
/// light tokens. Open the test's view with `add_window_view`.
pub(crate) fn app() -> TestAppContext {
    let cx = TestAppContext::single();
    cx.update(|cx| {
        crate::init(cx);
        cx.bind_keys(crate::default_bindings());
        UiTheme::set(Tokens::neutral_light(), cx);
    });
    cx
}
