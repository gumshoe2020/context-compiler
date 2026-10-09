use crate::Error;
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
const MAX_LINE: usize = 4 * 1024 * 1024;
pub struct Client {
    child: Child,
    input: ChildStdin,
    output: Receiver<Result<String, String>>,
    next_id: u64,
    failure: Option<String>,
    server_version: Option<String>,
    /// Every request ends by this instant (`spawn_until`).
    deadline: Option<Instant>,
    /// The child leads its own process group, killed whole when the client ends it.
    grouped: bool,
    ended: bool,
}
impl Client {
    /// Spawns a MemPalace MCP child for `palace` (sets MEMPALACE_PALACE_PATH, see `mine`).
    pub fn spawn(executable: &Path, args: &[&OsStr], palace: &Path) -> Result<Self, Error> {
        Self::start(Command::new(executable), args, palace)
    }
    /// `spawn`, with the child in a process group of its own and every request (the handshake
    /// included) ending by `deadline`; a failure, a missed deadline or the drop kills the whole
    /// group, so no descendant of the server outlives the client (M5c.1 item 3).
    pub fn spawn_until(
        executable: &Path,
        args: &[&OsStr],
        palace: &Path,
        deadline: Instant,
    ) -> Result<Self, Error> {
        let mut command = Command::new(executable);
        command.process_group(0);
        let mut client = Self::start(command, args, palace)?;
        client.grouped = true;
        client.deadline = Some(deadline);
        Ok(client)
    }
    fn start(mut command: Command, args: &[&OsStr], palace: &Path) -> Result<Self, Error> {
        crate::mine::palace_env(&mut command, palace);
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().ok_or("missing stdin")?;
        let output = child.stdout.take().ok_or("missing stdout")?;
        Ok(Self {
            child,
            input,
            output: lines(output),
            next_id: 0,
            failure: None,
            server_version: None,
            deadline: None,
            grouped: false,
            ended: false,
        })
    }
    pub fn initialize(&mut self, required_tool: &str, allow_version: bool) -> Result<(), Error> {
        let pin = crate::version::pin()?;
        self.handshake(required_tool, |result| {
            check_server_version(result, &pin, allow_version)
        })
    }
    /// The handshake without the version gate, for read-only callers that need no pin script
    /// (`palace context`, M5c item 2): the server's version is recorded, never checked.
    pub fn initialize_unpinned(&mut self, required_tool: &str) -> Result<(), Error> {
        self.handshake(required_tool, |_| Ok(()))
    }
    fn handshake(
        &mut self,
        required_tool: &str,
        check: impl FnOnce(&Value) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = self.request_until("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"context-compiler","version":env!("CARGO_PKG_VERSION")}}), deadline)?;
        if result["protocolVersion"] != "2024-11-05" {
            return Err("unsupported MCP protocol".into());
        }
        check(&result)?;
        self.server_version = result["serverInfo"]["version"].as_str().map(str::to_owned);
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}))?;
        let tools = self.request_until("tools/list", json!({}), deadline)?;
        if !tools["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|t| t["name"] == required_tool))
        {
            return Err("required MCP tool unavailable".into());
        }
        Ok(())
    }
    fn send(&mut self, value: &Value) -> Result<(), Error> {
        if let Some(failure) = &self.failure {
            return Err(format!("MCP connection is closed after failure: {failure}").into());
        }
        writeln!(self.input, "{value}")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        self.request_until(method, params, Instant::now() + Duration::from_secs(20))
    }
    fn request_until(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, Error> {
        let deadline = self.deadline.map_or(deadline, |d| d.min(deadline));
        self.next_id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":self.next_id,"method":method,"params":params}))?;
        let result = self.receive(deadline);
        if let Err(error) = &result {
            self.failure.get_or_insert_with(|| error.to_string());
            self.end();
        }
        result
    }
    fn receive(&self, deadline: Instant) -> Result<Value, Error> {
        loop {
            let line = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
            let response: Value = serde_json::from_str(&line)?;
            if response["jsonrpc"] != "2.0" {
                return Err("invalid JSON-RPC response".into());
            }
            if response.get("id").is_none() && response["method"].is_string() {
                continue;
            }
            if response["id"] != self.next_id {
                return Err("MCP response id mismatch".into());
            }
            if let Some(error) = response.get("error") {
                return Err(format!("MCP error: {error}").into());
            }
            return response
                .get("result")
                .cloned()
                .ok_or("missing MCP result".into());
        }
    }
}
/// The child's stdout as a channel of lines, read on a thread of its own until EOF or an error.
fn lines(output: std::process::ChildStdout) -> Receiver<Result<String, String>> {
    let (send, receive) = mpsc::sync_channel(16);
    thread::spawn(move || {
        let mut reader = BufReader::new(output);
        loop {
            let line = read_line(&mut reader);
            let done = line.is_err();
            if send.send(line).is_err() || done {
                break;
            }
        }
    });
    receive
}
fn read_line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|e| e.to_string())?;
        if available.is_empty() {
            return Err("MCP child closed stdout".into());
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if bytes.len() + count > MAX_LINE {
            return Err("MCP response exceeds 4 MiB".into());
        }
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if bytes.last() == Some(&b'\n') {
            return String::from_utf8(bytes).map_err(|e| e.to_string());
        }
    }
}
impl Client {
    /// The first transport failure (timeout, EOF, protocol error); the connection is unusable after it.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
    /// `initialize.serverInfo.version`, once the handshake has succeeded.
    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }
}
impl Client {
    /// Kills the child (and, when grouped, its whole process group) and reaps it, once.
    fn end(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        if self.grouped {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.end();
    }
}

pub fn check_server_version(result: &Value, pin: &str, allow: bool) -> Result<(), Error> {
    let version = result["serverInfo"]["version"]
        .as_str()
        .ok_or("missing MCP server version")?;
    if version != pin {
        if !allow {
            return Err(format!("MemPalace MCP version {version}; required {pin}").into());
        }
        eprintln!("warning: allowing MemPalace MCP version {version}; pin is {pin}");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn server_pin() {
        for (version, allowed, ok) in [
            ("3.5.0", false, true),
            ("3.5.1", false, false),
            ("3.5.1", true, true),
        ] {
            assert_eq!(
                super::check_server_version(
                    &serde_json::json!({"serverInfo":{"version":version}}),
                    "3.5.0",
                    allowed
                )
                .is_ok(),
                ok
            );
        }
        assert!(super::check_server_version(&serde_json::json!({}), "3.5.0", true).is_err());
    }
}
