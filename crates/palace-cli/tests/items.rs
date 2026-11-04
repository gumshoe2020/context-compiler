//! Stand-in-only integration tests; scratch and child output stay under target/init1b/.
use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
struct Scratch {
    root: PathBuf,
    dir: PathBuf,
}
impl Scratch {
    fn new(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let dir = root.join(format!("target/init1b/cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for file in ["conversation.jsonl", "replies.json", "model.py"] {
            fs::copy(root.join("testdata/items").join(file), dir.join(file)).unwrap();
        }
        Self { root, dir }
    }
    fn call(&self, person: Option<&str>, args: &[&str]) -> String {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_palace"));
        cmd.current_dir(&self.root)
            .arg("items")
            .args(args)
            .arg("--store")
            .arg(self.dir.join("compiler.sqlite"));
        if let Some(person) = person {
            cmd.args(["--person", person]);
        }
        let output = cmd.output().unwrap();
        fs::write(self.dir.join("last-stdout.log"), &output.stdout).unwrap();
        fs::write(self.dir.join("last-stderr.log"), &output.stderr).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn update(&self, person: &str, from: &str, to: &str, model: &str) -> Value {
        let text = self.call(
            Some(person),
            &[
                "update",
                "--transcript",
                self.dir.join("conversation.jsonl").to_str().unwrap(),
                "--from-line",
                from,
                "--to-line",
                to,
                "--llm",
                model,
            ],
        );
        serde_json::from_str(text.trim().strip_prefix("ITEMS ").unwrap()).unwrap()
    }
    fn list(&self, person: Option<&str>) -> Vec<Value> {
        serde_json::from_str(&self.call(person, &["list", "--json"])).unwrap()
    }
    fn table(&self) -> String {
        format!("table:{}", self.dir.join("replies.json").display())
    }
}
#[test]
fn valid_actions_continue_past_rejection_and_later_completion_closes() {
    let s = Scratch::new("updates");
    let first = s.update("elora", "1", "4", &s.table());
    assert_eq!(first["created"], 2);
    assert_eq!(first["rejected"], 1);
    let before = s.list(Some("elora"));
    assert_eq!(before.len(), 2);
    assert_eq!(before[1]["kind"], "pastoral");
    let second = s.update("elora", "5", "6", &s.table());
    assert_eq!(second["updated"], 1);
    assert_eq!(second["closed"], 1);
    let after = s.list(Some("elora"));
    assert_eq!(after[0]["opened_at"], before[0]["opened_at"]);
    assert_eq!(after[0]["status"], "done");
    assert_eq!(after[0]["close_quote"], "Okay sent.");
    assert_eq!(after[0]["title"], "Send the harbor chart");
    assert_eq!(after[0]["times_raised"], 0);
    let repeat = s.update("elora", "1", "4", &s.table());
    assert_eq!(repeat["created"], 0);
    assert_eq!(repeat["rejected"], 3);
    // Expiry is a terminal state, with evidence retained.
    assert_eq!(s.update("elora", "7", "7", &s.table())["created"], 1);
    assert_eq!(s.update("elora", "8", "8", &s.table())["expired"], 1);
    let store = palace_store::claims::ClaimStore::open(&s.dir.join("compiler.sqlite")).unwrap();
    store.verify_chain().unwrap();
    assert_eq!(store.events("items_rejected").unwrap().len(), 4);
    assert_eq!(store.events("items_action").unwrap().len(), 6);
}
#[test]
fn command_prompt_is_scoped_and_cross_person_actions_are_refused() {
    let s = Scratch::new("privacy");
    s.update("elora", "1", "4", &s.table());
    let model = format!("command:{}", s.dir.join("model.py").display());
    let summary = s.update("niven", "5", "6", &model);
    assert_eq!(summary["rejected"], 2);
    assert!(s.list(Some("niven")).is_empty());
    let prompt = fs::read_to_string(s.dir.join("prompt-5-6.txt")).unwrap();
    assert!(prompt.contains("Open items: []"));
    assert!(!prompt.contains("elora"));
    assert!(!prompt.contains("Please check in about my worry tomorrow."));
    assert!(!prompt.contains("I'll send the harbor map."));
    assert_eq!(s.list(None).len(), 2);
}
#[test]
fn malformed_action_and_wrong_line_do_not_abort_valid_following_actions() {
    let s = Scratch::new("malformed");
    let mut table: Value =
        serde_json::from_str(&fs::read_to_string(s.dir.join("replies.json")).unwrap()).unwrap();
    let actions = table["1-4"]["actions"].as_array_mut().unwrap();
    actions[0]["line"] = serde_json::json!(2); // quote exists, but on another line
    actions.insert(0, serde_json::json!(false));
    fs::write(
        s.dir.join("replies.json"),
        serde_json::to_vec(&table).unwrap(),
    )
    .unwrap();
    let summary = s.update("elora", "1", "4", &s.table());
    assert_eq!(summary["rejected"], 3);
    assert_eq!(summary["created"], 1);
    assert_eq!(s.list(Some("elora"))[0]["kind"], "pastoral");
}
#[test]
fn replay_windows_match_updates_and_refuse_an_existing_store() {
    let s = Scratch::new("replay");
    let output = s.call(
        Some("elora"),
        &[
            "replay",
            "--transcript",
            s.dir.join("conversation.jsonl").to_str().unwrap(),
            "--llm",
            &s.table(),
        ],
    );
    let summary: Value =
        serde_json::from_str(output.trim().strip_prefix("ITEMS REPLAY ").unwrap()).unwrap();
    assert_eq!(
        summary,
        serde_json::json!({"created":3,"updated":1,"closed":1,"expired":1,"open":1,"rejected":1,"windows":4})
    );
    let rows = s.list(Some("elora"));
    assert_eq!(rows[0]["status"], "done");
    assert_eq!(rows[1]["kind"], "pastoral");
    assert_eq!(rows[2]["status"], "expired");
    let output = Command::new(env!("CARGO_BIN_EXE_palace"))
        .current_dir(&s.root)
        .args(["items", "replay", "--person", "elora", "--transcript"])
        .arg(s.dir.join("conversation.jsonl"))
        .args(["--store"])
        .arg(s.dir.join("compiler.sqlite"))
        .args(["--llm", &s.table()])
        .output()
        .unwrap();
    fs::write(s.dir.join("existing-store-error.log"), &output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fresh scratch"));
}
#[test]
fn replay_invalid_clock_fails_before_writing() {
    let s = Scratch::new("clock");
    let path = s.dir.join("conversation.jsonl");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("2031-04-03T09:00:00Z", "missing-clock");
    fs::write(&path, text).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_palace"))
        .current_dir(&s.root)
        .args(["items", "replay", "--person", "elora", "--transcript"])
        .arg(path)
        .args(["--store"])
        .arg(s.dir.join("compiler.sqlite"))
        .args(["--llm", &s.table()])
        .output()
        .unwrap();
    fs::write(s.dir.join("bad-clock-error.log"), &output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(!s.dir.join("compiler.sqlite").exists());
}
#[test]
fn settle_hook_is_opt_in_scoped_and_nonfatal() {
    let base = Scratch::new("hook");
    for (name, setting, wing, distill_exit, item_exit, map, expected) in [
        ("off", "", "memory", "0", "0", false, false),
        ("on", "1", "memory", "0", "0", false, true),
        ("failure", "1", "memory", "0", "7", false, true),
        ("distill-failure", "1", "memory", "9", "0", false, false),
        ("private", "1", "seabrook", "0", "0", true, true),
        ("unmapped", "1", "seabrook", "0", "0", false, false),
    ] {
        let dir = base.dir.join(name);
        for sub in [
            "bin",
            "state",
            "logs",
            "palace",
            "diary-root/diary",
            "wings",
            "scripts",
            "prompts",
        ] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::copy(
            base.root.join("dist/common/settle.sh"),
            dir.join("bin/settle.sh"),
        )
        .unwrap();
        fs::copy(
            base.root.join("testdata/items/settle-palace.sh"),
            dir.join("bin/palace"),
        )
        .unwrap();
        fs::copy(
            base.root.join("testdata/items/settle-cron.sh"),
            dir.join("bin/cron.sh"),
        )
        .unwrap();
        let transcript = base.dir.join("conversation.jsonl");
        fs::write(
            dir.join("state/settle.request"),
            format!("openclaw\t{}\n", transcript.display()),
        )
        .unwrap();
        fs::write(
            dir.join("state/distill.positions"),
            format!("openclaw\t{}\t4\n", transcript.display()),
        )
        .unwrap();
        // Empty entries fingerprint keeps the daily run quiet, apart from the no-op cron.
        fs::write(dir.join("state/last-settle"), "42949672950\n").unwrap();
        let mapping = dir.join("people.tsv");
        fs::write(&mapping, "seabrook\tniven\n").unwrap();
        fs::write(
            dir.join("env.sh"),
            format!(
                "WING=memory\nPERSON=elora\nASSISTANT=quill\nSETTLE_ITEMS='{setting}'\n{}",
                if map {
                    format!("ITEMS_PERSON_MAP='{}'\n", mapping.display())
                } else {
                    String::new()
                }
            ),
        )
        .unwrap();
        let output = Command::new("/bin/sh")
            .arg(dir.join("bin/settle.sh"))
            .args(["--daily", "--llm", "command:synthetic-never-invoked", "--"])
            .arg(dir.join("bin/cron.sh"))
            .env("HOOK_CALL_LOG", dir.join("calls.log"))
            .env("HOOK_WING", wing)
            .env("HOOK_DISTILL_EXIT", distill_exit)
            .env("HOOK_ITEMS_EXIT", item_exit)
            .output()
            .unwrap();
        fs::write(dir.join("stdout.log"), &output.stdout).unwrap();
        fs::write(dir.join("stderr.log"), &output.stderr).unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = fs::read_to_string(dir.join("calls.log")).unwrap();
        assert_eq!(calls.contains("\nitems\n"), expected, "{name}: {calls}");
        if expected {
            let call = calls
                .split("CALL\n")
                .find(|c| c.contains("\nitems\n"))
                .unwrap();
            assert!(call.contains("--from-line\n5\n--to-line\n8\n"), "{call}");
            assert!(
                call.contains(&format!(
                    "--person\n{}\n",
                    if map { "niven" } else { "elora" }
                )),
                "{call}"
            );
        }
        if item_exit != "0" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("items update failed"));
        }
    }
}
