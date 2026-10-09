//! PKG-1.2 item 2: `palace_diary_write` through the binary, with a stand-in `mempalace` that
//! records its argv: the entry is a file by the diary recipe, then the diary root is mined into
//! the palace with the wing and agent asked for. Scratch directories under target/.
use serde_json::{Value, json};
use std::{
    ffi::OsString,
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
        let dir = root.join(format!("target/mcp-diary-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("diary-root/diary")).unwrap();
        fs::create_dir_all(dir.join("palace")).unwrap();
        fs::write(dir.join("check.sh"), "PIN=\"3.5.0\"\n").unwrap();
        Self { dir }
    }
    /// A stand-in mempalace: `--version` prints `MemPalace <version>`; `mine` logs and exits `code`.
    fn mempalace(&self, version: &str, code: i32) -> PathBuf {
        let path = self.dir.join("mempalace");
        let log = self.dir.join("mine.log");
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 'MemPalace {version}'; exit 0; }}\n\
             echo \"$MEMPALACE_PALACE_PATH $*\" >>'{}'\nexit {code}\n",
            log.display()
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    fn mine_log(&self) -> String {
        fs::read_to_string(self.dir.join("mine.log")).unwrap_or_default()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// One `tools/call` of `palace_diary_write`; the response's text parsed, or the JSON-RPC error.
fn call(args: &[OsString], arguments: Value) -> Result<Value, Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_palace-mcp"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"palace_diary_write","arguments":arguments}});
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
    let response: Value =
        serde_json::from_slice(&child.wait_with_output().unwrap().stdout).unwrap();
    match response["result"]["content"][0]["text"].as_str() {
        Some(text) => Ok(serde_json::from_str(text).unwrap()),
        None => Err(response["error"].clone()),
    }
}

fn flags(s: &Scratch, mempalace: &Path, extra: &[&str]) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    for (flag, value) in [
        ("--palace", s.dir.join("palace")),
        ("--diary-root", s.dir.join("diary-root")),
        ("--mempalace-check", s.dir.join("check.sh")),
        ("--mempalace", mempalace.to_path_buf()),
    ] {
        args.extend([flag.into(), value.into_os_string()]);
    }
    args.extend(extra.iter().map(OsString::from));
    args
}

#[test]
fn an_entry_is_a_file_then_mined() {
    let s = Scratch::new("write");
    let mempalace = s.mempalace("3.5.0", 0);
    let args = flags(&s, &mempalace, &["--wing", "memory", "--agent", "codex"]);
    let result = call(
        &args,
        json!({"text":"  We chose Rust.\n","title":"Language"}),
    )
    .unwrap();
    assert_eq!(result["mined"], true, "{result}");
    assert_eq!(result["wing"], "memory");
    let file = PathBuf::from(result["file"].as_str().unwrap());
    let name = file.file_name().unwrap().to_str().unwrap();
    assert_eq!(file.parent().unwrap(), s.dir.join("diary-root/diary"));
    assert!(name.len() == 26 && name.ends_with(".md"), "{name}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "# Language\n\nWe chose Rust.\n"
    );
    let palace = s.dir.join("palace");
    let root = s.dir.join("diary-root");
    let expected = format!(
        "{} --palace {} mine {} --wing memory --agent codex\n",
        palace.display(),
        palace.display(),
        root.display()
    );
    assert_eq!(s.mine_log(), expected);
    let other = call(&args, json!({"text":"Another.","wing":"work"})).unwrap();
    assert_eq!(other["wing"], "work");
    assert!(s.mine_log().ends_with("--wing work --agent codex\n"));
    assert_ne!(other["file"], result["file"]);
}

#[test]
fn a_failed_mine_still_files_the_entry() {
    let s = Scratch::new("unmined");
    for (version, code) in [("3.5.0", 1), ("3.4.0", 0)] {
        let mempalace = s.mempalace(version, code);
        let result = call(&flags(&s, &mempalace, &[]), json!({"text":"x"})).unwrap();
        assert_eq!(result["mined"], false, "{result}");
        assert_eq!(result["wing"], "openclaw");
        assert!(
            result["not_mined"]
                .as_str()
                .unwrap()
                .ends_with("the nightly job mines the diary")
        );
        assert!(Path::new(result["file"].as_str().unwrap()).is_file());
    }
    assert_eq!(
        s.mine_log().lines().count(),
        1,
        "the 3.4.0 stand-in was never mined with"
    );
}

#[test]
fn bad_arguments_write_nothing() {
    let s = Scratch::new("bad");
    let mempalace = s.mempalace("3.5.0", 0);
    let error = call(&flags(&s, &mempalace, &[]), json!({"text":" "})).unwrap_err();
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(
        fs::read_dir(s.dir.join("diary-root/diary"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(s.mine_log(), "");
}

/// PKG-1.3 item 3: with `--wings-root`, an entry for another wing is written and mined outside the
/// diary root, also when its mine fails; this memory's own wing stays in the diary root.
#[test]
fn another_wing_stays_outside_the_diary_root() {
    let s = Scratch::new("wings");
    let wings = s.dir.join("wings");
    let wings_flag = wings.to_str().unwrap();
    let extra = [
        "--wing",
        "memory",
        "--agent",
        "codex",
        "--wings-root",
        wings_flag,
    ];
    let failing = s.mempalace("3.5.0", 1);
    let private = call(
        &flags(&s, &failing, &extra),
        json!({"text":"Mine.","wing":"private"}),
    );
    let private = private.unwrap();
    assert_eq!(
        (&private["wing"], &private["mined"]),
        (&json!("private"), &json!(false))
    );
    let file = PathBuf::from(private["file"].as_str().unwrap());
    assert_eq!(file.parent().unwrap(), wings.join("private/diary"));
    let mempalace = s.mempalace("3.5.0", 0);
    let own = call(&flags(&s, &mempalace, &extra), json!({"text":"Ours."})).unwrap();
    assert_eq!(own["mined"], true, "{own}");
    let own_file = PathBuf::from(own["file"].as_str().unwrap());
    assert_eq!(own_file.parent().unwrap(), s.dir.join("diary-root/diary"));
    let diary: Vec<_> = fs::read_dir(s.dir.join("diary-root/diary"))
        .unwrap()
        .collect();
    assert_eq!(
        diary.len(),
        1,
        "only this memory's own entry in the diary root"
    );
    let again = call(
        &flags(&s, &mempalace, &extra),
        json!({"text":"More.","wing":"private"}),
    );
    assert_eq!(again.unwrap()["mined"], true);
    let palace = s.dir.join("palace").display().to_string();
    let lines: Vec<String> = s.mine_log().lines().map(str::to_owned).collect();
    let root = |dir: &Path| format!("{palace} --palace {palace} mine {}", dir.display());
    assert_eq!(
        lines,
        [
            format!(
                "{} --wing private --agent codex",
                root(&wings.join("private"))
            ),
            format!(
                "{} --wing memory --agent codex",
                root(&s.dir.join("diary-root"))
            ),
            format!(
                "{} --wing private --agent codex",
                root(&wings.join("private"))
            ),
        ]
    );
}

/// PKG-2 item 0: a mine refused because another process holds the palace's lock (MemPalace's own
/// message) is tried again within the deadline: mined once the holder lets go, else the last
/// refusal; a held lock that outlasts the deadline is never waited out.
#[test]
fn a_held_lock_is_retried_within_the_deadline() {
    let s = Scratch::new("held");
    let held = s.dir.join("held");
    let mempalace = s.dir.join("mempalace");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 'MemPalace 3.5.0'; exit 0; }}\n\
         echo try >>'{}'\n\
         [ -e '{}' ] && {{ echo \"mempalace: palace $MEMPALACE_PALACE_PATH is held by pid 1; \
         wait for it to finish or stop the holder before retrying\" >&2; exit 1; }}\nexit 0\n",
        s.dir.join("mine.log").display(),
        held.display()
    );
    fs::write(&mempalace, script).unwrap();
    fs::set_permissions(&mempalace, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&held, "").unwrap();
    let release = {
        let held = held.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            fs::remove_file(held).unwrap();
        })
    };
    let result = call(&flags(&s, &mempalace, &[]), json!({"text":"Held."})).unwrap();
    release.join().unwrap();
    assert_eq!(result["mined"], true, "{result}");
    assert_eq!(result["mine_tries"], 3, "tries at 0, 1 and 3 s: {result}");
    assert!(result.get("not_mined").is_none(), "{result}");
    fs::write(&held, "").unwrap();
    fs::remove_file(s.dir.join("mine.log")).unwrap();
    let started = std::time::Instant::now();
    let extra = ["--mine-timeout-ms", "2500"];
    let result = call(
        &flags(&s, &mempalace, &extra),
        json!({"text":"Still held."}),
    )
    .unwrap();
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    assert_eq!(result["mined"], false, "{result}");
    assert_eq!(
        result["mine_tries"], 2,
        "no wait past the deadline: {result}"
    );
    let reason = result["not_mined"].as_str().unwrap();
    assert!(reason.contains(" is held by pid 1;"), "{reason}");
    assert!(
        reason.ends_with("the nightly job mines the diary"),
        "{reason}"
    );
    assert!(Path::new(result["file"].as_str().unwrap()).is_file());
}

/// PKG-1.3 item 6: a mine past `--mine-timeout-ms` is ended with its descendants; the file stays
/// and the reply says so.
#[test]
fn a_hung_mine_is_ended_and_the_file_stays() {
    let s = Scratch::new("hung");
    let mempalace = s.dir.join("mempalace");
    let pid = s.dir.join("mine.pid");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 'MemPalace 3.5.0'; exit 0; }}\n\
         sleep 30 &\necho $! >'{}'\nwait\n",
        pid.display()
    );
    fs::write(&mempalace, script).unwrap();
    fs::set_permissions(&mempalace, fs::Permissions::from_mode(0o755)).unwrap();
    let started = std::time::Instant::now();
    let extra = ["--mine-timeout-ms", "500"];
    let result = call(&flags(&s, &mempalace, &extra), json!({"text":"Kept."})).unwrap();
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    assert_eq!(result["mined"], false, "{result}");
    let reason = result["not_mined"].as_str().unwrap();
    assert!(
        reason.starts_with(
            "mempalace mine did not finish within 500 ms and was ended; the entry's file stays"
        ),
        "{reason}"
    );
    assert!(Path::new(result["file"].as_str().unwrap()).is_file());
    let descendant = fs::read_to_string(&pid).unwrap();
    let alive = || {
        Command::new("/bin/kill")
            .args(["-0", descendant.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    // A killed process answers `kill -0` until it is reaped: give it up to 3 s.
    let gone = (0..60).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        !alive()
    });
    assert!(gone, "the mine's descendant {descendant} outlived it");
}
