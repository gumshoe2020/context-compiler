//! M4d-2b pre-item S-1 (REVIEW-opus55-m4d2a1 S-1): a `set_entity` on an existing entity is
//! journaled column by column, so recovery sees and repairs a torn entity write — the review's
//! two probes (KG half committed without its event; event committed, KG half lost).
use palace_store::graph::{Graph, check_divergence};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// PUB-1a item 2: the invented cast as the person and the assistant, set once per test binary
/// before any name is read (every test here calls it first).
fn cast() {
    static CAST: std::sync::Once = std::sync::Once::new();
    CAST.call_once(|| palace_core::names::set(Some("callum"), Some("fergus")).unwrap());
}

fn paths(dir: &Path) -> (PathBuf, PathBuf) {
    (
        dir.join("compiler.sqlite"),
        dir.join("knowledge_graph.sqlite3"),
    )
}

fn setup(dir: &Path) {
    let (_, kg) = paths(dir);
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);").unwrap();
    drop(c);
    set(dir, json!({})).unwrap();
}

fn open(dir: &Path) -> Graph {
    let (compiler, kg) = paths(dir);
    Graph::open(&compiler, &kg, None).unwrap()
}

fn set(dir: &Path, properties: Value) -> Result<Value, palace_store::Error> {
    open(dir).call(
        "set_entity",
        &json!({"name":"palace-graph-wiring","type":"project","properties":properties}),
    )
}

fn properties(dir: &Path) -> Value {
    let c = rusqlite::Connection::open(paths(dir).1).unwrap();
    let text: String = c
        .query_row(
            "SELECT properties FROM entities WHERE id='palace-graph-wiring'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_torn_entity_write_without_its_event_is_rolled_back() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (compiler, kg) = paths(dir.path());
    palace_store::crash::fail_event_half(&compiler).unwrap();
    assert!(
        set(dir.path(), json!({"probe":"P1"})).is_err(),
        "the staged crash"
    );
    palace_store::crash::clear(&compiler).unwrap();
    assert!(palace_store::crash::commit_kg_half(&compiler, &kg).unwrap() >= 1);
    assert_eq!(properties(dir.path())["probe"], "P1", "KG half alone");
    let pending = check_divergence(&compiler, &kg).unwrap();
    assert!(pending[0].contains("recovered_rollback"), "{pending:?}");
    let acted = open(dir.path()).reconcile().unwrap();
    assert_eq!(acted[0]["operation"], "recovered_rollback");
    assert_eq!(properties(dir.path()), json!({}), "the rollback is real");
    assert!(check_divergence(&compiler, &kg).unwrap().is_empty());
}

#[test]
fn a_lost_entity_write_is_reapplied() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (compiler, kg) = paths(dir.path());
    set(dir.path(), json!({"probe":"P2"})).unwrap();
    palace_store::crash::lose_kg_half(&compiler, &kg).unwrap();
    assert_eq!(properties(dir.path()), json!({}), "KG half lost");
    let acted = open(dir.path()).reconcile().unwrap();
    assert_eq!(acted[0]["operation"], "recovered_reapply", "{acted:?}");
    assert_eq!(properties(dir.path())["probe"], "P2");
    assert!(check_divergence(&compiler, &kg).unwrap().is_empty());
}

/// M4d-2b pre-item S-2 (REVIEW-opus55-m4d2a1 S-2): the review's retry probe — after a torn write,
/// a writer that does not reconcile (`palace kg …`, `palace-mcp`) is refused instead of writing
/// over the torn row; once a reconciling command settled the intent, the retry succeeds and is
/// not reverted.
#[test]
fn a_retry_over_a_torn_commit_is_refused_until_reconciled() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (compiler, kg) = paths(dir.path());
    palace_store::crash::fail_event_half(&compiler).unwrap();
    assert!(
        set(dir.path(), json!({"probe":"P1"})).is_err(),
        "the staged crash"
    );
    palace_store::crash::clear(&compiler).unwrap();
    palace_store::crash::commit_kg_half(&compiler, &kg).unwrap();
    let retry = set(dir.path(), json!({"probe":"P3"})).unwrap_err();
    assert!(palace_store::Refused::is(&retry), "{retry}");
    assert!(retry.to_string().contains("unsettled intent"), "{retry}");
    assert_eq!(properties(dir.path())["probe"], "P1", "nothing written");
    open(dir.path()).reconcile().unwrap();
    set(dir.path(), json!({"probe":"P3"})).unwrap();
    assert!(
        open(dir.path()).reconcile().unwrap().is_empty(),
        "nothing to revert"
    );
    assert_eq!(properties(dir.path())["probe"], "P3");
}

/// A torn `set_entity` (KG half alone) whose row someone else then changed: DIVERGED.
fn diverged(dir: &Path, foreign: &str) -> String {
    let (compiler, kg) = paths(dir);
    palace_store::crash::fail_event_half(&compiler).unwrap();
    assert!(set(dir, json!({"probe":"P1"})).is_err(), "the staged crash");
    palace_store::crash::clear(&compiler).unwrap();
    palace_store::crash::commit_kg_half(&compiler, &kg).unwrap();
    palace_store::crash::hand_edit_entity(&kg, "palace-graph-wiring", foreign).unwrap();
    let error = open(dir).reconcile().unwrap_err();
    let diverged = palace_store::graph::Diverged::of(&error).expect("diverged");
    assert!(error.to_string().contains(&format!(
        "palace doctor --settle {} --keep-current|--restore-before --by callum|fergus",
        diverged.intent
    )));
    let retry = set(dir, json!({"probe":"P2"})).unwrap_err().to_string();
    assert!(retry.contains("palace doctor --settle k_"), "{retry}");
    diverged.intent.clone()
}

fn manual_lines(dir: &Path) -> Vec<Value> {
    palace_store::claims::ClaimStore::open(&paths(dir).0)
        .unwrap()
        .events("recovered_manual")
        .unwrap()
}

/// M4d-2c item 6 (review S-5): a DIVERGED intent is settled by a person's audited choice —
/// never by a rule, never when not diverged; keep-current leaves the foreign value,
/// restore-before writes `before` back; either way writes resume.
#[test]
fn a_diverged_intent_is_settled_by_hand() {
    cast();
    use palace_store::graph::Choice;
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let intent = diverged(dir.path(), r#"{"probe":"P9"}"#);
    let mut g = open(dir.path());
    assert!(g.settle(&intent, Choice::KeepCurrent, "rule:auto").is_err());
    let line = g.settle(&intent, Choice::KeepCurrent, "fergus").unwrap();
    assert_eq!(
        (line["choice"].as_str(), line["by"].as_str()),
        (Some("keep-current"), Some("fergus"))
    );
    assert!(
        g.settle(&intent, Choice::KeepCurrent, "fergus").is_err(),
        "settled once"
    );
    drop(g);
    assert_eq!(properties(dir.path())["probe"], "P9", "kept");
    let (compiler, kg) = paths(dir.path());
    assert!(check_divergence(&compiler, &kg).unwrap().is_empty());
    let again = diverged(dir.path(), r#"{"probe":"P8"}"#);
    open(dir.path())
        .settle(&again, Choice::RestoreBefore, "callum")
        .unwrap();
    assert_eq!(
        properties(dir.path())["probe"],
        "P9",
        "before = the kept P9"
    );
    assert_eq!(manual_lines(dir.path()).len(), 2);
    set(dir.path(), json!({"probe":"P3"})).unwrap();
    assert_eq!(properties(dir.path())["probe"], "P3", "writes resume");
}

/// M4d-2c item 6 (review S-5): `created_at` comes from the intent's time, so a rehearsal and the
/// real write agree whatever second each runs in.
#[test]
fn created_at_is_the_intents_time() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    set(dir.path(), json!({"probe":"P4"})).unwrap();
    let c = rusqlite::Connection::open(paths(dir.path()).0).unwrap();
    let at: String = c
        .query_row(
            "SELECT at FROM kg_intents ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let expected: String = c
        .query_row("SELECT datetime(?1)", [&at], |r| r.get(0))
        .unwrap();
    drop(c);
    let kg = rusqlite::Connection::open(paths(dir.path()).1).unwrap();
    let created: String = kg
        .query_row(
            "SELECT created_at FROM entities WHERE id='palace-graph-wiring'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(created, expected);
    assert_eq!(created.len(), 19, "CURRENT_TIMESTAMP's format");
}
