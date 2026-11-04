//! All scratch stays beneath the INIT-1b test root.
use palace_store::claims::ClaimStore;
use std::path::PathBuf;
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/init1b/store-tests");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("{name}-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}
#[test]
fn additive_migration_and_person_status_filters() {
    let path = scratch("migration");
    let store = ClaimStore::open(&path).unwrap();
    assert!(store.items(None, None).unwrap().is_empty());
    let counts = store.counts().unwrap();
    drop(store);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("DROP TABLE open_items;").unwrap();
    drop(c);
    let store = ClaimStore::open(&path).unwrap();
    assert_eq!(store.counts().unwrap(), counts);
    drop(store);
    let c = rusqlite::Connection::open(&path).unwrap();
    for (id, person, status) in [
        ("send-map", "elora", "open"),
        ("book-room", "niven", "done"),
    ] {
        c.execute("INSERT INTO open_items(id,person,owner,kind,title,status,opened_at,touched_at,source,quote) VALUES (?1,?2,'person','task','Synthetic task',?3,'2031-04-01','2031-04-01','synthetic:1','I will do it')", rusqlite::params![id,person,status]).unwrap();
    }
    c.execute_batch("INSERT INTO privacy_markers SELECT 'item',id,'synthetic:ledger-fixture','[]','[]',0,0 FROM open_items").unwrap();
    drop(c);
    let store = ClaimStore::open(&path).unwrap();
    assert_eq!(store.items(None, None).unwrap().len(), 2);
    assert_eq!(
        store.items(Some("elora"), Some("open")).unwrap()[0]["times_raised"],
        0
    );
    assert!(store.items(Some("elora"), Some("done")).unwrap().is_empty());
    store.verify_chain().unwrap();
}
