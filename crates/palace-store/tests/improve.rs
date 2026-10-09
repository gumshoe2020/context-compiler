//! M4d item 3 (store side): verbs without new triples, their edge effects, and undo restoring
//! the exact prior state. The write-gate verbs are exercised by the M4d-1 doctor on a copy.
use palace_core::diary_memory::Memory;
use palace_core::{claim, improve::Verb, rollup::Entry};
use palace_store::{
    Refused,
    claims::{Checked, ClaimStore},
    graph::{Graph, Request},
};
use serde_json::json;
use std::path::Path;

/// PUB-1a item 2: the invented cast as the person and the assistant, set once per test binary
/// before any name is read (every test here calls it first).
fn cast() {
    static CAST: std::sync::Once = std::sync::Once::new();
    CAST.call_once(|| palace_core::names::set(Some("callum"), Some("fergus")).unwrap());
}

const TEXT: &str = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — s.md — quote: \"Callum chose Rust\"\n- C2. Callum | quoted | Almanack 31:12 — 2026-09-21 — s.md\n- C3. Callum | chose | Valkey — 2026-09-21 — gone.md\n";

fn setup(dir: &Path) -> Graph {
    let kg = dir.join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP,FOREIGN KEY(subject) REFERENCES entities(id),FOREIGN KEY(object) REFERENCES entities(id));").unwrap();
    drop(c);
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
    drop(store);
    let mut g = Graph::open(&dir.join("compiler.sqlite"), &kg, None).unwrap();
    for (claim, object) in [(&parsed.claims[0], "rust"), (&parsed.claims[1], "almanack")] {
        let closet = json!({"claim_id": claim.id}).to_string();
        g.call("add", &json!({"subject":"callum","predicate":"chose","object":object,"confidence":0.9,"source_closet":closet})).unwrap();
    }
    g
}

fn state(dir: &Path) -> Vec<String> {
    let c = rusqlite::Connection::open(dir.join("compiler.sqlite")).unwrap();
    c.execute(
        "ATTACH DATABASE ?1 AS kg",
        [dir.join("knowledge_graph.sqlite3").to_str().unwrap()],
    )
    .unwrap();
    let mut rows = Vec::new();
    for sql in [
        "SELECT id||status||coalesce(superseded_by,'')||survived||weakened FROM claims ORDER BY id",
        "SELECT id||coalesce(decision,'open') FROM findings ORDER BY id",
        "SELECT id||coalesce(valid_to,'-')||confidence FROM kg.triples ORDER BY id",
        "SELECT triple_id||claim_id||coalesce(withdrawn_at,'-') FROM citations ORDER BY triple_id",
    ] {
        let mut s = c.prepare(sql).unwrap();
        rows.extend(
            s.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    rows
}

fn request<'a>(verb: Verb, target: &'a str, by: &'a str) -> Request<'a> {
    Request {
        verb,
        target,
        text: None,
        by,
        supersede: false,
        grounding_failed: false,
    }
}

#[test]
fn verbs_change_edges_and_undo_restores_the_exact_state() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let confirm = g
        .prepare(&request(Verb::Confirm, "2026-09-22 C1", "fergus"))
        .unwrap();
    let applied = g.commit(&confirm, &[]).unwrap();
    assert_eq!(applied.axis, ["currency"]);
    let weaken = g
        .prepare(&request(Verb::Weaken, "r C2", "rule:auto"))
        .unwrap();
    g.commit(&weaken, &[]).unwrap();
    drop(g);
    let rows = state(dir.path());
    assert!(
        rows.iter()
            .any(|r| r.contains("chose_rust") && r.ends_with("0.85")),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.contains("chose_almanack") && r.ends_with("0.45")),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|r| r.contains("\"verb\":\"weaken\"")),
        "the unsupported finding is decided"
    );
    let before = state(dir.path());
    let mut g = Graph::open(
        &dir.path().join("compiler.sqlite"),
        &dir.path().join("knowledge_graph.sqlite3"),
        None,
    )
    .unwrap();
    let expire = g
        .prepare(&request(Verb::Expire, "2026-09-22 C1", "rule:auto"))
        .unwrap();
    let expired = g.commit(&expire, &[]).unwrap();
    drop(g);
    assert_ne!(state(dir.path()), before);
    let mut g = Graph::open(
        &dir.path().join("compiler.sqlite"),
        &dir.path().join("knowledge_graph.sqlite3"),
        None,
    )
    .unwrap();
    let undo = g.undo(&expired.event, "fergus").unwrap();
    assert_eq!(undo.event, expired.reversible_by);
    assert!(Refused::is(
        &g.undo(&expired.event, "fergus").err().unwrap()
    ));
    assert!(Refused::is(
        &g.prepare(&request(Verb::Retract, "2026-09-22 C9", "fergus"))
            .err()
            .unwrap()
    ));
    g.verify_chain().unwrap();
    drop(g);
    assert_eq!(state(dir.path()), before);
}

#[test]
fn depends_reports_rollup_edges_and_events_read_only() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let expire = g
        .prepare(&request(Verb::Expire, "2026-09-22 C1", "rule:auto"))
        .unwrap();
    let applied = g.commit(&expire, &[]).unwrap();
    drop(g);
    let before = state(dir.path());
    let d = palace_store::depends::depends(
        &dir.path().join("compiler.sqlite"),
        &dir.path().join("knowledge_graph.sqlite3"),
        "r C1",
    )
    .unwrap();
    assert_eq!(d["rollups"][0]["number"], "C1");
    assert_eq!(d["edges"][0]["link"], "created");
    assert!(d["edges"][0]["valid_to"].is_string());
    assert_eq!(d["events"][0]["id"], applied.event.as_str());
    assert_eq!(d["compiles"], json!([]));
    assert_eq!(state(dir.path()), before);
}

/// A pre-existing triple with its own provenance (fergus, confidence 0.9), cited by C1: the
/// review's C12 probe (M4d-2a item 1, S-2).
fn cited_triple(dir: &Path, mut g: Graph) -> (Graph, String) {
    let closet = json!({"supplied_by": "fergus", "evidence_refs": ["x"]}).to_string();
    let added = g.call("add", &json!({"subject":"callum","predicate":"bounded_by","object":"tier-3","confidence":0.9,"source_closet":closet})).unwrap();
    let triple = added["triple_id"].as_str().unwrap().to_owned();
    let c1 = claim::parse(TEXT).claims[0].id.clone();
    drop(g);
    let mut store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    assert!(store.cite(&triple, &c1, "r").unwrap());
    drop(store);
    (reopen(dir), triple)
}

fn triple_row(dir: &Path, triple: &str) -> String {
    let c = rusqlite::Connection::open(dir.join("knowledge_graph.sqlite3")).unwrap();
    c.query_row(
        "SELECT coalesce(valid_to,'-')||' '||confidence||' '||source_closet FROM triples WHERE id=?1",
        [triple],
        |r| r.get(0),
    )
    .unwrap()
}

fn reopen(dir: &Path) -> Graph {
    Graph::open(
        &dir.join("compiler.sqlite"),
        &dir.join("knowledge_graph.sqlite3"),
        None,
    )
    .unwrap()
}

#[test]
fn verbs_never_end_or_rescore_a_triple_the_claim_only_cites() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let (mut g, triple) = cited_triple(dir.path(), setup(dir.path()));
    let original = triple_row(dir.path(), &triple);
    let confirm = g
        .prepare(&request(Verb::Confirm, "2026-09-22 C1", "fergus"))
        .unwrap();
    g.commit(&confirm, &[]).unwrap();
    drop(g);
    assert_eq!(
        triple_row(dir.path(), &triple),
        original,
        "confirm re-scored a cited triple"
    );
    let before = state(dir.path());
    let mut g = reopen(dir.path());
    let retract = g
        .prepare(&request(Verb::Retract, "2026-09-22 C1", "rule:auto"))
        .unwrap();
    let retracted = g.commit(&retract, &[]).unwrap();
    drop(g);
    assert_eq!(
        triple_row(dir.path(), &triple),
        original,
        "retract ended a cited triple"
    );
    assert!(
        state(dir.path())
            .iter()
            .any(|r| r.contains("chose_rust") && !r.contains("-0.")),
        "own edge ended"
    );
    let logged = ClaimStore::open(&dir.path().join("compiler.sqlite"))
        .unwrap()
        .events("improvement_event")
        .unwrap();
    let changes = &logged.last().unwrap()["changes"];
    let withdrawn: Vec<_> = changes
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "citation_withdrawn")
        .collect();
    assert_eq!(withdrawn.len(), 1, "{changes}");
    assert_eq!(withdrawn[0]["id"], triple.as_str());
    assert!(withdrawn[0]["after"]["withdrawn_at"].is_string());
    let mut g = reopen(dir.path());
    g.undo(&retracted.event, "fergus").unwrap();
    drop(g);
    assert_eq!(state(dir.path()), before);
    let c = rusqlite::Connection::open(dir.path().join("compiler.sqlite")).unwrap();
    let active: i64 = c
        .query_row(
            "SELECT count(*) FROM citations WHERE withdrawn_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 1, "undo restores the citation");
}

/// The review's S-1 probe (M4d-2a item 2): confirm, confirm, undo the first → refused, naming the
/// second; undo in reverse order restores the original state exactly; undo of undo is CAS too.
#[test]
fn undo_is_compare_and_swap() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let original = state(dir.path());
    let first = g
        .prepare(&request(Verb::Confirm, "2026-09-22 C1", "fergus"))
        .unwrap();
    let first = g.commit(&first, &[]).unwrap();
    let second = g
        .prepare(&request(Verb::Confirm, "2026-09-22 C1", "fergus"))
        .unwrap();
    let second = g.commit(&second, &[]).unwrap();
    let refused = g.undo(&first.event, "fergus").err().unwrap();
    assert!(Refused::is(&refused), "{refused}");
    assert!(refused.to_string().contains(&second.event), "{refused}");
    let undone_second = g.undo(&second.event, "fergus").unwrap();
    let undone_first = g.undo(&first.event, "fergus").unwrap();
    drop(g);
    assert_eq!(state(dir.path()), original);
    let mut g = reopen(dir.path());
    let redo = g.undo(&undone_second.event, "fergus").err().unwrap();
    assert!(
        Refused::is(&redo),
        "redo of the second after the first was undone: {redo}"
    );
    assert!(redo.to_string().contains(&undone_first.event), "{redo}");
    g.undo(&undone_first.event, "fergus").unwrap();
    g.undo(&undone_second.event, "fergus").unwrap();
    g.verify_chain().unwrap();
}

/// M4d-2a item 3 (S-4): `add` on an ungrounded anchor supersedes it; on a grounded one only with
/// `--supersede`; undo reverses both the status and the successor.
#[test]
fn add_supersedes_an_ungrounded_anchor_and_undo_reverses_it() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let before = state(dir.path());
    let add = |target, supersede, text| Request {
        verb: Verb::Add,
        target,
        text: Some(text),
        by: "fergus",
        supersede,
        grounding_failed: false,
    };
    let applied = g
        .commit(
            &g.prepare(&add(
                "r C3",
                false,
                "Callum | chose | Valkey for the cache — 2026-09-21 — s.md",
            ))
            .unwrap(),
            &[],
        )
        .unwrap();
    let new = applied.to_claim.clone().unwrap();
    drop(g);
    let c3 = claim::parse(TEXT).claims[2].id.clone();
    assert!(
        state(dir.path()).contains(&format!("{c3}superseded{new}00")),
        "{:?}",
        state(dir.path())
    );
    let mut g = reopen(dir.path());
    g.undo(&applied.event, "fergus").unwrap();
    drop(g);
    let rows = state(dir.path());
    let without_new: Vec<String> = rows
        .iter()
        .filter(|r| !r.starts_with(&new))
        .cloned()
        .collect();
    assert_eq!(
        without_new, before,
        "undo restores everything but the (undone) new claim"
    );
    assert!(rows.contains(&format!("{c3}live00")), "{rows:?}");
    assert!(rows.contains(&format!("{new}undone00")), "{rows:?}");
    let g = reopen(dir.path());
    let grounded = g
        .prepare(&add("r C1", false, "Callum | chose | Rust for the backend"))
        .unwrap();
    assert!(!grounded.supersede);
    assert!(
        g.prepare(&add("r C1", true, "Callum | chose | Rust for the backend"))
            .unwrap()
            .supersede
    );
    let mut weaken = request(Verb::Weaken, "r C1", "fergus");
    weaken.supersede = true;
    assert!(Refused::is(&g.prepare(&weaken).err().unwrap()));
}

fn live_set(dir: &Path) -> palace_core::wal_replay::Live {
    let c = rusqlite::Connection::open(dir.join("knowledge_graph.sqlite3")).unwrap();
    let mut s = c
        .prepare("SELECT id, coalesce(confidence,1.0) FROM triples WHERE valid_to IS NULL")
        .unwrap();
    s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn wal_lines(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("wal/write_log.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// M4d-2a item 4 (S-3): verbs' and undos' graph changes reach the palace WAL, one line per change
/// carrying the event id, and replaying the WAL reproduces the KG's live set.
#[test]
fn graph_effects_are_audited_to_the_wal_and_replay() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let mut events = Vec::new();
    for (verb, target) in [
        (Verb::Confirm, "r C1"),
        (Verb::Retract, "r C1"),
        (Verb::Expire, "r C2"),
    ] {
        let prepared = g.prepare(&request(verb, target, "fergus")).unwrap();
        events.push(g.commit(&prepared, &[]).unwrap().event);
    }
    for undone in [events[2].clone(), events[1].clone()] {
        events.push(g.undo(&undone, "fergus").unwrap().event);
    }
    drop(g);
    let lines = wal_lines(dir.path());
    let audited: Vec<_> = lines
        .iter()
        .filter(|l| l["params"]["improvement_event"].is_string())
        .collect();
    let ops: Vec<&str> = audited
        .iter()
        .map(|l| l["operation"].as_str().unwrap())
        .collect();
    assert_eq!(
        ops,
        [
            "kg_set_confidence",
            "kg_invalidate",
            "kg_invalidate",
            "kg_reopen",
            "kg_reopen"
        ],
        "confirm re-scores C1's edge; retract and expire end one edge each; two undos reopen"
    );
    assert!(audited.iter().all(|l| {
        events.contains(
            &l["params"]["improvement_event"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }));
    let replayed = palace_core::wal_replay::replay(Default::default(), &lines).unwrap();
    assert_eq!(replayed, live_set(dir.path()));
}

/// M4d-2a item 5 (L-3): a call interrupted after the gate created its triple but before the event
/// committed is rolled back at the next call (triple ended, audited); a committed event without
/// its IMPROVE entry is reported for resume.
#[test]
fn an_interrupted_improve_is_rolled_back_and_a_missing_entry_resumed() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let add = Request {
        verb: Verb::Add,
        target: "r C1",
        text: Some("Callum | chose | Rust for the backend"),
        by: "fergus",
        supersede: false,
        grounding_failed: false,
    };
    let mut prepared = g.prepare(&add).unwrap();
    g.begin(&mut prepared).unwrap();
    let new = prepared.new_claims[0].id.clone();
    let closet = json!({"claim_id": new}).to_string();
    let orphan = g.call("add", &json!({"subject":"callum","predicate":"chose","object":"rust-backend","source_closet":closet})).unwrap()["triple_id"].as_str().unwrap().to_owned();
    drop(g); // the crash: no commit
    let mut g = reopen(dir.path());
    let rolled = g.recover().unwrap();
    assert_eq!(rolled.len(), 1, "{rolled:?}");
    assert!(!live_set(dir.path()).contains_key(&orphan));
    assert!(
        wal_lines(dir.path())
            .iter()
            .any(|l| l["operation"] == "kg_invalidate"
                && l["params"]["triple_id"] == orphan.as_str())
    );
    assert!(g.recover().unwrap().is_empty(), "rolled back once");
    let confirm = g
        .prepare(&request(Verb::Confirm, "r C1", "fergus"))
        .unwrap();
    let applied = g.commit(&confirm, &[]).unwrap();
    let missing: Vec<String> = g
        .unrecorded()
        .unwrap()
        .into_iter()
        .map(|a| a.event)
        .collect();
    assert_eq!(missing.as_slice(), std::slice::from_ref(&applied.event));
    g.log(&json!({"operation":"improve_entry","event":applied.event}))
        .unwrap();
    assert!(g.unrecorded().unwrap().is_empty());
}

/// M4d-2a item 6 (L-9): the event records the finding it decided even when the target was a
/// claim; a named reversal finding refuses a rule or judge.
#[test]
fn the_event_records_the_decided_finding() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let mut g = setup(dir.path());
    let weaken = g
        .prepare(&request(Verb::Weaken, "r C2", "rule:auto"))
        .unwrap();
    let applied = g.commit(&weaken, &[]).unwrap();
    let finding = applied.finding.clone().expect("decided finding recorded");
    assert!(finding.starts_with("f_"));
    drop(g);
    let events = ClaimStore::open(&dir.path().join("compiler.sqlite"))
        .unwrap()
        .improvement_events()
        .unwrap();
    assert_eq!(events.last().unwrap()["finding"], finding.as_str());
}

fn confidence(dir: &Path, number: &str) -> f64 {
    let store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    let id = store.resolve(&format!("r {number}")).unwrap().claim_id;
    store.claim(&id).unwrap().unwrap()["confidence"]
        .as_f64()
        .unwrap()
}

/// M4d-2a item 11: a failed grounding attempt subtracts 0.15 although the claim has an open
/// `unsupported` finding (oracle C13: 0.45 → 0.30) and leaves the finding open; a plain `weaken`
/// on an unsupported finding decides it and does not subtract again (one decrement per fact).
#[test]
fn a_failed_grounding_attempt_always_subtracts() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let g = setup(dir.path());
    drop(g);
    assert!((confidence(dir.path(), "C3") - 0.45).abs() < 1e-9);
    let mut g = reopen(dir.path());
    let mut grounding = request(Verb::Weaken, "r C3", "rule:auto");
    grounding.grounding_failed = true;
    let applied = g.commit(&g.prepare(&grounding).unwrap(), &[]).unwrap();
    assert_eq!(applied.text, "grounding attempt failed");
    assert_eq!(applied.finding, None, "the finding stays open for the ask");
    let plain = g
        .prepare(&request(Verb::Weaken, "r C2", "rule:auto"))
        .unwrap();
    g.commit(&plain, &[]).unwrap();
    let mut wrong = request(Verb::Confirm, "r C1", "rule:auto");
    wrong.grounding_failed = true;
    assert!(Refused::is(&g.prepare(&wrong).err().unwrap()));
    drop(g);
    assert!((confidence(dir.path(), "C3") - 0.30).abs() < 1e-9);
    assert!((confidence(dir.path(), "C2") - 0.45).abs() < 1e-9);
}

/// M4d-2a.1 item 1 (review S-2): a rule or judge may not undo a person's event, nor redo it, so
/// the review's reopen-then-re-close of a reversal by `rule:auto` is impossible; a rule may undo
/// a rule's event; a person may undo anything.
#[test]
fn rules_do_not_undo_peoples_decisions() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let g = setup(dir.path());
    drop(g);
    let c = rusqlite::Connection::open(dir.path().join("compiler.sqlite")).unwrap();
    c.execute("INSERT INTO findings(id,claim,rollup,kind,evidence,checker,checked_at) SELECT 'f_reversal00', id, 'r', 'reversal', '{}', 'judge:fake', '2026-09-22' FROM claims WHERE number='C1'", []).unwrap();
    drop(c);
    let mut g = reopen(dir.path());
    let confirm = g
        .prepare(&request(Verb::Confirm, "r C1", "fergus"))
        .unwrap();
    let confirm = g.commit(&confirm, &[]).unwrap();
    for rule in ["rule:auto", "judge:sonnet"] {
        let refused = g.undo(&confirm.event, rule).err().unwrap();
        assert!(Refused::is(&refused), "{refused}");
        let message = refused.to_string();
        assert!(
            message.contains(&confirm.event) && message.contains("fergus"),
            "{message}"
        );
    }
    let reopened = g.undo(&confirm.event, "fergus").unwrap();
    let redo = g.undo(&reopened.event, "rule:auto").err().unwrap();
    assert!(
        Refused::is(&redo),
        "a rule cannot re-close the reversal: {redo}"
    );
    g.undo(&reopened.event, "callum").unwrap();
    let weaken = g
        .prepare(&request(Verb::Weaken, "r C2", "rule:auto"))
        .unwrap();
    let weaken = g.commit(&weaken, &[]).unwrap();
    g.undo(&weaken.event, "rule:auto").unwrap();
    g.verify_chain().unwrap();
}

fn rust_edge(dir: &Path) -> (String, f64) {
    let c = rusqlite::Connection::open(dir.join("knowledge_graph.sqlite3")).unwrap();
    c.query_row(
        "SELECT id, confidence FROM triples WHERE object='rust'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn paths(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    (
        dir.join("compiler.sqlite"),
        dir.join("knowledge_graph.sqlite3"),
    )
}

fn operations(dir: &Path) -> Vec<String> {
    let c = rusqlite::Connection::open(dir.join("compiler.sqlite")).unwrap();
    let mut s = c
        .prepare("SELECT json_extract(payload,'$.operation') FROM write_log ORDER BY sequence")
        .unwrap();
    s.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// M4d-2a.1 item 2 (review S-1), the rollback half: a trigger fails the event half, a second
/// connection commits the KG half alone; `check_divergence` sees the pending intent and
/// `reconcile` restores `before` with a `recovered_rollback` line (the event never happened).
#[test]
fn a_kg_half_without_its_event_is_rolled_back() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let (compiler, kg) = paths(dir.path());
    drop(setup(dir.path()));
    assert!(
        palace_store::graph::check_divergence(&compiler, &kg)
            .unwrap()
            .is_empty()
    );
    let (edge, original) = rust_edge(dir.path());
    palace_store::crash::fail_event_half(&compiler).unwrap();
    let mut g = reopen(dir.path());
    let confirm = g
        .prepare(&request(Verb::Confirm, "r C1", "fergus"))
        .unwrap();
    assert!(g.commit(&confirm, &[]).is_err(), "the staged crash");
    drop(g);
    palace_store::crash::clear(&compiler).unwrap();
    assert_eq!(
        palace_store::crash::commit_kg_half(&compiler, &kg).unwrap(),
        1
    );
    assert!(
        (rust_edge(dir.path()).1 - original).abs() > 0.01,
        "KG half alone"
    );
    let pending = palace_store::graph::check_divergence(&compiler, &kg).unwrap();
    assert!(pending[0].contains("recovered_rollback"), "{pending:?}");
    let mut g = reopen(dir.path());
    let acted = g.reconcile().unwrap();
    assert_eq!(acted[0]["operation"], "recovered_rollback", "{acted:?}");
    assert!(g.reconcile().unwrap().is_empty(), "settled once");
    g.verify_chain().unwrap();
    drop(g);
    assert_eq!(rust_edge(dir.path()), (edge, original));
    assert!(!operations(dir.path()).contains(&"improvement_event".to_owned()));
    assert!(
        palace_store::graph::check_divergence(&compiler, &kg)
            .unwrap()
            .is_empty()
    );
}

/// Item 2, the re-apply half: the event committed but its KG half was lost before the intent
/// settled; `reconcile` writes `after` again and logs `recovered_reapply`. A hand-edited row is
/// DIVERGED: nothing changes until the row is put back.
#[test]
fn a_lost_kg_half_is_reapplied_and_a_foreign_edit_diverges() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let (compiler, kg) = paths(dir.path());
    let mut g = setup(dir.path());
    let confirm = g
        .prepare(&request(Verb::Confirm, "r C1", "fergus"))
        .unwrap();
    g.commit(&confirm, &[]).unwrap();
    drop(g);
    let (edge, confirmed) = rust_edge(dir.path());
    palace_store::crash::lose_kg_half(&compiler, &kg).unwrap();
    assert!((rust_edge(dir.path()).1 - confirmed).abs() > 0.01);
    let mut g = reopen(dir.path());
    assert_eq!(g.reconcile().unwrap()[0]["operation"], "recovered_reapply");
    drop(g);
    assert!((rust_edge(dir.path()).1 - confirmed).abs() < 1e-9);
    palace_store::crash::lose_kg_half(&compiler, &kg).unwrap();
    palace_store::crash::hand_edit(&kg, &edge, 0.123).unwrap();
    let mut g = reopen(dir.path());
    let diverged = g.reconcile().err().unwrap();
    let lines = palace_store::graph::Diverged::of(&diverged)
        .unwrap()
        .lines
        .clone();
    assert_eq!(lines, [format!("DIVERGED triples {edge} confidence")]);
    assert!(diverged.to_string().contains("palace doctor --settle k_"));
    drop(g);
    let report = palace_store::graph::check_divergence(&compiler, &kg).unwrap();
    assert!(report.contains(&lines[0]), "{report:?}");
    assert!(
        (rust_edge(dir.path()).1 - 0.123).abs() < 1e-9,
        "left untouched"
    );
    palace_store::crash::hand_edit(&kg, &edge, confirmed).unwrap();
    reopen(dir.path()).reconcile().unwrap();
    assert!(
        palace_store::graph::check_divergence(&compiler, &kg)
            .unwrap()
            .is_empty()
    );
}

fn event_count(dir: &Path) -> usize {
    ClaimStore::open(&dir.join("compiler.sqlite"))
        .unwrap()
        .improvement_events()
        .unwrap()
        .len()
}

/// M4d-2a.1 item 3 (review L-5): `--grounding-failed` only on an ungrounded claim (C3's source is
/// absent; C1 is grounded, C2's source exists but does not support it), never by a judge, and
/// once per claim per UTC day — a second attempt is `Skipped` with no event.
#[test]
fn grounding_attempts_are_constrained_and_once_a_day() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let g = setup(dir.path());
    let attempt = |target, by| {
        let mut r = request(Verb::Weaken, target, by);
        r.grounding_failed = true;
        r
    };
    for (target, by) in [
        ("r C1", "rule:auto"),
        ("r C2", "rule:auto"),
        ("r C3", "judge:sonnet"),
    ] {
        let refused = g.prepare(&attempt(target, by)).err().unwrap();
        assert!(Refused::is(&refused), "{target} {by}: {refused}");
    }
    let mut g = g;
    let first = g.prepare(&attempt("r C3", "rule:auto")).unwrap();
    g.commit(&first, &[]).unwrap();
    drop(g);
    let events = event_count(dir.path());
    let g = reopen(dir.path());
    for by in ["rule:auto", "fergus"] {
        let again = g.prepare(&attempt("r C3", by)).err().unwrap();
        assert!(palace_store::Skipped::is(&again), "{again}");
        assert!(
            again.to_string().contains("already attempted today"),
            "{again}"
        );
    }
    drop(g);
    assert_eq!(
        event_count(dir.path()),
        events,
        "no event for the second attempt"
    );
}

fn derived(dir: &Path, number: &str) -> (f64, String) {
    let c = rusqlite::Connection::open(dir.join("compiler.sqlite")).unwrap();
    let (id, status): (String, String) = c
        .query_row(
            "SELECT id, status FROM claims WHERE number=?1",
            [number],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (palace_store::claims::derived(&c, &id).unwrap(), status)
}

/// M4d-2c item 1 (SPEC §5.1 revised 2026-09-27): `weaken` on a reversal finding by rule:auto
/// disputes the claim — no second decrement, the finding stays open, a second dispute is
/// refused, `depends` shows it — and only a person's verb settles it: the assistant's `confirm` makes
/// it live again; undoing that makes it disputed again; a person may contradict a disputed claim.
#[test]
fn a_reversal_is_disputed_not_decided_by_the_loop() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    drop(setup(dir.path()));
    let c = rusqlite::Connection::open(dir.path().join("compiler.sqlite")).unwrap();
    c.execute("INSERT INTO findings(id,claim,rollup,kind,evidence,checker,checked_at) SELECT 'f_reversal00', id, 'r', 'reversal', '{}', 'judge:fake', '2026-09-22' FROM claims WHERE number='C1'", []).unwrap();
    drop(c);
    let (flagged, _) = derived(dir.path(), "C1");
    let mut g = reopen(dir.path());
    let mut dispute = request(Verb::Weaken, "f_reversal00", "rule:auto");
    dispute.text = Some(palace_core::auto::DISPUTE_REASON);
    let applied = g.commit(&g.prepare(&dispute).unwrap(), &[]).unwrap();
    assert_eq!(applied.axis, ["scope"]);
    let again = g.prepare(&dispute).err().unwrap();
    assert!(Refused::is(&again), "{again}");
    drop(g);
    let (after, status) = derived(dir.path(), "C1");
    assert_eq!(
        (after, status.as_str()),
        (flagged, "disputed"),
        "no second decrement"
    );
    let store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    assert_eq!(store.with_status("disputed").unwrap().len(), 1);
    let open = store
        .findings(&palace_store::lookup::FindingFilter {
            open: true,
            ..Default::default()
        })
        .unwrap();
    assert!(
        open.iter().any(|f| f["id"] == "f_reversal00"),
        "left for fergus"
    );
    drop(store);
    let depends = palace_store::depends::depends(
        &dir.path().join("compiler.sqlite"),
        &dir.path().join("knowledge_graph.sqlite3"),
        "r C1",
    )
    .unwrap();
    assert_eq!(depends["claim"]["status"], "disputed");
    let mut g = reopen(dir.path());
    let keep = g
        .commit(
            &g.prepare(&request(Verb::Confirm, "f_reversal00", "fergus"))
                .unwrap(),
            &[],
        )
        .unwrap();
    drop(g);
    assert_eq!(derived(dir.path(), "C1").1, "live");
    let store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let deciding = store.deciding_event("f_reversal00").unwrap();
    assert_eq!(
        deciding.as_deref(),
        Some(keep.event.as_str()),
        "the undo hint's event"
    );
    drop(store);
    let mut g = reopen(dir.path());
    g.undo(&keep.event, "fergus").unwrap();
    drop(g);
    assert_eq!(derived(dir.path(), "C1").1, "disputed");
    let mut g = reopen(dir.path());
    let retract = g
        .prepare(&request(Verb::Retract, "f_reversal00", "fergus"))
        .unwrap();
    g.commit(&retract, &[]).unwrap();
    drop(g);
    assert_eq!(derived(dir.path(), "C1").1, "retracted");
}

fn open_reversal(dir: &Path, number: &str) {
    let c = rusqlite::Connection::open(dir.join("compiler.sqlite")).unwrap();
    c.execute("INSERT INTO findings(id,claim,rollup,kind,evidence,checker,checked_at) SELECT 'f_reversal00', id, 'r', 'reversal', '{}', 'judge:fake', '2026-09-22' FROM claims WHERE number=?1", [number]).unwrap();
}

fn with_text<'a>(verb: Verb, target: &'a str, by: &'a str, text: &'a str) -> Request<'a> {
    Request {
        text: Some(text),
        ..request(verb, target, by)
    }
}

/// M4d-2d item 1 (review S-1): the review's two probes — a rule contradicting by claim id, a
/// judge by `"<date> Cn"` — are refused naming the actor, and so is a rule's split; nothing
/// changes; nor may they confirm the disputed claim (L-1). A person's contradict by claim id
/// decides the open reversal (it leaves the queue).
#[test]
fn asserting_verbs_are_person_only_whatever_the_target() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    drop(setup(dir.path()));
    open_reversal(dir.path(), "C1");
    let c1 = claim::parse(TEXT).claims[0].id.clone();
    let g = reopen(dir.path());
    let text = "Callum | rejected | Rust";
    let probes = [
        with_text(Verb::Contradict, &c1, "rule:auto", text),
        with_text(Verb::Contradict, "2026-09-22 C1", "judge:sonnet", text),
        with_text(Verb::Contradict, "f_reversal00", "rule:auto", text),
        with_text(
            Verb::Split,
            &c1,
            "rule:auto",
            "Callum | chose | Rust || Callum | chose | Go",
        ),
    ];
    let mut dispute = request(Verb::Weaken, "f_reversal00", "rule:auto");
    dispute.text = Some(palace_core::auto::DISPUTE_REASON);
    let mut g = g;
    g.commit(&g.prepare(&dispute).unwrap(), &[]).unwrap();
    let before = state(dir.path());
    for by in ["rule:auto", "judge:sonnet"] {
        let confirm = g.prepare(&request(Verb::Confirm, &c1, by)).err().unwrap();
        assert!(
            Refused::is(&confirm) && confirm.to_string().contains("disputed"),
            "{confirm}"
        );
    }
    for probe in &probes {
        let refused = g.prepare(probe).err().unwrap();
        assert!(Refused::is(&refused), "{refused}");
        assert!(refused.to_string().contains(probe.by), "{refused}");
        assert!(refused.to_string().contains("person-only"), "{refused}");
    }
    drop(g);
    assert_eq!(state(dir.path()), before, "a refusal changes nothing");
    let mut g = reopen(dir.path());
    let applied = g
        .commit(
            &g.prepare(&with_text(Verb::Contradict, &c1, "fergus", text))
                .unwrap(),
            &[],
        )
        .unwrap();
    assert_eq!(applied.finding.as_deref(), Some("f_reversal00"));
    drop(g);
    let store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let open = store
        .findings(&palace_store::lookup::FindingFilter {
            open: true,
            ..Default::default()
        })
        .unwrap();
    assert!(
        !open.iter().any(|f| f["kind"] == "reversal"),
        "decided: out of the queue"
    );
}

/// M4d-2d item 1: a rule or judge may `add` only as re-grounding — the anchor's own subject,
/// predicate and object on an ungrounded anchor; a new fact, or a grounded anchor, is refused.
#[test]
fn a_rule_adds_only_to_re_ground() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let g = setup(dir.path());
    let refused = |target, by, text| {
        let e = g
            .prepare(&with_text(Verb::Add, target, by, text))
            .err()
            .unwrap();
        assert!(
            Refused::is(&e) && e.to_string().contains("re-grounding"),
            "{e}"
        );
    };
    refused(
        "r C3",
        "rule:auto",
        "Callum | chose | Valkey for the cache — 2026-09-21 — s.md",
    );
    refused(
        "r C1",
        "judge:sonnet",
        "Callum | chose | Rust — 2026-09-21 — other.md",
    );
    for by in ["rule:auto", "judge:sonnet"] {
        let ok = g.prepare(&with_text(
            Verb::Add,
            "r C3",
            by,
            "Callum | chose | Valkey — 2026-09-21 — s.md",
        ));
        assert!(
            ok.unwrap().supersede,
            "re-grounding supersedes the ungrounded anchor"
        );
    }
    assert!(
        g.prepare(&with_text(
            Verb::Add,
            "r C1",
            "fergus",
            "Callum | chose | Go"
        ))
        .is_ok()
    );
}

/// M4d-2d item 3: the digest reads each event since a time with its claim's label, statement and
/// status change; an undone event drops out (its undo is listed instead).
#[test]
fn the_digest_reads_events_and_drops_undone_ones() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    drop(setup(dir.path()));
    open_reversal(dir.path(), "C1");
    let c1 = claim::parse(TEXT).claims[0].id.clone();
    let mut g = reopen(dir.path());
    let mut dispute = request(Verb::Weaken, "f_reversal00", "rule:auto");
    dispute.text = Some(palace_core::auto::DISPUTE_REASON);
    g.commit(&g.prepare(&dispute).unwrap(), &[]).unwrap();
    let text = "Callum | rejected | Rust";
    let fix = g
        .commit(
            &g.prepare(&with_text(Verb::Contradict, &c1, "fergus", text))
                .unwrap(),
            &[],
        )
        .unwrap();
    drop(g);
    let store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let events = store.digest_events("2000-01-01").unwrap();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0].status, ("live".into(), "disputed".into()));
    assert_eq!(
        (events[1].by.as_str(), events[1].id.as_str()),
        ("fergus", fix.event.as_str())
    );
    assert_eq!(events[1].label, "2026-09-22 C1");
    assert_eq!(events[1].statement, "Callum chose Rust");
    assert_eq!(events[1].successor.as_deref(), Some("Callum rejected Rust"));
    assert!(store.digest_events("2999-01-01").unwrap().is_empty());
    drop(store);
    let mut g = reopen(dir.path());
    g.undo(&fix.event, "callum").unwrap();
    drop(g);
    let store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let events = store.digest_events("2000-01-01").unwrap();
    let ids: Vec<&str> = events.iter().map(|e| e.verb.as_str()).collect();
    assert_eq!(ids, ["weaken", "undo"]);
}

fn disputed_c1(dir: &Path) -> String {
    drop(setup(dir));
    open_reversal(dir, "C1");
    let mut g = reopen(dir);
    let mut dispute = request(Verb::Weaken, "f_reversal00", "rule:auto");
    dispute.text = Some(palace_core::auto::DISPUTE_REASON);
    g.commit(&g.prepare(&dispute).unwrap(), &[]).unwrap();
    claim::parse(TEXT).claims[0].id.clone()
}

fn reversal_decision(dir: &Path) -> serde_json::Value {
    let c = rusqlite::Connection::open(dir.join("compiler.sqlite")).unwrap();
    let decision: Option<String> = c
        .query_row(
            "SELECT decision FROM findings WHERE id='f_reversal00'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    decision.map_or(serde_json::Value::Null, |d| {
        serde_json::from_str(&d).unwrap()
    })
}

/// M4d-2e item 1 (review M-2, probes on `pC`/`pD`): a rule's or judge's `expire` or `retract` of
/// a disputed claim is refused naming the actor, by claim id or by the reversal, and changes
/// nothing; the reversal stays open.
#[test]
fn a_rule_may_not_end_a_disputed_claim() {
    cast();
    let dir = tempfile::tempdir().unwrap();
    let c1 = disputed_c1(dir.path());
    let before = state(dir.path());
    let g = reopen(dir.path());
    for verb in [Verb::Expire, Verb::Retract] {
        for by in ["rule:auto", "judge:sonnet"] {
            for target in [c1.as_str(), "2026-09-22 C1"] {
                let refused = g.prepare(&request(verb, target, by)).err().unwrap();
                assert!(Refused::is(&refused), "{refused}");
                assert!(refused.to_string().contains(by), "{refused}");
                assert!(refused.to_string().contains("disputed"), "{refused}");
            }
        }
    }
    drop(g);
    assert_eq!(state(dir.path()), before, "a refusal changes nothing");
    assert!(reversal_decision(dir.path()).is_null());
}

/// M4d-2e item 1: a person ending or correcting a disputed claim by any verb decides its open
/// reversal in the same event (Fergus's split, expire, retract, confirm by claim id).
#[test]
fn a_persons_verb_on_a_disputed_claim_decides_its_reversal() {
    cast();
    let verbs = [
        (
            Verb::Split,
            Some("Callum | chose | Rust for the server || Callum | chose | Go for tools"),
            "split",
        ),
        (Verb::Expire, None, "expired"),
        (Verb::Retract, None, "retracted"),
        (Verb::Confirm, None, "live"),
    ];
    for (verb, text, status) in verbs {
        let dir = tempfile::tempdir().unwrap();
        let c1 = disputed_c1(dir.path());
        let mut g = reopen(dir.path());
        let mut ask = request(verb, &c1, "fergus");
        ask.text = text;
        let applied = g.commit(&g.prepare(&ask).unwrap(), &[]).unwrap();
        drop(g);
        let decision = reversal_decision(dir.path());
        assert_eq!(decision["by"], "fergus", "{verb:?}");
        assert_eq!(decision["verb"], verb.name(), "{verb:?}");
        assert_eq!(applied.finding.as_deref(), Some("f_reversal00"), "{verb:?}");
        assert_eq!(derived(dir.path(), "C1").1, status, "{verb:?}");
    }
}

fn record(dir: &Path, rollup: &str, text: &str) {
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary.0.insert(
        "s".into(),
        "# s\nCallum chose Rust. Callum chose Go.".into(),
    );
    let mut rows = claim::check(
        &Entry::parse(rollup, text.into()),
        &parsed.claims,
        &diary,
        None::<&palace_core::diary_memory::FakeJudge>,
    );
    let mut store = ClaimStore::open(&dir.join("compiler.sqlite")).unwrap();
    let checked = Checked {
        rollup,
        date: "2026-09-24",
        thread: "t",
        judge: "judge:fake",
        claims: &parsed.claims,
        rows: &mut rows,
    };
    store.record(checked, "2026-09-24T00:00:00Z").unwrap();
}

/// M4d-2e item 1: a later rollup that marks the disputed claim corrected (or superseded) has
/// resolved it: the reversal leaves the queue, decided by `rule:recheck` with status `superseded`.
#[test]
fn a_later_rollups_marker_closes_the_queue_item() {
    cast();
    for marker in ["corrected", "superseded"] {
        let dir = tempfile::tempdir().unwrap();
        disputed_c1(dir.path());
        let later = format!(
            "## Claims\n- C1. ({marker} by C4) Callum | chose | Rust — 2026-09-21 — s.md — quote: \"Callum chose Rust\"\n- C4. Callum | chose | Go — 2026-09-23 — s.md\n"
        );
        record(dir.path(), "r2", &later);
        let decision = reversal_decision(dir.path());
        assert_eq!(decision["by"], "rule:recheck", "{marker}");
        assert_eq!(decision["status"], "superseded", "{marker}");
        assert_eq!(derived(dir.path(), "C1").1, marker);
    }
}
