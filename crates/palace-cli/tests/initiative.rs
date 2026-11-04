//! INIT-1c stand-ins; every file and child log stays in target/init1c.
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
#[test]
fn past_replay_uses_cited_dates_and_missing_timestamp_is_audited() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/init1c/past-replay");
    fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let db = dir.join("compiler.sqlite");
    let _ = fs::remove_file(&db);
    let transcript = dir.join("past.jsonl");
    let lines = [
        json!({"ts":"2001-04-01T12:00:00Z","role":"user","text":"I will send the chart."}),
        json!({"ts":"2001-04-01T13:00:00Z","role":"user","text":"Use the blue chart."}),
        json!({"ts":"2001-04-01T14:00:00Z","role":"user","text":"Okay sent."}),
        json!({"ts":"2001-04-01T15:00:00Z","role":"user","text":"The date has passed."}),
        json!({"role":"user","text":"I will file the sketch."}),
    ];
    fs::write(
        &transcript,
        lines.iter().map(|v| format!("{v}\n")).collect::<String>(),
    )
    .unwrap();
    let replies = dir.join("replies.json");
    fs::write(&replies, json!({
        "1-1":{"actions":[
            {"action":"create","id":"chart","owner":"person","kind":"task","title":"Send chart","line":1,"quote":"I will send the chart."},
            {"action":"create","id":"meeting","owner":"person","kind":"task","title":"Meeting","line":1,"quote":"I will send the chart."}]},
        "2-2":{"actions":[{"action":"update","id":"chart","line":2,"quote":"Use the blue chart."}]},
        "3-3":{"actions":[{"action":"close","id":"chart","status":"done","line":3,"close_quote":"Okay sent."}]},
        "4-4":{"actions":[{"action":"expire","id":"meeting","line":4,"close_quote":"The date has passed."}]},
        "5-5":{"actions":[{"action":"create","id":"sketch","owner":"person","kind":"task","title":"File sketch","line":5,"quote":"I will file the sketch."}]}
    }).to_string()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_palace"))
        .args(["items", "replay", "--person", "elora", "--store"])
        .arg(&db)
        .arg("--transcript")
        .arg(&transcript)
        .arg("--llm")
        .arg(format!("table:{}", replies.display()))
        .output()
        .unwrap();
    fs::write(dir.join("stdout.log"), &out.stdout).unwrap();
    fs::write(dir.join("stderr.log"), &out.stderr).unwrap();
    assert!(out.status.success());
    let store = palace_store::claims::ClaimStore::open(&db).unwrap();
    let rows = store.items(Some("elora"), None).unwrap();
    assert_eq!(rows[0]["opened_at"], "2001-04-01T12:00:00Z");
    assert_eq!(rows[0]["touched_at"], "2001-04-01T14:00:00Z");
    assert_eq!(rows[0]["closed_at"], "2001-04-01T14:00:00Z");
    assert_eq!(rows[1]["closed_at"], "2001-04-01T15:00:00Z");
    let events: Vec<Value> = store.events("items_action").unwrap();
    assert_eq!(events[2]["after"]["touched_at"], "2001-04-01T13:00:00Z");
    assert_eq!(store.events("items_timestamp_fallback").unwrap().len(), 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing ts"));
    store.verify_chain().unwrap();
}

#[test]
fn poke_and_skip_are_atomic_scoped_and_audited() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/init1c/poke-skip");
    fs::create_dir_all(&dir).unwrap();
    let db = dir.join("test.sqlite");
    let _ = fs::remove_file(&db);
    let mut store = palace_store::claims::ClaimStore::open(&db).unwrap();
    let at = "2001-04-01T12:00:00Z";
    store.apply_item_at("elora", &json!({"action":"create","id":"chart","owner":"person","kind":"task","title":"Chart","line":1,"quote":"Synthetic"}), "synthetic", Some(at)).unwrap();
    assert!(store.initiative_poked("niven", "chart", at).is_err());
    assert_eq!(store.initiative_state("niven").unwrap(), json!({"pokes":0}));
    assert_eq!(
        store.items(Some("elora"), None).unwrap()[0]["times_raised"],
        0
    );
    store
        .initiative_skipped("elora", at, 4 * 3_600_000)
        .unwrap();
    assert_eq!(
        store.initiative_state("elora").unwrap()["next_due_at"],
        "2001-04-01T16:00:00.000Z"
    );
    assert_eq!(store.initiative_state("elora").unwrap()["pokes"], 0);
    store.initiative_poked("elora", "chart", at).unwrap();
    store
        .initiative_poked("elora", "chart", "2001-04-01T20:00:00Z")
        .unwrap();
    let state = store.initiative_state("elora").unwrap();
    assert_eq!(state["pokes"], 2);
    assert_eq!(state["last_poke_at"], "2001-04-01T20:00:00Z");
    assert_eq!(state["next_due_at"], Value::Null);
    assert_eq!(
        store.items(Some("elora"), None).unwrap()[0]["times_raised"],
        2
    );
    assert_eq!(
        store.items(Some("elora"), None).unwrap()[0]["touched_at"],
        at
    );
    assert!(store.initiative_poked("elora", "chart", "invalid").is_err());
    assert_eq!(store.events("initiative_poked").unwrap().len(), 2);
    assert_eq!(store.events("initiative_skipped").unwrap().len(), 1);
    store.verify_chain().unwrap();
}
