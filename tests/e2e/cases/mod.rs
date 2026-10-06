//! The cases. Each is one function; `all` lists them in run order.

mod colors;
mod dump;
mod geometry;
mod settings;
mod styles;
mod titles;
mod tmux;

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
