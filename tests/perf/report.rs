//! What a run measured: parsing Brindle's latency summary and vtebench's
//! `.dat` files, the `summary.toml` kept with each run, and the printed
//! tables (with deltas against a baseline run).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Result;

/// min, p50, p95, p99 and max, in milliseconds.
pub type Percentiles = [f64; 5];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencyResult {
    pub samples: u64,
    pub timeouts: u64,
    pub output: Percentiles,
    pub paint: Percentiles,
    pub frame: Percentiles,
    /// Whether the agent's reply was still streaming when the bench ended
    /// (`agent-latency-trickle`, OpenSpec change `agents-absorb-cost`): a
    /// run whose stream ended first measured part of the bench beside an
    /// idle zone. Absent from the other cases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_outlasted: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThroughputResult {
    /// `COLSxROWS` the benchmark ran at.
    pub grid: String,
    /// Mean milliseconds per sample, by vtebench benchmark.
    pub mean_ms: BTreeMap<String, f64>,
}

/// A reply streamed into an agent zone, timed in milliseconds from the
/// gate that lets the agent answer to Brindle's exit on the reply's last
/// line: once in small chunks and once whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamResult {
    /// The reply's length.
    pub bytes: u64,
    /// How many `agent_message_chunk`s carried it in the streamed run.
    pub chunks: u64,
    /// Gate to exit, streamed in `chunks` chunks.
    pub streamed_ms: f64,
    /// Gate to exit, sent as one chunk.
    pub whole_ms: f64,
    /// `streamed_ms - whole_ms`: what streaming costs over one parse.
    pub difference_ms: f64,
    /// Gate to exit, streamed in 32-byte pieces (`agent-tool-output`'s
    /// third run, OpenSpec change `agents-checkpoint-deltas`); absent from
    /// the other cases and from runs made before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_ms: Option<f64>,
    /// The step interval Brindle waits between seeing the last line and
    /// quitting, part of both times.
    pub step_interval_ms: f64,
    /// Gate to exit, the streamed run's chunks about 10 ms apart
    /// (`agent-markdown-stream`'s third run, OpenSpec change
    /// `agents-absorb-cost`); absent from the other cases and older runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trickled_ms: Option<f64>,
    /// The Markdown renders the zone made in the streamed run, and the
    /// bytes they were given (Brindle's `agent renders=` log line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamed_renders: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamed_render_bytes: Option<u64>,
    /// The same for the trickled run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trickled_renders: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trickled_render_bytes: Option<u64>,
}

/// A tool-heavy agent turn (OpenSpec change `agents-checkpoint-deltas`,
/// design.md decision 15): permission-gated tool calls and one long line,
/// timed in milliseconds from the gate to Brindle's exit on a
/// `--wait-for` of the turn's last message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolHeavyResult {
    /// The tool calls, each asking a permission the agent withdraws.
    pub calls: u64,
    /// The long line's length.
    pub line_bytes: u64,
    /// Gate to exit.
    pub turn_ms: f64,
    /// The step interval Brindle waits between seeing the last message and
    /// quitting, part of `turn_ms`.
    pub step_interval_ms: f64,
}

/// A long agent conversation attached again (OpenSpec change
/// `agents-log-paging`, design.md decision 14), timed in milliseconds from
/// Brindle's launch to its exit on a `--wait-for` of the conversation's
/// last entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttachResult {
    /// The conversation's text, every entry's.
    pub bytes: u64,
    /// Its entries, the prompt included.
    pub entries: u64,
    /// Launch to exit, restoring the tab that attaches it.
    pub attach_ms: f64,
    /// The step interval Brindle waits between seeing the last entry and
    /// quitting, part of `attach_ms`.
    pub step_interval_ms: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    /// Environment notes: versions, CPU governor, display.
    pub meta: BTreeMap<String, String>,
    pub latency: BTreeMap<String, LatencyResult>,
    pub throughput: BTreeMap<String, ThroughputResult>,
    /// Absent from runs made before the stream cases existed.
    #[serde(default)]
    pub stream: BTreeMap<String, StreamResult>,
    /// Absent from runs made before the attach case existed.
    #[serde(default)]
    pub attach: BTreeMap<String, AttachResult>,
    /// Absent from runs made before the tool-heavy case existed.
    #[serde(default)]
    pub tool_heavy: BTreeMap<String, ToolHeavyResult>,
}

impl Summary {
    pub fn load(path: &std::path::Path) -> Result<Summary> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("summary serializes")
    }
}

/// Parses `--latency-bench` output:
/// ```text
/// --- latency samples=200 timeouts=0
/// output min=… p50=… p95=… p99=… max=…
/// ```
pub fn parse_latency(text: &str) -> Result<LatencyResult> {
    let mut result = LatencyResult::default();
    let header = text
        .lines()
        .find_map(|l| l.strip_prefix("--- latency "))
        .ok_or("no `--- latency` line in Brindle's output")?;
    let field = |line: &str, name: &str| -> Option<f64> {
        line.split_whitespace().find_map(|f| f.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
    };
    result.samples = field(header, "samples").ok_or("no sample count")? as u64;
    result.timeouts = field(header, "timeouts").ok_or("no timeout count")? as u64;
    for (stage, slot) in [("output", &mut result.output), ("paint", &mut result.paint), ("frame", &mut result.frame)] {
        let line = text
            .lines()
            .find(|l| l.split_whitespace().next() == Some(stage))
            .ok_or_else(|| format!("no `{stage}` line"))?;
        for (ix, name) in ["min", "p50", "p95", "p99", "max"].into_iter().enumerate() {
            slot[ix] = field(line, name).ok_or_else(|| format!("no {name} on the `{stage}` line"))?;
        }
    }
    Ok(result)
}

/// Mean milliseconds per benchmark from a vtebench `.dat` file: a header of
/// benchmark names, then one row per sample, with `_` where a benchmark ran
/// out of samples.
pub fn parse_dat(text: &str) -> Result<BTreeMap<String, f64>> {
    let mut lines = text.lines();
    let names: Vec<&str> = lines.next().ok_or("empty .dat file")?.split_whitespace().collect();
    let mut sums = vec![(0.0, 0usize); names.len()];
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != names.len() {
            return Err(format!("row {line:?} has {} fields for {} benchmarks", fields.len(), names.len()));
        }
        for (sum, field) in sums.iter_mut().zip(fields) {
            if field != "_" {
                sum.0 += field.parse::<f64>().map_err(|_| format!("bad sample {field:?}"))?;
                sum.1 += 1;
            }
        }
    }
    Ok(names
        .into_iter()
        .zip(sums)
        .filter(|(_, (_, n))| *n > 0)
        .map(|(name, (sum, n))| (name.to_string(), sum / n as f64))
        .collect())
}

/// `value`, plus its change from `base` when there is one.
fn cell(value: f64, base: Option<f64>, percent: bool) -> String {
    match base {
        Some(b) if percent && b != 0.0 => format!("{value:.2} ({:+.1}%)", (value - b) / b * 100.0),
        Some(b) if !percent => format!("{value:.2} ({:+.2})", value - b),
        _ => format!("{value:.2}"),
    }
}

fn table(rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..rows[0].len()).map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(ix, (text, w))| if ix == 0 { format!("{text:<w$}") } else { format!("{text:>w$}") })
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// The printed report. Latency deltas are in milliseconds, throughput
/// deltas in percent; lower is better for both.
pub fn render(summary: &Summary, baseline: Option<&Summary>) -> String {
    let mut out = String::new();
    if !summary.latency.is_empty() {
        let mut rows = vec![["latency (ms)", "p50", "p95", "p99", "max", "timeouts"].map(String::from).to_vec()];
        for (case, result) in &summary.latency {
            let base = baseline.and_then(|b| b.latency.get(case));
            for (stage, values, base_values) in [
                ("output", result.output, base.map(|b| b.output)),
                ("paint", result.paint, base.map(|b| b.paint)),
                ("frame", result.frame, base.map(|b| b.frame)),
            ] {
                let mut row = vec![format!("{case} {stage}")];
                for ix in 1..5 {
                    row.push(cell(values[ix], base_values.map(|b| b[ix]), false));
                }
                row.push(format!("{}/{}", result.timeouts, result.samples));
                rows.push(row);
            }
        }
        out.push_str(&table(&rows));
    }
    if !summary.throughput.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        let cases: Vec<&String> = summary.throughput.keys().collect();
        let mut header = vec!["throughput (mean ms)".to_string()];
        header.extend(cases.iter().map(|c| c.trim_start_matches("throughput-").to_string()));
        let mut grid = vec!["grid".to_string()];
        grid.extend(summary.throughput.values().map(|t| t.grid.clone()));
        let mut rows = vec![header, grid];
        let mut benches: Vec<&String> = summary.throughput.values().flat_map(|t| t.mean_ms.keys()).collect();
        benches.sort();
        benches.dedup();
        for bench in benches {
            let mut row = vec![bench.clone()];
            for case in &cases {
                let base = baseline.and_then(|b| b.throughput.get(*case)).and_then(|t| t.mean_ms.get(bench)).copied();
                row.push(match summary.throughput[*case].mean_ms.get(bench) {
                    Some(v) => cell(*v, base, true),
                    None => "-".into(),
                });
            }
            rows.push(row);
        }
        out.push_str(&table(&rows));
    }
    if !summary.stream.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        // The 32-byte run's column shows only when a case has one.
        let small = summary.stream.values().any(|result| result.small_ms.is_some());
        let mut header = ["stream (ms)", "streamed", "one chunk", "difference", "chunks"].map(String::from).to_vec();
        if small {
            header.push("32 B pieces".into());
        }
        // The trickled run's columns, likewise.
        let trickled = summary.stream.values().any(|result| result.trickled_ms.is_some());
        if trickled {
            header.extend(["trickled", "renders", "trickled renders"].map(String::from));
        }
        let mut rows = vec![header];
        for (case, result) in &summary.stream {
            let base = baseline.and_then(|b| b.stream.get(case));
            let mut row = vec![
                case.clone(),
                cell(result.streamed_ms, base.map(|b| b.streamed_ms), false),
                cell(result.whole_ms, base.map(|b| b.whole_ms), false),
                cell(result.difference_ms, base.map(|b| b.difference_ms), false),
                result.chunks.to_string(),
            ];
            if small {
                row.push(match result.small_ms {
                    Some(ms) => cell(ms, base.and_then(|b| b.small_ms), false),
                    None => "-".into(),
                });
            }
            if trickled {
                let count = |n: Option<u64>| n.map_or_else(|| "-".to_string(), |n| n.to_string());
                row.push(match result.trickled_ms {
                    Some(ms) => cell(ms, base.and_then(|b| b.trickled_ms), false),
                    None => "-".into(),
                });
                row.push(count(result.streamed_renders));
                row.push(count(result.trickled_renders));
            }
            rows.push(row);
        }
        out.push_str(&table(&rows));
    }
    if !summary.attach.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        let mut rows = vec![["attach (ms)", "launch to shown", "entries", "MiB"].map(String::from).to_vec()];
        for (case, result) in &summary.attach {
            let base = baseline.and_then(|b| b.attach.get(case));
            rows.push(vec![
                case.clone(),
                cell(result.attach_ms, base.map(|b| b.attach_ms), false),
                result.entries.to_string(),
                format!("{:.1}", result.bytes as f64 / (1024.0 * 1024.0)),
            ]);
        }
        out.push_str(&table(&rows));
    }
    if !summary.tool_heavy.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        let mut rows = vec![["tool-heavy (ms)", "gate to shown", "calls", "line MiB"].map(String::from).to_vec()];
        for (case, result) in &summary.tool_heavy {
            let base = baseline.and_then(|b| b.tool_heavy.get(case));
            rows.push(vec![
                case.clone(),
                cell(result.turn_ms, base.map(|b| b.turn_ms), false),
                result.calls.to_string(),
                format!("{:.1}", result.line_bytes as f64 / (1024.0 * 1024.0)),
            ]);
        }
        out.push_str(&table(&rows));
    }
    out
}

pub fn self_test_latency() -> Result {
    // A dump's lines may come first; its fonts line names families that
    // could look like a stage (`output`).
    let text = "noise\n--- grid origin=0,0 cell=8x16 scale=1\n--- fonts terminal=\"output\" ui=\"frame\"\n\
                --- latency samples=200 timeouts=2\n\
                output min=0.10 p50=0.40 p95=0.90 p99=1.50 max=3.00\n\
                paint  min=1.00 p50=2.00 p95=3.00 p99=4.00 max=5.00\n\
                frame  min=6.00 p50=7.00 p95=8.00 p99=9.00 max=10.00\n";
    let r = parse_latency(text)?;
    crate::ensure!(r.samples == 200 && r.timeouts == 2, "counts {r:?}");
    crate::ensure!(r.output == [0.1, 0.4, 0.9, 1.5, 3.0], "output {:?}", r.output);
    crate::ensure!(r.frame == [6.0, 7.0, 8.0, 9.0, 10.0], "frame {:?}", r.frame);
    crate::ensure!(parse_latency("--- screen 80x24").is_err(), "parsed a dump as latency");
    Ok(())
}

pub fn self_test_dat() -> Result {
    let means = parse_dat(include_str!("fixtures/sample.dat"))?;
    let expected: BTreeMap<String, f64> =
        [("cursor_motion", 12.0), ("dense_cells", 21.0), ("unicode", 31.0)].map(|(k, v)| (k.to_string(), v)).into();
    crate::ensure!(means == expected, "means {means:?}");
    crate::ensure!(parse_dat("a b\n1\n").is_err(), "accepted a short row");
    Ok(())
}

pub fn self_test_render() -> Result {
    let base = Summary::load(&fixture("base.toml"))?;
    let current = Summary::load(&fixture("current.toml"))?;
    let text = render(&current, Some(&base));
    let expected = include_str!("fixtures/render.txt");
    crate::ensure!(text == expected, "rendered:\n{text}\nexpected:\n{expected}");
    // A round trip keeps everything.
    let again: Summary = toml::from_str(&current.to_toml()).map_err(|e| e.to_string())?;
    crate::ensure!(
        again.latency == current.latency
            && again.throughput == current.throughput
            && again.stream == current.stream
            && again.attach == current.attach
            && again.tool_heavy == current.tool_heavy,
        "round trip lost data"
    );
    // A summary from before the stream cases still loads, with none.
    let text = std::fs::read_to_string(fixture("base.toml")).map_err(|e| e.to_string())?;
    let before = text.split("[stream.").next().unwrap_or_default();
    let old: Summary = toml::from_str(before).map_err(|e| e.to_string())?;
    crate::ensure!(old.stream.is_empty(), "an old summary has stream results: {:?}", old.stream);
    Ok(())
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/fixtures").join(name)
}
