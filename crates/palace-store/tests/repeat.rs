//! M5b-5 item 1 (review M-1): a finding that recurs on a (claim, kind) decided by a standing
//! `rule:auto` event on an unchanged claim is a repeat (no new finding row, listed by `repeats`);
//! after the event's undo, or once the claim's status changed, it is a new finding again.
use palace_core::diary_memory::Memory;
use palace_core::{claim, rollup::Entry};
use palace_store::{
    auto::candidates,
    claims::{Checked, ClaimStore},
    events::{self, Event, Side},
    lookup::FindingFilter,
    repeat,
};
use serde_json::{Value, json};
use std::path::Path;

/// PUB-1a item 2: the invented cast as the person and the assistant, set once per test binary
/// before any name is read (every test here calls it first).
fn cast() {
    static CAST: std::sync::Once = std::sync::Once::new();
    CAST.call_once(|| palace_core::names::set(Some("callum"), Some("fergus")).unwrap());
}

const TEXT: &str = "## Claims\n- C3. Callum | chose | Valkey — 2026-09-21 — gone.md\n";

fn make_kg(dir: &Path) -> std::path::PathBuf {
    let kg = dir.join("knowledge_graph.sqlite3");
    rusqlite::Connection::open(&kg)
        .unwrap()
        .execute_batch("CREATE TABLE IF NOT EXISTS entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);")
        .unwrap();
    kg
}

/// One watch of the rollup at `at`: C3's source is absent, so it raises `unsupported`.
fn watch(dir: &Path, at: &str) {
    let parsed = claim::parse(TEXT);
    let mut rows = claim::check(
        &Entry::parse("r", TEXT.into()),
        &parsed.claims,
        &Memory::default(),
        None::<&palace_core::diary_memory::FakeJudge>,
    );
    let checked = Checked {
        rollup: "r",
        date: "2026-09-22",
        thread: "t",
        judge: "judge:fake",
        claims: &parsed.claims,
        rows: &mut rows,
    };
    let mut store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    store.record(checked, at).unwrap();
}

/// A `rule:auto` event deciding `finding` (as `improve --auto` would), and the claim's state
/// right after it recorded on the finding. Returns the event id.
fn decide_by_auto(dir: &Path, finding: &str, claim: &str, at: &str) -> String {
    let c = rusqlite::Connection::open(dir.join("compiler.sqlite")).unwrap();
    let decision = json!({"by":"rule:auto","at":at,"verb":"expire","note":null});
    let change = events::change("finding", finding, Value::Null, decision);
    events::apply(&c, &change, Side::After).unwrap();
    let event = Event {
        by: "rule:auto",
        verb: "expire",
        from_claim: Some(claim),
        to_claim: None,
        finding: Some(finding),
        text: "test",
        axis: &["currency"],
        changes: vec![change],
    };
    let id = events::append(&c, &event, at).unwrap();
    drop(c);
    let mut store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    let after = store.claim(claim).unwrap().unwrap();
    let detail = repeat::detail(&id, &after);
    store
        .mark(finding, &repeat::mark_name(&id), claim, &detail)
        .unwrap();
    id
}

fn findings(dir: &Path) -> Vec<Value> {
    let store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    store.findings(&FindingFilter::default()).unwrap()
}

#[test]
fn a_standing_auto_event_is_acted_on_once() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let (compiler, kg) = (dir.path().join("compiler.sqlite"), make_kg(dir.path()));
    watch(dir.path(), "2026-09-22T00:00:00Z");
    let open = candidates(&compiler, &kg).unwrap();
    let finding = open[0].finding["id"].as_str().unwrap().to_owned();
    let claim = open[0].claim["id"].as_str().unwrap().to_owned();
    let event = decide_by_auto(dir.path(), &finding, &claim, "2026-09-22T00:00:01Z");
    watch(dir.path(), "2026-09-23T00:00:00Z");
    watch(dir.path(), "2026-09-24T00:00:00Z");
    let all = findings(dir.path());
    assert_eq!(all.len(), 1, "a repeat inserts no finding: {all:?}");
    assert_eq!(all[0]["seen_count"], 3, "seen once per run");
    let repeats = repeat::repeats(&compiler, &kg).unwrap();
    assert_eq!(repeats.len(), 1);
    assert_eq!(
        (repeats[0].event.as_str(), &repeats[0].finding["id"]),
        (event.as_str(), &json!(finding))
    );
    assert!(
        candidates(&compiler, &kg).unwrap().is_empty(),
        "nothing to apply again"
    );
}

/// A fresh store after one watch and a `rule:auto` event deciding C3's finding: (dir, claim,
/// event).
fn decided() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let (compiler, kg) = (dir.path().join("compiler.sqlite"), make_kg(dir.path()));
    watch(dir.path(), "2026-09-22T00:00:00Z");
    let open = candidates(&compiler, &kg).unwrap();
    let finding = open[0].finding["id"].as_str().unwrap().to_owned();
    let claim = open[0].claim["id"].as_str().unwrap().to_owned();
    let event = decide_by_auto(dir.path(), &finding, &claim, "2026-09-22T00:00:01Z");
    (dir, claim, event)
}

#[test]
fn a_changed_claim_makes_a_new_finding() {
    cast();
    let (dir, claim, _) = decided();
    let (compiler, kg) = (dir.path().join("compiler.sqlite"), make_kg(dir.path()));
    let c = rusqlite::Connection::open(&compiler).unwrap();
    c.execute("UPDATE claims SET status='disputed' WHERE id=?1", [&claim])
        .unwrap();
    drop(c);
    watch(dir.path(), "2026-09-23T00:00:00Z");
    assert_eq!(
        findings(dir.path()).len(),
        2,
        "status changed: a new finding"
    );
    assert!(repeat::repeats(&compiler, &kg).unwrap().is_empty());
}

#[test]
fn an_undone_event_makes_a_new_finding() {
    cast();
    let (dir, claim, event) = decided();
    let c = rusqlite::Connection::open(dir.path().join("compiler.sqlite")).unwrap();
    let undo = Event {
        by: "callum",
        verb: "undo",
        from_claim: Some(&claim),
        to_claim: None,
        finding: None,
        text: "undo",
        axis: &["currency"],
        changes: vec![],
    };
    events::insert(&c, &events::undo_id(&event), &undo, "2026-09-22T00:00:02Z").unwrap();
    drop(c);
    watch(dir.path(), "2026-09-23T00:00:00Z");
    assert_eq!(findings(dir.path()).len(), 2, "event undone: a new finding");
}
