use palace_core::traits::{GraphHistory, GraphStore};
use palace_store::{claims::ClaimStore, context::Reader, graph::GraphReader};
use serde_json::json;

#[test]
fn missing_policy_hides_context_and_every_scoped_graph_operation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/priv1/scoped-reads-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let kg = root.join("knowledge_graph.sqlite3");
    let c = rusqlite::Connection::open(&kg).unwrap();
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT,type TEXT,properties TEXT,created_at TEXT);
        CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,confidence REAL,source_closet TEXT,valid_from TEXT,valid_to TEXT,extracted_at TEXT);
        INSERT INTO entities VALUES ('invented_finch','Invented Finch','person','{}',NULL),('cobalt_heron','cobalt heron','thing','{}',NULL);
        INSERT INTO triples VALUES ('t_canary','invented_finch','uses','cobalt_heron',1,'{}',NULL,NULL,'2031-06-01');").unwrap();
    let store_path = root.join("compiler.sqlite");
    // No compiler metadata at all must be empty, including the vector admit-list.
    rusqlite::Connection::open(&store_path).unwrap();
    let reader = Reader::open(&store_path, &kg).unwrap();
    assert!(reader.nodes().unwrap().is_empty());
    assert!(!reader.entry_visible("missing.md").unwrap());
    assert_eq!(reader.marked_entries().unwrap(), Some(vec![]));
    drop(reader);
    drop(ClaimStore::open(&store_path).unwrap());
    let mut store = palace_store::Store::open(&store_path).unwrap();
    let assert_hidden = || {
        let reader = GraphReader::open(&kg)
            .unwrap()
            .with_readers(vec!["ann".into()]);
        // Supplying null or a wider audience through call cannot replace reader scope.
        for spoof in [serde_json::Value::Null, json!(["wren"])] {
            assert_eq!(
                reader
                    .call(
                        "query",
                        &json!({"entity":"Invented Finch","_readers":spoof})
                    )
                    .unwrap()["count"],
                0
            );
            assert_eq!(
                reader
                    .call(
                        "timeline",
                        &json!({"entity":"cobalt heron","_readers":spoof})
                    )
                    .unwrap()["count"],
                0
            );
            assert_eq!(
                reader.call("stats", &json!({"_readers":spoof})).unwrap()["triples"],
                0
            );
            assert_eq!(
                reader.call("entities", &json!({"_readers":spoof})).unwrap(),
                json!([])
            );
        }
        assert!(
            reader
                .query_entity("Invented Finch", palace_core::triple::Direction::Both, None)
                .unwrap()
                .is_empty()
        );
        assert!(reader.entity("cobalt heron").unwrap().is_none());
        assert!(reader.list_entities().unwrap().is_empty());
        assert!(reader.timeline(None).unwrap().is_empty());
        assert_eq!(reader.stats().unwrap()["entities"], 0);
        let automatic = Reader::open(&store_path, &kg).unwrap();
        assert!(automatic.nodes().unwrap().is_empty());
    };
    assert_hidden();
    store
        .apply_privacy_backfill(&[(
            "triple".into(),
            "t_canary".into(),
            "wren"
                .parse::<palace_store::privacy::Audience>()
                .unwrap()
                .marker("synthetic-private"),
        )])
        .unwrap();
    assert_hidden();
    let private = GraphReader::open(&kg)
        .unwrap()
        .with_readers(vec!["wren".into()]);
    assert_eq!(private.timeline(Some("cobalt heron")).unwrap().len(), 1);
    assert_eq!(private.stats().unwrap()["triples"], 1);
    assert_eq!(private.list_entities().unwrap().len(), 2);
    // Deliberate operator search remains complete and annotated.
    let operator = GraphReader::open(&kg).unwrap();
    assert!(
        operator
            .call("query", &json!({"entity":"Invented Finch"}))
            .unwrap()
            .to_string()
            .contains("[private: wren; source: synthetic-private]")
    );
}
