//! A child process with a deadline, for the tools that run a program inside an interactive turn
//! (`palace_reply`, PKG-1.3 item 5; `palace_diary_write`'s mine, item 6). The child leads a
//! process group of its own; at the deadline the whole group is killed, as `palace context` ends
//! MemPalace (review S4), so no descendant outlives the call and the server answers in time.
use palace_store::Error;
use std::{
    io::Read,
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// What a finished (or ended) child left: its exit status, `None` when the deadline ended it.
pub struct Finished {
    pub status: Option<ExitStatus>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `command` (stdin closed, output captured) and waits at most `limit`.
pub fn run(mut command: Command, limit: Duration) -> Result<Finished, Error> {
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = wait(&mut child, Instant::now() + limit)?;
    let grace = Duration::from_secs(1);
    Ok(Finished {
        status,
        stdout: stdout.recv_timeout(grace).unwrap_or_default(),
        stderr: stderr.recv_timeout(grace).unwrap_or_default(),
    })
}

fn wait(child: &mut Child, deadline: Instant) -> Result<Option<ExitStatus>, Error> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            kill_group(child);
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Kills the child's whole process group, then the child, and reaps it.
fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// The pipe's text, read on a thread of its own so that a full pipe never blocks the child.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut text);
        }
        let _ = send.send(String::from_utf8_lossy(&text).into_owned());
    });
    receive
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }
    #[test]
    fn a_child_that_finishes_gives_its_output() {
        let done = run(
            sh("echo out; echo err >&2; exit 3"),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(done.status.and_then(|s| s.code()), Some(3));
        assert_eq!(
            (done.stdout.as_str(), done.stderr.as_str()),
            ("out\n", "err\n")
        );
    }
    #[test]
    fn the_deadline_ends_the_child_and_its_descendants() {
        let started = Instant::now();
        let done = run(
            sh("sleep 30 & echo started; wait"),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(done.status.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(done.stdout, "started\n");
    }
}
