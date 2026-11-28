//! A terminal login's zone (OpenSpec change `agents-login`, decision 9):
//! opened beside its agent zone, never recorded, and ended with its login.

use gpui::{AppContext as _, Context, Window};

use crate::agent::LoginZone;
use crate::agent::login::{self, TerminalLogin};
use crate::agent::model::LoginZoneEnd;
use crate::config::Config;
use crate::settings::Settings;
use crate::terminal::{Terminal, TerminalEvent};
use crate::zone_tree::{SplitAxis, ZoneId};

use super::{ZoneSpec, ZoneStart, ZoneTreeView};

/// Where a terminal login's zone goes beside its agent zone: to the right
/// when the agent zone is wide enough to split that way, else below.
pub fn login_axis(fits_right: bool) -> SplitAxis {
    if fits_right { SplitAxis::Horizontal } else { SplitAxis::Vertical }
}

impl ZoneTreeView {
    /// Agent zone `agent_zone` asks for its terminal login's zone, or for
    /// it to close (Cancel login).
    pub(super) fn on_login_zone(&mut self, agent_zone: ZoneId, event: &LoginZone, window: &mut Window, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        match event {
            LoginZone::Open(plan) => self.open_login_zone(agent_zone, plan.clone(), window, cx),
            LoginZone::Close => {
                if let Some(id) = self.login_zone_of(agent_zone) {
                    self.close_zone_by_id(id, window, cx);
                }
            }
        }
    }

    /// The terminal login's zone of agent zone `agent_zone`, while open.
    pub(super) fn login_zone_of(&self, agent_zone: ZoneId) -> Option<ZoneId> {
        self.zones.iter().find(|(_, zone)| zone.login_for == Some(agent_zone)).map(|(id, _)| *id)
    }

    /// The zones running terminal logins.
    pub(super) fn login_zones(&self) -> Vec<ZoneId> {
        self.zones.iter().filter(|(_, zone)| zone.login_for.is_some()).map(|(id, _)| *id).collect()
    }

    /// Opens a terminal zone beside agent zone `agent_zone` running `plan`
    /// on that zone's host, in the agent's directory, with its profile's
    /// environment and the login's variables over it (decision 9), drawn as
    /// the agent zone is. It isn't a profile's zone: no split starts from
    /// it, its program ends with the tab or Brindle (a command's terminal,
    /// `Closing::Ends`), and it takes focus, since the login asks for
    /// typing. One per agent zone.
    pub(super) fn open_login_zone(&mut self, agent_zone: ZoneId, plan: TerminalLogin, window: &mut Window, cx: &mut Context<Self>) {
        if self.login_zone_of(agent_zone).is_some() {
            return;
        }
        let Some(zone) = self.zones.get(&agent_zone) else { return };
        let spec = ZoneSpec {
            profile: zone.profile.clone(),
            host: zone.host.clone(),
            theme: zone.theme.clone(),
            font_size: zone.font_size,
        };
        let axis = login_axis(self.fits(agent_zone, SplitAxis::Horizontal, window, cx));
        let cwd = self.zone_working_directory(agent_zone, cx);
        let config: Config = Settings::get(cx).config.clone();
        let ignore_case = spec.host.is_local() && crate::terminal::process::LOCAL_ENV_IGNORES_CASE;
        let mut profile = spec.profile.clone();
        profile.env = login::overlay(&profile.env, &plan.env, ignore_case);
        let new_id = ZoneId(self.next_id);
        self.next_id += 1;
        if !self.tree.split(agent_zone, axis, new_id) {
            return;
        }
        let host = spec.host.clone();
        let command = Some((plan.command, plan.args));
        let terminal = cx.new(|cx| Terminal::spawn(&profile, &host, command, cwd, &config, cx));
        self.refresh_zone_title(agent_zone, cx);
        self.insert_zone(new_id, spec, ZoneStart::Terminal(terminal), window, cx);
        if let Some(zone) = self.zones.get_mut(&new_id) {
            zone.login_for = Some(agent_zone);
        }
        self.refresh_zone_title(new_id, cx);
        self.active = new_id;
        let focus = self.focus_handle(cx);
        window.focus(&focus);
        cx.emit(TerminalEvent::TitleChanged);
        cx.notify();
    }

    /// Zone `id`'s terminal login ended as `end`: its agent zone hears it,
    /// once.
    pub(super) fn end_login(&mut self, id: ZoneId, end: LoginZoneEnd, cx: &mut Context<Self>) {
        let Some(agent_zone) = self.zones.get_mut(&id).and_then(|zone| zone.login_for.take()) else { return };
        let Some(view) = self.zones.get(&agent_zone).and_then(|zone| zone.content.agent_view()) else { return };
        let agent = view.read(cx).agent().clone();
        agent.update(cx, |agent, cx| agent.login_zone_ended(end, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ZoneTree, recorded_leaves};
    use super::*;

    /// `agents-login` decision 9: a terminal login's zone goes right of its
    /// agent zone when that fits, else below, and is never recorded.
    #[test]
    fn a_login_zone_goes_beside_and_is_never_recorded() {
        assert_eq!(login_axis(true), SplitAxis::Horizontal);
        assert_eq!(login_axis(false), SplitAxis::Vertical);
        let leaves = vec![ZoneId(1), ZoneId(2), ZoneId(3)];
        assert_eq!(recorded_leaves(leaves.clone(), &[]), leaves);
        assert_eq!(recorded_leaves(leaves, &[ZoneId(2)]), [ZoneId(1), ZoneId(3)]);
        // The tree without the login zone is the layout a restore rebuilds.
        let mut tree = ZoneTree::Leaf(ZoneId(1));
        assert!(tree.split(ZoneId(1), login_axis(true), ZoneId(2)));
        assert_eq!(recorded_leaves(tree.leaves(), &[ZoneId(2)]), [ZoneId(1)]);
    }
}
