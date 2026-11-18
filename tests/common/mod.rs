//! Helpers shared by the end-to-end and performance suites, included by
//! each with `#[path]`.

#![allow(dead_code)]

pub mod server;
#[cfg(windows)]
pub mod winproc;

pub type Result<T = ()> = std::result::Result<T, String>;

/// Lines with trailing spaces and trailing empty lines removed.
pub fn trim_screen<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = lines.map(|l| l.trim_end().to_string()).collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Fails with a message unless `cond` holds.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}

/// Which cases a runner runs: its arguments that aren't options are name
/// filters (a case runs when its name contains one, or when there are
/// none), and `--shard K/N` takes the next argument.
#[derive(Debug, Default, PartialEq)]
pub struct Selection<'a> {
    pub filters: Vec<&'a str>,
    pub shard: Option<(usize, usize)>,
}

impl<'a> Selection<'a> {
    /// The runner's arguments; `Err` says what is wrong with `--shard`.
    pub fn parse(args: &'a [String]) -> Result<Self> {
        let mut selection = Selection::default();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if arg == "--shard" {
                let value = it.next().ok_or("--shard needs K/N")?;
                selection.shard = Some(parse_shard(value)?);
            } else if !arg.starts_with("--") {
                selection.filters.push(arg);
            }
        }
        Ok(selection)
    }

    /// The cases the filters name.
    pub fn matching<T>(&self, cases: Vec<T>, name: impl Fn(&T) -> &str) -> Vec<T> {
        cases
            .into_iter()
            .filter(|case| self.filters.is_empty() || self.filters.iter().any(|f| name(case).contains(f)))
            .collect()
    }

    /// The cases the filters name, then the shard's share of them.
    pub fn select<T>(&self, cases: Vec<T>, name: impl Fn(&T) -> &str) -> Vec<T> {
        let matched = self.matching(cases, name);
        match self.shard {
            Some((k, n)) => take_shard(matched, k, n),
            None => matched,
        }
    }
}

/// `K/N` for `--shard`: shard K (1-based) of N.
pub fn parse_shard(value: &str) -> Result<(usize, usize)> {
    let bad = || format!("--shard needs K/N with 1 <= K <= N, got {value:?}");
    let (k, n) = value.split_once('/').ok_or_else(bad)?;
    let (k, n): (usize, usize) = (k.parse().map_err(|_| bad())?, n.parse().map_err(|_| bad())?);
    if n == 0 || k == 0 || k > n {
        return Err(bad());
    }
    Ok((k, n))
}

/// Every Nth item from the Kth: round-robin, so each area of a suite (its
/// cases are registered by area) spreads across the shards.
pub fn take_shard<T>(items: Vec<T>, k: usize, n: usize) -> Vec<T> {
    items.into_iter().enumerate().filter(|(i, _)| i % n == k - 1).map(|(_, item)| item).collect()
}

/// Both runners' check of `--shard` and the filters (`--self-test`).
pub fn self_test_shard() -> Result {
    let shards: Vec<Vec<usize>> = (1..=3).map(|k| take_shard((0..10).collect(), k, 3)).collect();
    ensure!(shards == [vec![0, 3, 6, 9], vec![1, 4, 7], vec![2, 5, 8]], "10 cases in 3 shards: {shards:?}");
    ensure!(parse_shard("2/4") == Ok((2, 4)), "2/4");
    for bad in ["0/4", "5/4", "1/0", "x", "1/", "/4", "-1/4"] {
        ensure!(parse_shard(bad).is_err(), "{bad:?} accepted");
    }
    let args = |text: &str| text.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    let given = args("--list palette --shard 2/3 tabs");
    let selection = Selection::parse(&given)?;
    ensure!(
        selection == Selection { filters: vec!["palette", "tabs"], shard: Some((2, 3)) },
        "filters and shard: {selection:?}"
    );
    ensure!(Selection::parse(&args("--shard")).is_err(), "--shard with no value accepted");
    ensure!(Selection::parse(&args("--shard 4/3")).is_err(), "--shard 4/3 accepted");
    let names = ["palette-a", "tabs-a", "zones-a", "palette-b", "tabs-b"];
    let picked = selection.select(names.to_vec(), |name| name);
    // The filters keep palette-a, tabs-a, palette-b and tabs-b; the 2nd of
    // 3 shards takes the 2nd of those.
    ensure!(picked == ["tabs-a"], "shard 2/3 of the filtered cases: {picked:?}");
    let every = args("--shard 1/2");
    let half = Selection::parse(&every)?.select(names.to_vec(), |name| name);
    ensure!(half == ["palette-a", "zones-a", "tabs-b"], "shard 1/2 of every case: {half:?}");
    Ok(())
}
