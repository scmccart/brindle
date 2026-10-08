//! Keystroke latency probes behind `--latency-log` and `--latency-bench`.
//!
//! A probe starts when a terminal view receives a keystroke and records
//! three stages, in milliseconds from that moment:
//! - `output`: the program's output for the terminal arrived;
//! - `paint`: the terminal painted a grid that includes that output;
//! - `frame`: the next frame began (GPUI presents right after painting,
//!   so this bounds when the frame was handed to the GPU).
//!
//! Output is tracked with a per-terminal generation counter (see
//! [`crate::terminal::OutputMarker`]), so repaints caused by the keystroke
//! itself (scrolling back to the prompt, clearing a selection) can't
//! complete a probe. None of this measures GPU, compositor or scanout time.
//!
//! The state is a GPUI global that exists only when one of the options is
//! given; every hook first checks for it.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use gpui::{App, EntityId, Global, Window};

/// A probe with no paint by then is reported as a timeout.
pub const TIMEOUT: Duration = Duration::from_secs(1);

struct Probe {
    id: u64,
    label: String,
    start: Instant,
    /// The terminal's output generation when the key arrived.
    start_generation: u64,
    /// When output arrived, and the generation a paint must reach.
    output: Option<(Instant, u64)>,
    paint: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done { output: Duration, paint: Duration, frame: Duration },
    Timeout,
}

impl Outcome {
    /// Output, paint and frame times, unless it timed out.
    fn stages(&self) -> Option<[Duration; 3]> {
        match *self {
            Outcome::Done { output, paint, frame } => Some([output, paint, frame]),
            Outcome::Timeout => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub label: String,
    pub outcome: Outcome,
}

/// The probe life cycle, with time passed in so it can be tested.
#[derive(Default)]
pub struct Probes {
    pending: HashMap<EntityId, VecDeque<Probe>>,
    next_id: u64,
    started: u64,
    /// Finished samples not yet taken by [`Probes::take_finished`].
    finished: Vec<Sample>,
}

impl Probes {
    /// A key reached `terminal`, whose output generation is `generation`.
    pub fn start(&mut self, terminal: EntityId, label: String, generation: u64, now: Instant) -> u64 {
        self.next_id += 1;
        self.started += 1;
        let probe = Probe {
            id: self.next_id,
            label,
            start: now,
            start_generation: generation,
            output: None,
            paint: None,
        };
        self.pending.entry(terminal).or_default().push_back(probe);
        self.next_id
    }

    /// How many probes have been started.
    pub fn started(&self) -> u64 {
        self.started
    }

    /// Output reached `terminal`, advancing it to `generation` at `at`.
    /// Echoes of several quick keys can arrive together, so every waiting
    /// probe takes it.
    pub fn output(&mut self, terminal: EntityId, generation: u64, at: Instant) {
        for probe in self.pending.get_mut(&terminal).into_iter().flatten() {
            if probe.output.is_none() && generation > probe.start_generation {
                probe.output = Some((at.max(probe.start), generation));
            }
        }
    }

    /// `terminal` painted a grid as of `generation` at `at`. `output_at` is
    /// when that generation's output arrived, for probes whose output event
    /// hasn't been handled yet. Returns the probes now waiting for a frame.
    pub fn painted(&mut self, terminal: EntityId, generation: u64, output_at: Instant, at: Instant) -> Vec<u64> {
        let mut ready = Vec::new();
        for probe in self.pending.get_mut(&terminal).into_iter().flatten() {
            if probe.output.is_none() && generation > probe.start_generation {
                probe.output = Some((output_at.clamp(probe.start, at), generation));
            }
            if probe.paint.is_none()
                && let Some((_, target)) = probe.output
                && generation >= target
            {
                probe.paint = Some(at);
                ready.push(probe.id);
            }
        }
        ready
    }

    /// The frame after probe `id`'s paint began at `at`.
    pub fn frame(&mut self, id: u64, at: Instant) {
        for queue in self.pending.values_mut() {
            if let Some(ix) = queue.iter().position(|p| p.id == id) {
                let probe = queue.remove(ix).expect("index from position");
                let (output, _) = probe.output.expect("painted probes have output");
                let paint = probe.paint.expect("frame follows paint");
                self.finished.push(Sample {
                    label: probe.label,
                    outcome: Outcome::Done {
                        output: output - probe.start,
                        paint: paint - probe.start,
                        frame: at.max(paint) - probe.start,
                    },
                });
                return;
            }
        }
    }

    /// Turns probes older than [`TIMEOUT`] into timeouts. A painted probe
    /// waiting for its frame only times out after twice that.
    pub fn expire(&mut self, now: Instant) {
        for queue in self.pending.values_mut() {
            queue.retain(|probe| {
                let limit = if probe.paint.is_some() { TIMEOUT * 2 } else { TIMEOUT };
                if now.duration_since(probe.start) < limit {
                    return true;
                }
                self.finished.push(Sample { label: probe.label.clone(), outcome: Outcome::Timeout });
                false
            });
        }
        self.pending.retain(|_, queue| !queue.is_empty());
    }

    /// Whether any probe is still waiting.
    pub fn is_idle(&self) -> bool {
        self.pending.values().all(VecDeque::is_empty)
    }

    pub fn take_finished(&mut self) -> Vec<Sample> {
        std::mem::take(&mut self.finished)
    }
}

/// Nearest-rank percentile of sorted values (`p` in 0..=100).
pub fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn ms(d: Duration) -> String {
    format!("{:.2}", d.as_secs_f64() * 1000.0)
}

/// The printed summary: a header, then min/p50/p95/p99/max per stage.
pub fn summary(samples: &[Sample]) -> String {
    let timeouts = samples.iter().filter(|s| s.outcome == Outcome::Timeout).count();
    let mut out = format!("--- latency samples={} timeouts={timeouts}\n", samples.len());
    for (ix, name) in ["output", "paint", "frame"].into_iter().enumerate() {
        let mut values: Vec<Duration> = samples.iter().filter_map(|s| s.outcome.stages()).map(|t| t[ix]).collect();
        values.sort();
        out.push_str(&format!(
            "{name:<6} min={} p50={} p95={} p99={} max={}\n",
            ms(percentile(&values, 0.0)),
            ms(percentile(&values, 50.0)),
            ms(percentile(&values, 95.0)),
            ms(percentile(&values, 99.0)),
            ms(values.last().copied().unwrap_or_default()),
        ));
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One `--latency-log` line.
pub fn json_line(sample: &Sample) -> String {
    match &sample.outcome {
        Outcome::Done { output, paint, frame } => format!(
            "{{\"key\":{},\"output_ms\":{},\"paint_ms\":{},\"frame_ms\":{}}}\n",
            json_string(&sample.label),
            ms(*output),
            ms(*paint),
            ms(*frame)
        ),
        Outcome::Timeout => format!("{{\"key\":{},\"timeout\":true}}\n", json_string(&sample.label)),
    }
}

/// The app-wide probe state; present only when probing is enabled.
pub struct Latency {
    probes: Probes,
    log: Option<File>,
    /// Every sample since the last [`Latency::reset`].
    samples: Vec<Sample>,
    /// Every sample since startup, for the log's summary.
    all_samples: Vec<Sample>,
}

impl Global for Latency {}

impl Latency {
    /// Enables probing, appending samples to `log` if given.
    pub fn install(log: Option<&Path>, cx: &mut App) -> std::io::Result<()> {
        let log = log
            .map(|path| std::fs::OpenOptions::new().create(true).append(true).open(path))
            .transpose()?;
        cx.set_global(Latency { probes: Probes::default(), log, samples: Vec::new(), all_samples: Vec::new() });
        Ok(())
    }

    pub fn enabled(cx: &App) -> bool {
        cx.has_global::<Latency>()
    }

    fn get(cx: &mut App) -> Option<&mut Latency> {
        cx.has_global::<Latency>().then(|| cx.global_mut::<Latency>())
    }

    /// Hook: a terminal view received a keystroke.
    pub fn key(terminal: EntityId, label: impl FnOnce() -> String, generation: u64, cx: &mut App) {
        if let Some(this) = Self::get(cx) {
            this.collect(Instant::now());
            this.probes.start(terminal, label(), generation, Instant::now());
        }
    }

    /// Hook: output reached a terminal.
    pub fn output(terminal: EntityId, generation: u64, at: Instant, cx: &mut App) {
        if let Some(this) = Self::get(cx) {
            this.probes.output(terminal, generation, at);
        }
    }

    /// Hook: a terminal finished painting a grid as of `generation`.
    pub fn painted(terminal: EntityId, generation: u64, output_at: Instant, window: &mut Window, cx: &mut App) {
        let Some(this) = Self::get(cx) else {
            return;
        };
        for id in this.probes.painted(terminal, generation, output_at, Instant::now()) {
            window.on_next_frame(move |_, cx| {
                if let Some(this) = Latency::get(cx) {
                    this.probes.frame(id, Instant::now());
                    this.collect(Instant::now());
                }
            });
        }
    }

    /// Expires old probes and moves finished samples into the totals and log.
    fn collect(&mut self, now: Instant) {
        self.probes.expire(now);
        for sample in self.probes.take_finished() {
            if let Some(log) = &mut self.log
                && let Err(err) = log.write_all(json_line(&sample).as_bytes())
            {
                log::error!("--latency-log: {err}");
                self.log = None;
            }
            self.samples.push(sample.clone());
            self.all_samples.push(sample);
        }
    }

    /// Expires and collects; returns how many samples there are and whether
    /// any probe is still waiting.
    pub fn poll(cx: &mut App) -> Option<(usize, bool)> {
        let this = Self::get(cx)?;
        this.collect(Instant::now());
        Some((this.samples.len(), this.probes.is_idle()))
    }

    pub fn started(cx: &App) -> u64 {
        cx.try_global::<Latency>().map_or(0, |this| this.probes.started())
    }

    /// Drops the samples gathered so far (benchmark warm-up).
    pub fn reset(cx: &mut App) {
        if let Some(this) = Self::get(cx) {
            this.samples.clear();
        }
    }

    /// The summary of samples since the last reset.
    pub fn summary(cx: &mut App) -> String {
        Self::poll(cx);
        cx.try_global::<Latency>().map(|this| summary(&this.samples)).unwrap_or_default()
    }

    /// The summary of every sample, for the log at quit.
    pub fn log_summary(cx: &mut App) -> Option<String> {
        Self::poll(cx);
        let this = cx.try_global::<Latency>()?;
        this.log.is_some().then(|| summary(&this.all_samples))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn terminal() -> EntityId {
        EntityId::from(1u64)
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        assert_eq!(percentile(&[], 50.0), Duration::ZERO);
        assert_eq!(percentile(&[ms(7)], 0.0), ms(7));
        assert_eq!(percentile(&[ms(7)], 99.0), ms(7));
        let four = [ms(1), ms(2), ms(3), ms(4)];
        assert_eq!(percentile(&four, 50.0), ms(2));
        assert_eq!(percentile(&four, 95.0), ms(4));
        assert_eq!(percentile(&four, 0.0), ms(1));
        let same = [ms(5); 10];
        assert_eq!(percentile(&same, 99.0), ms(5));
        let hundred: Vec<Duration> = (1..=100).map(ms).collect();
        assert_eq!(percentile(&hundred, 95.0), ms(95));
        assert_eq!(percentile(&hundred, 99.0), ms(99));
    }

    #[test]
    fn summary_lists_each_stage() {
        let samples = vec![
            Sample { label: "x".into(), outcome: Outcome::Done { output: ms(1), paint: ms(3), frame: ms(5) } },
            Sample { label: "x".into(), outcome: Outcome::Timeout },
        ];
        assert_eq!(
            summary(&samples),
            "--- latency samples=2 timeouts=1\n\
             output min=1.00 p50=1.00 p95=1.00 p99=1.00 max=1.00\n\
             paint  min=3.00 p50=3.00 p95=3.00 p99=3.00 max=3.00\n\
             frame  min=5.00 p50=5.00 p95=5.00 p99=5.00 max=5.00\n"
        );
    }

    #[test]
    fn json_lines() {
        let done = Sample {
            label: "backspace".into(),
            outcome: Outcome::Done { output: ms(1), paint: Duration::from_micros(2500), frame: ms(9) },
        };
        assert_eq!(json_line(&done), "{\"key\":\"backspace\",\"output_ms\":1.00,\"paint_ms\":2.50,\"frame_ms\":9.00}\n");
        let timeout = Sample { label: "\"\n".into(), outcome: Outcome::Timeout };
        assert_eq!(json_line(&timeout), "{\"key\":\"\\\"\\u000a\",\"timeout\":true}\n");
    }

    #[test]
    fn probe_runs_through_its_stages() {
        let t0 = Instant::now();
        let mut p = Probes::default();
        let id = p.start(terminal(), "x".into(), 5, t0);
        p.output(terminal(), 6, t0 + ms(2));
        assert_eq!(p.painted(terminal(), 6, t0, t0 + ms(4)), vec![id]);
        p.frame(id, t0 + ms(10));
        assert!(p.is_idle());
        assert_eq!(
            p.take_finished(),
            vec![Sample { label: "x".into(), outcome: Outcome::Done { output: ms(2), paint: ms(4), frame: ms(10) } }]
        );
    }

    #[test]
    fn a_paint_without_new_output_does_not_complete_a_probe() {
        // The keystroke itself repaints (scroll to bottom, selection cleared)
        // before its echo arrives: same generation, so nothing completes.
        let t0 = Instant::now();
        let mut p = Probes::default();
        p.start(terminal(), "x".into(), 5, t0);
        assert!(p.painted(terminal(), 5, t0, t0 + ms(1)).is_empty());
        assert!(!p.is_idle());
    }

    #[test]
    fn batched_echoes_complete_every_waiting_probe() {
        let t0 = Instant::now();
        let mut p = Probes::default();
        let a = p.start(terminal(), "a".into(), 5, t0);
        let b = p.start(terminal(), "b".into(), 5, t0 + ms(1));
        p.output(terminal(), 7, t0 + ms(3));
        assert_eq!(p.painted(terminal(), 7, t0, t0 + ms(5)), vec![a, b]);
        p.frame(b, t0 + ms(9));
        p.frame(a, t0 + ms(9));
        let finished = p.take_finished();
        assert_eq!(finished.len(), 2);
        assert_eq!(finished[0].outcome, Outcome::Done { output: ms(2), paint: ms(4), frame: ms(8) });
        assert_eq!(finished[1].outcome, Outcome::Done { output: ms(3), paint: ms(5), frame: ms(9) });
    }

    #[test]
    fn a_paint_ahead_of_the_output_event_still_counts() {
        // The I/O thread advanced the generation and the grid was painted
        // before the output event was handled on the main thread.
        let t0 = Instant::now();
        let mut p = Probes::default();
        let id = p.start(terminal(), "x".into(), 5, t0);
        assert_eq!(p.painted(terminal(), 6, t0 + ms(2), t0 + ms(3)), vec![id]);
        p.output(terminal(), 6, t0 + ms(4));
        p.frame(id, t0 + ms(7));
        assert_eq!(p.take_finished()[0].outcome, Outcome::Done { output: ms(2), paint: ms(3), frame: ms(7) });
    }

    #[test]
    fn other_terminals_do_not_complete_a_probe() {
        let t0 = Instant::now();
        let mut p = Probes::default();
        p.start(terminal(), "x".into(), 5, t0);
        p.output(EntityId::from(2u64), 9, t0 + ms(1));
        assert!(p.painted(EntityId::from(2u64), 9, t0, t0 + ms(2)).is_empty());
        assert!(!p.is_idle());
    }

    #[test]
    fn silent_keys_time_out() {
        let t0 = Instant::now();
        let mut p = Probes::default();
        p.start(terminal(), "f13".into(), 5, t0);
        p.expire(t0 + ms(999));
        assert!(!p.is_idle());
        p.expire(t0 + TIMEOUT);
        assert!(p.is_idle());
        assert_eq!(p.take_finished(), vec![Sample { label: "f13".into(), outcome: Outcome::Timeout }]);
    }
}
