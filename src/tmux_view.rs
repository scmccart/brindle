//! STUB — replaced by the control-mode implementation.
use gpui::{App, Context, Entity, FocusHandle, IntoElement, Render, Window, div};
use crate::terminal::Terminal;
use crate::theme::Theme;
use crate::tmux::TmuxSession;

pub struct TmuxWindowView { pub window_id: u32, pub theme: Theme, focus: FocusHandle, _session: Entity<TmuxSession> }
impl TmuxWindowView {
    pub fn new(session: Entity<TmuxSession>, window_id: u32, theme: Theme, _font: Option<f32>, _w: &mut Window, cx: &mut Context<Self>) -> Self {
        Self { window_id, theme, focus: cx.focus_handle(), _session: session }
    }
    pub fn title(&self, _cx: &App) -> String { String::new() }
    pub fn focus_handle(&self, _cx: &App) -> FocusHandle { self.focus.clone() }
    pub fn active_terminal(&self, _cx: &App) -> Option<Entity<Terminal>> { None }
    pub fn set_theme(&mut self, theme: Theme, _cx: &mut Context<Self>) { self.theme = theme; }
}
impl Render for TmuxWindowView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
}
