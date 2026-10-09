use palace_core::diary_memory::Memory;
use palace_core::{claim, rollup::Entry};
use palace_store::claims::{Checked, ClaimStore};
#[test]
fn repeated_findings_are_deduped_and_decay_once() {
    let text = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md\n- C2. Callum | chose | Go — 2026-09-21 — missing.md\n- C3. not a claim\n";
    let entry = Entry::parse("r", text.into());
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary.0.insert("source".into(), "Callum chose Rust".into());
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let mut confidences = Vec::new();
    for run in 0..2 {
        let mut rows = claim::check(
            &entry,
            &parsed.claims,
            &diary,
            None::<&palace_core::diary_memory::FakeJudge>,
        );
        rows.extend(claim::malformed_rows(&entry, &parsed.malformed));
        let at = format!("2026-09-2{run}T00:00:00Z");
        let checked = Checked {
            rollup: "r",
            date: "2026-09-22",
            thread: "t",
            judge: "judge:fake",
            claims: &parsed.claims,
            rows: &mut rows,
        };
        assert_eq!(store.record(checked, &at).unwrap(), 2);
        confidences.push((rows[0]["confidence"].clone(), rows[1]["confidence"].clone()));
    }
    assert_eq!(
        confidences[0],
        (serde_json::json!(0.6), serde_json::json!(0.45))
    );
    assert_eq!(confidences[1], confidences[0]);
    assert_eq!(store.counts().unwrap(), (2, 2, 0));
    let c2 = &parsed.claims[1].id;
    assert_eq!(
        store.open_findings(c2).unwrap(),
        [("unsupported".to_owned(), 2)]
    );
    store.verify_chain().unwrap();
}
#[test]
fn citation_is_recorded_once_and_logged() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    assert!(store.cite("t_a_b_c", "c_1", "r.md").unwrap());
    assert!(!store.cite("t_a_b_c", "c_1", "r.md").unwrap());
    assert_eq!(
        store.citations("t_a_b_c").unwrap(),
        [("c_1".to_owned(), "r.md".to_owned())]
    );
    assert_eq!(store.events("citation").unwrap().len(), 1);
    store.verify_chain().unwrap();
}
#[test]
fn findings_table_from_before_m4b1_gains_seen_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compiler.sqlite");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE findings(id TEXT PRIMARY KEY, claim TEXT, rollup TEXT NOT NULL, kind TEXT NOT NULL, evidence TEXT NOT NULL, proposal TEXT, checker TEXT NOT NULL, checked_at TEXT NOT NULL, decision TEXT); INSERT INTO findings VALUES ('f_1','c_1','r','stale','{}',NULL,'rule','2026-09-26T00:00:00Z',NULL);")
        .unwrap();
    let store = ClaimStore::open(&path).unwrap();
    assert_eq!(
        store.open_findings("c_1").unwrap(),
        [("stale".to_owned(), 1)]
    );
}
/// M4d item 1: finding shape, `(rollup, number)` lookup, per-run seen_count, recheck closing.
#[test]
fn findings_carry_passage_proposal_checker_and_close_on_recheck() {
    use palace_store::lookup::FindingFilter;
    let text = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md — quote: \"Callum chose Rust\"\n";
    let entry = Entry::parse("r", text.into());
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary
        .0
        .insert("source".into(), "# s\nline two\nCallum chose Go.".into());
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let run = |store: &mut ClaimStore, diary: &Memory, at: &str, twice: bool| {
        for _ in 0..if twice { 2 } else { 1 } {
            let mut rows = claim::check(
                &entry,
                &parsed.claims,
                diary,
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
            store.record(checked, at).unwrap();
        }
    };
    run(&mut store, &diary, "2026-09-22T00:00:00Z", true);
    let open = store
        .findings(&FindingFilter {
            open: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(open.len(), 1);
    let f = &open[0];
    assert_eq!(f["kind"], "unsupported");
    assert_eq!(f["seen_count"], 1, "two observations in one run count once");
    assert_eq!(f["evidence"]["passage"], "# s\nline two\nCallum chose Go.");
    assert_eq!(f["evidence"]["locator"], "L1-L3");
    assert_eq!(f["proposal"]["verb"], "weaken");
    assert_eq!(
        f["checker"],
        "rule:source-absent".replace("source-absent", "claims-check")
    );
    let id = &parsed.claims[0].id;
    assert_eq!(store.resolve("2026-09-22 C1").unwrap().claim_id, *id);
    assert_eq!(store.resolve("r c1").unwrap().claim_id, *id);
    let token = palace_core::finding::reply_token(f["id"].as_str().unwrap());
    assert_eq!(
        store.resolve(&token).unwrap().finding.as_deref(),
        f["id"].as_str()
    );
    assert!(palace_store::Refused::is(
        &store.resolve("2026-09-22 C9").unwrap_err()
    ));
    diary
        .0
        .insert("source".into(), "# s\nCallum chose Rust".into());
    run(&mut store, &diary, "2026-09-23T00:00:00Z", false);
    assert!(
        store
            .findings(&FindingFilter {
                open: true,
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
    let events = store.improvement_events().unwrap();
    assert_eq!(
        (events[0]["verb"].as_str(), events[0]["by"].as_str()),
        (Some("confirm"), Some("rule:recheck"))
    );
    assert_eq!(store.claim(id).unwrap().unwrap()["confidence"], 0.8);
    diary
        .0
        .insert("source".into(), "# s\nCallum chose Go.".into());
    run(&mut store, &diary, "2026-09-24T00:00:00Z", false);
    let all = store.findings(&FindingFilter::default()).unwrap();
    assert_eq!(
        all.len(),
        2,
        "a closed finding that recurs is a new finding"
    );
    assert_eq!(store.claim(id).unwrap().unwrap()["confidence"], 0.45);
    store.verify_chain().unwrap();
}

/// M4d-2a item 6 (L-14): a later `preserved` verdict does not close a reversal (only a reply or
/// the person or the assistant does); `rule:recheck` still closes the other kinds.
#[test]
fn recheck_never_closes_a_reversal() {
    use palace_core::diary_memory::FakeJudge;
    use palace_store::lookup::FindingFilter;
    let text = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md — quote: \"Callum chose Rust\"\n";
    let entry = Entry::parse("r", text.into());
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary
        .0
        .insert("source".into(), "# s\nCallum chose Rust.".into());
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    for (verdict, at) in [
        ("reversed", "2026-09-22T00:00:00Z"),
        ("preserved", "2026-09-23T00:00:00Z"),
    ] {
        let mut rows = claim::check(&entry, &parsed.claims, &diary, Some(&FakeJudge(verdict)));
        let checked = Checked {
            rollup: "r",
            date: "2026-09-22",
            thread: "t",
            judge: "judge:fake",
            claims: &parsed.claims,
            rows: &mut rows,
        };
        store.record(checked, at).unwrap();
    }
    let open = store
        .findings(&FindingFilter {
            open: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["kind"], "reversal");
    assert!(store.improvement_events().unwrap().is_empty());
}

/// M4d-2c item 5 (review S-4): five watches with the same judge and source leave a judged claim at
/// 0.85 (base 0.80 + one survived check); an edited source counts once more, a new judge once more.
#[test]
fn a_survived_check_counts_once_per_judge_and_source() {
    let text = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md — quote: \"Callum chose Rust\"\n";
    let entry = Entry::parse("r", text.into());
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary.0.insert("source".into(), "Callum chose Rust.".into());
    let dir = tempfile::tempdir().unwrap();
    let mut store = ClaimStore::open(&dir.path().join("compiler.sqlite")).unwrap();
    let judge = palace_core::diary_memory::FakeJudge("preserved");
    let watch = |store: &mut ClaimStore, diary: &Memory, judge_label: &str, run: usize| {
        let mut rows = claim::check(&entry, &parsed.claims, diary, Some(&judge));
        let checked = Checked {
            rollup: "r",
            date: "2026-09-22",
            thread: "t",
            judge: judge_label,
            claims: &parsed.claims,
            rows: &mut rows,
        };
        store
            .record(checked, &format!("2026-09-2{run}T00:00:00Z"))
            .unwrap();
        rows[0]["confidence"].as_f64().unwrap()
    };
    let five: Vec<f64> = (0..5)
        .map(|run| watch(&mut store, &diary, "judge:table", run))
        .collect();
    assert_eq!(five, [0.85; 5], "no inflation by repetition");
    diary
        .0
        .insert("source".into(), "Callum chose Rust, again.".into());
    assert_eq!(
        watch(&mut store, &diary, "judge:table", 5),
        0.9,
        "edited source"
    );
    assert_eq!(watch(&mut store, &diary, "judge:table", 6), 0.9);
    assert_eq!(
        watch(&mut store, &diary, "judge:opus", 7),
        0.95,
        "a second judge"
    );
}

/// M4d-2c item 5 (review d6): a 0.95 claim with a stale finding sits at 0.80, but the §5.1
/// threshold is measured without that finding, so it gets its second check.
#[test]
fn the_second_check_threshold_ignores_the_finding_under_decision() {
    let text = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md — quote: \"Callum chose Rust\"\n";
    let parsed = claim::parse(text);
    let mut diary = Memory::default();
    diary.0.insert("source".into(), "Callum chose Rust.".into());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compiler.sqlite");
    let mut store = ClaimStore::open(&path).unwrap();
    let entry = Entry::parse("r", text.into());
    let mut rows = claim::check(
        &entry,
        &parsed.claims,
        &diary,
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
    store.record(checked, "2026-09-22T00:00:00Z").unwrap();
    drop(store);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute("UPDATE claims SET survived=3", []).unwrap();
    c.execute("INSERT INTO findings(id,claim,rollup,kind,evidence,checker,checked_at) SELECT 'f_stale0', id, 'r', 'stale', '{}', 'rule:stale', '2026-09-23' FROM claims", []).unwrap();
    let id: String = c
        .query_row("SELECT id FROM claims", [], |r| r.get(0))
        .unwrap();
    let with = palace_store::claims::derived(&c, &id).unwrap();
    let without = palace_store::claims::derived_without(&c, &id, "f_stale0").unwrap();
    assert_eq!((with, without), (0.8, 0.95));
    assert!(!palace_core::auto::needs_second_check(with));
    assert!(
        palace_core::auto::needs_second_check(without),
        "the mature claim is re-checked"
    );
}
