//! Invented identities only; CLI writes and status stay inside the repository scratch root.
use palace_store::{claims::ClaimStore, privacy};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn authenticated_diary_markers_and_counts_only_status() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root.join(format!(
        "target/settle1k-m3b/diary-cli-{}",
        std::process::id()
    ));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(work.join("main/diary")).unwrap();
    fs::create_dir_all(work.join("palace")).unwrap();
    let people = work.join("people.json");
    fs::write(
        &people,
        json!({"people":{
            "elira":{"owner":true,"handles":[]},"wren":{"handles":[]}},
            "conversations":[{"match":"invented:private","people":["wren"],"private":true}]
        })
        .to_string(),
    )
    .unwrap();
    let mine = work.join("mempalace");
    fs::write(&mine, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&mine, fs::Permissions::from_mode(0o700)).unwrap();
    let command = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_palace"));
        c.arg(format!("--roots={}", work.display()))
            .arg("--people-map")
            .arg(&people);
        c
    };
    let write = |identity: &[&str]| {
        command()
            .arg("diary-write")
            .args(identity)
            .arg("--palace")
            .arg(work.join("palace"))
            .arg("--diary-root")
            .arg(work.join("main"))
            .arg("--mempalace")
            .arg(&mine)
            .args([
                "--text",
                "Invented amber glider continuity note.",
                "--title",
                "Invented decision",
            ])
            .output()
            .unwrap()
    };
    for identity in [&[][..], &["--conversation=invented:unknown"][..]] {
        let output = write(identity);
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap().lines().count(), 1);
        assert_eq!(fs::read_dir(work.join("main/diary")).unwrap().count(), 0);
        assert!(!work.join("palace/compiler.sqlite").exists());
    }
    for (identity, reader, other, source) in [
        (
            "--conversation=invented:private",
            "wren",
            "elira",
            "invented:private",
        ),
        ("--owner", "elira", "wren", "owner-private:diary"),
    ] {
        let output = write(&[identity]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["mined"], true);
        let file = PathBuf::from(result["file"].as_str().unwrap());
        assert_eq!(file.parent().unwrap(), work.join("main/diary"));
        assert!(
            fs::read_to_string(&file)
                .unwrap()
                .contains("Invented amber glider")
        );
        let store = ClaimStore::open(&work.join("palace/compiler.sqlite")).unwrap();
        let markers = store
            .privacy_markers("entry", file.file_name().unwrap().to_str().unwrap())
            .unwrap();
        assert_eq!(markers.len(), 1);
        let marker = &markers[0];
        assert!(marker.private && !marker.quarantine);
        assert_eq!(marker.source, source);
        assert!(marker.admits(&[reader.into()]));
        assert!(!marker.admits(&[other.into()]));
        assert!(!marker.admits(&[reader.into(), other.into()]));
        assert_eq!(marker.people, [reader.to_owned()].into());
    }
    assert_eq!(fs::read_dir(work.join("main/diary")).unwrap().count(), 2);
    let conflicting = write(&["--owner", "--conversation=invented:private"]);
    assert!(!conflicting.status.success());
    assert_eq!(fs::read_dir(work.join("main/diary")).unwrap().count(), 2);

    // Minimal invented Chroma schema for the existing read-only drawer counter.
    let db = Connection::open(work.join("palace/chroma.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE collections(id TEXT,name TEXT); CREATE TABLE segments(id TEXT,collection TEXT,scope TEXT); CREATE TABLE embeddings(id TEXT,segment_id TEXT);").unwrap();
    drop(db);
    let status = |identity: &[&str]| {
        command()
            .arg("status")
            .args(identity)
            .arg("--store")
            .arg(work.join("palace/compiler.sqlite"))
            .arg("--palace")
            .arg(work.join("palace"))
            .output()
            .unwrap()
    };
    for identity in [
        &[][..],
        &["--conversation=invented:private"][..],
        &["--owner", "--conversation=invented:private"][..],
    ] {
        let output = status(identity);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let output = status(&["--owner"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result,
        json!({"counts":{"entries":2,"drawers":0,"claims":0,"findings":0,"improvement_events":0}})
    );
    assert!(!String::from_utf8(output.stdout).unwrap().contains("amber"));
    // Verify resolution separately without configuring process-global privacy identity.
    let people = privacy::People::parse(&fs::read_to_string(people).unwrap()).unwrap();
    assert!(people.resolve("invented:private", None).private);
}
