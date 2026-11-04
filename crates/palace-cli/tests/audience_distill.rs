//! New-entry replay fixtures contain invented content only. No model or live transport.
mod common;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
#[test]
fn distill_replay_retains_one_audience_tag_and_shared_entries_have_none() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    for name in ["audience.md", "distill.md"] {
        let policy = fs::read_to_string(root.join("prompts").join(name)).unwrap();
        assert_eq!(policy.matches("{AUDIENCE_TABLE}").count(), 1, "{name}");
        assert!(
            !policy.contains("| audience-"),
            "{name}: the table is rendered, not written"
        );
    }
    let work = root.join(format!("target/priv2/distill-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(work.join("diary")).unwrap();
    let transcript = work.join("invented.jsonl");
    fs::write(&transcript,json!({"role":"user","text":"Orin Vale discussed an invented observatory.","ts":"2031-06-01T12:00:00Z"}).to_string()+"\n").unwrap();
    let replay = work.join("reply.json");
    let people = work.join("people.json");
    fs::write(&people, common::PEOPLE).unwrap();
    let run_with = |map: Option<&PathBuf>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_palace"));
        command.arg(format!("--roots={}", root.display()));
        if let Some(map) = map {
            command.arg("--people-map").arg(map);
        }
        command
            .args([
                "--person",
                "orin",
                "--assistant",
                "pip",
                "distill",
                "--dry-run",
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
            .arg(&work)
            .arg("--wings")
            .arg(work.join("wings"))
            .arg("--transcripts")
            .arg(work.join("transcripts"))
            .arg("--palace")
            .arg(work.join("palace"))
            .arg("--kg")
            .arg(work.join("palace/knowledge_graph.sqlite3"))
            .args(["--llm", "replay", "--replay-reply"])
            .arg(&replay)
            .arg("--prompts")
            .arg(root.join("prompts"))
            .output()
            .unwrap()
    };
    let run = || run_with(Some(&people));
    for (tag, body) in [
        (
            Some("audience-elira-health"),
            "Elira Quarrie's invented MRI appointment.",
        ),
        (
            Some("audience-household"),
            "Mira Fen discussed her own illness and household finances.",
        ),
        (
            Some("audience-orin"),
            "Orin Vale's confidential spiritual reflection.",
        ),
        (Some("audience-nella"), "Nella's own private conversation."),
        (Some("audience-torin"), "Torin's own private conversation."),
        (Some("audience-wren"), "Wren's own private conversation."),
        (
            Some("audience-study"),
            "Orin, Torin and Nella discussed theology together.",
        ),
        (None, "The invented observatory software uses Rust."),
    ] {
        let mut tags = vec!["observatory"];
        tags.extend(tag);
        fs::write(&replay,json!({"entries":[{"title":"Invented thread","body":body,"tags":tags}],"claims":[],"relations":[],"skipped":[]}).to_string()).unwrap();
        let result = run();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let reply: Value = serde_json::Deserializer::from_slice(&result.stdout)
            .into_iter::<Value>()
            .next()
            .unwrap()
            .unwrap();
        let actual = reply["entries"][0]["tags"].as_array().unwrap();
        assert_eq!(
            actual
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| t.starts_with("audience-"))
                .collect::<Vec<_>>(),
            tag.into_iter().collect::<Vec<_>>()
        );
    }
    for tags in [
        json!(["audience-household", "audience-orin"]),
        json!(["audience-unrecognized"]),
        json!(["audience-household."]),
    ] {
        fs::write(
            &replay,
            json!({"entries":[{"title":"Invented","body":"Invented thread.","tags":tags}]})
                .to_string(),
        )
        .unwrap();
        assert_eq!(run().status.code(), Some(4));
    }
    // Without a people map no audience tag is approved: shared entries pass, any audience tag fails.
    for (tags, code) in [
        (json!(["observatory"]), Some(0)),
        (json!(["audience-household"]), Some(4)),
    ] {
        let entry = json!({"title":"Invented","body":"Invented thread.","tags":tags});
        fs::write(&replay, json!({"entries":[entry]}).to_string()).unwrap();
        assert_eq!(run_with(None).status.code(), code);
    }
    assert!(!work.join("palace/compiler.sqlite").exists());
    assert_eq!(fs::read_dir(work.join("diary")).unwrap().count(), 0);
}
