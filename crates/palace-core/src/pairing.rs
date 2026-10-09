//! M4d item 9 (Decided 2026-09-26 12:41): reversal ↔ `contradicts` pairing. A `(corrected by Cn)`
//! claim was WRONG when written and needs a `contradicts` relation citing the pair; a
//! `(superseded by Cn)` claim was true then and carries no negative edge. Pure.
use crate::{claim::Claim, proposal::Proposal};

/// Every pairing problem of a rollup's claims and proposed relations, as validator messages.
pub fn problems(claims: &[Claim], relations: &[Proposal]) -> Vec<String> {
    let mut problems = unknown_targets(claims);
    for old in unpaired(claims, relations) {
        let by = old.superseded_by.as_deref().unwrap_or_default();
        problems.push(format!(
            "{}: (corrected by {by}) requires a contradicts relation citing {by}",
            old.number
        ));
    }
    for (i, r) in relations.iter().enumerate() {
        problems.extend(relation_problem(i, r, claims));
    }
    problems
}

/// `(corrected by …)` claims left without a contradicts relation of their OWN (M4d-2a item 8,
/// review L-5): each relation pairs at most one claim; relations citing the old claim's own
/// number are assigned first, then those citing the shared replacement.
fn unpaired<'a>(claims: &'a [Claim], relations: &[Proposal]) -> Vec<&'a Claim> {
    let corrected: Vec<&Claim> = claims.iter().filter(|c| c.corrected).collect();
    let mut used = vec![false; relations.len()];
    let mut paired = vec![false; corrected.len()];
    for exact in [true, false] {
        for (k, old) in corrected.iter().enumerate() {
            if paired[k] {
                continue;
            }
            let own = |r: &Proposal| r.claim.as_deref() == Some(old.number.as_str());
            let found = relations.iter().enumerate().position(|(i, r)| {
                !used[i] && is_contradicts(r) && cites(r, old) && (own(r) || !exact)
            });
            if let Some(i) = found {
                used[i] = true;
                paired[k] = true;
            }
        }
    }
    corrected
        .into_iter()
        .zip(paired)
        .filter_map(|(c, p)| (!p).then_some(c))
        .collect()
}

fn is_contradicts(r: &Proposal) -> bool {
    r.relation.trim().eq_ignore_ascii_case("contradicts")
}

/// `r` cites the pair of which `old` is the old claim: `old` itself or its replacement.
fn cites(r: &Proposal, old: &Claim) -> bool {
    r.claim
        .as_deref()
        .is_some_and(|n| n == old.number || Some(n) == old.superseded_by.as_deref())
}

fn unknown_targets(claims: &[Claim]) -> Vec<String> {
    claims
        .iter()
        .filter_map(|c| {
            let by = c.superseded_by.as_deref().filter(|b| !b.is_empty())?;
            let known = claims
                .iter()
                .any(|n| n.number == by && n.number != c.number);
            (!known).then(|| format!("{}: marker names unknown claim {by}", c.number))
        })
        .collect()
}

/// A `contradicts` must cite a `(corrected by …)` pair; any other relation must not cite a
/// superseded or corrected claim.
fn relation_problem(i: usize, r: &Proposal, claims: &[Claim]) -> Option<String> {
    let cited = r.claim.as_deref()?;
    let marked = |corrected: bool| {
        claims
            .iter()
            .filter(|c| c.superseded_by.is_some() && c.corrected == corrected)
            .any(|old| cites(r, old))
    };
    if is_contradicts(r) {
        if marked(true) {
            return None;
        }
        if marked(false) {
            return Some(format!(
                "relation {i}: contradicts cites {cited}, a (superseded by …) pair; use (corrected by …) for a reversal"
            ));
        }
        return Some(format!(
            "relation {i}: contradicts must cite a (corrected by …) pair"
        ));
    }
    let old = claims
        .iter()
        .find(|c| c.number == cited && c.superseded_by.is_some())?;
    Some(format!(
        "relation {i} cites {}, a {} claim",
        old.number,
        if old.corrected {
            "corrected"
        } else {
            "superseded"
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{batch::parse_batch, claim::parse};

    const CLAIMS: &str = "## Claims\n- C3. Fergus | attributed | the cycle to Gaddis — 2026-09-21 — a.md\n- C7. Fergus | recommended | option 1 — 2026-09-22 — a.md\n- C8. (superseded by C7) Fergus | recommended | option 2 — 2026-09-22 — a.md\n- C14. (corrected by C3) Fergus | attributed | the cycle to Hollander — 2026-09-21 — a.md\n";

    fn relation(kind: &str, claim: &str) -> String {
        format!(
            r#"{{"from":"a","to":"b","type":"{kind}","from_type":"fact","to_type":"fact","evidence":[{{"query":"q"}}],"confidence":0.8,"supplied_by":"fergus","claim":"{claim}"}}"#
        )
    }
    fn problems_with(claims: &str, relations: &[String]) -> Vec<String> {
        let relations = parse_batch(&format!("[{}]", relations.join(",")), None).unwrap();
        problems(&parse(claims).claims, &relations)
    }
    #[test]
    fn a_paired_reversal_passes() {
        assert!(problems_with(CLAIMS, &[relation("contradicts", "C3")]).is_empty());
        assert!(problems_with(CLAIMS, &[relation("contradicts", "C14")]).is_empty());
    }
    #[test]
    fn corrected_by_without_contradicts_fails() {
        assert_eq!(
            problems_with(CLAIMS, &[relation("supports", "C3")]),
            ["C14: (corrected by C3) requires a contradicts relation citing C3"]
        );
    }
    #[test]
    fn contradicts_without_corrected_by_fails() {
        let claims = CLAIMS.replace("(corrected by C3) ", "");
        assert_eq!(
            problems_with(&claims, &[relation("contradicts", "C3")]),
            ["relation 0: contradicts must cite a (corrected by …) pair"]
        );
    }
    #[test]
    fn superseded_by_with_contradicts_fails() {
        let found = problems_with(
            CLAIMS,
            &[relation("contradicts", "C3"), relation("contradicts", "C7")],
        );
        assert_eq!(
            found,
            [
                "relation 1: contradicts cites C7, a (superseded by …) pair; use (corrected by …) for a reversal"
            ]
        );
    }
    #[test]
    fn relations_may_not_cite_a_superseded_or_corrected_claim() {
        let found = problems_with(
            CLAIMS,
            &[
                relation("contradicts", "C3"),
                relation("supports", "C8"),
                relation("supports", "C14"),
            ],
        );
        assert_eq!(
            found,
            [
                "relation 1 cites C8, a superseded claim",
                "relation 2 cites C14, a corrected claim"
            ]
        );
    }
    #[test]
    fn each_corrected_claim_needs_its_own_contradicts() {
        let claims = format!(
            "{CLAIMS}- C15. (corrected by C3) Fergus | attributed | the cycle to Pellham — 2026-09-21 — a.md\n"
        );
        assert_eq!(
            problems_with(&claims, &[relation("contradicts", "C3")]),
            ["C15: (corrected by C3) requires a contradicts relation citing C3"]
        );
        assert!(
            problems_with(
                &claims,
                &[
                    relation("contradicts", "C3"),
                    relation("contradicts", "C14")
                ]
            )
            .is_empty()
        );
    }
    #[test]
    fn a_marker_must_name_a_known_claim() {
        let claims = CLAIMS.replace("(superseded by C7)", "(superseded by C9)");
        assert!(
            problems_with(&claims, &[relation("contradicts", "C3")])
                .contains(&"C8: marker names unknown claim C9".to_owned())
        );
    }
    #[test]
    fn grammar_edges_of_review_l3() {
        let one = |line: &str| parse(&format!("## Claims\n{line}\n"));
        let passive = one("- C1. Fergus | corrected by | Callum — 2026-09-21 — a.md");
        assert!(passive.malformed[0].reason.contains("passive predicate"));
        assert!(
            one("- C1. A | bounded by | B — 2026-09-21 — a.md")
                .malformed
                .is_empty()
        );
        assert!(
            one("- C1. A | bounded_by | B — 2026-09-21 — a.md")
                .malformed
                .is_empty()
        );
        let two = one("- C2. (superseded by C7 and C9) A | likes | B — 2026-09-21 — a.md");
        assert!(two.malformed[0].reason.contains("exactly one claim"));
        let corrected = one("- C2. (Corrected by c3) A | likes | B — 2026-09-21 — a.md");
        assert!(corrected.claims[0].corrected);
        assert_eq!(corrected.claims[0].superseded_by.as_deref(), Some("C3"));
    }
    fn one(line: &str) -> crate::claim::Parsed {
        parse(&format!("## Claims\n{line}\n"))
    }
    /// M4d-2a item 8 (L-5): a leading `(corrected…`/`(superseded…` that is not exactly a marker
    /// is malformed, never absorbed into the subject.
    #[test]
    fn inexact_markers_are_malformed() {
        for bad in [
            "(corrected by: C3)",
            "(corrected)",
            "(superseded)",
            "(Superseded By  C7)",
            "(corrected by C3 )",
            "(corrected by C3)A",
            "(correctedby C3)",
        ] {
            let m = one(&format!("- C2. {bad} A | likes | B — 2026-09-21 — a.md"));
            assert_eq!(m.claims.len(), 0, "{bad} must not parse");
            assert_eq!(m.malformed.len(), 1, "{bad} must be malformed_claim");
        }
    }
    /// M4d-2a.1 item 5 (review L-1): a correction or supersession marker anywhere else in the
    /// subject is malformed — extra spaces, a second marker, full-width or square brackets,
    /// `correction by`.
    #[test]
    fn stray_markers_are_malformed() {
        for bad in [
            "- C4.  (corrected by C3) A | likes | B — 2026-09-21 — a.md",
            "- C4. (corrected by C3) (superseded by C5) A | likes | B — 2026-09-21 — a.md",
            "- C4. （corrected by C3） A | likes | B — 2026-09-21 — a.md",
            "- C4. [corrected by C3] A | likes | B — 2026-09-21 — a.md",
            "- C4. (correction by C3) A | likes | B — 2026-09-21 — a.md",
            "- C4. A (superseded by C5) | likes | B — 2026-09-21 — a.md",
            "- C4. A [ Superseded-By C5 ] | likes | B — 2026-09-21 — a.md",
        ] {
            let m = one(bad);
            assert_eq!((m.claims.len(), m.malformed.len()), (0, 1), "{bad}");
        }
        for fine in [
            "- C4. (corrected by C3) A | likes | B — 2026-09-21 — a.md",
            "- C4. A (the corrected text) | likes | B — 2026-09-21 — a.md",
            "- C4. A | likes | B (corrected by C3) — 2026-09-21 — a.md",
        ] {
            assert!(one(fine).malformed.is_empty(), "{fine}");
        }
    }
    /// M4d-2a.1 item 5: `read by` and `overseen by` are passive; `set`, `hit`, `led`, `built`,
    /// `paid`, `bought`, `held`, `kept` are active without `by`.
    #[test]
    fn participles_before_by_only() {
        for passive in [
            "read by",
            "overseen by",
            "paid by",
            "kept_by_callum",
            "was held by",
        ] {
            let m = one(&format!("- C1. A | {passive} | B — 2026-09-21 — a.md"));
            assert!(m.malformed[0].reason.contains("passive"), "{passive}");
        }
        for active in [
            "set", "hit", "led", "built", "paid", "bought", "held", "kept", "read",
        ] {
            let m = one(&format!("- C1. A | {active} | B — 2026-09-21 — a.md"));
            assert!(m.malformed.is_empty(), "{active}: {:?}", m.malformed);
        }
    }
    /// M4d-2a item 8 (L-5): passive `by` with objects, hyphens and underscores is refused; active
    /// phrasal verbs are not.
    #[test]
    fn passive_by_and_phrasal_verbs() {
        for passive in [
            "corrected by callum",
            "corrected-by",
            "approved_by_callum",
            "written by",
            "was told by",
        ] {
            let m = one(&format!("- C1. A | {passive} | B — 2026-09-21 — a.md"));
            assert!(m.malformed[0].reason.contains("passive"), "{passive}");
        }
        for active in [
            "stands by",
            "went by",
            "stopped by",
            "bounded-by",
            "Bounded By",
        ] {
            let m = one(&format!("- C1. A | {active} | B — 2026-09-21 — a.md"));
            assert!(m.malformed.is_empty(), "{active}: {:?}", m.malformed);
        }
    }
    #[test]
    fn check_routes_each_pair() {
        use crate::{claim::check, rollup::Entry, test_support::Memory};
        let claims = parse(CLAIMS).claims;
        let diary = Memory::default();
        let rows = check::<_, crate::test_support::Preserving>(
            &Entry::parse("r", String::new()),
            &claims,
            &diary,
            None,
        );
        assert_eq!(rows[2]["status"], "superseded");
        assert_eq!(rows[2]["pair"]["kind"], "stale");
        assert_eq!(rows[2]["pair"]["stale"], true);
        assert_eq!(rows[3]["status"], "corrected");
        assert_eq!(rows[3]["pair"]["kind"], "reversal");
        assert_eq!(rows[3]["pair"]["by"], "C3");
    }
}
