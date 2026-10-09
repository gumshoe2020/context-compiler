//! `palace rollup --keep <dir>` (M5b-10 item 3): the attempts a run keeps, as
//! `<date>-<thread>-attempt-<n>.md` (the body) and `.json` (its report, with each claim problem's
//! line and the words its quote lacked). They hold diary text: the directory is 0700, each file
//! 0600. A rerun of the same date and thread replaces that run's files; the 20 newest runs stay.
use crate::rollup_write::Attempt;
use palace_core::rollup_keep::{self, KEEP_RUNS};
use palace_store::Error;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    time::UNIX_EPOCH,
};

/// Keeps what the run should keep; a failure to keep is named on stderr and does not change the
/// run's exit code (the rollup's own outcome stands).
pub fn keep_reporting(dir: &Path, run: &str, attempts: &[Attempt]) {
    match keep(dir, run, attempts) {
        Ok(0) => {}
        Ok(n) => eprintln!("rollup: kept {n} attempt(s) in {}", dir.display()),
        Err(error) => eprintln!("rollup: --keep {}: {error}", dir.display()),
    }
}

fn keep(dir: &Path, run: &str, attempts: &[Attempt]) -> Result<usize, Error> {
    let passed = attempts.last().is_some_and(|a| a.validation.passed());
    let count = rollup_keep::kept(attempts.len(), passed);
    if count == 0 {
        return Ok(0);
    }
    private_dir(dir)?;
    remove_runs(dir, &[run.to_owned()])?;
    for (index, attempt) in attempts[..count].iter().enumerate() {
        let n = index + 1;
        write_private(dir, &rollup_keep::file_name(run, n, "md"), &attempt.body)?;
        let report = serde_json::to_string_pretty(&kept_report(attempt, n))? + "\n";
        write_private(dir, &rollup_keep::file_name(run, n, "json"), &report)?;
    }
    remove_runs(dir, &rollup_keep::stale(&runs(dir)?, run, KEEP_RUNS))?;
    Ok(count)
}

fn kept_report(attempt: &Attempt, n: usize) -> Value {
    let mut report = attempt.report.clone();
    report["attempt"] = n.into();
    report["mode"] = attempt.mode.as_str().into();
    report["repaired"] = attempt.repaired.into();
    report
}

/// The directory, created 0700 (and made 0700 when it exists).
fn private_dir(dir: &Path) -> Result<(), Error> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// `name` in `dir`, written 0600 (confined: a symlink is refused).
fn write_private(dir: &Path, name: &str, text: &str) -> Result<(), Error> {
    let path = crate::paths::database(&dir.join(name))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

/// Every kept run in `dir` with the newest modification time of its files (nanoseconds).
fn runs(dir: &Path) -> Result<Vec<(String, u128)>, Error> {
    let mut runs: Vec<(String, u128)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(run) = rollup_keep::run_of(&name) else {
            continue;
        };
        let at = entry
            .metadata()?
            .modified()?
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        match runs.iter_mut().find(|(r, _)| r == run) {
            Some((_, newest)) => *newest = (*newest).max(at),
            None => runs.push((run.to_owned(), at)),
        }
    }
    Ok(runs)
}

/// Removes every kept file of the named runs.
fn remove_runs(dir: &Path, gone: &[String]) -> Result<(), Error> {
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if rollup_keep::run_of(&name).is_some_and(|run| gone.iter().any(|g| g == run)) {
            fs::remove_file(dir.join(&name))?;
        }
    }
    Ok(())
}
