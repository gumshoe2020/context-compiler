//! SETTLE-1b items 2–3 (spec §4): distilled claims with their memory type, replacement of a
//! specific, and the finding of a demoted one.
use palace_store::claims::{ClaimStore, Distilled, DistilledEntry};

fn specific<'a>(id: &'a str, object: &'a str, date: &'a str) -> Distilled<'a> {
    Distilled {
        id,
        number: "D1",
        kind: "specific",
        subject: "Larkspur ferry inspection",
        predicate: "is scheduled on",
        object,
        date,
        quote: Some(object),
        transcript: "/t/copy.jsonl:1",
        demoted: None,
    }
}

#[test]
fn a_changed_value_supersedes_the_old_specific_and_both_keep_their_dates() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let entry = |file| DistilledEntry {
        file,
        date: "2031-04-02",
        thread: "Larkspur",
    };
    let old = specific("c_old", "May 14", "2031-04-02");
    assert_eq!(
        store
            .record_distilled(&entry("a.md"), &[old], "t1")
            .unwrap(),
        1
    );
    let mut new = specific("c_new", "May 21", "2031-04-03");
    new.subject = "larkspur ferry  inspection ";
    store
        .record_distilled(&entry("b.md"), &[new], "t2")
        .unwrap();
    let rows = store.typed_claims().unwrap();
    let get = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap().clone();
    assert_eq!(get("c_old")["status"], "superseded");
    assert_eq!(get("c_old")["superseded_by"], "c_new");
    assert_eq!(get("c_old")["date"], "2031-04-02");
    assert_eq!(get("c_new")["status"], "live");
    assert_eq!(get("c_new")["date"], "2031-04-03");
    let live = store.live_specifics(&["b".into(), "a.md".into()]).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0]["object"], "May 21");
    store.verify_chain().unwrap();
}

#[test]
fn a_demoted_specific_is_a_concept_with_a_finding() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let mut claim = specific("c_d", "3,200 crowns", "2031-04-02");
    claim.kind = "concept";
    claim.quote = None;
    claim.demoted = Some("marked specific without a verbatim quote");
    let entry = DistilledEntry {
        file: "a.md",
        date: "2031-04-02",
        thread: "t",
    };
    store.record_distilled(&entry, &[claim], "t1").unwrap();
    let kinds: Vec<_> = store
        .open_findings("c_d")
        .unwrap()
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    assert_eq!(kinds, ["unquoted_specific"]);
    assert_eq!(store.typed_claims().unwrap()[0]["kind"], "concept");
    assert!(store.live_specifics(&["a".into()]).unwrap().is_empty());
}
