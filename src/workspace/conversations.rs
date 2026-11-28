//! The conversations palette (OpenSpec change `agents-session-list`): what
//! the window knows of its zones for the conversations plans, the palette's
//! lister and pages, and where a chosen conversation opens; and the agent
//! options palette a zone opens the same way (`open_agent_options`).

use std::path::PathBuf;

use gpui::{App, AppContext as _, Context, Entity, SharedString, Subscription, Window};

use crate::actions::*;
use crate::agent::conversations::{
    self as conversations, ConversationsModel, Lister, Open, Pane, Place, ZoneAt, ZoneFacts, list_dir, lister_plan,
    open_mark, open_plan,
};
use crate::agent::model::{ListFor, ListerState};
use crate::agent::commands::{self, OptionPick};
use crate::agent::{Agent, AgentListed};
use crate::agent::spawn::OpenedBy;
use crate::config::{Profile, ProfileKey, ProfileKind};
use crate::picker::{Palette, PaletteGroup};
use crate::remote::Host;
use crate::zone_view::{SplitAxis, ZoneSpec, ZoneStart, ZoneTreeView};

use super::overlays::{OverlayKind, PaletteRole};
use super::tabs::{Pick, TabId};
use super::{Overlay, PaletteKind, Workspace, profile_rows, unix_now};

/// The conversations palette (OpenSpec change `agents-session-list`
/// decisions 3 to 5): its model, `profile`'s conversations on `host` in
/// `dir`, asked from the pane `from`, listed through `lister`.
pub(super) struct ConversationsPalette {
    pub(super) model: ConversationsModel,
    profile: Profile,
    host: Host,
    dir: Option<PathBuf>,
    from: ZoneAt,
    lister: Entity<Agent>,
    /// The agent started for the list (`lister` itself), which the palette
    /// holds until a choice takes it: closing the palette ends it.
    chooser: Option<Entity<Agent>>,
    /// Whether the first page was asked for.
    asked: bool,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// What the conversations plans know of every zone of the window, the
    /// active tab's the most recent.
    fn conversation_facts(&self, cx: &App) -> Vec<ZoneFacts> {
        let active = self.active_tab().map(|tab| tab.id);
        self.tabs
            .iter()
            .flat_map(|tab| {
                let base = if Some(tab.id) == active { 1 << 32 } else { 0 };
                tab.zones.read(cx).zone_facts(tab.id, base, cx)
            })
            .collect()
    }

    /// The active tab's active zone.
    fn active_zone_at(&self, cx: &App) -> Option<ZoneAt> {
        let tab = self.active_tab()?;
        Some(ZoneAt { tab: tab.id, zone: tab.zones.read(cx).active_id() })
    }

    fn tab_zones(&self, tab: TabId) -> Option<Entity<ZoneTreeView>> {
        self.tabs.iter().find(|t| t.id == tab).map(|t| t.zones.clone())
    }

    /// An agent zone's options palette (OpenSpec change `agents-commands`,
    /// flag 7): every option of zone `from`'s session, or option
    /// `option`'s choices; the choice goes back to the zone, which sends
    /// the set. Nothing when there is nothing to list.
    pub(super) fn open_agent_options(&mut self, from: ZoneAt, option: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.tab_zones(from.tab).and_then(|zones| zones.read(cx).agent_of(from.zone, cx)) else {
            return;
        };
        let Some(list) = commands::options_palette(agent.read(cx).model().options(), option) else { return };
        let palette = cx.new(|cx| Palette::list_grouped(list.groups, list.title, None, cx));
        self.show_palette(palette, PaletteKind::AgentOptions { from, picks: list.picks }, window, cx);
    }

    /// A row of the options palette was chosen: a select's choices listed,
    /// or the set sent through the zone.
    pub(super) fn agent_options_confirmed(&mut self, from: ZoneAt, pick: Option<OptionPick>, window: &mut Window, cx: &mut Context<Self>) {
        match pick {
            Some(OptionPick::Open { option }) => self.open_agent_options(from, Some(&option), window, cx),
            Some(OptionPick::Set { option, name, value }) => {
                self.dismiss_overlay(window, cx);
                if let Some(agent) = self.tab_zones(from.tab).and_then(|zones| zones.read(cx).agent_of(from.zone, cx)) {
                    agent.update(cx, |agent, cx| agent.set_option(&option, name, value, cx));
                }
            }
            None => {}
        }
    }

    /// An agent zone's Conversations… (and `agent_conversations`, and a
    /// `-r` launch): the palette for its own profile, from it.
    pub(super) fn open_zone_conversations(&mut self, from: ZoneAt, window: &mut Window, cx: &mut Context<Self>) {
        let Some(zones) = self.tab_zones(from.tab) else { return };
        let Some(zone) = zones.read(cx).zone(from.zone) else { return };
        let config = Self::settings(cx);
        // The config's profile of that name, else the zone's as last seen.
        let profile = config
            .profile_index(&zone.profile.name)
            .map_or_else(|| zone.profile.clone(), |ix| config.profiles[ix].clone());
        self.open_conversations(profile, from, window, cx);
    }

    /// `agent_continue_conversation`: the conversations palette for the
    /// config's agent profile, asking which first when it has several (the
    /// active zone's first when it is one), from the active zone.
    pub(super) fn continue_agent_conversation(&mut self, _: &AgentContinueConversation, window: &mut Window, cx: &mut Context<Self>) {
        if !self.toggles_open(OverlayKind::Palette(PaletteRole::Conversations), window, cx) {
            return;
        }
        let Some(from) = self.active_zone_at(cx) else { return };
        let config = Self::settings(cx);
        let names = config.profiles.iter().map(|p| p.name.as_str());
        let agents: Vec<ProfileKey> = (0..config.profiles.len())
            .filter(|&ix| config.profiles[ix].kind() == ProfileKind::Agent)
            .filter_map(|ix| ProfileKey::of(names.clone(), ix))
            .collect();
        let active = self
            .tab_zones(from.tab)
            .and_then(|zones| zones.read(cx).zone(from.zone).map(|zone| zone.profile.name.clone()))
            .and_then(|name| config.profile_index(&name))
            .and_then(|ix| ProfileKey::of(names.clone(), ix));
        let keys = conversations::ask_order(agents, active.as_ref());
        let profiles: Vec<Profile> =
            keys.iter().filter_map(|key| config.find_profile(key).map(|ix| config.profiles[ix].clone())).collect();
        match profiles.as_slice() {
            [] => {}
            [profile] => {
                let profile = profile.clone();
                self.open_conversations(profile, from, window, cx);
            }
            [..] => {
                let rows = profile_rows(&profiles, &[]);
                let palette = cx.new(|cx| Palette::list(rows, "Continue a conversation of…", cx));
                self.show_palette(palette, PaletteKind::ConversationsProfile { keys, from }, window, cx);
            }
        }
    }

    /// Opens the conversations palette for `profile` from the pane `from`
    /// (decisions 3 and 4): its directory as a new zone of the profile
    /// split from the pane would get, listed through a ready zone of the
    /// profile there or an agent started choosing for it.
    pub(super) fn open_conversations(&mut self, profile: Profile, from: ZoneAt, window: &mut Window, cx: &mut Context<Self>) {
        let Some(zones) = self.tab_zones(from.tab) else { return };
        let (host, dir) = {
            let zones = zones.read(cx);
            let Some(zone) = zones.zone(from.zone) else { return };
            let pane_dir = zones.zone_working_directory(from.zone, cx);
            let pane = Pane { profile: &zone.profile, host: &zone.host, dir: pane_dir.as_deref() };
            list_dir(pane, &profile, dirs::home_dir().as_deref())
        };
        let facts = self.conversation_facts(cx);
        let (lister, chooser) = match lister_plan(&profile.name, &host, dir.as_deref(), from, &facts) {
            Lister::Zone(at) => {
                let Some(agent) = self.tab_zones(at.tab).and_then(|zones| zones.read(cx).agent_of(at.zone, cx)) else {
                    return;
                };
                (agent, None)
            }
            Lister::Chooser => {
                let agent = cx.new(|cx| {
                    Agent::spawn_choosing(&profile, &host, dir.clone(), OpenedBy::Window, None, None, cx)
                });
                (agent.clone(), Some(agent))
            }
        };
        let mut model =
            ConversationsModel::new(profile.name.clone(), Agent::dir_shown(&host, dir.as_deref()), chooser.is_some());
        model.set_open(
            facts
                .iter()
                .filter_map(|zone| zone.conversation.clone())
                .filter(|id| open_mark(id, &profile.name, &host, &facts))
                .collect(),
        );
        let subscriptions = vec![
            cx.observe_in(&lister, window, |this, _, _, cx| this.conversations_lister_changed(cx)),
            cx.subscribe(&lister, |this, _, AgentListed(outcome): &AgentListed, cx| {
                this.conversations_listed(outcome.clone(), cx);
            }),
        ];
        let title = model.title();
        let empty = model.empty_text().map(SharedString::from);
        let palette = cx.new(|cx| Palette::list_grouped(vec![PaletteGroup::unnamed(Vec::new())], title, empty, cx));
        let state = ConversationsPalette {
            model,
            profile,
            host,
            dir,
            from,
            lister: lister.clone(),
            chooser,
            asked: false,
            _subscriptions: subscriptions,
        };
        self.show_palette(palette, PaletteKind::Conversations(Box::new(state)), window, cx);
        // A ready zone lists at once; a chooser once it is choosing.
        self.conversations_lister_changed(cx);
    }

    /// The open conversations palette, if that is what shows.
    pub(super) fn conversations_palette(&mut self) -> Option<&mut ConversationsPalette> {
        match &mut self.overlay {
            Overlay::Palette { kind: PaletteKind::Conversations(palette), .. } => Some(palette),
            Overlay::Closed
            | Overlay::SessionsOpening(_)
            | Overlay::Palette { .. }
            | Overlay::Settings
            | Overlay::ProfileMenu
            | Overlay::ContextMenu { .. }
            | Overlay::CloseConfirm { .. }
            | Overlay::LinkConfirm { .. } => None,
        }
    }

    /// The lister's agent changed: once it can list, the first page is
    /// asked for; one that can't says why in the palette.
    fn conversations_lister_changed(&mut self, cx: &mut Context<Self>) {
        let Some(palette) = self.conversations_palette() else { return };
        let lister = palette.lister.clone();
        match palette.model.state() {
            conversations::ListState::Starting | conversations::ListState::Listing => {}
            conversations::ListState::Listed | conversations::ListState::Failed(_) => return,
        }
        match lister.read(cx).lister_state() {
            ListerState::Starting => return,
            ListerState::Lists if !palette.asked => {
                palette.model.listing();
                palette.asked = true;
                lister.update(cx, |agent, cx| agent.list(None, ListFor::Palette, cx));
            }
            ListerState::Lists => return,
            ListerState::Failed(why) => palette.model.failed(why),
        }
        self.show_conversations(false, cx);
    }

    /// A list the palette asked for was answered.
    fn conversations_listed(&mut self, outcome: Result<conversations::Page, String>, cx: &mut Context<Self>) {
        let Some(palette) = self.conversations_palette() else { return };
        match outcome {
            Ok(page) => palette.model.page(page),
            Err(why) => palette.model.failed(why),
        }
        self.show_conversations(true, cx);
    }

    /// Shows the conversations palette's model, keeping the highlighted row
    /// when `keep` (a page appended below it).
    pub(super) fn show_conversations(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(view) = self.palette.view().cloned() else { return };
        let Some(palette) = self.conversations_palette() else { return };
        let rows = palette.model.rows(unix_now() as i64);
        let empty = palette.model.empty_text().map(SharedString::from);
        let keep = if keep { view.read(cx).highlighted(cx) } else { None };
        view.update(cx, |view, cx| view.set_groups(vec![PaletteGroup::unnamed(rows)], empty, keep, cx));
        cx.notify();
    }

    /// Takes the conversations palette out of the overlay, so closing the
    /// palette leaves its chooser to whoever takes it.
    fn take_conversations(&mut self) -> Option<Box<ConversationsPalette>> {
        self.conversations_palette()?;
        match std::mem::replace(&mut self.overlay, Overlay::Closed) {
            Overlay::Palette { kind: PaletteKind::Conversations(palette), .. } => Some(palette),
            Overlay::Closed
            | Overlay::SessionsOpening(_)
            | Overlay::Palette { .. }
            | Overlay::Settings
            | Overlay::ProfileMenu
            | Overlay::ContextMenu { .. }
            | Overlay::CloseConfirm { .. }
            | Overlay::LinkConfirm { .. } => None,
        }
    }

    /// Ends the agent the open conversations palette started, which no
    /// choice took (decision 3: escape, another overlay, the window closing,
    /// a quit).
    pub(super) fn close_palette_chooser(&mut self, cx: &mut Context<Self>) {
        if let Some(chooser) = self.conversations_palette().and_then(|palette| palette.chooser.take()) {
            chooser.update(cx, |agent, _| agent.close());
        }
    }

    /// Confirming row `ix` of the conversations palette.
    pub(super) fn conversations_confirmed(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.conversations_palette() else { return };
        match palette.model.confirm(ix) {
            conversations::Pick::Nothing => {}
            conversations::Pick::More(cursor) => {
                let lister = palette.lister.clone();
                lister.update(cx, |agent, cx| agent.list(Some(cursor), ListFor::Palette, cx));
                self.show_conversations(true, cx);
            }
            conversations::Pick::Conversation(listed) => {
                let (profile, host, from) = (palette.profile.name.clone(), palette.host.clone(), palette.from);
                let facts = self.conversation_facts(cx);
                match open_plan(&listed.id, &profile, &host, from, &facts) {
                    Open::Show(at) => {
                        self.dismiss_overlay(window, cx);
                        self.show_zone(at, window, cx);
                    }
                    Open::Here(at) => self.open_here(at, listed.id, window, cx),
                    Open::Ask => {
                        let splits = self.tab_zones(from.tab).is_some_and(|zones| {
                            let zones = zones.read(cx);
                            zones.can_split(from.zone, SplitAxis::Horizontal, window, cx)
                                || zones.can_split(from.zone, SplitAxis::Vertical, window, cx)
                        });
                        let Some(palette) = self.conversations_palette() else { return };
                        palette.model.ask(listed, splits);
                        self.clear_palette_filter(window, cx);
                        self.show_conversations(false, cx);
                    }
                }
            }
            conversations::Pick::Place(listed, place) => self.open_placed(listed.id, place, window, cx),
        }
    }

    /// Empties the open palette's filter (the question's rows are no
    /// conversation's).
    fn clear_palette_filter(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.palette.view().cloned() else { return };
        let input = view.read(cx).input(cx);
        input.update(cx, |input, cx| input.set_text("", cx));
    }

    /// Activates the tab of `at` and focuses that zone.
    fn show_zone(&mut self, at: ZoneAt, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(Pick::Tab(at.tab), window, cx);
        if let Some(zones) = self.tab_zones(at.tab) {
            zones.update(cx, |zones, cx| zones.focus_zone(at.zone, window, cx));
        }
    }

    /// The pane `at` takes conversation `id` (decision 5's `Here`): a
    /// choosing zone chooses it; a stopped zone shows the chooser that
    /// listed it, its own session closed, or, listed through another zone's
    /// agent, a new session continuing it.
    fn open_here(&mut self, at: ZoneAt, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.take_conversations() else { return };
        self.dismiss_overlay(window, cx);
        let Some(zones) = self.tab_zones(at.tab) else { return };
        let Some(agent) = zones.read(cx).agent_of(at.zone, cx) else { return };
        if agent.read(cx).model().is_choosing() {
            // It listed for itself: it is the lister, no chooser.
            agent.update(cx, |agent, cx| agent.choose(Some(id), cx));
        } else if let Some(chooser) = palette.chooser {
            chooser.update(cx, |agent, cx| agent.choose(Some(id), cx));
            zones.update(cx, |zones, cx| zones.replace_agent(at.zone, chooser, window, cx));
        } else {
            agent.update(cx, |agent, cx| agent.continue_listed(&id, cx));
        }
        self.show_zone(at, window, cx);
    }

    /// The question's answer (decision 5's `Ask`): conversation `id` opens
    /// in a split of the pane or a new tab, the chooser taking it or a new
    /// session continuing it.
    fn open_placed(&mut self, id: String, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.take_conversations() else { return };
        self.dismiss_overlay(window, cx);
        let ConversationsPalette { profile, host, dir, from, chooser, .. } = *palette;
        let agent = match chooser {
            Some(chooser) => {
                chooser.update(cx, |agent, cx| agent.choose(Some(id), cx));
                chooser
            }
            None => cx.new(|cx| Agent::spawn_continuing(&profile, &host, dir, &id, cx)),
        };
        let axis = match place {
            Place::SplitRight => SplitAxis::Horizontal,
            Place::SplitDown => SplitAxis::Vertical,
            Place::NewTab => {
                self.insert_zone_tab(profile, host, ZoneStart::Agent(agent), true, window, cx);
                return;
            }
        };
        let spec = ZoneSpec::of(profile, host, Self::settings(cx));
        let split = self.tab_zones(from.tab).is_some_and(|zones| {
            zones.update(cx, |zones, cx| zones.split_zone_with(from.zone, axis, spec, ZoneStart::Agent(agent.clone()), window, cx))
        });
        if !split {
            // The pane went, or got too small: the conversation's agent
            // goes too, and the notice says why.
            agent.update(cx, |agent, _| agent.close());
            self.show_notice("the zone can't be split now; the conversation wasn't opened".into(), cx);
        }
    }
}
