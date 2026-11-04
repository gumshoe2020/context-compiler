//! Report reuse exercises only invented entries and repository-local offline copies.
mod common;
use palace_core::ids::sha256;
use palace_store::{claims::ClaimStore, privacy::Audience};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn run(work: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_palace"))
        .current_dir(work)
        .arg(format!("--roots={}", work.display()))
        .args(["privacy", "classify"])
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn read(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn classify(work: &Path, copy: &str, out: &str, reuse: Option<&str>) -> Output {
    let store = format!("{copy}/compiler.sqlite");
    let mut args = vec![
        "--diary",
        copy,
        "--store",
        &store,
        "--llm",
        "replay",
        "--replay-reply",
        "replies.json",
        "--prompts",
        "prompts",
        "--out",
        out,
    ];
    if let Some(reuse) = reuse {
        args.extend(["--reuse", reuse]);
    }
    run(work, &args)
}
fn fixture(label: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root.join(format!("target/priv2-reuse/{label}-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(work.join("prompts")).unwrap();
    fs::copy(
        root.join("prompts/audience.md"),
        work.join("prompts/audience.md"),
    )
    .unwrap();
    work
}
fn copy(work: &Path, name: &str, private: &[&str]) {
    let copy = work.join(name);
    fs::create_dir_all(copy.join("diary")).unwrap();
    let mut database = ClaimStore::open(&copy.join("compiler.sqlite")).unwrap();
    for file in private {
        database
            .mark_entry(file, &"wren".parse::<Audience>().unwrap().marker(file))
            .unwrap();
    }
    drop(database);
    let graph = rusqlite::Connection::open(copy.join("knowledge_graph.sqlite3")).unwrap();
    graph
        .execute_batch("CREATE TABLE triples(id TEXT PRIMARY KEY, source_closet TEXT);")
        .unwrap();
    fs::write(copy.join("people.json"), common::PEOPLE).unwrap();
}

#[test]
fn reuse_skips_calls_rechecks_privacy_and_applies_to_new_copy() {
    let work = fixture("acceptance");
    copy(&work, "old", &["kept.md", "lost.md"]);
    let names = ["same", "changed", "marker", "tagged", "kept", "lost"];
    let mut replies = json!({});
    for name in names {
        let file = format!("{name}.md");
        let tag = if name == "tagged" { "cobalt" } else { "brass" };
        fs::write(
            work.join("old/diary").join(&file),
            format!("# Invented {name}\nTags: {tag}.\n\nThe imaginary observatory uses brass.\n"),
        )
        .unwrap();
        if !["kept", "lost"].contains(&name) {
            replies[&file] =
                json!({"category":"audience-household", "reason":"Invented reviewed decision."});
        }
    }
    fs::write(work.join("replies.json"), replies.to_string()).unwrap();
    success(classify(&work, "old", "previous.json", None));
    let previous = read(work.join("previous.json"));
    // Old reports without the new field remain valid reuse inputs.
    let mut legacy = previous.clone();
    for entry in legacy["entries"].as_array_mut().unwrap() {
        entry.as_object_mut().unwrap().remove("reused");
    }
    fs::write(work.join("previous.json"), legacy.to_string()).unwrap();
    copy(&work, "new", &["kept.md", "marker.md"]);
    for name in names {
        let file = format!("{name}.md");
        fs::copy(
            work.join("old/diary").join(&file),
            work.join("new/diary").join(&file),
        )
        .unwrap();
    }
    let mut people = read(work.join("new/people.json"));
    // A current tag rule makes an unchanged entry private too.
    people["private_tags"]["cobalt"] = json!(["wren"]);
    fs::write(work.join("new/people.json"), people.to_string()).unwrap();
    fs::write(
        work.join("new/diary/changed.md"),
        "# Invented changed\nTags: brass.\n\nThe telescope now uses silver.\n",
    )
    .unwrap();
    // Identical content under a new filename must still call the model.
    fs::copy(
        work.join("new/diary/same.md"),
        work.join("new/diary/new.md"),
    )
    .unwrap();
    // Missing replies prove unchanged/private entries cannot invoke the backend.
    fs::write(
        work.join("replies.json"),
        json!({
            "changed.md":{"category":"shared","reason":"Changed telescope."},
            "new.md":{"category":"shared","reason":"New telescope."},
            "lost.md":{"category":"shared","reason":"No current restriction."}
        })
        .to_string(),
    )
    .unwrap();
    // Reuse must not depend on the old copy still existing or on its store hash.
    fs::remove_dir_all(work.join("old")).unwrap();
    let output = success(classify(&work, "new", "report.json", Some("previous.json")));
    let counts: Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(counts, json!({"reused":1,"classified":3,"keep":3}));
    let report = read(work.join("report.json"));
    assert_eq!(report["diary"], json!(work.join("new")));
    assert_eq!(
        report["store_hash"],
        sha256(&fs::read(work.join("new/compiler.sqlite")).unwrap())
    );
    for entry in report["entries"].as_array().unwrap() {
        let path = Path::new(entry["path"].as_str().unwrap());
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(path.parent().unwrap(), work.join("new/diary"));
        assert_eq!(entry["hash"], sha256(&fs::read(path).unwrap()));
        assert_eq!(entry["reused"], name == "same.md");
        if name == "same.md" {
            assert_eq!(
                entry["decision"],
                json!({"category":"audience-household","reason":"Invented reviewed decision."})
            );
        } else if ["kept.md", "marker.md", "tagged.md"].contains(&name) {
            assert_eq!(
                entry["decision"],
                json!({"category":"keep","reason":"already private"})
            );
        } else {
            assert_eq!(entry["decision"]["category"], "shared");
        }
    }
    success(run(&work, &["--apply", "report.json"]));
    assert!(
        fs::read_to_string(work.join("new/diary/same.md"))
            .unwrap()
            .contains("audience-household")
    );
}

#[test]
fn reuse_refuses_unreadable_corrupt_and_incompatible_reports() {
    let work = fixture("refusal");
    copy(&work, "copy", &[]);
    fs::write(
        work.join("copy/diary/note.md"),
        "# Invented\nTags: brass.\n",
    )
    .unwrap();
    fs::write(
        work.join("replies.json"),
        json!({"note.md":{"category":"shared","reason":"Invented telescope."}}).to_string(),
    )
    .unwrap();
    success(classify(&work, "copy", "previous.json", None));
    let previous = read(work.join("previous.json"));
    fs::write(work.join("corrupt.json"), "{broken").unwrap();
    for file in ["missing.json", "corrupt.json"] {
        assert!(
            !classify(&work, "copy", "refused.json", Some(file))
                .status
                .success()
        );
        assert!(!work.join("refused.json").exists());
    }
    for variant in ["layout", "duplicate", "version", "decision", "hash"] {
        let mut bad = previous.clone();
        match variant {
            "layout" => bad["entries"][0]["path"] = json!(work.join("copy/other/note.md")),
            "duplicate" => {
                let entry = bad["entries"][0].clone();
                bad["entries"].as_array_mut().unwrap().push(entry);
            }
            "version" => bad["version"] = json!(2),
            "decision" => bad["entries"][0]["decision"]["category"] = json!("unknown"),
            "hash" => bad["entries"][0]["hash"] = json!("bad"),
            _ => unreachable!(),
        }
        fs::write(work.join("bad.json"), bad.to_string()).unwrap();
        let output = classify(&work, "copy", "refused.json", Some("bad.json"));
        assert!(!output.status.success(), "{variant}");
        assert!(!work.join("refused.json").exists());
    }
    for mode in ["--apply", "--undo"] {
        assert!(
            !run(&work, &[mode, "previous.json", "--reuse", "previous.json"])
                .status
                .success()
        );
    }
}
