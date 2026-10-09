//! PKG-1.3 item 5: `palace_reply` through the binary, with a stand-in `palace` that records its
//! argv: the reply is the corrections path's `palace improve --reply … --token … --by <person>`
//! on this memory, under the names palace-mcp was started with. Scratch directories under target/.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

struct Scratch {
    dir: PathBuf,
}
impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let dir = root.join(format!("target/mcp-reply-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("diary-root/diary")).unwrap();
        fs::create_dir_all(dir.join("palace")).unwrap();
        fs::create_dir_all(dir.join("uv")).unwrap();
        Self { dir }
    }
    /// A stand-in palace: logs its arguments one per line, then prints `out` on stdout (exit 0)
    /// or on stderr (any other exit).
    fn palace(&self, code: i32, out: &str) -> PathBuf {
        let path = self.dir.join("palace-bin");
        let log = self.dir.join("argv.log");
        let stream = if code == 0 { "" } else { " >&2" };
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >'{}'\necho '{out}'{stream}\nexit {code}\n",
            log.display()
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    fn path(&self, name: &str) -> String {
        self.dir.join(name).display().to_string()
    }
    fn args(&self, palace_bin: &Path) -> Vec<String> {
        let mut args = vec![
            "--kg".to_owned(),
            self.path("palace/knowledge_graph.sqlite3"),
        ];
        for (flag, value) in [
            ("--palace", self.path("palace")),
            ("--diary-root", self.path("diary-root")),
            ("--mempalace", self.path("uv/mempalace")),
            ("--mempalace-check", self.path("check.sh")),
            ("--person", "maria".to_owned()),
            ("--assistant", "codex".to_owned()),
            ("--palace-bin", palace_bin.display().to_string()),
        ] {
            args.extend([flag.to_owned(), value]);
        }
        args
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Sends each request line; returns the response lines.
fn session(args: &[String], requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_palace-mcp"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn reply(token: &str, text: &str) -> Value {
    json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"palace_reply","arguments":{"token":token,"reply":text}}})
}

fn result(response: &Value) -> Value {
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn a_reply_runs_the_corrections_path_as_the_person() {
    let s = Scratch::new("applied");
    let palace = s.palace(0, "IMPROVE drop applied reply=#f_1a2b3c4d");
    let list = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
    let responses = session(&s.args(&palace), &[list, reply("#f_1a2b3c4d", "drop")]);
    let tools: Vec<&str> = responses[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools.last(), Some(&"palace_reply"), "{tools:?}");
    let done = result(&responses[1]);
    assert_eq!(done["applied"], true, "{done}");
    assert_eq!(done["result"], "IMPROVE drop applied reply=#f_1a2b3c4d");
    let argv = fs::read_to_string(s.dir.join("argv.log")).unwrap();
    let expected = [
        "--person".to_owned(),
        "maria".to_owned(),
        "--assistant".to_owned(),
        "codex".to_owned(),
        "--mempalace-check".to_owned(),
        s.path("check.sh"),
        "improve".to_owned(),
        "--reply=drop".to_owned(),
        "--token=#f_1a2b3c4d".to_owned(),
        "--by".to_owned(),
        "maria".to_owned(),
        "--kg".to_owned(),
        s.path("palace/knowledge_graph.sqlite3"),
        "--palace".to_owned(),
        s.path("palace"),
        "--diary".to_owned(),
        s.path("diary-root"),
        "--mempalace".to_owned(),
        s.path("uv/mempalace"),
        "--mempalace-mcp".to_owned(),
        s.path("uv/mempalace-mcp"),
    ];
    assert_eq!(argv.lines().collect::<Vec<_>>(), expected);
}

#[test]
fn a_refusal_is_reported_and_bad_arguments_run_nothing() {
    let s = Scratch::new("refused");
    let palace = s.palace(
        1,
        "palace improve --reply: refused: use keep, accept or fix",
    );
    let responses = session(&s.args(&palace), &[reply("f_1a2b3c4d", "ok")]);
    let done = result(&responses[0]);
    assert_eq!(done["applied"], false, "{done}");
    assert_eq!(
        done["refused"],
        "palace improve --reply: refused: use keep, accept or fix"
    );
    fs::remove_file(s.dir.join("argv.log")).unwrap();
    let responses = session(&s.args(&palace), &[reply("#f_1a2b3c4d", "keep\nfix: x")]);
    assert_eq!(responses[0]["error"]["code"], -32602, "{}", responses[0]);
    assert!(!s.dir.join("argv.log").exists(), "palace ran");
}

#[test]
fn no_reply_tool_without_palace_bin() {
    let s = Scratch::new("absent");
    let args: Vec<String> = ["--palace", &s.path("palace")].map(str::to_owned).to_vec();
    let responses = session(&args, &[reply("#f_1a2b3c4d", "drop")]);
    assert_eq!(responses[0]["error"]["message"], "unknown tool");
}
