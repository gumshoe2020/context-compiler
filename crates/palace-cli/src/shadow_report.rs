//! `palace shadow-report --night <dir> --python-reports <dir> [--relations-dir <dir>] --date <d>
//! --old-run <YYYY-MM-DDTHH:MM:SSZ>` (M5a item 1, M5b-1 item 3): the comparison step of
//! `scripts/palace-shadow.sh`. Reads what the script's steps left in the night directory
//! (`steps.tsv`, `steps/*.out`, `watch-reports/`), compares old and new (`palace_core::shadow`;
//! only a Python report stamped at or after `--old-run` is compared), writes `REPORT.md` and
//! `report.json` there and prints one line. Exit 0 agree or quiet, 1 disagree or uncompared,
//! 3 a step was unavailable (retry later), 2 broken. A line the reader no longer takes (a closed
//! pipe) is lost, not an error: the exit code stands (M5c.3 item 1). Never sends.
use clap::Args;
use palace_core::shadow::{self, Disagreement, Night, Python, Step};
use palace_store::Error;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Args)]
pub struct Options {
    /// `<shadow-root>/<date>`, as the script left it.
    #[arg(long)]
    night: PathBuf,
    /// The Python watcher's report directory (its newest report is compared).
    #[arg(long)]
    python_reports: PathBuf,
    /// Relation files the old path applied, each with `<name>.result.json` beside it.
    #[arg(long)]
    relations_dir: Option<PathBuf>,
    #[arg(long)]
    date: String,
    /// When tonight's old-path run began (UTC, `YYYY-MM-DDTHH:MM:SSZ`): an older Python report is
    /// stale, and a rollup dated on or after that day is fresh.
    #[arg(long)]
    old_run: String,
}

pub fn run(o: Options) -> i32 {
    match report(&o) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("palace shadow-report: {e}");
            shadow::EXIT_BROKEN
        }
    }
}

fn report(o: &Options) -> Result<i32, Error> {
    if !old_run_stamp(&o.old_run) {
        return Err(format!("--old-run {}: expected YYYY-MM-DDTHH:MM:SSZ", o.old_run).into());
    }
    if !shadow::old_run_on_night(&o.date, &o.old_run) {
        return Err(format!(
            "--old-run {} must fall on --date {} or the day before",
            o.old_run, o.date
        )
        .into());
    }
    let now = crate::watch::utc_name(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());
    if !shadow::old_run_not_future(&o.old_run, &now) {
        return Err(format!("--old-run {} is later than now ({now})", o.old_run).into());
    }
    let night = crate::paths::input(&o.night)?;
    let steps = steps(&night)?;
    let (python_path, future) = newest_stamped(&crate::paths::input(&o.python_reports)?)?;
    let python_name = name(python_path.as_deref());
    let python_report = json_file(python_path.as_deref())?;
    let python = python_side(&python_name, &python_report, &o.old_run);
    let watch = newest(&night.join("watch-reports"))?;
    let watch_report = json_file(watch.as_deref())?;
    let fidelity = shadow::fidelity(python.compared(), &watch_report);
    let coverage = shadow::coverage(&python, &watch_report, &fidelity, &o.old_run);
    let (disagreements, relations, diff) = disagreements(o, &night, &steps, &fidelity)?;
    let (asks, digest) = (lines(&night, "unsent"), lines(&night, "digest"));
    let summary = Night {
        date: &o.date,
        steps: &steps,
        fidelity: &fidelity,
        disagreements: &disagreements,
        asks: &asks,
        digest: &digest,
        coverage: &coverage,
    };
    let counts = json!({"rollups":fidelity.rollups.len(),"agree":fidelity.agree(),"disagreements":disagreements.len(),
        "not_compared":coverage.not_compared.len(),"asks":asks.len(),"digest_lines":digest.len(),"relations":relations,
        "graph_rows":diff["rows"],"graph_unkeyed":diff["unkeyed"]});
    let sources = json!({"python_report":python_name,"python_report_state":python.state(),
        "old_run":o.old_run,"watch_report":name(watch.as_deref()),"future_reports":future});
    write(
        &night,
        "report.json",
        &serde_json::to_string_pretty(&summary.json(counts, sources))?,
    )?;
    write(&night, "REPORT.md", &summary.markdown())?;
    let _ = writeln!(io::stdout().lock(), "{}", summary.line());
    Ok(summary.exit())
}

/// Fidelity, then relation, then graph disagreements; the relation counts; the graph diff.
fn disagreements(
    o: &Options,
    night: &Path,
    steps: &[Step],
    fidelity: &shadow::Fidelity,
) -> Result<(Vec<Disagreement>, Value, Value), Error> {
    let mut all = fidelity.disagreements.clone();
    let (relations, counts) = relation_disagreements(o, night, steps)?;
    all.extend(relations);
    let diff = json_file(Some(&night.join("steps/shadow-diff.out")))?;
    all.extend(shadow::graph(&diff));
    Ok((all, counts, diff))
}

/// `steps.tsv`: one `<name>\t<exit>` per step, in run order.
fn steps(night: &Path) -> Result<Vec<Step>, Error> {
    let text = fs::read_to_string(night.join("steps.tsv"))
        .map_err(|e| format!("{}/steps.tsv: {e}", night.display()))?;
    text.lines()
        .map(|line| {
            let (name, exit) = line.split_once('\t').ok_or("steps.tsv: malformed line")?;
            Ok(Step {
                name: name.to_owned(),
                exit: exit.trim().parse()?,
            })
        })
        .collect()
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn old_run_stamp(stamp: &str) -> bool {
    let compact: String = stamp.chars().filter(|c| !matches!(c, '-' | ':')).collect();
    stamp.len() == 20
        && stamp
            .get(..10)
            .is_some_and(palace_core::rollup_prep::valid_date)
        && shadow::report_stamp(&compact)
}

/// The Python side: missing, stale, or fresh with its report.
fn python_side<'a>(name: &'a Value, report: &'a Value, old_run: &str) -> Python<'a> {
    match name.as_str() {
        None => Python::Missing,
        Some(n) if shadow::fresh_report(n, old_run) => Python::Fresh(n, report),
        Some(n) => Python::Stale(n),
    }
}

/// The newest `<YYYYMMDD>T<HHMMSS>Z.json` report in `directory` stamped no later than now (other
/// files are ignored), and the names of those stamped in the future (ignored, M5b-2 item 6).
fn newest_stamped(directory: &Path) -> Result<(Option<PathBuf>, Vec<String>), Error> {
    let entries = fs::read_dir(directory)
        .map_err(|e| format!("--python-reports {}: {e}", directory.display()))?;
    let now = crate::watch::utc_name(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());
    let stamp = |p: &PathBuf| -> Option<String> {
        let name = p.file_name()?.to_str()?;
        name.strip_suffix(".json")
            .filter(|s| shadow::report_stamp(s))
            .map(str::to_owned)
    };
    let (mut reports, mut future) = (Vec::new(), Vec::new());
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        match stamp(&path) {
            Some(s) if s > now => future.push(format!("{s}.json")),
            Some(_) => reports.push(path),
            None => {}
        }
    }
    reports.sort();
    future.sort();
    Ok((reports.pop(), future))
}

/// The newest `*.json` report in `directory` by name (UTC-stamped names sort by time).
fn newest(directory: &Path) -> Result<Option<PathBuf>, Error> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(None);
    };
    let mut reports: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    reports.sort();
    Ok(reports.pop())
}

fn json_file(path: Option<&Path>) -> Result<Value, Error> {
    match path.map(fs::read_to_string) {
        Some(Ok(text)) if !text.trim().is_empty() => Ok(serde_json::from_str(&text)
            .map_err(|e| format!("{}: {e}", path.unwrap_or(Path::new("?")).display()))?),
        _ => Ok(Value::Null),
    }
}

/// A JSON array, or one JSON value per line.
fn json_values(text: &str) -> Result<Vec<Value>, Error> {
    if let Ok(Value::Array(values)) = serde_json::from_str(text) {
        return Ok(values);
    }
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(Into::into))
        .collect()
}

/// A step's non-empty stdout lines (none when the step left no output).
fn lines(night: &Path, step: &str) -> Vec<String> {
    let text = fs::read_to_string(night.join(format!("steps/{step}.out"))).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// Each `relations:<file>` step's dry-run verdicts against `<name>.result.json` beside the file;
/// a file without a recorded result is counted, not compared.
fn relation_disagreements(
    o: &Options,
    night: &Path,
    steps: &[Step],
) -> Result<(Vec<Disagreement>, Value), Error> {
    let (mut found, mut compared, mut unrecorded) = (Vec::new(), 0, 0);
    for file in steps
        .iter()
        .filter_map(|s| s.name.strip_prefix("relations:"))
    {
        let stem = file.strip_suffix(".json").unwrap_or(file);
        let recorded = o
            .relations_dir
            .as_ref()
            .map(|d| d.join(format!("{stem}.result.json")))
            .filter(|p| p.is_file());
        let Some(recorded) = recorded else {
            unrecorded += 1;
            continue;
        };
        let old = json_values(&fs::read_to_string(crate::paths::input(&recorded)?)?)?;
        let new_text = fs::read_to_string(night.join(format!("steps/relations-{stem}.out")))
            .unwrap_or_default();
        found.extend(shadow::relations(file, &old, &json_values(&new_text)?));
        compared += 1;
    }
    Ok((found, json!({"compared":compared,"unrecorded":unrecorded})))
}

fn name(path: Option<&Path>) -> Value {
    path.and_then(Path::file_name)
        .map_or(Value::Null, |n| json!(n.to_string_lossy()))
}

fn write(night: &Path, file: &str, text: &str) -> Result<(), Error> {
    fs::write(crate::paths::database(&night.join(file))?, text)?;
    Ok(())
}
