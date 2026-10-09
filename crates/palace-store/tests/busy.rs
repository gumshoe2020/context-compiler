//! M5a item 3 (review S-1): a write that meets another connection's `BEGIN IMMEDIATE` past the
//! busy timeout is a typed `Busy` naming the database held — the KG or the compiler store (both
//! WAL: the read-only probe of M5b-1 item 9 cannot single out a WAL writer, so every database
//! the process opened is named).
use palace_store::{
    busy::{Busy, named},
    graph::Graph,
    hold::Held,
};
use serde_json::json;

fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let kg = dir.path().join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP,FOREIGN KEY(subject) REFERENCES entities(id),FOREIGN KEY(object) REFERENCES entities(id));").unwrap();
    dir
}

fn add_while_held(dir: &std::path::Path, held: &str) -> String {
    let (compiler, kg) = (
        dir.join("compiler.sqlite"),
        dir.join("knowledge_graph.sqlite3"),
    );
    let mut graph = Graph::open(&compiler, &kg, None).unwrap();
    let holder = Held::begin(&dir.join(held)).unwrap();
    let error = graph
        .call(
            "add",
            &json!({"subject":"a","predicate":"supports","object":"b"}),
        )
        .unwrap_err();
    drop(graph);
    let error = named(error);
    drop(holder);
    assert!(Busy::is(&error), "{error}");
    error.to_string()
}

#[test]
fn a_held_kg_or_store_is_busy_and_named() {
    let dir = setup();
    let kg = add_while_held(dir.path(), "knowledge_graph.sqlite3");
    assert!(kg.starts_with("retry later: KG "), "{kg}");
    assert!(
        kg.contains("knowledge_graph.sqlite3") && kg.contains("database is locked"),
        "{kg}"
    );
    let store = add_while_held(dir.path(), "compiler.sqlite");
    let held = dir.path().join("compiler.sqlite");
    assert!(
        store.starts_with("retry later: ")
            && store.contains(&format!("compiler store {}", held.display()))
            && store.contains("database is locked"),
        "{store}"
    );
    let other: palace_store::Error = "no such table".into();
    assert!(!Busy::is(&named(other)));
}
