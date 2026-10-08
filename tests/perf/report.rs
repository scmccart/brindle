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
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThroughputResult {
    /// `COLSxROWS` the benchmark ran at.
    pub grid: String,
    /// Mean milliseconds per sample, by vtebench benchmark.
    pub mean_ms: BTreeMap<String, f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    /// Environment notes: versions, CPU governor, display.
    pub meta: BTreeMap<String, String>,
    pub latency: BTreeMap<String, LatencyResult>,
    pub throughput: BTreeMap<String, ThroughputResult>,
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
    out
}

pub fn self_test_latency() -> Result {
    let text = "noise\n--- latency samples=200 timeouts=2\n\
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
    crate::ensure!(again.latency == current.latency && again.throughput == current.throughput, "round trip lost data");
    Ok(())
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/fixtures").join(name)
}
