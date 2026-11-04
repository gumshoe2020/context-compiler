//! The audience table is rendered from the people map; `<prompts>/local/<name>.md` is appended.
//! The model is a stand-in script that records the prompt it is sent. Invented content only.
mod common;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const LOCAL_POLICY: &str = "Synthetic local rule: Nella's lantern notes are audience-nella.";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

struct Bench {
    work: PathBuf,
}
impl Bench {
    fn new(name: &str) -> Self {
        let work = root().join(format!("target/pub2-tests/{name}-{}", std::process::id()));
        if work.exists() {
            fs::remove_dir_all(&work).unwrap();
        }
        fs::create_dir_all(work.join("prompts")).unwrap();
        fs::create_dir_all(work.join("diary")).unwrap();
        for file in fs::read_dir(root().join("prompts")).unwrap() {
            let path = file.unwrap().path();
            if path.is_file() {
                fs::copy(&path, work.join("prompts").join(path.file_name().unwrap())).unwrap();
            }
        }
        fs::write(work.join("people.json"), common::PEOPLE).unwrap();
        Self { work }
    }
    fn model(&self, reply: &str) -> String {
        let script = self.work.join("model.sh");
        let captured = self.work.join("prompt.txt");
        let body = format!(
            "#!/bin/sh\ncat > '{}'\ncat <<'EOF'\n{reply}\nEOF\n",
            captured.display()
        );
        fs::write(&script, body).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        format!("command:{}", script.display())
    }
    fn local(&self, name: &str) {
        fs::create_dir_all(self.work.join("prompts/local")).unwrap();
        fs::write(self.work.join("prompts/local").join(name), LOCAL_POLICY).unwrap();
    }
    fn sent(&self) -> String {
        fs::read_to_string(self.work.join("prompt.txt")).unwrap()
    }
    fn palace(&self, with_people: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_palace"));
        command.arg(format!("--roots={}", root().display()));
        if with_people {
            command
                .arg("--people-map")
                .arg(self.work.join("people.json"));
        }
        command
    }
    fn distill(&self, with_people: bool) {
        let transcript = self.work.join("invented.jsonl");
        let line = r#"{"role":"user","text":"Orin Vale discussed an invented observatory.","ts":"2031-06-01T12:00:00Z"}"#;
        fs::write(&transcript, format!("{line}\n")).unwrap();
        let reply = r#"{"entries":[],"claims":[],"relations":[],"skipped":[]}"#;
        let output = self
            .palace(with_people)
            .args([
                "--person",
                "orin",
                "--assistant",
                "pip",
                "distill",
                "--dry-run",
            ])
            .args([
                "--source",
                "openclaw",
                "--min-turns",
                "1",
                "--wing",
                "synthetic",
            ])
            .arg("--transcript")
            .arg(&transcript)
            .arg("--diary")
            .arg(&self.work)
            .arg("--wings")
            .arg(self.work.join("wings"))
            .arg("--transcripts")
            .arg(self.work.join("transcripts"))
            .arg("--palace")
            .arg(self.work.join("palace"))
            .arg("--kg")
            .arg(self.work.join("palace/knowledge_graph.sqlite3"))
            .args(["--llm", &self.model(reply), "--prompts"])
            .arg(self.work.join("prompts"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
    }
    fn classify(&self) {
        fs::write(
            self.work.join("diary/note.md"),
            "# Invented\nTags: lantern.\n\nA lantern.\n",
        )
        .unwrap();
        let store = self.work.join("compiler.sqlite");
        drop(palace_store::claims::ClaimStore::open(&store).unwrap());
        let reply = r#"{"category":"shared","reason":"Synthetic."}"#;
        let output = self
            .palace(false)
            .args(["privacy", "classify", "--diary"])
            .arg(&self.work)
            .arg("--store")
            .arg(&store)
            .args(["--llm", &self.model(reply), "--prompts"])
            .arg(self.work.join("prompts"))
            .arg("--out")
            .arg(self.work.join("report.json"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
    }
}
fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_table(prompt: &str) {
    assert!(!prompt.contains("{AUDIENCE_TABLE}"));
    for row in [
        "| audience-elira-health | orin, mira, nella |",
        "| audience-household | orin, mira |",
        "| audience-orin | orin |",
        "| audience-nella | orin, nella |",
        "| audience-torin | orin, torin |",
        "| audience-wren | orin, wren |",
        "| audience-study | orin, torin, nella |",
        "| shared | everyone |",
    ] {
        assert!(prompt.contains(row), "missing {row}");
    }
}

#[test]
fn distill_prompt_renders_the_table_and_appends_local_policy_only_when_present() {
    let bench = Bench::new("distill");
    bench.distill(true);
    assert_table(&bench.sent());
    assert!(!bench.sent().contains("\nLocal policy\n"));
    bench.local("distill.md");
    bench.distill(true);
    let prompt = bench.sent();
    assert_table(&prompt);
    let (policy, local) = prompt
        .split_once("\n\nLocal policy\n\n")
        .expect("local policy heading");
    assert!(local.starts_with(LOCAL_POLICY));
    assert!(policy.contains("AUDIENCE TAGS") && !local.contains("AUDIENCE TAGS"));
}

#[test]
fn distill_prompt_without_a_people_map_offers_only_shared() {
    let bench = Bench::new("distill-bare");
    bench.distill(false);
    let prompt = bench.sent();
    assert!(prompt.contains("| shared | everyone |"));
    assert!(!prompt.contains("| audience-"));
}

#[test]
fn classify_prompt_renders_the_table_and_appends_local_policy_only_when_present() {
    let bench = Bench::new("classify");
    bench.classify();
    assert_table(&bench.sent());
    assert!(!bench.sent().contains("\nLocal policy\n"));
    bench.local("audience.md");
    fs::remove_file(bench.work.join("report.json")).unwrap();
    bench.classify();
    let prompt = bench.sent();
    assert_table(&prompt);
    assert!(prompt.contains(&format!("Local policy\n\n{LOCAL_POLICY}")));
    assert!(
        Path::new(&bench.work)
            .join("prompts/local/audience.md")
            .exists()
    );
}
