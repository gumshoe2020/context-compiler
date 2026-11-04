//! SETTLE-1k: invented people/content; every artifact stays below target/settle1k.
use palace_store::{claims::ClaimStore, privacy::Audience};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn separate_audiences_commit_resume_and_record_private_fidelity() {
    let root = root();
    let work = root.join(format!("target/settle1k/rollup-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    let diary = work.join("root/diary");
    fs::create_dir_all(&diary).unwrap();
    let palace = work.join("palace");
    fs::create_dir_all(&palace).unwrap();
    let kg = palace.join("knowledge_graph.sqlite3");
    Connection::open(&kg).unwrap().execute_batch(
        "CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP);
         CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,valid_from TEXT,valid_to TEXT,confidence REAL,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);"
    ).unwrap();
    let store_path = palace.join("compiler.sqlite");
    let mut store = ClaimStore::open(&store_path).unwrap();
    fs::write(work.join("canaries.json"), "[]").unwrap();
    let mut cases = Vec::new();
    for (n, audience, object) in [
        (1, "shared", "amber glider"),
        (2, "wren", "violet submarine"),
    ] {
        let name = format!("2031-05-10-{n:012x}.md");
        let sentence = format!("Elira Fen chose {object}.");
        fs::write(
            diary.join(&name),
            format!("# Observatory plan\n\nTags: dome-plan.\n\n{sentence}\n"),
        )
        .unwrap();
        store
            .mark_entry(&name, &audience.parse::<Audience>().unwrap().marker(&name))
            .unwrap();
        let mut body = "# ROLLUP: dome-plan — state as of 2031-05-10\n\n## Claims\n".to_owned();
        for i in 1..=3 {
            body += &format!("- C{i}. Elira Fen | chose | {object} — 2031-05-10 — {name}\n");
        }
        body += &format!(
            "\n## Current consolidated truth\n{}\n\n## Supersedes\n- {name}\n\n## Open items\n- None.\n\n## Graph relations (proposed)\n```json relations\n{}\n```\n",
            sentence.repeat(70),
            json!([{"from":"Elira Fen","to":object,"type":"chose","evidence":[{"query":object}],"confidence":0.85,"supplied_by":"synthetic","claim":"C1"}])
        );
        let reply = work.join(format!("{audience}.md"));
        fs::write(&reply, body).unwrap();
        cases.push((audience, name, object, reply));
    }
    drop(store);
    let run = |audience: &str, reply: &PathBuf| {
        let failing = audience == "each-fail";
        let selected = if failing { "each" } else { audience };
        let output = Command::new(env!("CARGO_BIN_EXE_palace"))
            .arg(format!("--roots={}", root.display()))
            .args(["--person", "orin", "--assistant", "finch"])
            .args([
                "rollup",
                "--audience",
                selected,
                "--min-cluster",
                "1",
                "--thread",
                "dome-plan",
                "--as-of",
                "2031-05-10",
            ])
            .arg("--diary")
            .arg(work.join("root"))
            .arg("--palace")
            .arg(&palace)
            .arg("--kg")
            .arg(&kg)
            .arg("--llm")
            .arg(format!(
                "command:{}",
                root.join("testdata/settle1k/audience-model.py").display()
            ))
            .arg("--canaries")
            .arg(work.join("canaries.json"))
            .arg("--prompts")
            .arg(root.join("prompts"))
            .arg("--mempalace")
            .arg(root.join("testdata/privacy/memory.py"))
            .arg("--mempalace-mcp")
            .arg(root.join("testdata/privacy/memory.py"))
            .env("PRIV1_DIARY", &diary)
            .env("PRIV1_REPLY", reply)
            .env("FAIL_SHARED", if failing { "yes" } else { "" })
            .env("EXCLUDE_WINGS", "")
            .output()
            .unwrap();
        fs::write(work.join(format!("{audience}.stdout")), &output.stdout).unwrap();
        fs::write(work.join(format!("{audience}.stderr")), &output.stderr).unwrap();
        assert!(
            output.status.success() != failing,
            "{audience}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run("each-fail", &work);
    {
        let c = Connection::open(&store_path).unwrap();
        assert!(
            c.query_row("SELECT count(*) FROM fidelity", [], |r| r.get::<_, i64>(0))
                .unwrap()
                > 0
        );
        let err = fs::read_to_string(work.join("each-fail.stderr")).unwrap();
        assert!(err.contains("audience-pass=0 failed"), "{err}");
    }
    run("each", &work);
    let c = Connection::open(&store_path).unwrap();
    let commits = || -> Vec<Value> {
        c.prepare("SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')='rollup_commit' ORDER BY sequence").unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| serde_json::from_str(&r.unwrap()).unwrap()).collect()
    };
    assert_eq!(commits().len(), 2);
    let mut events = commits();
    events.reverse();
    for ((audience, name, object, reply), event) in cases.iter().zip(events) {
        let file = event["file"].as_str().unwrap();
        let body = fs::read_to_string(diary.join(file)).unwrap();
        assert!(body.contains(name) && body.contains(object));
        let other = cases.iter().find(|case| case.0 != *audience).unwrap();
        assert!(!body.contains(&other.1) && !body.contains(other.2));
        let marker = palace_store::privacy::markers(&c, "entry", file).unwrap();
        assert!(
            !marker.is_empty()
                && marker
                    .iter()
                    .all(|m| audience.parse::<Audience>().unwrap().matches(m))
        );
        run(audience, reply);
        let output = Command::new(env!("CARGO_BIN_EXE_palace"))
            .arg(format!("--roots={}", root.display()))
            .args(["--person", "orin", "--assistant", "finch"])
            .args(["fidelity", "--rollup"])
            .arg(diary.join(file))
            .arg("--diary")
            .arg(work.join("root"))
            .arg("--kg")
            .arg(&kg)
            .arg("--prompts")
            .arg(root.join("prompts"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: i64 = c
            .query_row("SELECT count(*) FROM fidelity WHERE file=?1", [file], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(rows > 0);
        let fidelity_marker = palace_store::privacy::markers(&c, "fidelity", file).unwrap();
        assert!(
            fidelity_marker
                .iter()
                .all(|m| audience.parse::<Audience>().unwrap().matches(m))
        );
    }
    assert_eq!(
        commits().len(),
        2,
        "resume must not duplicate or cross audiences"
    );
    assert_eq!(
        ClaimStore::open(&store_path)
            .unwrap()
            .fidelity_counts(None)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn conversation_search_obeys_markers_and_unknown_identity_returns_nothing() {
    let root = root();
    let work = root.join(format!("target/settle1k/search-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(&work).unwrap();
    let store_path = work.join("compiler.sqlite");
    let people = work.join("people.json");
    fs::write(&people, json!({
        "people":{"ann":{"handles":[]},"wren":{"handles":[],"private":true},"cy":{"handles":[],"owner":true}},
        "conversations":[{"match":"session:ann","people":["ann"]},{"match":"session:wren","people":["wren"]},{"match":"session:cy","people":["cy"]},{"match":"session:group","people":["ann","wren"]}]
    }).to_string()).unwrap();
    let mut store = ClaimStore::open(&store_path).unwrap();
    let mut rows = Vec::new();
    // First 150 hits are private; the admitted shared hit must survive that ranked window.
    for i in 0..150 {
        let name = format!("hidden-{i:03}");
        store
            .mark_entry(&name, &"wren".parse::<Audience>().unwrap().marker(&name))
            .unwrap();
        rows.push(json!({"source_file":format!("{name}.md"),"source_path":format!("/invented/{name}.md"),"text":"violet submarine saturation","wing":"synthetic-private-wing","room":"invented","distance":0.1}));
    }
    for (name, audience, text) in [
        ("shared", "shared", "amber glider saturation"),
        ("group", "ann,wren", "copper telescope"),
        ("other", "cy", "indigo balloon"),
    ] {
        store
            .mark_entry(name, &audience.parse::<Audience>().unwrap().marker(name))
            .unwrap();
        rows.push(json!({"source_file":format!("{name}.md"),"source_path":format!("/invented/{name}.md"),"text":text,"wing":"another-synthetic-wing","room":"invented","distance":0.2}));
    }
    for i in 0..847 {
        let name = format!("filler-{i:03}");
        store
            .mark_entry(&name, &Audience::default().marker(&name))
            .unwrap();
        rows.push(json!({"source_file":format!("{name}.md"),"text":"invented filler"}));
    }
    store
        .mark_entry(
            "quarantine",
            &palace_store::privacy::Marker::quarantine("session:wren"),
        )
        .unwrap();
    store
        .mark_entry(
            "retired",
            &"shared".parse::<Audience>().unwrap().marker("retired"),
        )
        .unwrap();
    drop(store);
    let c = Connection::open(&store_path).unwrap();
    c.execute(
        "INSERT INTO privacy_retired VALUES ('entry','retired','retired')",
        [],
    )
    .unwrap();
    drop(c);
    for name in ["unmarked", "quarantine", "retired"] {
        rows.push(json!({"source_file":format!("{name}.md"),"source_path":format!("/invented/{name}.md"),"text":"denied crimson airship","wing":"invented","room":"invented"}));
    }
    fs::write(work.join("rows.json"), json!(rows).to_string()).unwrap();
    let before = fs::read(&store_path).unwrap();
    let run = |session: Option<&str>, query: &str| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_palace"));
        cmd.arg(format!("--roots={}", root.display()))
            .arg("--people-map")
            .arg(&people);
        if let Some(session) = session.filter(|s| *s != "--owner") {
            cmd.args(["--conversation", session]);
        }
        let started = std::time::Instant::now();
        cmd.arg("search");
        if session == Some("--owner") {
            cmd.arg("--owner");
        }
        let output = cmd
            .args(["--query", query, "--limit", "5"])
            .arg("--store")
            .arg(&store_path)
            .arg("--palace")
            .arg(&work)
            .arg("--mempalace-mcp")
            .arg(root.join("testdata/settle1k/search.py"))
            .env("SETTLE1K_SEARCH", &work)
            .output()
            .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let results = |session, query| -> Vec<Value> {
        let output = run(Some(session), query);
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["results"]
            .as_array()
            .unwrap()
            .clone()
    };
    assert!(results("session:ann", "violet submarine").is_empty());
    assert!(!results("session:wren", "violet submarine").is_empty());
    assert!(results("session:cy", "violet submarine").is_empty());
    for session in ["session:wren", "session:ann", "session:cy", "session:group"] {
        let shared = results(session, "amber glider");
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0]["source_file"], "shared.md");
    }
    assert!(results("session:group", "violet submarine").is_empty());
    assert_eq!(results("session:group", "copper telescope").len(), 1);
    assert!(results("session:cy", "copper telescope").is_empty());
    assert!(results("session:wren", "indigo balloon").is_empty());
    assert_eq!(
        results("session:ann", "saturation")[0]["source_file"],
        "shared.md"
    );
    for session in ["session:ann", "session:wren", "session:cy"] {
        assert!(results(session, "denied crimson airship").is_empty());
    }
    assert_eq!(results("--owner", "indigo balloon").len(), 1);
    assert_eq!(results("--owner", "amber glider").len(), 1);
    assert!(results("--owner", "violet submarine").is_empty());
    assert!(results("--owner", "copper telescope").is_empty());
    let starts = fs::read(work.join("server-starts")).unwrap();
    for session in [None, Some("unknown"), Some("session:wren:spoof")] {
        let output = run(session, "amber glider");
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("unknown or missing conversation identity")
        );
    }
    assert_eq!(
        fs::read(work.join("server-starts")).unwrap(),
        starts,
        "unknown identity must not start a search"
    );
    assert_eq!(
        fs::read(&store_path).unwrap(),
        before,
        "search must not mutate the store"
    );
}

#[test]
fn owner_search_refuses_missing_or_ambiguous_owners_and_conversation() {
    let root = root();
    let work = root.join(format!("target/settle1k-m3/owner-{}", std::process::id()));
    fs::create_dir_all(&work).unwrap();
    for people in [
        json!({"ann":{"handles":[]}}),
        json!({"ann":{"handles":[],"owner":true},"cy":{"handles":[],"owner":true}}),
    ] {
        let map = work.join("people.json");
        fs::write(
            &map,
            json!({"people":people,"conversations":[]}).to_string(),
        )
        .unwrap();
        for conflict in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_palace"));
            cmd.arg(format!("--roots={}", root.display()))
                .arg("--people-map")
                .arg(&map);
            if conflict {
                cmd.args(["--conversation", "invented"]);
            }
            let output = cmd
                .args([
                    "search", "--owner", "--store", "missing", "--palace", "missing", "--query",
                    "invented",
                ])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(
                err.contains(if conflict {
                    "cannot be used with"
                } else {
                    "exactly one owner"
                }),
                "{err}"
            );
        }
    }
}
