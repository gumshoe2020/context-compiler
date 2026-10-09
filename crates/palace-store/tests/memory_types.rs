//! SETTLE-1b item 1 (spec §4.3, test 14): the claim store migration to memory types.
use palace_core::diary_memory::Memory;
use palace_core::{claim, rollup::Entry};
use palace_store::claims::{Checked, ClaimStore, memory_types_backup};
use rusqlite::Connection;
use std::path::Path;

/// A store as a binary from before the migration left it: claims recorded, no `kind` or
/// `transcript` column, no backup.
fn store_before_migration(path: &Path) {
    let text = "## Claims\n- C1. Maren | chose | Rust — 2026-09-21 — source.md\n- C2. Maren | chose | Go — 2026-09-21 — missing.md\n";
    let entry = Entry::parse("r", text.into());
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary.0.insert("source".into(), "Maren chose Rust".into());
    let mut rows = claim::check(
        &entry,
        &parsed.claims,
        &diary,
        None::<&palace_core::diary_memory::FakeJudge>,
    );
    let mut store = ClaimStore::open(path).unwrap();
    let checked = Checked {
        rollup: "r",
        date: "2026-09-22",
        thread: "t",
        judge: "judge:fake",
        claims: &parsed.claims,
        rows: &mut rows,
    };
    store.record(checked, "2026-09-22T00:00:00Z").unwrap();
    drop(store);
    let c = Connection::open(path).unwrap();
    c.execute_batch(
        "ALTER TABLE claims DROP COLUMN kind; ALTER TABLE claims DROP COLUMN transcript;",
    )
    .unwrap();
    drop(c);
    std::fs::remove_file(memory_types_backup(path)).ok();
}

fn columns(path: &Path) -> Vec<String> {
    let c = Connection::open(path).unwrap();
    let mut s = c
        .prepare("SELECT name FROM pragma_table_info('claims') ORDER BY cid")
        .unwrap();
    s.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn kinds(path: &Path) -> Vec<(String, Option<String>)> {
    let c = Connection::open(path).unwrap();
    let mut s = c
        .prepare("SELECT kind, transcript FROM claims ORDER BY id")
        .unwrap();
    s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn every_existing_claim_becomes_a_concept_after_a_backup_and_a_rerun_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compiler.sqlite");
    store_before_migration(&path);
    assert!(!columns(&path).contains(&"kind".to_owned()));
    let before = std::fs::read(&path).unwrap();

    let store = ClaimStore::open(&path).unwrap();
    assert_eq!(store.counts().unwrap().0, 2);
    drop(store);
    let backup = memory_types_backup(&path);
    assert!(
        backup.is_file(),
        "the store is copied before it is migrated"
    );
    assert!(!columns(&backup).contains(&"kind".to_owned()));
    assert_eq!(kinds(&path), vec![("concept".to_owned(), None); 2]);
    assert!(!before.is_empty());

    let (columns_once, backup_once) = (columns(&path), std::fs::read(&backup).unwrap());
    let store = ClaimStore::open(&path).unwrap();
    store.verify_chain().unwrap();
    drop(store);
    assert_eq!(columns(&path), columns_once, "a re-run adds nothing");
    assert_eq!(
        std::fs::read(&backup).unwrap(),
        backup_once,
        "the first copy is kept"
    );
    assert_eq!(kinds(&path), vec![("concept".to_owned(), None); 2]);
}

#[test]
fn a_new_store_is_created_migrated_without_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compiler.sqlite");
    drop(ClaimStore::open(&path).unwrap());
    let names = columns(&path);
    assert!(names.contains(&"kind".to_owned()) && names.contains(&"transcript".to_owned()));
    assert!(!memory_types_backup(&path).exists());
}
