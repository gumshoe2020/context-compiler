//! M4d item 4 and 5 (store side): the read-only candidate view, asks recorded once and answered,
//! marks recorded once, and replies keyed by (token, normalised text).
use palace_core::diary_memory::Memory;
use palace_core::{claim, rollup::Entry};
use palace_store::{
    auto::{Ask, candidates},
    claims::{Checked, ClaimStore},
};
use serde_json::json;
use std::path::Path;

const TEXT: &str = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — s.md — quote: \"Callum chose Rust\"\n- C3. Callum | chose | Valkey — 2026-09-21 — gone.md\n";

fn setup(dir: &Path) {
    let kg = dir.join("knowledge_graph.sqlite3");
    rusqlite::Connection::open(&kg)
        .unwrap()
        .execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);")
        .unwrap();
    let parsed = claim::parse(TEXT);
    let mut diary = Memory::default();
    diary.0.insert("s".into(), "# s\nCallum chose Rust.".into());
    let mut rows = claim::check(
        &Entry::parse("r", TEXT.into()),
        &parsed.claims,
        &diary,
        None::<&palace_core::diary_memory::FakeJudge>,
    );
    let mut store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    let checked = Checked {
        rollup: "r",
        date: "2026-09-22",
        thread: "t",
        judge: "judge:fake",
        claims: &parsed.claims,
        rows: &mut rows,
    };
    store.record(checked, "2026-09-22T00:00:00Z").unwrap();
}

#[test]
fn asks_marks_and_replies_are_recorded_once() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (compiler, kg) = (
        dir.path().join("compiler.sqlite"),
        dir.path().join("knowledge_graph.sqlite3"),
    );
    let found = candidates(&compiler, &kg).unwrap();
    assert_eq!(found.len(), 1, "C3's absent source is the one open finding");
    assert!(!found[0].asked && !found[0].vetoed && !found[0].person_acted);
    let finding = found[0].finding["id"].as_str().unwrap().to_owned();
    let mut store = ClaimStore::open(&compiler).unwrap();
    let ask = Ask {
        finding: &finding,
        claim: found[0].claim["id"].as_str().unwrap(),
        kind: "ungrounded",
        token: "#f_x",
        line: "🎩 line",
        proposal: None,
    };
    assert!(store.record_ask(&ask, "2026-09-27T00:00:00Z").unwrap());
    assert!(
        !store.record_ask(&ask, "2026-09-27T00:00:01Z").unwrap(),
        "never twice"
    );
    assert_eq!(store.open_asks().unwrap().len(), 1);
    assert!(store.mark(&finding, "vetoed", "c", &json!({})).unwrap());
    assert!(!store.mark(&finding, "vetoed", "c", &json!({})).unwrap());
    drop(store);
    let again = candidates(&compiler, &kg).unwrap();
    assert!(again[0].asked && again[0].vetoed);
    let mut store = ClaimStore::open(&compiler).unwrap();
    store
        .answer_ask(&finding, "no", "2026-09-27T00:00:02Z")
        .unwrap();
    assert!(store.open_asks().unwrap().is_empty());
    assert_eq!(store.replied("#f_x", "no").unwrap(), None);
    store.record_reply("#f_x", "no", Some("e_1")).unwrap();
    assert_eq!(store.replied("#f_x", "no").unwrap().as_deref(), Some("e_1"));
    store.verify_chain().unwrap();
}

/// M4d-2c item 4 (review S-3): an ask closes whenever its finding is decided — here by the
/// watch's recheck once the claim's own source appears — and an orphan ask (open although its
/// finding was decided, as before M4d-2c) is not an open ask, so it never blocks a reply.
#[test]
fn asks_close_with_their_finding_and_orphans_are_not_open() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let compiler = dir.path().join("compiler.sqlite");
    let mut store = ClaimStore::open(&compiler).unwrap();
    let finding = store.findings(&Default::default()).unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let claim_id = store.resolve("r C3").unwrap().claim_id;
    let ask = Ask {
        finding: &finding,
        claim: &claim_id,
        kind: "ungrounded",
        token: "#f_x",
        line: "🎩 line",
        proposal: None,
    };
    store.record_ask(&ask, "2026-09-27T00:00:00Z").unwrap();
    let parsed = claim::parse(TEXT);
    let mut diary = Memory::default();
    diary.0.insert("s".into(), "# s\nCallum chose Rust.".into());
    diary
        .0
        .insert("gone".into(), "# gone\nCallum chose Valkey.".into());
    let entry = Entry::parse("r", TEXT.into());
    let judge = palace_core::diary_memory::FakeJudge("preserved");
    let mut rows = claim::check(&entry, &parsed.claims, &diary, Some(&judge));
    let checked = Checked {
        rollup: "r",
        date: "2026-09-22",
        thread: "t",
        judge: "judge:fake",
        claims: &parsed.claims,
        rows: &mut rows,
    };
    store.record(checked, "2026-09-28T00:00:00Z").unwrap();
    assert!(
        store.open_asks().unwrap().is_empty(),
        "closed by the recheck"
    );
    let closed = store.ask(&finding).unwrap().unwrap();
    assert_eq!(closed["answer"], "confirm by rule:recheck");
    assert_eq!(store.events("ask_closed").unwrap().len(), 1);
    drop(store);
    let c = rusqlite::Connection::open(&compiler).unwrap();
    c.execute("UPDATE asks SET answered_at=NULL, answer=NULL", [])
        .unwrap();
    drop(c);
    let store = ClaimStore::open(&compiler).unwrap();
    assert!(
        store.open_asks().unwrap().is_empty(),
        "an orphan is not open"
    );
}

/// M4d-2e item 2 (review M-1): an ask stays in `unsent_asks` until `mark_sent`; an unknown token
/// refuses the whole call (nothing marked); a second mark is a no-op keeping the first time; an
/// answered ask is not listed.
#[test]
fn asks_are_listed_until_marked_sent() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());
    let (compiler, kg) = (
        dir.path().join("compiler.sqlite"),
        dir.path().join("knowledge_graph.sqlite3"),
    );
    let found = candidates(&compiler, &kg).unwrap();
    let finding = found[0].finding["id"].as_str().unwrap().to_owned();
    let mut store = ClaimStore::open(&compiler).unwrap();
    let ask = Ask {
        finding: &finding,
        claim: found[0].claim["id"].as_str().unwrap(),
        kind: "ungrounded",
        token: "#f_x",
        line: "🎩 line #f_x",
        proposal: None,
    };
    store.record_ask(&ask, "2026-09-27T00:00:00Z").unwrap();
    let unsent = store.unsent_asks().unwrap();
    assert_eq!(unsent.len(), 1);
    assert_eq!(unsent[0]["line"], "🎩 line #f_x");
    let tokens = |t: &[&str]| t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let refused = store.mark_sent(&tokens(&["#f_x", "#f_nope"])).unwrap_err();
    assert!(palace_store::Refused::is(&refused), "{refused}");
    assert_eq!(store.unsent_asks().unwrap().len(), 1, "nothing marked");
    let first = store.mark_sent(&tokens(&["#f_x"])).unwrap();
    assert!(first[0].newly);
    assert!(store.unsent_asks().unwrap().is_empty());
    assert_eq!(store.open_asks().unwrap().len(), 1, "sent is still open");
    let second = store.mark_sent(&tokens(&["#f_x"])).unwrap();
    assert!(!second[0].newly);
    assert_eq!(second[0].sent_at, first[0].sent_at);
    store.verify_chain().unwrap();
}
