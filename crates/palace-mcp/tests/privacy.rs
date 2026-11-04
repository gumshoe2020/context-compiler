use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
#[test]
fn diary_identity_is_adapter_owned_and_missing_conversation_refuses() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root.join(format!("target/priv1/mcp-diary-{}", std::process::id()));
    fs::create_dir_all(work.join("root/diary")).unwrap();
    fs::create_dir_all(work.join("palace")).unwrap();
    for (index, session) in [None, Some("synthetic-private"), Some("synthetic-shared")]
        .iter()
        .enumerate()
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_palace-mcp"));
        command
            .arg("--palace")
            .arg(work.join("palace"))
            .arg("--diary-root")
            .arg(work.join("root"))
            .arg("--mempalace")
            .arg(root.join("testdata/privacy/memory.py"))
            .arg("--people-map-if-exists")
            .arg(root.join("testdata/privacy/people.json"))
            .env_remove("CONTEXT_COMPILER_CONVERSATION");
        if let Some(session) = session {
            command
                .arg("--conversation-env")
                .arg("CONTEXT_COMPILER_CONVERSATION")
                .env("CONTEXT_COMPILER_CONVERSATION", session);
        }
        let stdout = work.join(format!("{index}.stdout"));
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(work.join(format!("{index}.stderr"))).unwrap())
            .spawn()
            .unwrap();
        let input = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"palace_diary_write","arguments":{"title":"Invented plan","wing":"archive","text":"Zebulon Quarrie uses cobalt heron.","conversation":"synthetic-shared"}}});
        writeln!(child.stdin.take().unwrap(), "{input}").unwrap();
        assert!(child.wait().unwrap().success());
        let reply: Value =
            serde_json::from_str(fs::read_to_string(stdout).unwrap().trim()).unwrap();
        if session.is_none() {
            assert!(
                reply["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("authenticated conversation")
            );
            assert_eq!(fs::read_dir(work.join("root/diary")).unwrap().count(), 0);
        } else {
            let result: Value =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            let name = std::path::Path::new(result["file"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            let store =
                palace_store::claims::ClaimStore::open(&work.join("palace/compiler.sqlite"))
                    .unwrap();
            let markers = store.privacy_markers("entry", name).unwrap();
            assert_eq!(markers[0].private, *session == Some("synthetic-private"));
            if markers[0].private {
                assert!(markers[0].audience.contains("wren"));
            }
            assert_eq!(markers[0].source, session.unwrap());
        }
    }
    let stdout = work.join("metadata.stdout");
    let mut child = Command::new(env!("CARGO_BIN_EXE_palace-mcp"))
        .arg("--palace")
        .arg(work.join("palace"))
        .arg("--diary-root")
        .arg(work.join("root"))
        .arg("--mempalace")
        .arg(root.join("testdata/privacy/memory.py"))
        .arg("--people-map")
        .arg(root.join("testdata/privacy/people.json"))
        .stdin(Stdio::piped())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(work.join("metadata.stderr")).unwrap())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for index in 0..3 {
        let mut params = json!({"name":"palace_diary_write","arguments":{"text":"Invented metadata source.","conversation":"synthetic-shared"}});
        if index == 0 {
            params["_meta"] = json!({"context_compiler":{"conversation":"synthetic-private"}});
        }
        if index == 2 {
            params = json!({"name":"mempalace_kg_add","arguments":{"subject":"Invented Finch","predicate":"uses","object":"amber kite","source_closet":"{}"}});
        }
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":index,"method":"tools/call","params":params})
        )
        .unwrap();
    }
    drop(input);
    assert!(child.wait().unwrap().success());
    let lines = fs::read_to_string(stdout).unwrap();
    let replies: Vec<Value> = lines
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(replies[0]["result"].is_object());
    assert!(
        replies[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("authenticated conversation")
    );
    assert!(
        replies[2]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("authenticated conversation")
    );
}

#[test]
fn diary_tags_narrow_shared_conversation_before_indexing() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root.join(format!("target/priv1c/mcp-tags-{}", std::process::id()));
    fs::create_dir_all(work.join("root/diary")).unwrap();
    fs::create_dir_all(work.join("palace")).unwrap();
    let people = work.join("people.json");
    fs::write(&people, r#"{"people":{"ann":{"handles":[]},"wren":{"handles":[]}},"conversations":[{"match":"synthetic-shared","people":["ann"]}],"private_tags":{"wren-private":["wren"]}}"#).unwrap();
    for (index, text) in [
        "Invented teal kite.\nTags: wren-private.",
        "---\ntags: [wren-private]\n---\nInvented copper finch.",
        "---\ntags:\n  - wren-private\n---\nInvented silver tern.",
    ]
    .iter()
    .enumerate()
    {
        let stdout = work.join(format!("{index}.stdout"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_palace-mcp"))
            .arg("--palace")
            .arg(work.join("palace"))
            .arg("--diary-root")
            .arg(work.join("root"))
            .arg("--mempalace")
            .arg(root.join("testdata/privacy/memory.py"))
            .arg("--people-map")
            .arg(&people)
            .arg("--conversation-env")
            .arg("CONTEXT_COMPILER_CONVERSATION")
            .env("CONTEXT_COMPILER_CONVERSATION", "synthetic-shared")
            .stdin(Stdio::piped())
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(work.join(format!("{index}.stderr"))).unwrap())
            .spawn()
            .unwrap();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"palace_diary_write","arguments":{"text":text,"title":"Invented tagged note"}}});
        writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
        assert!(child.wait().unwrap().success());
        let reply: Value =
            serde_json::from_str(fs::read_to_string(stdout).unwrap().trim()).unwrap();
        let result: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(result["mined"], true);
        let file = std::path::Path::new(result["file"].as_str().unwrap())
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let store =
            palace_store::claims::ClaimStore::open(&work.join("palace/compiler.sqlite")).unwrap();
        let markers = store.privacy_markers("entry", file).unwrap();
        assert_eq!(markers.len(), 1);
        assert!(
            markers[0].private
                && markers[0].admits(&["wren".into()])
                && !markers[0].admits(&["ann".into()])
        );
        assert_eq!(markers[0].source, "synthetic-shared");
        assert_eq!(store.privacy_source(file).unwrap(), markers[0]);
    }
}
