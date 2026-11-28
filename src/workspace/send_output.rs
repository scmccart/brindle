//! A prompt's attachments from outside its zone (OpenSpec change
//! `agents-prompt-context`, design decision 8): a terminal's last command's
//! output for an agent zone, and Attach File…'s path. The decisions are
//! `targets.rs`'s; this gathers and applies them.

use std::rc::Rc;

use gpui::{AppContext as _, Context, Window};

use super::targets::{AgentTarget, NO_AGENT_ZONE, Target, output_name, target_plan};
use super::{PaletteKind, Workspace};
use crate::agent::conversations::{ZoneAt, ZoneRun};
use crate::agent::model::Draft;
use crate::picker::{Palette, PaletteRow, PromptRequest, SubmitFn};
use crate::terminal_view::SendOutput;
use crate::workspace::tabs::TabId;

/// A command's output on its way to an agent zone, named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub text: String,
}

impl Workspace {
    /// The window's agent zones, as the list names them, the active tab's
    /// the most recent.
    fn agent_targets(&self, cx: &gpui::App) -> Vec<AgentTarget> {
        let active = self.active_tab().map(|tab| tab.id);
        self.tabs
            .iter()
            .flat_map(|tab| {
                let base = if Some(tab.id) == active { 1 << 32 } else { 0 };
                let zones = tab.zones.read(cx);
                zones
                    .zone_facts(tab.id, base, cx)
                    .into_iter()
                    .filter(|facts| facts.run != ZoneRun::Terminal)
                    .map(|facts| AgentTarget {
                        at: facts.at,
                        tab_title: tab.title.to_string(),
                        zone_title: zones.zone(facts.at.zone).map(|zone| zone.title.to_string()).unwrap_or_default(),
                        recency: facts.recency,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Send Last Command Output to Agent from a terminal of tab `tab`: the
    /// only agent zone takes it, or the window asks which, or the notice
    /// line says there is none.
    pub(super) fn send_output(&mut self, tab: TabId, send: &SendOutput, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.tabs.iter().find(|t| t.id == tab).map(|t| t.title.to_string()).unwrap_or_default();
        let output = Output { name: output_name(send.command.as_deref(), &title), text: send.text.clone() };
        match target_plan(self.agent_targets(cx)) {
            Target::None => self.show_notice(NO_AGENT_ZONE.into(), cx),
            Target::One(at) => self.attach_output(at, output, window, cx),
            Target::Ask(targets) => {
                let rows = targets
                    .iter()
                    .map(|target| PaletteRow { label: target.label().into(), detail: None, shortcut: None, disabled: false })
                    .collect();
                let palette = cx.new(|cx| Palette::list(rows, "Send the output to…", cx));
                let targets = targets.into_iter().map(|target| target.at).collect();
                self.show_palette(palette, PaletteKind::AgentTarget { targets, output }, window, cx);
            }
        }
    }

    /// `output` joins zone `at`'s draft (or its refusal shows), and that
    /// zone is shown with its prompt input focused. Nothing is sent.
    pub(super) fn attach_output(&mut self, at: ZoneAt, output: Output, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.tab_zones(at.tab).and_then(|zones| zones.read(cx).agent_of(at.zone, cx)) else {
            return;
        };
        agent.update(cx, |agent, cx| agent.attach_draft(Draft::text(&output.name, &output.text), "the output", cx));
        self.show_zone(at, window, cx);
    }

    /// Attach File…: asks for a path on zone `at`'s host and attaches it,
    /// as typed, as a link; a blank one attaches nothing.
    pub(super) fn ask_attach_file(&mut self, at: ZoneAt, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.tab_zones(at.tab).and_then(|zones| zones.read(cx).agent_of(at.zone, cx)) else {
            return;
        };
        let agent = agent.downgrade();
        let submit: SubmitFn = Rc::new(move |text, cx| {
            let path = text.trim();
            if path.is_empty() {
                return;
            }
            let draft = Draft::link(path);
            let name = draft.name().to_string();
            agent.update(cx, |agent, cx| agent.attach_draft(draft, &name, cx)).ok();
        });
        let label = "Attach file (a path on the agent's host)".into();
        self.open_prompt(PromptRequest { label, initial: String::new(), submit }, window, cx);
    }
}
