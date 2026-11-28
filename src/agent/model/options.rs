//! Slash commands and session options (OpenSpec change `agents-commands`):
//! the completion list and its keys, and the option commands.

use crate::agent::commands::{self, Completion, CompletionAction, CompletionKey, Local, OptionItem, Pick, Stage};
use crate::agent::transcript::{OptionControl, SessionOption};
use crate::remote::protocol::client::OptionValue;

use super::ChatModel;

/// What a key the completion list takes does (OpenSpec change
/// `agents-commands`, decision 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The list changed (its highlight, or it was dismissed): redraw it.
    Redraw,
    /// Put this text in the input, the cursor at this byte.
    Text(String, usize),
    /// Ask the server to set option `option` (named `name`) to `value`,
    /// the input emptied.
    Set { option: String, name: String, value: OptionValue },
}

impl ChatModel {
    /// What submitting `text` does when it is an option command of this
    /// session's (`commands::local_command`).
    pub fn local(&self, text: &str) -> Option<Local> {
        commands::local_command(text, &self.options, &self.commands)
    }

    /// The session's options.
    pub fn options(&self) -> &[SessionOption] {
        &self.options
    }

    /// The options under the prompt input.
    pub fn option_items(&self) -> Vec<OptionItem> {
        commands::option_items(&self.options)
    }

    /// The set `agent_next_mode` asks for, if there is one.
    pub fn next_mode(&self) -> Option<(String, String, OptionValue)> {
        let (option, value) = commands::next_mode(&self.options)?;
        let name = self.option_name(&option);
        Some((option, name, value))
    }

    /// Whether the session offers options (`agent_options` is listed), and
    /// a mode option `agent_next_mode` can step.
    pub fn has_options(&self) -> (bool, bool) {
        (!self.options.is_empty(), commands::next_mode(&self.options).is_some())
    }

    /// Option `id`'s name, else the id.
    pub fn option_name(&self, id: &str) -> String {
        self.options.iter().find(|option| option.id == id).map_or_else(|| id.to_string(), |option| option.name.clone())
    }

    /// The set a press on option `id` under the input asks for, when it is
    /// a toggle: the other value. `None` for a select, whose choices show.
    pub fn footer_flip(&self, id: &str) -> Option<(String, OptionValue)> {
        let option = self.options.iter().find(|option| option.id == id)?;
        match &option.kind {
            OptionControl::Toggle { on } => Some((option.name.clone(), OptionValue::Toggle { on: !on })),
            OptionControl::Select { .. } => None,
        }
    }

    /// The prompt input's text and cursor (a byte) changed: the completion
    /// list follows, unless an input method composes (it is left as it
    /// is). True when the list changed.
    pub fn typed(&mut self, text: &str, cursor: usize, composing: bool) -> bool {
        if composing || (self.draft.0 == text && self.draft.1 == cursor) {
            return false;
        }
        self.draft = (text.to_string(), cursor.min(text.len()));
        let before = self.completion.clone();
        self.refresh_completion();
        before != self.completion
    }

    /// The list for the draft as it is: none while dismissed, the
    /// highlight kept on the same row while the list's stage and word stay.
    pub(super) fn refresh_completion(&mut self) {
        let (text, cursor) = (&self.draft.0, self.draft.1);
        if !commands::in_command(text, cursor) {
            self.dismissed = false;
        }
        let fresh = if self.dismissed { None } else { commands::completion(text, cursor, &self.commands, &self.options) };
        self.completion = match (fresh, self.completion.take()) {
            (Some(mut fresh), Some(old)) if fresh.stage == old.stage && fresh.query == old.query => {
                if let Some(at) = old.items.get(old.highlighted).and_then(|kept| fresh.items.iter().position(|i| i == kept)) {
                    fresh.highlighted = at;
                }
                Some(fresh)
            }
            (fresh, _) => fresh,
        };
    }

    /// The completion list while it shows.
    pub fn completion(&self) -> Option<&Completion> {
        self.completion.as_ref()
    }

    /// A key the list takes while it shows; `None` with no list.
    pub fn completion_key(&mut self, key: CompletionKey) -> Option<KeyOutcome> {
        let completion = self.completion.as_ref()?;
        let (len, highlighted) = (completion.items.len(), completion.highlighted);
        let action = commands::completion_key(key, &completion.stage);
        match action {
            CompletionAction::Previous | CompletionAction::Next => {
                let moved = commands::moved(highlighted, len, action == CompletionAction::Next);
                if let Some(completion) = &mut self.completion {
                    completion.highlighted = moved;
                }
                Some(KeyOutcome::Redraw)
            }
            CompletionAction::Dismiss => {
                self.dismissed = true;
                self.completion = None;
                Some(KeyOutcome::Redraw)
            }
            CompletionAction::Accept | CompletionAction::Set => self.take(highlighted),
            CompletionAction::Write => self.write_choice(),
        }
    }

    /// Row `label` of the list was pressed: taken as enter takes the
    /// highlighted one. `None` when no row has it now.
    pub fn suggestion_pressed(&mut self, label: &str) -> Option<KeyOutcome> {
        let item = self.completion.as_ref()?.items.iter().position(|item| item.label == label)?;
        self.take(item)
    }

    /// Item `item` taken: a command's name written into the text, or a
    /// choice set.
    fn take(&mut self, item: usize) -> Option<KeyOutcome> {
        let completion = self.completion.as_ref()?;
        let suggestion = completion.items.get(item)?;
        match (&completion.stage, &suggestion.pick) {
            (Stage::Choices { option, .. }, Pick::Choice { value }) => {
                let (option, value) = (option.clone(), value.clone());
                let name = self.option_name(&option);
                self.completion = None;
                Some(KeyOutcome::Set { option, name, value })
            }
            (Stage::Commands, Pick::Command { .. }) => {
                let (text, cursor) = commands::accept(&self.draft.0, completion, item)?;
                self.typed(&text, cursor, false);
                Some(KeyOutcome::Text(text, cursor))
            }
            // Never built: each stage lists its own picks.
            (Stage::Commands, Pick::Choice { .. }) | (Stage::Choices { .. }, Pick::Command { .. }) => None,
        }
    }

    /// Tab in the second stage: the highlighted choice's name written in.
    pub fn write_choice(&mut self) -> Option<KeyOutcome> {
        let completion = self.completion.as_ref()?;
        let (text, cursor) = commands::accept(&self.draft.0, completion, completion.highlighted)?;
        self.typed(&text, cursor, false);
        Some(KeyOutcome::Text(text, cursor))
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::Event;
    use super::super::prompt::Why;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// `agents-commands` 6.1a: an option command is Brindle's, set at once
    /// during a turn too; a bare one asks for the choices; a choice the
    /// option lacks keeps the text with its reason; whitespace before a
    /// command the agent's goes without it.
    #[test]
    fn an_option_command_is_brindles_and_the_rest_go_to_the_agent() {
        let mut zone = with_lists();
        zone.feed(&[turn(TurnState::Working)]);
        let set = zone.model.submit("/model sonnet", false);
        assert_eq!(
            set,
            Submit::Set { option: "model".into(), name: "Model".into(), value: OptionValue::Choice { value: "sonnet".into() } }
        );
        assert_eq!(zone.model.submit("/mode", false), Submit::Choose { option: "mode".into() });
        assert_eq!(zone.model.submit("/model gpt", false), Submit::Nothing(Why::NoSuchChoice));
        assert_eq!(zone.model.refusal(), Some("Model has no choice \"gpt\""));
        assert_eq!(zone.model.submit("/model gpt", true), Submit::Nothing(Why::Composing), "composing");
        zone.feed(&[turn(TurnState::Idle)]);
        assert_eq!(zone.model.submit("  /compact now", false), Submit::Send("/compact now".into()));
        assert_eq!(zone.model.refusal(), None);
        assert_eq!(zone.model.submit("  plain", false), Submit::Send("  plain".into()), "not a command: as typed");
    }

    /// `agents-commands` 6.1: the list follows the text, its keys move,
    /// accept and dismiss, a press takes a row, composition leaves it
    /// alone, and a choice taken asks for the set.
    #[test]
    fn the_completion_list_follows_the_text_and_takes_its_keys() {
        let mut zone = with_lists();
        assert_eq!(zone.model.completion_key(CompletionKey::Down), None, "no list, no key");
        assert!(zone.model.typed("/", 1, false));
        assert_eq!(zone.model.completion().map(|c| c.items.len()), Some(5), "two commands, three options");
        assert!(!zone.model.typed("/c", 2, true), "composing leaves it");
        assert!(zone.model.typed("/co", 3, false));
        assert_eq!(zone.model.completion_key(CompletionKey::Down), Some(KeyOutcome::Redraw));
        assert_eq!(zone.model.completion().map(|c| c.highlighted), Some(1));
        assert_eq!(zone.model.completion_key(CompletionKey::Enter), Some(KeyOutcome::Text("/context ".into(), 9)));
        assert!(zone.model.completion().is_none(), "the command's words take no list");
        zone.model.typed("/co", 3, false);
        assert_eq!(zone.model.completion_key(CompletionKey::Escape), Some(KeyOutcome::Redraw));
        assert!(zone.model.completion().is_none());
        zone.model.typed("/com", 4, false);
        assert!(zone.model.completion().is_none(), "dismissed while the word is a command");
        zone.model.typed("", 0, false);
        zone.model.typed("/mo", 3, false);
        assert!(zone.model.completion().is_some(), "dismissal ends with the command word");
        assert_eq!(zone.model.suggestion_pressed("/model"), Some(KeyOutcome::Text("/model ".into(), 7)));
        let choices = zone.model.completion().expect("the model's choices");
        assert_eq!(choices.highlighted, 0, "on the current choice, opus");
        assert_eq!(zone.model.completion_key(CompletionKey::Tab), Some(KeyOutcome::Text("/model Opus".into(), 11)));
        zone.model.typed("/model ", 7, false);
        zone.model.completion_key(CompletionKey::Down);
        assert_eq!(
            zone.model.completion_key(CompletionKey::Enter),
            Some(KeyOutcome::Set {
                option: "model".into(),
                name: "Model".into(),
                value: OptionValue::Choice { value: "sonnet".into() }
            })
        );
        assert_eq!(zone.model.suggestion_pressed("/nothing"), None);
    }

    /// A set's answer: an OK changes nothing until the agent's options do;
    /// the server's refusal is the notice's text, an unsent set the
    /// input's.
    #[test]
    fn a_set_options_answer() {
        let mut zone = with_lists();
        zone.model.sent(3, Sent::SetOption { name: "Model".into() });
        assert_eq!(zone.model.replied(3, Ok(())), Replied::Nothing);
        zone.model.sent(4, Sent::SetOption { name: "Model".into() });
        assert_eq!(
            zone.model.replied(4, Err("session 2: an earlier change of an option hasn't been answered yet".into())),
            Replied::OptionRefused("Model wasn't changed: an earlier change of an option hasn't been answered yet".into())
        );
        assert_eq!(zone.model.refusal(), None, "not beside the input");
        assert_eq!(zone.model.not_sent(Sent::SetOption { name: "Model".into() }, NotSent::Gone), Replied::Nothing);
        assert_eq!(zone.model.refusal(), Some(UNSENT));
        assert_eq!(zone.model.footer_flip("fast"), Some(("Fast \"mode\"".into(), OptionValue::Toggle { on: true })));
        assert_eq!(zone.model.footer_flip("model"), None, "a select's choices show");
        assert_eq!(zone.model.has_options(), (true, true));
        assert_eq!(
            zone.model.next_mode(),
            Some(("mode".into(), "Mode".into(), OptionValue::Choice { value: "plan".into() }))
        );
        // The agent stopped: both lists go.
        zone.feed(&[Event::Commands { commands: vec![] }, Event::Options { options: vec![] }]);
        assert_eq!(zone.model.has_options(), (false, false));
        assert!(zone.model.option_items().is_empty());
    }
}
