//! PKG-1.2 item 4: with the names set to another person and assistant (once per process, so in
//! a test binary of its own), every role check and message follows them and other names (the
//! invented cast, or a build's owner names) carry no role.
use palace_core::{canary, digest, improve, names, reconcile, reply};

#[test]
fn other_names_take_the_roles() {
    names::set(Some("maria"), Some("codex")).unwrap();
    assert!(names::set(None, None).is_err(), "set once");
    assert!(improve::valid_by("maria") && improve::valid_by("codex"));
    assert!(!improve::valid_by("callum") && !improve::valid_by("fergus"));
    assert!(improve::may_report_grounding("codex") && !improve::may_report_grounding("fergus"));
    let refused = improve::may_undo("rule:auto", "maria", false).unwrap_err();
    assert_eq!(refused, "maria's decision: only maria or codex may undo it");
    assert!(improve::may_undo("callum", "rule:auto", true).is_err());
    assert!(canary::makes_canary("contradict", "maria"));
    assert!(!canary::makes_canary("contradict", "callum"));
    assert!(!digest::included("maria", "confirm") && digest::included("callum", "confirm"));
    assert!(reconcile::settle_hint("i_1").contains("--by maria|codex"));
    assert!(
        reply::escalatable("omission")
            .unwrap_err()
            .ends_with("--by codex")
    );
    if let Some((person, assistant)) = names::OWNER {
        let (person, assistant) = (names::display(person), names::display(assistant));
        let prompt = format!("Designed with {person}. {assistant} conceded; {person} decided.");
        assert_eq!(
            names::in_prose(&prompt),
            "Designed with Maria. Codex conceded; Maria decided."
        );
    }
    assert_eq!(
        names::in_prose("a summary for {Person}; {Assistant} acts; by {assistant}"),
        "a summary for Maria; Codex acts; by codex"
    );
}

#[test]
fn the_names_are_checked() {
    assert!(names::set(Some("same"), Some("same")).is_err());
}
