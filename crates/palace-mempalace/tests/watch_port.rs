//! The watch port's cases that need no fixture (claims checking, the reversal gate's confidence
//! and boundaries, diary reads, the claim grammar), through MemPalace's diary reader where a diary
//! is read; the cases on the fixture diary are the M4a doctor's (private, PUB-1a item 3).
use palace_mempalace::diary::Diary;

#[test]
fn claims_judge_reversal_confidence_and_boundaries() {
    use palace_core::diary_memory::{FakeJudge, Memory};
    use palace_core::{claim, rollup::Entry};
    let text = "## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md\n";
    let entry = Entry::parse("test", text.into());
    let claims = claim::parse(text).claims;
    let mut diary = Memory::default();
    diary.0.insert("source".into(), "Callum chose Rust".into());
    let result = claim::check(&entry, &claims, &diary, Some(&FakeJudge("reversed")));
    assert_eq!(result[0]["status"], "reversal");
    assert_eq!(result[0]["confidence"], 0.45);
    let result = claim::check(&entry, &claims, &diary, Some(&FakeJudge("preserved")));
    assert_eq!(result[0]["status"], "preserved");
    assert_eq!(result[0]["confidence"], 0.65);
    diary
        .0
        .insert("source".into(), format!("Callum{}Rust", " ".repeat(591)));
    assert_eq!(
        claim::check(&entry, &claims, &diary, None::<&FakeJudge>)[0]["status"],
        "unsupported"
    );
    diary
        .0
        .insert("source".into(), format!("Callum{}Rust", " ".repeat(590)));
    assert_eq!(
        claim::check(&entry, &claims, &diary, None::<&FakeJudge>)[0]["status"],
        "unchecked"
    );
    assert_eq!(claim::confidence(0.8, 100, 0), 0.99);
    assert_eq!(claim::confidence(0.6, 0, 100), 0.05);
    assert!(
        !claim::parse(&text.replace("source.md", "../source.md"))
            .malformed
            .is_empty()
    );
    assert!(
        !claim::parse(&format!(
            "{text}- C1. Callum | chose | Go — 2026-09-22 — source.md"
        ))
        .malformed
        .is_empty()
    );
}

#[test]
fn diary_filename_reads_reject_traversal_and_symlink_escape() {
    use palace_core::traits::DiaryReader;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("diary");
    std::fs::create_dir(&root).unwrap();
    let outside = directory.path().join("outside.md");
    std::fs::write(&outside, "outside").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link.md")).unwrap();
    let diary = Diary::open(&root).unwrap();
    assert!(diary.read("../outside.md").is_err());
    assert!(diary.read("link.md").is_err());
    assert!(diary.read("missing.md").unwrap().is_none());
}

#[test]
fn later_live_claim_in_another_rollup_makes_claim_stale() {
    use palace_core::diary_memory::{FakeJudge, Memory};
    use palace_core::{claim, rollup::Entry};
    let old=Entry::parse("old","## Claims\n- C1. Callum | chose | Rust — 2026-09-21 — source.md — quote: \"Callum chose Rust\"".into());
    let claims = claim::parse(&old.text).claims;
    let later =
        claim::parse("## Claims\n- C1. Callum | chose | Go — 2026-09-22 — source.md").claims;
    let mut diary = Memory::default();
    diary.0.insert("source".into(), "Callum chose Rust".into());
    assert_eq!(
        claim::check_against(&old, &claims, &later, &diary, None::<&FakeJudge>)[0]["status"],
        "stale"
    );
}

#[test]
fn claims_grammar_is_pinned_by_pipe_delimiters() {
    use palace_core::claim::parse;
    let claims = parse("## Claims (2)\n- C1. Alice Smith | likes | Rust — 2026-09-21 — source.md\n- C2. the substrate | depends on | MemPalace – 2026-09-21 – source.md – quote: “Yes — but pinned”").claims;
    assert_eq!(claims.len(), 2);
    assert_eq!(claims[0].subject, "Alice Smith");
    assert_eq!(claims[0].predicate, "likes");
    assert_eq!(claims[1].subject, "the substrate");
    assert_eq!(claims[1].predicate, "depends_on");
    assert_eq!(claims[1].quote.as_deref(), Some("Yes — but pinned"));
    let same =
        parse("## Claims\n- C9.  alice  smith |  Likes | rust - 2026-09-21 - source.md").claims;
    assert_eq!(
        same[0].id, claims[0].id,
        "ids derive from normalised fields"
    );
    let superseded = parse("## Claims\n- C3. (Superseded by C7) Fergus | recommended | option 2 — 2026-09-22 — source.md").claims;
    assert_eq!(superseded[0].superseded_by.as_deref(), Some("C7"));
    assert_eq!(superseded[0].subject, "Fergus");
    for line in [
        "- C1. Callum | was corrected by | Fergus — 2026-09-21 — source.md",
        "- C1. Fergus | got corrected by | Callum — 2026-09-21 — source.md",
        "- C1. Fergus got corrected by Callum — 2026-09-21 — source.md",
        "- C1. a | b | c | d — 2026-09-21 — source.md",
        "Claims below are from round three.",
        "- C1. Callum | chose | Rust — 21 Sept — source.md",
    ] {
        let parsed = parse(&format!("## Claims\n{line}"));
        assert!(parsed.claims.is_empty(), "{line}");
        assert_eq!(parsed.malformed[0].raw, line);
    }
}
