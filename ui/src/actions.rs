//! The crate's actions and their default keybindings.

use gpui::{KeyBinding, actions};

actions!(
    brindle_ui,
    [
        // Text inputs.
        TextInputLeft,
        TextInputRight,
        TextInputWordLeft,
        TextInputWordRight,
        TextInputHome,
        TextInputEnd,
        TextInputSelectLeft,
        TextInputSelectRight,
        TextInputSelectWordLeft,
        TextInputSelectWordRight,
        TextInputSelectHome,
        TextInputSelectAll,
        TextInputSelectEnd,
        TextInputBackspace,
        TextInputDelete,
        TextInputDeleteWordBack,
        TextInputCopy,
        TextInputCut,
        TextInputPaste,
        // Overlays. `Dismiss` asks the view of the layer with focus to
        // close; it closes only when it agrees (by emitting `DismissEvent`).
        Dismiss,
        // Focus scopes: Tab and shift-Tab among a view's listed stops.
        FocusNext,
        FocusPrev,
        // Lists: the highlight of a select's menu or a command list, and
        // confirming the highlighted option.
        ListUp,
        ListDown,
        ListFirst,
        ListLast,
        ListConfirm,
    ]
);

/// The crate's default keybindings. The crate binds none itself: the app
/// chains these into its own defaults, so rebinding on a config reload
/// restores them.
pub fn default_bindings() -> Vec<KeyBinding> {
    let i = Some("TextInput");
    let l = Some("UiLayer");
    let f = Some("UiFocusScope");
    let s = Some("UiList");
    let c = Some("UiCommand");
    vec![
        // Escape asks the overlay to close. Bound in the layer around the
        // view, so an inner context that handles escape itself wins.
        KeyBinding::new("escape", Dismiss, l),
        // A command list takes escape itself and reports it as cancelled,
        // so one used inline (in no overlay) still sees it; a hosting view
        // turns that into its own dismissal. Being nearer the focus than the
        // layer's binding, it fires alone.
        KeyBinding::new("escape", Dismiss, c),
        // Tab cycles through the view's stops and never leaves it.
        KeyBinding::new("tab", FocusNext, f),
        KeyBinding::new("shift-tab", FocusPrev, f),
        // Lists: the highlight, and enter to confirm it. A text input
        // inside the list (a command list's filter) keeps home and end,
        // since its own bindings are nearer the focus.
        KeyBinding::new("up", ListUp, s),
        KeyBinding::new("down", ListDown, s),
        KeyBinding::new("ctrl-p", ListUp, s),
        KeyBinding::new("ctrl-n", ListDown, s),
        KeyBinding::new("home", ListFirst, s),
        KeyBinding::new("end", ListLast, s),
        KeyBinding::new("enter", ListConfirm, s),
        // Text inputs leave enter, escape, tab, up/down and ctrl-p/ctrl-n
        // to the palette or dialog around them.
        KeyBinding::new("left", TextInputLeft, i),
        KeyBinding::new("right", TextInputRight, i),
        KeyBinding::new("ctrl-left", TextInputWordLeft, i),
        KeyBinding::new("ctrl-right", TextInputWordRight, i),
        KeyBinding::new("home", TextInputHome, i),
        KeyBinding::new("end", TextInputEnd, i),
        KeyBinding::new("ctrl-e", TextInputEnd, i),
        KeyBinding::new("shift-left", TextInputSelectLeft, i),
        KeyBinding::new("shift-right", TextInputSelectRight, i),
        KeyBinding::new("ctrl-shift-left", TextInputSelectWordLeft, i),
        KeyBinding::new("ctrl-shift-right", TextInputSelectWordRight, i),
        KeyBinding::new("shift-home", TextInputSelectHome, i),
        KeyBinding::new("shift-end", TextInputSelectEnd, i),
        KeyBinding::new("ctrl-shift-e", TextInputSelectEnd, i),
        // Select all, as in desktop text boxes; ctrl-shift-a is also the
        // terminal's select-all key.
        KeyBinding::new("ctrl-a", TextInputSelectAll, i),
        KeyBinding::new("ctrl-shift-a", TextInputSelectAll, i),
        KeyBinding::new("backspace", TextInputBackspace, i),
        KeyBinding::new("shift-backspace", TextInputBackspace, i),
        KeyBinding::new("delete", TextInputDelete, i),
        KeyBinding::new("ctrl-backspace", TextInputDeleteWordBack, i),
        KeyBinding::new("ctrl-w", TextInputDeleteWordBack, i),
        KeyBinding::new("ctrl-insert", TextInputCopy, i),
        KeyBinding::new("ctrl-shift-c", TextInputCopy, i),
        KeyBinding::new("ctrl-c", TextInputCopy, i),
        KeyBinding::new("shift-delete", TextInputCut, i),
        KeyBinding::new("ctrl-x", TextInputCut, i),
        KeyBinding::new("shift-insert", TextInputPaste, i),
        KeyBinding::new("ctrl-shift-v", TextInputPaste, i),
        KeyBinding::new("ctrl-v", TextInputPaste, i),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bindings in a context, as `(keystrokes, action name)`.
    fn in_context(bindings: &[KeyBinding], context: &str) -> Vec<(String, String)> {
        bindings
            .iter()
            .filter(|b| b.predicate().is_some_and(|p| p.to_string() == context))
            .map(|b| (format!("{:?}", b.keystrokes()), b.action().name().to_string()))
            .collect()
    }

    #[test]
    fn default_bindings_parse() {
        // KeyBinding::new panics on a keystroke it can't parse.
        let bindings = default_bindings();
        let text_input = in_context(&bindings, "TextInput");
        assert_eq!(text_input.len(), 29);
        assert!(text_input.iter().all(|(_, name)| name.starts_with("brindle_ui::TextInput")));
        let layer = in_context(&bindings, "UiLayer");
        assert_eq!(layer.len(), 1);
        assert_eq!(layer[0].1, "brindle_ui::Dismiss");
        // A command list binds escape to the same action, nearer the focus.
        let command = in_context(&bindings, "UiCommand");
        assert_eq!(command.len(), 1);
        assert_eq!(command[0], layer[0]);
        let scope = in_context(&bindings, "UiFocusScope");
        let scope: Vec<&str> = scope.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(scope, ["brindle_ui::FocusNext", "brindle_ui::FocusPrev"]);
        let list = in_context(&bindings, "UiList");
        assert_eq!(list.len(), 7);
        let list_actions: Vec<&str> = list.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(
            list_actions,
            [
                "brindle_ui::ListUp",
                "brindle_ui::ListDown",
                "brindle_ui::ListUp",
                "brindle_ui::ListDown",
                "brindle_ui::ListFirst",
                "brindle_ui::ListLast",
                "brindle_ui::ListConfirm",
            ]
        );
        // Every binding is in one of the crate's contexts.
        assert_eq!(bindings.len(), text_input.len() + layer.len() + command.len() + scope.len() + list.len());
    }
}
