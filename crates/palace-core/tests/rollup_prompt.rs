//! PKG-1.3 item 1: prompts/rollup.md names no real person and carries no design provenance; it
//! keeps every step and rule in order, and `palace rollup` fills in the names it runs under.
use palace_core::{names, quote_rule, rollup_layout};

fn policy() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../prompts/rollup.md"
    ))
    .unwrap()
}

/// PUB-1a item 1: the private terms (real names, handles, topics) are not in this file; a checkout
/// that has the export's private list, dev/public/deny-terms, checks them too (`word`: whole word,
/// case-sensitive; `any`: anywhere, any case).
fn private_terms() -> Vec<(String, String)> {
    let list = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/public/deny-terms");
    let Ok(text) = std::fs::read_to_string(list) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(kind, _)| ["word", "any"].contains(kind))
        .map(|(kind, term)| (kind.to_owned(), term.to_owned()))
        .collect()
}

#[test]
fn the_prompt_names_no_owner() {
    let policy = policy();
    let text = policy.to_lowercase();
    let words: Vec<&str> = policy.split(|c: char| !c.is_alphanumeric()).collect();
    for (kind, term) in private_terms() {
        let found = match kind.as_str() {
            "word" => words.contains(&term.as_str()),
            _ => text.contains(&term.to_lowercase()),
        };
        assert!(!found, "{term}");
    }
    for term in ["designed with", "palace-fidelity-watch"] {
        assert!(!text.contains(term), "{term}");
    }
    let milestone = text.split(|c: char| !c.is_ascii_alphanumeric()).find(|w| {
        w.len() >= 2 && w.starts_with('m') && w[1..2].chars().all(|c| c.is_ascii_digit())
    });
    assert_eq!(milestone, None, "a milestone tag");
}

#[test]
fn every_step_in_order() {
    let text = policy();
    let mut at = 0;
    for step in [
        "RUNTIME NOTE:",
        "Step 0 — CHEAP BAIL CHECK",
        "Step 1 — SELECT ONE NODE/THREAD:",
        "SUPERSEDING A PRIOR ROLLUP:",
        "NO-COMPRESS TAG — GENERAL OPT-OUT:",
        "DESIGN THREADS — ALWAYS IN SCOPE, COMPRESSED DIFFERENTLY:",
        "Step 2 — PULL THE CLUSTER:",
        "Step 3 — WRITE THE ROLLUP (append-only):",
        "SUPERSEDES LAYOUT",
        rollup_layout::OPEN,
        rollup_layout::CLOSE,
        "Step 3b — CLAIMS FIRST",
        "QUOTES MUST CARRY THE OBJECT",
        quote_rule::OPEN,
        quote_rule::CLOSE,
        "SPECIFICS — CITE BY ID, NEVER RESTATE:",
        "Step 4 — WIRE INTO THE GRAPH",
        "Step 5 — REPORT:",
        "Guardrails:",
    ] {
        let found = text[at..].find(step).unwrap_or_else(|| panic!("{step}"));
        at += found + step.len();
    }
    assert_eq!(quote_rule::check_examples(&text), Ok(()));
}

#[test]
fn the_names_are_filled_in() {
    names::set(Some("callum"), Some("fergus")).unwrap();
    let text = names::in_prose(&policy());
    assert!(!text.contains("{Person}") && !text.contains("{Assistant}"));
    assert!(text.contains("Final reply is a short summary for Callum:"));
    assert!(text.contains("for Fergus to act on"));
    assert!(text.contains("\"supplied_by\": \"fergus\"}"));
}
