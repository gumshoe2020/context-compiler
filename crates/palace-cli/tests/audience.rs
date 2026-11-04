//! PRIV-2 fixtures: invented people/content, replay only, repository-local copies.
mod common;
use palace_store::claims::ClaimStore;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn run(work: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_palace"))
        .arg(format!("--roots={}", work.display()))
        .args(["privacy", "classify"])
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn classifier_replay_is_read_only_counts_samples_and_fences_inputs() {
    let work = root().join(format!("target/priv2/classifier-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(work.join("diary")).unwrap();
    fs::create_dir_all(work.join("prompts")).unwrap();
    fs::copy(
        root().join("prompts/audience.md"),
        work.join("prompts/audience.md"),
    )
    .unwrap();
    fs::write(work.join("people.json"), common::PEOPLE).unwrap();
    let store = work.join("compiler.sqlite");
    drop(ClaimStore::open(&store).unwrap());
    let before = fs::read(&store).unwrap();
    let mut replies = serde_json::Map::new();
    for (i, (category, content)) in [
        ("audience-elira-health", "Elira Quarrie's MRI appointment."),
        (
            "audience-household",
            "Mira Fen discussed her own health and household savings.",
        ),
        (
            "audience-orin",
            "Orin Vale shared a confidential spiritual reflection.",
        ),
        (
            "audience-nella",
            "Nella's own conversation about a private concern.",
        ),
        (
            "audience-torin",
            "Torin's own conversation about a private concern.",
        ),
        (
            "audience-wren",
            "Wren's own conversation about a private concern.",
        ),
        (
            "audience-study",
            "Orin, Torin and Nella discussed theology together.",
        ),
        ("shared", "The observatory software uses Rust."),
        (
            "audience-household",
            "Elira's health and Mira's own illness are mixed.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("entry-{i}.md");
        fs::write(
            work.join("diary").join(&name),
            format!("# Invented {i}\nTags: observatory.\n\n{content}\n"),
        )
        .unwrap();
        replies.insert(
            name,
            json!({"category":category,"reason":"Synthetic reviewed category."}),
        );
    }
    for i in 9..19 {
        let name = format!("entry-{i}.md");
        fs::write(
            work.join("diary").join(&name),
            format!("# Invented {i}\nTags: software.\n\nThe invented telescope uses Rust.\n"),
        )
        .unwrap();
        replies.insert(
            name,
            json!({"category":"shared","reason":"General technology."}),
        );
    }
    let replay = work.join("replies.json");
    fs::write(&replay, json!(replies).to_string()).unwrap();
    let report = work.join("report.json");
    let prompts = work.join("prompts");
    let args = [
        "--diary",
        work.to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--llm",
        "replay",
        "--replay-reply",
        replay.to_str().unwrap(),
        "--prompts",
        prompts.to_str().unwrap(),
        "--out",
        report.to_str().unwrap(),
    ];
    let output = run(&work, &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    assert_eq!(report["entries"].as_array().unwrap().len(), 19);
    assert_eq!(report["counts"]["audience-household"]["count"], 2);
    assert_eq!(report["counts"]["audience-study"]["count"], 1);
    assert_eq!(report["counts"].as_object().unwrap().len(), 9);
    assert_eq!(
        report["counts"]["shared"]["samples"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(fs::read(&store).unwrap(), before);
    assert!(!work.join("compiler.sqlite-wal").exists());
    assert!(!work.join("compiler.sqlite-shm").exists());
    for entry in report["entries"].as_array().unwrap() {
        let text = fs::read(entry["path"].as_str().unwrap()).unwrap();
        assert_eq!(entry["hash"], palace_core::ids::sha256(&text));
    }
    // Existing source outside this copy's roots stands in for a live store: refused
    // before model invocation; no forbidden path is ever opened or created.
    let outside = root().join("Cargo.toml");
    let mut bad = args;
    bad[3] = outside.to_str().unwrap();
    let output = run(&work, &bad);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("under --roots"));
    fs::write(
        &replay,
        r#"{"entry-0.md":{"category":"unrecognized","reason":"No."}}"#,
    )
    .unwrap();
    fs::remove_file(work.join("report.json")).unwrap();
    assert!(!run(&work, &args).status.success());
    assert!(!work.join("report.json").exists());
}

#[test]
fn already_private_entries_keep_without_a_model_call() {
    use palace_store::privacy::Audience;

    let work = root().join(format!("target/priv2r/private-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(work.join("diary")).unwrap();
    fs::create_dir_all(work.join("prompts")).unwrap();
    fs::copy(
        root().join("prompts/audience.md"),
        work.join("prompts/audience.md"),
    )
    .unwrap();
    let store = work.join("compiler.sqlite");
    let mut database = ClaimStore::open(&store).unwrap();
    let private = "wren".parse::<Audience>().unwrap();
    database
        .mark_entry("marker.md", &private.marker("marker.md"))
        .unwrap();
    database
        .mark_entry("both.md", &private.marker("both.md"))
        .unwrap();
    // A shared marker cannot override a private tag not yet applied to the store.
    database
        .mark_entry("inline.md", &Audience::default().marker("inline.md"))
        .unwrap();
    drop(database);
    fs::write(
        work.join("people.json"),
        json!({
            "people":{"wren":{"handles":[]}}, "conversations":[],
            "private_tags":{"Wren-Private":["wren"]}
        })
        .to_string(),
    )
    .unwrap();
    for (name, tags) in [
        ("marker", "Tags: telescope."),
        ("inline", "tags: [\"WREN-PRIVATE\"]"),
        ("block", "---\ntags:\n  - wren-private\n---"),
        ("both", "Tags: wren-private."),
    ] {
        fs::write(
            work.join(format!("diary/{name}.md")),
            format!("# Invented Wren\n{tags}\n\nWren's confidential observatory note.\n"),
        )
        .unwrap();
    }
    // No replies at all: Replay::complete fails for every possible invocation.
    fs::write(work.join("replies.json"), "{}").unwrap();
    let before = fs::read(&store).unwrap();
    let args = [
        "--diary",
        work.to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--llm",
        "replay",
        "--replay-reply",
        "replies.json",
        "--prompts",
        "prompts",
        "--out",
        "report.json",
    ];
    // Relative inputs are resolved against the CLI working directory.
    let output = Command::new(env!("CARGO_BIN_EXE_palace"))
        .current_dir(&work)
        .arg(format!("--roots={}", work.display()))
        .args(["privacy", "classify"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(work.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["counts"]["keep"]["count"], 4);
    for entry in report["entries"].as_array().unwrap() {
        assert_eq!(
            entry["decision"],
            json!({"category":"keep", "reason":"already private"})
        );
    }
    assert_eq!(fs::read(&store).unwrap(), before);
    // Control: the same empty replay fails for an unmarked, untagged entry.
    fs::write(
        work.join("diary/shared.md"),
        "# Invented public observatory\nTags: telescope.\n",
    )
    .unwrap();
    fs::remove_file(work.join("report.json")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_palace"))
        .current_dir(&work)
        .arg(format!("--roots={}", work.display()))
        .args(["privacy", "classify"])
        .args(args)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing classifier replay reply"));
}
