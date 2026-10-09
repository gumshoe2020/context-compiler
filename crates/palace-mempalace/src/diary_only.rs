//! Make a palace COPY diary-only, as the frozen search answers were captured (MANIFEST
//! `mempalace_search_50`: import drawers and closets deleted first). Import rows share wing
//! `openclaw` with the diary, so no search filter can exclude them. MemPalace's own delete tools
//! append to the fixed `~/.mempalace/wal/write_log.jsonl`, so the deletion goes through the pinned
//! tool's chromadb directly (the interpreter named by the `mempalace` entry point's shebang), with
//! the program on stdin; nothing outside the copy is written.
//!
//! The interpreter runs isolated (`python -I`: no cwd, PYTHONPATH or user site on `sys.path`)
//! from the copy's parent directory, and the program proves `sys.prefix`, `sys.executable` and
//! `chromadb.__file__` belong to the pinned uv tool tree before deleting anything (M4b.2 0d).
//! Errors: `Unavailable` (exit 3) when MemPalace cannot be run or the step times out; any other
//! error (not the pinned interpreter, a broken copy) means a broken run (exit 2).
use crate::{Error, Unavailable};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Rows whose `source_file` contains this marker are imports (MANIFEST `palace_drawers_import`).
pub const IMPORT_MARKER: &str = "/mempalace-import/";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const PROGRAM: &str = include_str!("diary_only.py");
const EXIT_NOT_PINNED: i32 = 10;
const EXIT_BROKEN_COPY: i32 = 11;

/// Deletes import drawers and closets from `palace` (a copy); returns
/// `{mempalace_drawers:{removed, remaining}, mempalace_closets:{removed, remaining}}`.
pub fn strip(mempalace: &Path, palace: &Path, timeout: Duration) -> Result<Value, Error> {
    strip_marked(mempalace, palace, IMPORT_MARKER, timeout)
}

/// Deletes the rows whose `source_file` contains `marker` from `palace` (a COPY). Besides `strip`,
/// only the M4c doctor uses it, to delete one diary entry from its own scratch copy.
pub fn strip_marked(
    mempalace: &Path,
    palace: &Path,
    marker: &str,
    timeout: Duration,
) -> Result<Value, Error> {
    if !palace.is_absolute() {
        return Err("diary-only strip requires an absolute palace path".into());
    }
    let script = locate(mempalace).ok_or_else(|| Unavailable::error("mempalace not found"))?;
    let python = interpreter(&script)?;
    let mut command = Command::new(&python);
    crate::mine::palace_env(&mut command, palace);
    command
        .arg("-I")
        .arg("-")
        .arg(palace)
        .arg(marker)
        .arg(tool_root(&python)?)
        .current_dir(palace.parent().ok_or("palace has no parent")?);
    let (status, stdout, stderr) = run(command, timeout)?;
    classify(status, &stderr)?;
    Ok(serde_json::from_slice(&stdout)?)
}

/// The interpreter in the `mempalace` entry point's `#!` line: one absolute `python*` path.
pub fn interpreter(script: &Path) -> Result<PathBuf, Error> {
    let text = fs::read(script)?;
    let first = text.split(|b| *b == b'\n').next().unwrap_or_default();
    shebang(&String::from_utf8_lossy(first))
}

pub fn shebang(line: &str) -> Result<PathBuf, Error> {
    let path = line
        .strip_prefix("#!")
        .map(str::trim)
        .ok_or("mempalace entry point has no #! line")?;
    let python = Path::new(path)
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("python"));
    if path.contains(char::is_whitespace) || !Path::new(path).is_absolute() || !python {
        return Err(format!("unsupported mempalace interpreter line: {line:?}").into());
    }
    Ok(PathBuf::from(path))
}

/// `<tool>` for `<tool>/bin/python*`, which must be a uv tool tree (`<tool>/uv-receipt.toml`).
pub fn tool_root(python: &Path) -> Result<PathBuf, Error> {
    let bin = python.parent().filter(|b| b.ends_with("bin"));
    let tool = bin
        .and_then(Path::parent)
        .ok_or("interpreter is not in a bin/ dir")?;
    if !tool.join("uv-receipt.toml").is_file() {
        return Err(format!("{} is not the pinned uv tool tree", tool.display()).into());
    }
    Ok(tool.to_owned())
}

/// Not the pinned interpreter or a broken copy → broken run; anything else → unavailable.
fn classify(status: ExitStatus, stderr: &[u8]) -> Result<(), Error> {
    let stderr = String::from_utf8_lossy(stderr);
    let skip = stderr.chars().count().saturating_sub(1024);
    let tail: String = stderr.chars().skip(skip).collect();
    match status.code() {
        Some(0) => Ok(()),
        Some(EXIT_NOT_PINNED | EXIT_BROKEN_COPY) => Err(format!("diary-only strip: {tail}").into()),
        _ => Err(Unavailable::error(format!(
            "diary-only strip failed ({status}): {tail}"
        ))),
    }
}

/// Spawns `command` with the program on stdin and waits up to `timeout` (then kills it).
fn run(mut command: Command, timeout: Duration) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), Error> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Unavailable::error(format!("cannot run the pinned interpreter: {e}")))?;
    // A child that dies before reading stdin (EPIPE) is still waited for and classified.
    let _ = child
        .stdin
        .take()
        .ok_or("missing stdin")?
        .write_all(PROGRAM.as_bytes());
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = wait(&mut child, timeout)?;
    let joined = |h: thread::JoinHandle<Vec<u8>>| h.join().map_err(|_| "reader panicked");
    Ok((status, joined(stdout)?, joined(stderr)?))
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

fn wait(child: &mut Child, timeout: Duration) -> Result<ExitStatus, Error> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Unavailable::error(format!(
                "diary-only strip timed out after {}s",
                timeout.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn locate(executable: &Path) -> Option<PathBuf> {
    if executable.components().count() > 1 {
        return executable.is_file().then(|| executable.to_owned());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(executable))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shebang_must_be_one_absolute_python() {
        assert_eq!(
            shebang("#!/opt/uv/mempalace/bin/python3").unwrap(),
            PathBuf::from("/opt/uv/mempalace/bin/python3")
        );
        assert!(shebang("#!/usr/bin/env python3").is_err());
        assert!(shebang("#!python3").is_err());
        assert!(shebang("#!/bin/sh").is_err(), "a trampoline is not python");
        assert!(shebang("import sys").is_err());
    }
    #[test]
    fn program_checks_the_interpreter_before_deleting_marked_rows() {
        assert!(PROGRAM.contains("mempalace_drawers") && PROGRAM.contains("mempalace_closets"));
        let check = PROGRAM.find("sys.flags.isolated").unwrap();
        let chromadb = PROGRAM.find("chromadb.__file__").unwrap();
        let delete = PROGRAM.find("delete(ids=").unwrap();
        assert!(check < delete && chromadb < delete);
        assert!(!PROGRAM.contains("import mempalace") && !PROGRAM.contains("from mempalace"));
    }
    #[test]
    fn tool_root_needs_a_uv_receipt() {
        let dir = std::env::temp_dir().join(format!("diary-only-tool-{}", std::process::id()));
        fs::create_dir_all(dir.join("bin")).unwrap();
        let python = dir.join("bin/python3");
        assert!(tool_root(&python).is_err());
        fs::write(dir.join("uv-receipt.toml"), "[tool]\n").unwrap();
        assert_eq!(tool_root(&python).unwrap(), dir);
        assert!(tool_root(Path::new("/opt/python3")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    /// A fake uv tool tree whose `python3` runs `body` (sh); returns the `mempalace` entry point.
    fn fake_tool(name: &str, body: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("diary-only-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("tool/bin")).unwrap();
        fs::create_dir_all(dir.join("copy/palace")).unwrap();
        fs::write(dir.join("tool/uv-receipt.toml"), "[tool]\n").unwrap();
        let python = dir.join("tool/bin/python3");
        fs::write(&python, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let entry = dir.join("tool/bin/mempalace");
        fs::write(&entry, format!("#!{}\n", python.display())).unwrap();
        (dir, entry)
    }
    fn strip_with(name: &str, body: &str, timeout: Duration) -> (PathBuf, Result<Value, Error>) {
        let (dir, entry) = fake_tool(name, body);
        let palace = dir.join("copy/palace");
        (dir, strip(&entry, &palace, timeout))
    }
    #[test]
    fn runs_isolated_from_the_copys_parent_and_parses_the_report() {
        let (dir, result) = strip_with(
            "ok",
            "cat >/dev/null; echo \"$1 $(pwd)\" >&2; [ \"$1\" = -I ] || exit 1; echo '{\"mempalace_drawers\":{}}'",
            DEFAULT_TIMEOUT,
        );
        assert_eq!(result.unwrap()["mempalace_drawers"], serde_json::json!({}));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn not_pinned_or_broken_copy_is_a_broken_run_other_failures_are_unavailable() {
        for (name, body, unavailable) in [
            ("notpinned", "cat >/dev/null; exit 10", false),
            ("brokencopy", "exit 11", false), // exits before reading stdin (EPIPE)
            ("crash", "cat >/dev/null; exit 1", true),
            ("slow", "sleep 5", true),
        ] {
            let (dir, result) = strip_with(name, body, Duration::from_secs(1));
            let error = result.expect_err(name);
            assert_eq!(Unavailable::is(&error), unavailable, "{name}: {error}");
            fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn a_non_python_entry_point_is_refused_before_running() {
        let (dir, entry) = fake_tool("shell", "exit 0");
        fs::write(&entry, "#!/bin/sh\n").unwrap();
        let error = strip(&entry, &dir.join("copy/palace"), DEFAULT_TIMEOUT).unwrap_err();
        assert!(!Unavailable::is(&error) && error.to_string().contains("interpreter"));
        fs::remove_dir_all(dir).unwrap();
    }
}
