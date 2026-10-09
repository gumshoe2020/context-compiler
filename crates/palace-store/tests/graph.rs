use palace_store::{graph::Graph, graph_checks};
use serde_json::json;
fn setup() -> (tempfile::TempDir, Graph) {
    let dir = tempfile::tempdir().unwrap();
    let kg = dir.path().join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP,FOREIGN KEY(subject) REFERENCES entities(id),FOREIGN KEY(object) REFERENCES entities(id));").unwrap();
    drop(c);
    let g = Graph::open(&dir.path().join("palace.sqlite"), &kg, None).unwrap();
    (dir, g)
}
#[test]
fn bridge_confidence_and_types() {
    let (_d, mut g) = setup();
    graph_checks::confidence_types(&mut g).unwrap();
}
#[test]
fn bridge_omitted_types_preserve_properties() {
    let (_d, mut g) = setup();
    graph_checks::omitted_types(&mut g).unwrap();
}
#[test]
fn bridge_live_duplicate_is_not_reinserted() {
    let (_d, mut g) = setup();
    graph_checks::duplicate(&mut g).unwrap();
}
#[test]
fn temporal_retirement_schema_and_audit() {
    let (d, mut g) = setup();
    let before = g.schema().unwrap();
    assert!(g.call("add",&json!({"subject":"a","predicate":"supports","object":"b","valid_from":"2026-09-24","valid_to":"2026-09-23"})).is_err());
    assert_eq!(g.call("stats", &json!({})).unwrap()["entities"], 0);
    g.call(
        "add",
        &json!({"subject":"a","predicate":"supports","object":"b","valid_from":"2026-09-24"}),
    )
    .unwrap();
    assert!(
        g.call(
            "invalidate",
            &json!({"subject":"a","predicate":"supports","object":"b","ended":"2026-09-23"})
        )
        .is_err()
    );
    assert_eq!(
        g.call(
            "retire_entity",
            &json!({"name":"a","by":"test","reason":"done"})
        )
        .unwrap()["refused"],
        true
    );
    g.call(
        "invalidate",
        &json!({"subject":"a","predicate":"supports","object":"b","ended":"2026-09-24"}),
    )
    .unwrap();
    assert_eq!(
        g.call(
            "query",
            &json!({"entity":"a","as_of":"2026-09-24T23:59:59+00:00"})
        )
        .unwrap()["count"],
        1
    );
    assert_eq!(
        g.call("query", &json!({"entity":"a","as_of":"2026-09-25"}))
            .unwrap()["count"],
        0
    );
    assert_eq!(
        g.call(
            "retire_entity",
            &json!({"name":"a","by":"test","reason":"done"})
        )
        .unwrap()["changed"],
        true
    );
    assert_eq!(
        g.call(
            "retire_entity",
            &json!({"name":"a","by":"test","reason":"again"})
        )
        .unwrap()["changed"],
        false
    );
    assert_eq!(g.schema().unwrap(), before);
    g.verify_chain().unwrap();
    let lines = std::fs::read_to_string(d.path().join("wal/write_log.jsonl")).unwrap();
    assert_eq!(lines.lines().count(), 5);
}

#[test]
fn chain_rejects_tampering_blocks_writes_and_keeps_reads() {
    let (dir, mut g) = setup();
    g.call(
        "add",
        &json!({"subject":"a","predicate":"supports","object":"b"}),
    )
    .unwrap();
    drop(g);
    let audit = dir.path().join("palace.sqlite");
    let kg = dir.path().join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&audit).unwrap();
    assert!(
        c.execute("UPDATE write_log SET payload='tampered'", [])
            .is_err()
    );
    assert!(c.execute("DELETE FROM write_log", []).is_err());
    c.execute_batch("DROP TRIGGER write_log_no_update; UPDATE write_log SET payload='tampered';")
        .unwrap();
    drop(c);
    assert!(palace_store::audit::verify(&audit).is_err());
    assert!(Graph::open(&audit, &kg, None).is_err());
    let reader = palace_store::graph::GraphReader::open(&kg).unwrap();
    assert_eq!(reader.call("stats", &json!({})).unwrap()["entities"], 2);
}

#[test]
fn graph_reads_ignore_writer_locks_and_torn_foreign_wal() {
    let (dir, mut writer) = setup();
    writer
        .call(
            "add",
            &json!({"subject":"a","predicate":"supports","object":"b"}),
        )
        .unwrap();
    std::fs::write(dir.path().join("wal/write_log.jsonl"), "torn").unwrap();
    let reader =
        palace_store::graph::GraphReader::open(&dir.path().join("knowledge_graph.sqlite3"))
            .unwrap();
    assert_eq!(
        reader.call("query", &json!({"entity":"a"})).unwrap()["count"],
        1
    );
    assert_eq!(
        reader
            .call("entities", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(reader.call("add", &json!({})).is_err());
}

#[test]
fn committed_mutations_warn_and_retry_pending_wal_then_recover_torn_line() {
    let (dir, mut graph) = setup();
    std::fs::write(dir.path().join("wal"), "obstacle").unwrap();
    let args = json!({"subject":"a","predicate":"p","object":"b"});
    let result = graph.call("add", &args).unwrap();
    assert_eq!(result["ok"], true);
    assert!(result["wal_warning"].is_string());
    assert_eq!(graph.call("stats", &json!({})).unwrap()["triples"], 1);
    std::fs::remove_file(dir.path().join("wal")).unwrap();
    std::fs::create_dir(dir.path().join("wal")).unwrap();
    std::fs::write(dir.path().join("wal/write_log.jsonl"), "partial").unwrap();
    assert!(
        graph
            .call("add", &args)
            .unwrap()
            .get("wal_warning")
            .is_none()
    );
    let text = std::fs::read_to_string(dir.path().join("wal/write_log.jsonl")).unwrap();
    assert!(text.starts_with("partial\n{"));
    assert_eq!(
        text.lines()
            .skip(1)
            .filter(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
            .count(),
        2
    );
    graph.verify_chain().unwrap();
}

#[test]
fn typed_graph_reads_preserve_confidence_temporals_and_retired_properties() {
    use palace_core::{traits::GraphStore, triple::Direction};
    let (_dir, mut graph) = setup();
    graph.call("add", &json!({"subject":"a","predicate":"p","object":"b","source_closet":"{\"confidence\":0.7}"})).unwrap();
    let rows = graph.query_entity("a", Direction::Outgoing, None).unwrap();
    assert_eq!(rows[0].confidence, Some(1.0));
    assert_eq!(rows[0].clone().surface_confidence().confidence, Some(0.7));
    assert!(rows[0].valid_to.is_none());
    assert_eq!(graph.list_entities().unwrap().len(), 2);
    graph
        .call(
            "set_entity",
            &json!({"name":"orphan","properties":{"retired":{"reason":"test"}}}),
        )
        .unwrap();
    assert!(graph.entity("orphan").unwrap().unwrap().retired().is_some());
}
