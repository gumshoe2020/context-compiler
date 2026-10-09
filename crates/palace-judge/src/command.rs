use crate::{Error, parse_response, prompt};
use palace_core::{canary::Question, traits::Judge};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
const OUTPUT_CAP: u64 = 1024 * 1024;
const STDERR_KEPT: u64 = 2048;
/// Default deadline for select and judge calls (M4b.1 S-6).
pub const MODEL_TIMEOUT: Duration = Duration::from_secs(120);
/// Default deadline for the rollup write call (M4b.1 S-6): up to 60k chars in, 40k out.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(600);
/// Text-in/text-out transport: prompt on stdin, answer on stdout, bounded by a deadline that
/// kills only the direct child.
pub struct CommandModel {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
    /// Run each call from a fresh empty directory under TMPDIR, removed after the call.
    pub fresh_cwd: bool,
}
impl CommandModel {
    /// `claude -p` run clean (P5): no user/local settings or hooks, no MCP servers, no tools,
    /// from an empty working directory, so the palace-context hook never sees the prompt.
    pub fn claude(executable: &Path, model: &str, settings: &Path) -> Result<Self, Error> {
        Ok(Self {
            executable: executable.to_owned(),
            args: clean_claude_args(model, settings),
            timeout: MODEL_TIMEOUT,
            cwd: None,
            fresh_cwd: true,
        })
    }
    pub fn generic(executable: &str) -> Self {
        Self {
            executable: executable.into(),
            args: vec![],
            timeout: MODEL_TIMEOUT,
            cwd: None,
            fresh_cwd: false,
        }
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn run(&self, text: &str) -> Result<String, Error> {
        if !self.fresh_cwd {
            return self.run_in(text, self.cwd.as_deref());
        }
        let dir = empty_directory()?;
        let result = self.run_in(text, Some(&dir));
        let removed = std::fs::remove_dir_all(&dir);
        let output = result?;
        removed.map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
        Ok(output)
    }
    fn run_in(&self, text: &str, cwd: Option<&Path>) -> Result<String, Error> {
        let mut command = Command::new(&self.executable);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", self.executable.display()))?;
        let streams = Streams::attach(&mut child, text.to_owned())?;
        let result = self.wait(&mut child, &streams);
        if result.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        result
    }
    fn wait(&self, child: &mut Child, streams: &Streams) -> Result<String, Error> {
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                return Err("model command timed out".into());
            }
            thread::sleep(Duration::from_millis(10));
        };
        let remaining = || deadline.saturating_duration_since(Instant::now());
        if !status.success() {
            let stderr = streams.stderr.recv_timeout(remaining()).unwrap_or_default();
            return Err(format!("model command exited {status}: {}", stderr.trim()).into());
        }
        streams
            .input
            .recv_timeout(remaining())
            .map_err(|_| "model command timed out")??;
        let raw = streams
            .output
            .recv_timeout(remaining())
            .map_err(|_| "model command timed out")??;
        if raw.len() as u64 > OUTPUT_CAP {
            return Err("model output exceeds 1 MiB".into());
        }
        Ok(String::from_utf8(raw)?)
    }
}
struct Streams {
    input: Receiver<std::io::Result<()>>,
    output: Receiver<std::io::Result<Vec<u8>>>,
    stderr: Receiver<String>,
}
impl Streams {
    fn attach(child: &mut Child, text: String) -> Result<Self, Error> {
        let mut stdin = child.stdin.take().ok_or("model stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("model stdout unavailable")?;
        let stderr = child.stderr.take().ok_or("model stderr unavailable")?;
        let (input_tx, input) = mpsc::channel();
        thread::spawn(move || {
            let result = stdin.write_all(text.as_bytes());
            drop(stdin);
            let _ = input_tx.send(result);
        });
        let (output_tx, output) = mpsc::channel();
        thread::spawn(move || {
            let mut raw = Vec::new();
            let result = stdout
                .take(OUTPUT_CAP + 1)
                .read_to_end(&mut raw)
                .map(|_| raw);
            let _ = output_tx.send(result);
        });
        let (stderr_tx, stderr_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut kept = Vec::new();
            let mut stream = stderr;
            let _ = stream.by_ref().take(STDERR_KEPT).read_to_end(&mut kept);
            let _ = std::io::copy(&mut stream, &mut std::io::sink());
            let _ = stderr_tx.send(String::from_utf8_lossy(&kept).into_owned());
        });
        Ok(Self {
            input,
            output,
            stderr: stderr_rx,
        })
    }
}
pub fn clean_claude_args(model: &str, settings: &Path) -> Vec<String> {
    [
        "-p",
        "--model",
        model,
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "project",
        "--settings",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .chain([settings.display().to_string()])
    .collect()
}
/// A fresh, empty directory under the temp root: no project settings or CLAUDE.md to discover.
/// One per call; `run` removes it afterwards (M4b.1 S-6, review L5).
fn empty_directory() -> Result<PathBuf, Error> {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "palace-judge-{}-{nanos}-{call}",
        std::process::id()
    ));
    std::fs::create_dir(&path)?;
    Ok(path)
}
/// Resolve `claude` to an absolute path: PATH first, then ~/.local/bin (cron PATHs often lack it).
pub fn resolve_executable(name: &str) -> Result<PathBuf, Error> {
    if name.contains('/') {
        return Ok(PathBuf::from(name));
    }
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin"));
    }
    candidates
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("{name} executable not found on PATH or ~/.local/bin").into())
}
impl palace_core::traits::Model for CommandModel {
    type Error = Error;
    fn complete(&self, prompt: &str) -> Result<String, Error> {
        self.run(prompt)
    }
}
/// Verdict adapter over the transport: canary/claim questions in, `{verdict, quote}` out.
pub struct CommandJudge(pub CommandModel);
impl CommandJudge {
    pub fn claude(model: &str, settings: &Path) -> Result<Self, Error> {
        Ok(Self(CommandModel::claude(
            &resolve_executable("claude")?,
            model,
            settings,
        )?))
    }
    pub fn generic(executable: &str) -> Self {
        Self(CommandModel::generic(executable))
    }
}
impl Judge for CommandJudge {
    type Error = Error;
    fn judge(&self, q: &Question) -> Result<Value, Error> {
        Ok(parse_response(&self.0.run(&prompt(q))?))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_args_disable_user_settings_mcp_and_tools() {
        let args = clean_claude_args("sonnet", Path::new("/p/judge-settings.json"));
        assert_eq!(
            args,
            [
                "-p",
                "--model",
                "sonnet",
                "--tools",
                "",
                "--strict-mcp-config",
                "--setting-sources",
                "project",
                "--settings",
                "/p/judge-settings.json"
            ]
        );
    }
    #[test]
    fn fresh_cwd_is_empty_during_the_call_and_removed_after() {
        let model = CommandModel {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), "cat >/dev/null; pwd; ls -A | wc -l".into()],
            timeout: MODEL_TIMEOUT,
            cwd: None,
            fresh_cwd: true,
        };
        let out = model.run("prompt").unwrap();
        let mut lines = out.lines();
        let dir = PathBuf::from(lines.next().unwrap());
        assert_eq!(lines.next().unwrap().trim(), "0");
        assert!(!dir.exists(), "{} leaked", dir.display());
    }
    #[test]
    fn deadlines_default_and_override() {
        let model = CommandModel::generic("x");
        assert_eq!(model.timeout, Duration::from_secs(120));
        assert_eq!(
            model.with_timeout(WRITE_TIMEOUT).timeout,
            Duration::from_secs(600)
        );
    }
    #[test]
    #[ignore = "live judge; Fergus runs explicitly"]
    fn live_directional_judge() {
        let settings =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/judge-settings.json");
        let judge = CommandJudge::claude("sonnet", &settings).unwrap();
        for (passage, expected) in [
            (
                "Callum corrected Fergus about beekeeping; Fergus conceded.",
                "preserved",
            ),
            (
                "Fergus corrected Callum about beekeeping; Callum conceded.",
                "reversed",
            ),
        ] {
            let q = Question {
                rollup: "live-test".into(),
                id: "direction".into(),
                statement: "Callum corrected Fergus on beekeeping, not the reverse.".into(),
                passage: passage.into(),
            };
            assert_eq!(judge.judge(&q).unwrap()["verdict"], expected);
        }
    }
}
