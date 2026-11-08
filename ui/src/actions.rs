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
    ]
);

/// The crate's default keybindings. The crate binds none itself: the app
/// chains these into its own defaults, so rebinding on a config reload
/// restores them.
pub fn default_bindings() -> Vec<KeyBinding> {
    let i = Some("TextInput");
    vec![
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

    #[test]
    fn default_bindings_parse() {
        // KeyBinding::new panics on a keystroke it can't parse.
        let bindings = default_bindings();
        assert_eq!(bindings.len(), 29);
        assert!(bindings.iter().all(|b| b.action().name().starts_with("brindle_ui::TextInput")));
    }
}
