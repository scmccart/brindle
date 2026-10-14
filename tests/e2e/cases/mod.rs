//! The cases. Each is one function; `all` lists them in run order.

mod colors;
mod dump;
mod geometry;
mod input;
mod links;
mod restore;
mod notify;
mod search;
mod session_end;
mod settings;
mod shell;
mod styles;
mod titles;
mod tmux;
mod windows;

use crate::capture::Cells;
use crate::brindle::Output;
use crate::tmux::TmuxServer;
use crate::{Case, Result};

pub fn all() -> Vec<Case> {
    let display = |name, run| Case { name, display: true, run };
    let headless = |name, run| Case { name, display: false, run };
    vec![
        headless("tmux-subscription-format", tmux::subscription_format),
        headless("tmux-border-status-no-layout-change", tmux::border_status_no_layout_change),
        headless("tmux-rotate-no-layout-change", tmux::rotate_no_layout_change),
        headless("tmux-color-report-precedence", tmux::color_report_precedence),
        headless("tmux-osc133-passthrough", shell::tmux_passthrough),
        headless("tmux-osc-in-output", notify::tmux_osc_in_output),
        display("tmux-subscriptions-removed", tmux::subscriptions_removed),
        display("dump-grid-geometry", dump::grid_geometry),
        display("geometry-plain", geometry::plain),
        display("geometry-top-before-attach", geometry::top_before_attach),
        display("geometry-top-while-attached", geometry::top_while_attached),
        display("geometry-fullscreen-less", geometry::fullscreen_less),
        display("geometry-bottom", geometry::bottom),
        display("geometry-zoom", geometry::zoom),
        display("geometry-background-window", geometry::background_window),
        display("geometry-rotate", geometry::rotate),
        display("geometry-rotate-unequal", geometry::rotate_unequal),
        display("colors-attach", colors::attach),
        display("colors-new-pane", colors::new_pane),
        display("colors-reload", colors::reload),
        display("colors-styled-pane", colors::styled_pane),
        display("titles-default-format", titles::default_format),
        display("titles-claude-code-format", titles::claude_code_format),
        display("titles-rename", titles::rename),
        display("titles-bottom-and-off", titles::bottom_and_off),
        display("titles-stacked", titles::stacked),
        display("styles-tint", styles::tint),
        display("styles-active-pane", styles::active_pane),
        display("styles-borders", styles::borders),
        display("settings-preview", settings::preview),
        display("settings-cancel", settings::cancel),
        display("settings-apply", settings::apply),
        display("settings-profile-preview", settings::profile_preview),
        display("settings-profile-save", settings::profile_save),
        display("settings-profile-rename", settings::profile_rename),
        display("settings-general", settings::general),
        display("session-end-close", session_end::close),
        display("session-end-detach-close", session_end::detach_close),
        display("session-end-new-tab", session_end::new_tab),
        display("session-end-ask", session_end::ask),
        display("session-end-close-tab", session_end::close_tab),
        display("session-open-elsewhere-goto", windows::open_elsewhere_goto),
        display("session-open-elsewhere-dismiss", windows::open_elsewhere_dismiss),
        display("new-window-asks", windows::new_window_asks),
        display("new-window-new-session", windows::new_window_new_session),
        display("profile-picker-open-state", windows::profile_picker_state),
        display("profile-picker-goes-to-session", windows::picker_goes_to_session),
        display("tmux-new-session", windows::new_session),
        display("tmux-new-session-duplicate", windows::new_session_duplicate),
        display("restore-dedupe", windows::restore_dedupe),
        display("input-palette", input::palette),
        display("input-palette-run", input::palette_run),
        display("input-select-all", input::select_all),
        display("input-settings-tab", input::settings_tab),
        display("input-prompt-preselected", input::prompt_preselected),
        display("links-local-click", links::local_click),
        display("links-mouse-mode", links::mouse_mode),
        display("links-tmux", links::tmux),
        display("restore-local-tabs", restore::local_tabs),
        display("restore-fallbacks", restore::fallbacks),
        display("restore-tmux-session", restore::tmux_session),
        display("restore-forget-on-exit", restore::forget_on_exit),
        display("restore-close-window", restore::close_window),
        display("restore-second-instance", restore::second_instance),
        display("shell-prompt-jump", shell::prompt_jump),
        display("shell-no-marks-scroll-line", shell::no_marks_scroll_line),
        display("shell-failed-mark", shell::failed_mark),
        display("shell-select-output", shell::select_output),
        display("shell-tmux-prompt-jump", shell::tmux_prompt_jump),
        display("shell-clear-up-down", shell::clear_up_down),
        display("shell-tmux-clear-up-down", shell::tmux_clear_up_down),
        display("notify-osc-local", notify::osc_local),
        display("notify-tmux-background", notify::tmux_background),
        display("search-local", search::local),
        display("search-step", search::step),
        display("search-long-query", search::long_query),
        display("search-input-selection", search::input_selection),
        display("search-close-restores", search::close_restores),
        display("search-tmux-pane", search::tmux_pane),
    ]
}

/// A server with one pane running the plain test shell.
fn server() -> Result<TmuxServer> {
    TmuxServer::new(113, 33)
}

/// The window's cell grid, from the run's dump and tmux's geometry for the
/// pane that was active when it was taken.
fn cells(server: &TmuxServer, out: &Output, active: &str) -> Result<Cells> {
    let [left, top, _, _] = server.geometry(active)?;
    Ok(Cells::from_pane(out.dump.grid()?, left, top))
}

/// Skips a case when a program it needs isn't installed.
fn require(program: &str) -> Option<crate::Outcome> {
    let found = std::process::Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .is_ok_and(|o| o.status.success());
    (!found).then(|| crate::Outcome::Skip(format!("{program} is not installed")))
}
