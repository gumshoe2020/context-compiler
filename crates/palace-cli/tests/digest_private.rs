//! Synthetic digest fixtures only; all stores stay inside the repository.
use palace_core::recall::{Counts, RULE_VALUES_ANYWHERE};
use palace_store::{claims::ClaimStore, fidelity::Row, privacy::People};
use serde_json::json;
use std::{path::PathBuf, process::Command};

const SENTENCE: &str = "The invented cobalt observatory received its secret shipment.";
const VALUE: &str = "quartz-817-secret";

fn fixture(name: &str) -> (PathBuf, ClaimStore) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/settle1j/{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.canonicalize().unwrap().join("compiler.sqlite");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut store = ClaimStore::open(&path).unwrap();
    let people = People::parse(
        r#"{"people":{"ann":{"owner":true,"handles":[]},"wren":{"handles":[]}},"conversations":[{"match":"owner-private","people":["ann"],"private":true},{"match":"other-private","people":["wren"],"private":true},{"match":"shared","people":["ann","wren"]}]}"#,
    ).unwrap();
    let at = store.now().unwrap();
    for (file, sampled, supported) in [
        ("shared", 10, 8),
        ("owner-private", 20, 15),
        ("other-private", 30, 22),
    ] {
        let marker = people.resolve(file, None);
        assert_eq!(marker.private, file != "shared");
        if file == "other-private" {
            assert!(!marker.admits(&["ann".into()]));
        }
        store.mark_entry(file, &marker).unwrap();
        store.record_fidelity(&Row {
            point: "distill",
            file,
            kind: "specific",
            counts: &Counts { sampled, supported, missing: sampled - supported, ..Default::default() },
            rollup_depth: None,
            primary_share: None,
            judge: RULE_VALUES_ANYWHERE,
            items: &json!({
                "items":[{"sentence":SENTENCE,"values":[VALUE],"file":"hidden/source/path"}],
                "values_unexplained":sampled - supported,
                "values_skipped":2,
                "checklist_values":sampled + 2,
                "values_kept_in":{"specific":supported - 1,"concept":1,"entry":0,"secret":VALUE},
                "passage":SENTENCE,
            }),
        }, &at).unwrap();
    }
    (path, store)
}

#[test]
fn digest_pools_shared_owner_private_and_other_person_private_counts() {
    let (path, store) = fixture("pooled");
    // The existing text-bearing reader retains its shared-only privacy boundary.
    assert_eq!(store.fidelity_rows(None).unwrap().len(), 1);
    drop(store);
    let output = Command::new(env!("CARGO_BIN_EXE_palace"))
        .args([
            "--person",
            "orin",
            "--assistant",
            "finch",
            "improve",
            "--digest",
            "--store",
        ])
        .arg(path)
        .args(["--since", "2000-01-01"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.lines().find(|line| line.starts_with("F:")),
        Some(
            "F: specifics 45/60 values (75%, 7d; 42 as specifics), unexplained 15/60, skipped 6/66 (9%), concepts unjudged (7d) (30-day -)"
        )
    );
}

#[test]
fn counts_reader_serializes_only_allowed_fields_and_numeric_item_counters() {
    let (_, store) = fixture("counts");
    let rows = store.fidelity_counts(None).unwrap();
    assert_eq!(rows.len(), 3);
    let serialized = serde_json::to_string(&rows).unwrap();
    for secret in [
        SENTENCE,
        VALUE,
        "hidden/source/path",
        "owner-private",
        "other-private",
    ] {
        assert!(!serialized.contains(secret), "leaked {secret}");
    }
    for (row, sampled, supported) in [(&rows[0], 10, 8), (&rows[1], 20, 15), (&rows[2], 30, 22)] {
        assert_eq!(
            *row,
            json!({
                "point":"distill", "type":"specific", "judge":RULE_VALUES_ANYWHERE,
                "sampled":sampled, "supported":supported, "unjudged":0,
                "items":{"values_unexplained":sampled - supported,"values_skipped":2,
                    "checklist_values":sampled + 2,
                    "values_kept_in":{"specific":supported - 1,"concept":1,"entry":0}},
            })
        );
    }
    assert_eq!(store.fidelity_counts(Some("2000-01-01")).unwrap(), rows);
    assert!(
        store
            .fidelity_counts(Some("9999-01-01"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn counts_reader_drops_text_in_counter_slots_and_tolerates_legacy_items() {
    let (_, mut store) = fixture("legacy");
    for (file, items) in [
        (
            "text-counters",
            json!({"values_unexplained":SENTENCE,"values_skipped":VALUE,
            "checklist_values":{"text":SENTENCE},"values_kept_in":{"specific":VALUE}}),
        ),
        ("legacy-array", json!([SENTENCE, VALUE])),
    ] {
        store
            .record_fidelity(
                &Row {
                    point: "consolidation",
                    file,
                    kind: "concept",
                    counts: &Counts::default(),
                    rollup_depth: None,
                    primary_share: None,
                    judge: "none",
                    items: &items,
                },
                "2001-01-01T00:00:00",
            )
            .unwrap();
    }
    // Default window excludes old rows; explicit window admits them without item text.
    assert_eq!(store.fidelity_counts(None).unwrap().len(), 3);
    let all = store.fidelity_counts(Some("2000-01-01")).unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(all[0]["items"], json!({}));
    assert_eq!(all[1]["items"], json!({}));
}
