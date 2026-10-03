//! STUB — replaced by the control-mode implementation.
use gpui::{Context, EventEmitter};
use crate::config::{Config, Profile};

pub enum TmuxEvent { WindowAdded(u32), WindowClosed(u32), WindowActivated(u32), Detached(String), Changed }

pub struct TmuxSession { pub profile: Profile, pub session_name: String }
impl EventEmitter<TmuxEvent> for TmuxSession {}
impl TmuxSession {
    pub fn attach(profile: Profile, _cwd: Option<std::path::PathBuf>, _config: &Config, _cx: &mut Context<Self>) -> Self {
        Self { session_name: profile.tmux_session.clone().unwrap_or_default(), profile }
    }
    pub fn select_window(&mut self, _id: u32) {}
    pub fn kill_window(&mut self, _id: u32) {}
    pub fn detach(&mut self) {}
}
