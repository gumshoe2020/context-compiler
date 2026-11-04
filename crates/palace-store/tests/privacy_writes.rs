use palace_store::{
    auto::Ask,
    claims::{ClaimStore, Distilled, DistilledEntry},
    privacy::{Marker, People},
};
use serde_json::json;
#[test]
fn claims_items_and_asks_inherit_source_and_model_window() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/priv1");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("write-paths-{}.sqlite", std::process::id()));
    let people = People::parse(include_str!("../../../testdata/privacy/people.json")).unwrap();
    let private = people.resolve("synthetic-private", None);
    let mut store = ClaimStore::open(&path).unwrap();
    store.mark_entry("synthetic-entry.md", &private).unwrap();
    let entry = DistilledEntry {
        file: "synthetic-entry.md",
        date: "2031-06-01",
        thread: "invented plan",
    };
    let claim = Distilled {
        id: "c_synthetic",
        number: "D1",
        kind: "specific",
        subject: "Zebulon Quarrie",
        predicate: "reading",
        object: "glucose 417",
        date: "2031-06-01",
        quote: Some("glucose 417"),
        transcript: "synthetic-entry.md:1",
        demoted: None,
    };
    store
        .record_distilled(&entry, &[claim], "2031-06-01T12:00:00Z")
        .unwrap();
    assert!(store.privacy_markers("claim", "c_synthetic").unwrap()[0].private);
    store
        .record_ask(
            &Ask {
                finding: "f_synthetic",
                claim: "c_synthetic",
                kind: "ungrounded",
                token: "#f_synthetic",
                line: "glucose 417",
                proposal: None,
            },
            "2031-06-01T12:00:00Z",
        )
        .unwrap();
    assert_eq!(
        store.privacy_markers("ask", "f_synthetic").unwrap(),
        vec![private.clone()]
    );
    assert!(store.unsent_asks().unwrap().is_empty());
    assert!(store.ask("f_synthetic").unwrap().is_some());
    let mixed = Marker::meet(
        &[Marker::shared("synthetic-shared"), private],
        "synthetic-window",
    );
    store.item_model_inputs(mixed);
    store.apply_item_at("ann",&json!({"action":"create","id":"synthetic-item","owner":"person","kind":"task","title":"cobalt heron","line":1,"quote":"glucose 417"}),"local-shared-source",Some("2031-06-01T12:00:00Z")).unwrap();
    assert!(store.items(Some("ann"), None).unwrap().is_empty());
    let rows = store
        .items_for_readers(Some("ann"), None, &["wren".into()])
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(store.privacy_markers("item", "synthetic-item").unwrap()[0].private);
    // A private question/proposal about a shared fact is itself held.
    store
        .mark_entry("shared-entry.md", &people.resolve("synthetic-shared", None))
        .unwrap();
    store
        .record_distilled(
            &DistilledEntry {
                file: "shared-entry.md",
                date: "2031-06-01",
                thread: "synthetic",
            },
            &[Distilled {
                id: "c_shared",
                number: "D1",
                kind: "concept",
                subject: "Invented Finch",
                predicate: "uses",
                object: "amber kite",
                date: "2031-06-01",
                quote: Some("amber kite"),
                transcript: "shared-entry.md:1",
                demoted: None,
            }],
            "2031-06-01T12:00:00Z",
        )
        .unwrap();
    palace_store::privacy::in_conversation(Some("unmapped-ask-source"), || {
        store
            .record_ask(
                &Ask {
                    finding: "f_private_question",
                    claim: "c_shared",
                    kind: "ungrounded",
                    token: "#f_private_question",
                    line: "Private invented question",
                    proposal: None,
                },
                "2031-06-01T12:00:00Z",
            )
            .unwrap();
    });
    assert!(store.unsent_asks().unwrap().is_empty());
    let ask_marker = &store.privacy_markers("ask", "f_private_question").unwrap()[0];
    assert!(ask_marker.private && ask_marker.quarantine);
    assert_eq!(ask_marker.source, "unmapped-ask-source");
    store.verify_chain().unwrap();
}

#[test]
fn private_retraction_retires_only_private_support_and_undo_restores_it() {
    use palace_core::improve::Verb;
    use palace_store::graph::{Graph, Request};
    palace_core::names::set(Some("ann"), Some("pip")).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/priv1/support-undo-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let kg = root.join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP);
      CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,valid_from TEXT,valid_to TEXT,confidence REAL,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);").unwrap();
    drop(c);
    let path = root.join("compiler.sqlite");
    let mut store = ClaimStore::open(&path).unwrap();
    let p = People::parse(include_str!("../../../testdata/privacy/people.json")).unwrap();
    store
        .mark_entry("private-entry.md", &p.resolve("synthetic-private", None))
        .unwrap();
    store
        .mark_entry("shared-entry.md", &p.resolve("synthetic-shared", None))
        .unwrap();
    store
        .record_distilled(
            &DistilledEntry {
                file: "private-entry.md",
                date: "2031-06-01",
                thread: "synthetic",
            },
            &[Distilled {
                id: "c_private",
                number: "D1",
                kind: "concept",
                subject: "Invented Finch",
                predicate: "uses",
                object: "amber kite",
                date: "2031-06-01",
                quote: Some("uses amber kite"),
                transcript: "private-entry.md:1",
                demoted: None,
            }],
            "2031-06-01T12:00:00Z",
        )
        .unwrap();
    drop(store);
    let mut graph = Graph::open(&path, &kg, None).unwrap();
    let args = |file: &str| json!({"subject":"Invented Finch","predicate":"uses","object":"amber kite","source_closet":json!({"transcript":format!("{file}:1"),"claim_id":"c_private"}).to_string()});
    graph.call("add", &args("private-entry.md")).unwrap();
    graph.call("add", &args("shared-entry.md")).unwrap();
    let prepared = graph
        .prepare(&Request {
            verb: Verb::Retract,
            target: "c_private",
            text: Some("Synthetic correction"),
            by: "ann",
            supersede: false,
            grounding_failed: false,
        })
        .unwrap();
    let applied = graph.commit(&prepared, &[]).unwrap();
    assert_eq!(
        graph
            .call("query", &json!({"entity":"Invented Finch"}))
            .unwrap()["facts"][0]["valid_to"],
        serde_json::Value::Null
    );
    let c = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM privacy_retired", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    graph.undo(&applied.event, "ann").unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM privacy_retired", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // A failed write leaves a private prelabel, never an unmarked KG row. Shared support
    // is not prepublished, and failed support on an existing triple does not widen it.
    c.execute_batch("CREATE TRIGGER privacy_abort_audit BEFORE INSERT ON write_log WHEN json_extract(NEW.payload,'$.operation')='kg_add' BEGIN SELECT RAISE(ABORT,'synthetic audit failure'); END;").unwrap();
    let count = || {
        c.query_row(
            "SELECT count(*) FROM privacy_markers WHERE kind='triple'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = count();
    let mut private_args = args("private-entry.md");
    private_args["object"] = json!("private orphan");
    assert!(graph.call("add", &private_args).is_err());
    assert_eq!(count(), before + 1);
    assert_eq!(
        graph
            .call("query", &json!({"entity":"Invented Finch"}))
            .unwrap()["count"],
        1
    );
    let mut shared_args = args("shared-entry.md");
    shared_args["object"] = json!("shared aborted");
    assert!(graph.call("add", &shared_args).is_err());
    assert_eq!(count(), before + 1);
    palace_store::privacy::register(
        &c,
        "other-private.md",
        &"cy"
            .parse::<palace_store::privacy::Audience>()
            .unwrap()
            .marker("synthetic-other-source"),
    )
    .unwrap();
    assert!(graph.call("add", &args("other-private.md")).is_err());
    assert_eq!(count(), before + 1);
}
