//! M5a item 2: the shadow diff reads both databases without changing them, finds a graph equal to
//! its logs, and names a hand-edited confidence.
use palace_core::shadow_diff::{compare, merge};
use palace_store::{graph::Graph, shadow_diff};
use serde_json::json;
use std::path::Path;

fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let kg = dir.path().join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP,FOREIGN KEY(subject) REFERENCES entities(id),FOREIGN KEY(object) REFERENCES entities(id));").unwrap();
    drop(c);
    let mut g = Graph::open(&dir.path().join("compiler.sqlite"), &kg, None).unwrap();
    for (object, confidence) in [("b", 0.8), ("c", 0.6)] {
        g.call(
            "add",
            &json!({"subject":"a","predicate":"supports","object":object,"confidence":confidence}),
        )
        .unwrap();
    }
    g.call(
        "invalidate",
        &json!({"subject":"a","predicate":"supports","object":"c","ended":"2026-09-27"}),
    )
    .unwrap();
    dir
}

fn hashes(dir: &Path) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && !p.to_string_lossy().ends_with("-shm"))
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (p.display().to_string(), palace_core::ids::sha256(&bytes))
        })
        .collect();
    files.sort();
    files
}

fn diff(dir: &Path) -> palace_core::shadow_diff::Outcome {
    let lines = shadow_diff::wal_lines(&dir.join("wal/write_log.jsonl")).unwrap();
    let (rows, log) = shadow_diff::read(
        &dir.join("compiler.sqlite"),
        &dir.join("knowledge_graph.sqlite3"),
        None,
    )
    .unwrap();
    compare(&merge(lines, log), &rows).unwrap()
}

#[test]
fn equal_on_a_clean_graph_read_only_and_a_hand_edit_is_named() {
    let dir = setup();
    let before = hashes(dir.path());
    let clean = diff(dir.path());
    assert!(clean.equal(), "{:?}", clean.differences);
    assert_eq!((clean.rows, clean.unkeyed), (2, 0));
    assert_eq!(hashes(dir.path()), before, "the diff wrote to a database");
    let c = rusqlite::Connection::open(dir.path().join("knowledge_graph.sqlite3")).unwrap();
    let id: String = c
        .query_row("SELECT id FROM triples WHERE valid_to IS NULL", [], |r| {
            r.get(0)
        })
        .unwrap();
    c.execute("UPDATE triples SET confidence=0.123 WHERE id=?1", [&id])
        .unwrap();
    drop(c);
    let edited = diff(dir.path());
    assert_eq!(edited.differences.len(), 1);
    assert_eq!(
        (
            edited.differences[0].triple.as_str(),
            edited.differences[0].field
        ),
        (id.as_str(), "confidence")
    );
}
