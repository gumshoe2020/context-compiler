//! M5b-10 item 3: `palace rollup --keep <dir>` — which attempts of a run are kept, their names
//! (`<date>-<thread>-attempt-<n>.md` and `.json`), and which older runs are pruned; M5b-11 item 4:
//! where they may not be kept. Pure.
use std::path::Path;

/// At most this many runs are kept (the newest).
pub const KEEP_RUNS: usize = 20;
const ATTEMPT: &str = "-attempt-";

/// `<as-of>-<thread>`, the thread made safe for a file name (anything but ASCII letters, digits,
/// `-`, `_` and `.` becomes `-`).
pub fn run_key(as_of: &str, thread: &str) -> String {
    let safe: String = thread
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect();
    format!("{as_of}-{safe}")
}

pub fn file_name(run: &str, attempt: usize, extension: &str) -> String {
    format!("{run}{ATTEMPT}{attempt}.{extension}")
}

/// The run of a kept file (`<YYYY-MM-DD>-<thread>-attempt-<n>.md|json`, the thread as `run_key`
/// makes it); `None` for any other file, which pruning never removes (M5b-11 item 4).
pub fn run_of(file: &str) -> Option<&str> {
    let stem = file
        .strip_suffix(".md")
        .or_else(|| file.strip_suffix(".json"))?;
    let (run, attempt) = stem.rsplit_once(ATTEMPT)?;
    let numbered = !attempt.is_empty() && attempt.bytes().all(|b| b.is_ascii_digit());
    let thread = run
        .get(10..)
        .and_then(|t| t.strip_prefix('-'))
        .unwrap_or_default();
    let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    let dated = crate::rollup_prep::entry_date(run).is_some();
    (numbered && dated && !thread.is_empty() && thread.chars().all(safe)).then_some(run)
}

/// Whether `path` is `store` or lies inside it; both canonical (symbolic links resolved by the
/// caller). Kept attempts there would be read or mined as memory (M5b-11 item 4).
pub fn inside(path: &Path, store: &Path) -> bool {
    path.starts_with(store)
}

/// How many of a run's attempts (from the first) are kept: all when the last failed, else the
/// failed ones before it.
pub fn kept(attempts: usize, passed: bool) -> usize {
    if passed {
        attempts.saturating_sub(1)
    } else {
        attempts
    }
}

/// The runs to remove: all but the `keep` newest (by time, then name), `current` always kept.
pub fn stale(runs: &[(String, u128)], current: &str, keep: usize) -> Vec<String> {
    let mut others: Vec<&(String, u128)> = runs.iter().filter(|(r, _)| r != current).collect();
    others.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    others
        .into_iter()
        .skip(keep.saturating_sub(1))
        .map(|(r, _)| r.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_runs() {
        let run = run_key("2026-09-29", "m5 cutover / plan");
        assert_eq!(run, "2026-09-29-m5-cutover---plan");
        assert_eq!(file_name(&run, 2, "json"), format!("{run}-attempt-2.json"));
        assert_eq!(run_of(&file_name(&run, 12, "md")), Some(run.as_str()));
        for other in [
            "notes.txt",
            "x-attempt-.md",
            "-attempt-1.md",
            "x-attempt-1a.json",
        ] {
            assert_eq!(run_of(other), None, "{other}");
        }
    }
    #[test]
    fn only_files_of_the_commands_form_are_runs() {
        assert_eq!(
            run_of("2026-09-22-x-attempt-2-attempt-1.md"),
            Some("2026-09-22-x-attempt-2")
        );
        for other in [
            "my-notes-attempt-1.md",
            "2026-09-22-attempt-1.md",
            "2026-9-22-thread-attempt-1.md",
            "2026-09-22-my notes-attempt-1.md",
            "2026-09-22thread-attempt-1.json",
        ] {
            assert_eq!(run_of(other), None, "{other}");
        }
    }
    #[test]
    fn a_path_equal_to_or_under_a_store_is_inside() {
        let store = Path::new("/copy/diary-root");
        assert!(inside(store, store));
        assert!(inside(Path::new("/copy/diary-root/state/kept"), store));
        assert!(!inside(Path::new("/copy/diary-rootx/kept"), store));
        assert!(!inside(Path::new("/copy/kept"), store));
    }
    #[test]
    fn failed_runs_keep_every_attempt_and_passed_runs_the_failed_ones() {
        assert_eq!(kept(3, false), 3);
        assert_eq!(kept(2, true), 1);
        assert_eq!(kept(1, true), 0);
    }
    #[test]
    fn only_the_newest_runs_stay() {
        let runs: Vec<(String, u128)> = (1..=22).map(|n| (format!("r{n:02}"), n as u128)).collect();
        let gone = stale(&runs, "r01", 20);
        assert_eq!(gone, ["r03", "r02"]);
        assert!(stale(&runs[..5], "r05", 20).is_empty());
    }
}
