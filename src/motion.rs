//! The window's one stepped motion clock (OpenSpec change
//! `agents-zone-attention`, design decision 1): what drives every spinner
//! and shimmer, at [`STEP`] a step, while some entity wants it.
//!
//! gpui 0.2.2's repeating animation asks for a frame on every display frame,
//! and since it caches no views each is a whole-window render
//! (`process R14`). This clock steps 10 times a second instead, runs only
//! while an entity wants it, and notifies only those entities. Each entity
//! that shows motion wants it from its own change handlers, never from
//! `render`, and stops wanting it when the motion goes and in `on_release`.
//!
//! The pure items ([`next`], [`loader_step`], [`shimmer_position`]) decide;
//! [`Motion`], a GPUI global with its task, applies (`fcis R1`).

use std::collections::BTreeSet;
use std::time::Duration;

use gpui::{App, EntityId, Global, Task};

/// How long one step lasts: 10 steps a second.
pub const STEP: Duration = Duration::from_millis(100);

/// A shimmer's band sweeps across its text in this many steps (2 s).
pub const SHIMMER_SWEEP: u32 = 20;

/// How far before the text's start and past its end the band runs, in
/// characters.
pub const SHIMMER_MARGIN: f32 = 3.0;

/// What the clock's task does after a change to who wants it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Nothing ran and something wants it: spawn the task.
    Start,
    /// It ran and nothing wants it: drop the task.
    Stop,
    /// As it is.
    Keep,
}

/// The clock's task after a change, `running` saying whether it runs and
/// `wanted` whether any entity wants it.
pub fn next(running: bool, wanted: bool) -> Tick {
    match (running, wanted) {
        (false, true) => Tick::Start,
        (true, false) => Tick::Stop,
        (false, false) | (true, true) => Tick::Keep,
    }
}

/// The loader's step for the clock's `step`: one of
/// [`brindle_ui::LOADER_STEPS`] a turn.
pub fn loader_step(step: u32) -> u32 {
    step % brindle_ui::LOADER_STEPS
}

/// The shimmer band's centre, in characters from the start of a text of
/// `len` characters, at the clock's `step`: a sweep of [`SHIMMER_SWEEP`]
/// steps from [`SHIMMER_MARGIN`] before the start to as far past the end.
pub fn shimmer_position(step: u32, len: usize) -> f32 {
    let at = (step % SHIMMER_SWEEP) as f32 / (SHIMMER_SWEEP - 1) as f32;
    -SHIMMER_MARGIN + at * (len as f32 + 2.0 * SHIMMER_MARGIN)
}

/// The one clock: its step, the entities that want it, and its task while
/// it runs.
#[derive(Default)]
pub struct Motion {
    step: u32,
    wanted: BTreeSet<EntityId>,
    task: Option<Task<()>>,
}

impl Global for Motion {}

impl Motion {
    /// The clock's step now: what `render` passes on, as
    /// [`loader_step`] and [`shimmer_position`].
    pub fn step(cx: &App) -> u32 {
        cx.try_global::<Motion>().map_or(0, |motion| motion.step)
    }

    /// Whether the clock runs (the dump's `--- motion` line).
    pub fn running(cx: &App) -> bool {
        cx.try_global::<Motion>().is_some_and(|motion| motion.task.is_some())
    }

    /// Whether `entity` wants the clock now.
    #[cfg(test)]
    pub fn wants(entity: EntityId, cx: &App) -> bool {
        cx.try_global::<Motion>().is_some_and(|motion| motion.wanted.contains(&entity))
    }

    /// `entity` wants the clock (`want`) or no longer does; the clock
    /// starts or stops as [`next`] says.
    pub fn want(entity: EntityId, want: bool, cx: &mut App) {
        let tick = {
            let motion = cx.default_global::<Motion>();
            let changed = if want { motion.wanted.insert(entity) } else { motion.wanted.remove(&entity) };
            if !changed {
                return;
            }
            next(motion.task.is_some(), !motion.wanted.is_empty())
        };
        match tick {
            Tick::Start => {
                let task = cx.spawn(async move |cx| {
                    loop {
                        cx.background_executor().timer(STEP).await;
                        let ticked = cx.update(|cx| {
                            let motion = cx.default_global::<Motion>();
                            motion.step = motion.step.wrapping_add(1);
                            let wanted: Vec<EntityId> = motion.wanted.iter().copied().collect();
                            for entity in &wanted {
                                cx.notify(*entity);
                            }
                            !wanted.is_empty()
                        });
                        if !matches!(ticked, Ok(true)) {
                            break;
                        }
                    }
                });
                cx.default_global::<Motion>().task = Some(task);
            }
            Tick::Stop => cx.default_global::<Motion>().task = None,
            Tick::Keep => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, Context, TestAppContext};

    use super::*;

    #[test]
    fn the_clock_starts_and_stops_with_who_wants_it() {
        assert_eq!(next(false, true), Tick::Start);
        assert_eq!(next(true, false), Tick::Stop);
        assert_eq!(next(false, false), Tick::Keep);
        assert_eq!(next(true, true), Tick::Keep);
    }

    #[test]
    fn a_loader_turns_in_twelve_steps() {
        assert_eq!(loader_step(0), 0);
        assert_eq!(loader_step(3), 3);
        assert_eq!(loader_step(11), 11);
        assert_eq!(loader_step(12), 0);
        assert_eq!(loader_step(25), 1);
        assert_eq!(loader_step(u32::MAX), u32::MAX % 12);
    }

    #[test]
    fn the_shimmer_sweeps_from_before_the_start_to_past_the_end() {
        assert_eq!(shimmer_position(0, 8), -3.0);
        assert_eq!(shimmer_position(19, 8), 11.0);
        assert_eq!(shimmer_position(20, 8), -3.0, "the sweep starts again");
        let positions: Vec<f32> = (0..20).map(|step| shimmer_position(step, 8)).collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{positions:?}");
        assert_eq!(shimmer_position(0, 0), -3.0);
        assert_eq!(shimmer_position(19, 0), 3.0);
    }

    struct Watcher {
        renders: usize,
    }

    impl gpui::Render for Watcher {
        fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            self.renders += 1;
            gpui::div()
        }
    }

    /// Decision 1's entity test: the step moves only while some entity
    /// wants the clock, the task is dropped once none does, and a released
    /// entity no longer counts.
    #[test]
    fn the_step_moves_only_while_wanted() {
        let mut app = TestAppContext::single();
        let (watcher, cx) = app.add_window_view(|_, _| Watcher { renders: 0 });
        cx.run_until_parked();
        let id = watcher.entity_id();
        let step = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| Motion::step(cx));
        let running = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| Motion::running(cx));
        assert!(!running(cx));
        cx.executor().advance_clock(STEP * 3);
        cx.run_until_parked();
        assert_eq!(step(cx), 0, "nothing wants it");

        cx.update(|_, cx| Motion::want(id, true, cx));
        assert!(running(cx));
        let before = watcher.read_with(cx, |w, _| w.renders);
        cx.executor().advance_clock(STEP);
        cx.run_until_parked();
        assert_eq!(step(cx), 1);
        cx.executor().advance_clock(STEP);
        cx.run_until_parked();
        assert_eq!(step(cx), 2);
        assert!(watcher.read_with(cx, |w, _| w.renders) > before, "the wanting entity is notified");

        // A second want of the same entity changes nothing.
        cx.update(|_, cx| Motion::want(id, true, cx));
        cx.update(|_, cx| Motion::want(id, false, cx));
        assert!(!running(cx), "the task is dropped once nothing wants it");
        cx.executor().advance_clock(STEP * 5);
        cx.run_until_parked();
        assert_eq!(step(cx), 2, "the step stays");

        // An entity that is released while it wants the clock stops it.
        let other = cx.update(|_, cx| {
            let other = cx.new(|_| ());
            let id = other.entity_id();
            Motion::want(id, true, cx);
            cx.observe_release(&other, move |_, cx| Motion::want(id, false, cx)).detach();
            other
        });
        assert!(running(cx));
        drop(other);
        // Released entities go when the app next flushes its effects.
        cx.update(|_, _| {});
        cx.run_until_parked();
        assert!(!running(cx), "released, it no longer wants the clock");
    }
}
