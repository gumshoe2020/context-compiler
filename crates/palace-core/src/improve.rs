//! SPEC §3 improvement verbs (M4d item 3), the pure part: what each verb decides, which axis it
//! improves, which claim status it leaves, who may act, and how its text becomes claims.
use crate::claim::{Claim, parse};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verb {
    Confirm,
    Contradict,
    Add,
    Weaken,
    Retract,
    Split,
    Expire,
    Coin,
}

pub const VERBS: [Verb; 8] = [
    Verb::Confirm,
    Verb::Contradict,
    Verb::Add,
    Verb::Weaken,
    Verb::Retract,
    Verb::Split,
    Verb::Expire,
    Verb::Coin,
];

impl Verb {
    pub fn parse(name: &str) -> Option<Self> {
        VERBS.into_iter().find(|v| v.name() == name)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Contradict => "contradict",
            Self::Add => "add",
            Self::Weaken => "weaken",
            Self::Retract => "retract",
            Self::Split => "split",
            Self::Expire => "expire",
            Self::Coin => "coin",
        }
    }
    /// The axis the verb improves (SPEC §3 table); never empty.
    pub fn axis(self) -> &'static [&'static str] {
        match self {
            Self::Confirm | Self::Expire => &["currency"],
            Self::Add => &["provenance"],
            Self::Weaken => &["scope"],
            Self::Contradict | Self::Retract | Self::Split | Self::Coin => &["precision"],
        }
    }
    /// Verbs whose text is required (it becomes claims).
    pub fn needs_text(self) -> bool {
        matches!(
            self,
            Self::Contradict | Self::Add | Self::Split | Self::Coin
        )
    }
    /// Finding kinds the verb decides on its claim (`confirm` decides any open finding).
    pub fn decides(self, kind: &str) -> bool {
        match self {
            Self::Confirm => true,
            Self::Contradict => kind == "reversal",
            Self::Add => kind == "omission",
            Self::Weaken | Self::Retract => kind == "unsupported",
            Self::Split => kind == "conflation",
            Self::Expire => kind == "stale",
            Self::Coin => kind == "vocabulary_gap",
        }
    }
    /// Whether the verb, acting for `by`, decides an open finding of `kind`. A `reversal` rests on
    /// a judge's reading, so only the person or the assistant close it, by any verb that ends, corrects or
    /// confirms the claim: confirm, contradict, retract, split or expire (SPEC §2.3 amendment c;
    /// M4d-2a item 6; M4d-2e item 1).
    pub fn decides_for(self, kind: &str, by: &str) -> bool {
        if kind == "reversal" {
            return self.settles_reversal() && is_person(by);
        }
        self.decides(kind)
    }
    /// Verbs by which a person settles a reversal (M4d-2e item 1).
    fn settles_reversal(self) -> bool {
        matches!(
            self,
            Self::Confirm | Self::Contradict | Self::Retract | Self::Split | Self::Expire
        )
    }
    /// The status the improved claim is left in, when the verb ends it.
    pub fn status_after(self) -> Option<&'static str> {
        match self {
            Self::Contradict => Some("superseded"),
            Self::Retract => Some("retracted"),
            Self::Split => Some("split"),
            Self::Expire => Some("expired"),
            _ => None,
        }
    }
    /// Verbs that act on a live claim (`add` and `coin` only anchor on theirs).
    pub fn requires_live(self) -> bool {
        !matches!(self, Self::Add | Self::Coin)
    }
    /// Verbs whose claim's direct edges get `valid_to`.
    pub fn ends_edges(self) -> bool {
        self.status_after().is_some()
    }
}

/// the person, the assistant, `rule:<name>` (`[a-z0-9._-]+`) or `judge:<model>` (`[a-z0-9._:-]+`):
/// nothing else, so `--by` can never carry markup or a second line (M4d-2a item 7, review L-4).
pub fn valid_by(by: &str) -> bool {
    let suffix = |prefix: &str, extra: &str| {
        by.strip_prefix(prefix).is_some_and(|rest| {
            !rest.is_empty()
                && rest.chars().all(|c| {
                    c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || "._-".contains(c)
                        || extra.contains(c)
                })
        })
    };
    is_person(by) || suffix("rule:", "") || suffix("judge:", ":")
}

fn is_person(by: &str) -> bool {
    crate::names::is_person(by)
}

/// Who may report a failed grounding attempt: a rule, the assistant or the person — never a judge
/// (M4d-2a.1 item 3, review L-5).
pub fn may_report_grounding(by: &str) -> bool {
    is_person(by) || by.starts_with("rule:")
}

/// People's decisions are not undone by rules (M4d-2a.1 item 1, review S-2): a `rule:*` or
/// `judge:*` may not undo (or redo) an event by the person or the assistant, nor any event whose changes
/// decide a `reversal` finding. The person and the assistant may undo anything. `Err` says why.
pub fn may_undo(by: &str, target_by: &str, decides_reversal: bool) -> Result<(), String> {
    if is_person(by) {
        return Ok(());
    }
    if is_person(target_by) {
        return Err(format!(
            "{target_by}'s decision: only {} may undo it",
            crate::names::either()
        ));
    }
    if decides_reversal {
        return Err(format!(
            "it decides a reversal finding: only {} may undo it",
            crate::names::either()
        ));
    }
    Ok(())
}

/// Who may assert (M4d-2d item 1, review S-1; SPEC §5.1 revised point 2): `contradict` and
/// `split` belong to the person or the assistant whatever the target; a rule or judge may `add` only as
/// re-grounding (`regrounds`: same fact, ungrounded anchor); `coin` stays open because it only
/// proposes a term (SPEC §2.4). `Err` says why.
pub fn may_assert(verb: Verb, by: &str, regrounds: bool) -> Result<(), String> {
    if is_person(by) {
        return Ok(());
    }
    match verb {
        Verb::Contradict | Verb::Split => Err(format!(
            "{} is person-only: {by} may not assert a correction (only {})",
            verb.name(),
            crate::names::either()
        )),
        Verb::Add if !regrounds => Err(format!(
            "add by {by} is re-grounding only: the same subject, predicate and object as an ungrounded anchor, only source and quote differ"
        )),
        _ => Ok(()),
    }
}

/// A disputed claim is confirmed, expired or retracted only by the person or the assistant: a rule's or
/// judge's verb would settle a reversal without a person (M4d-2d item 4, review L-1; M4d-2e
/// item 1, review M-2). `contradict` and `split` are person-only already (`may_assert`).
pub fn may_settle(verb: Verb, by: &str, disputed: bool) -> Result<(), String> {
    let settles = matches!(verb, Verb::Confirm | Verb::Expire | Verb::Retract);
    if disputed && settles && !is_person(by) {
        return Err(format!(
            "{by} may not {} a disputed claim: only {} settle a reversal",
            verb.name(),
            crate::names::either()
        ));
    }
    Ok(())
}

/// Whether `new` states the same fact as `anchor`: subject, predicate and object identical.
pub fn same_fact(anchor: &Claim, new: &Claim) -> bool {
    (&anchor.subject, &anchor.predicate, &anchor.object)
        == (&new.subject, &new.predicate, &new.object)
}

/// The free text of a verb is one line: a newline could forge headings or tags in the IMPROVE
/// entry (M4d-2a item 7, review L-4).
pub fn one_line(text: &str) -> bool {
    !text.contains(['\n', '\r'])
}

/// `text` as claim `number` in the pinned grammar (`<subject> | <predicate> | <object> [— <date>
/// — <source> [— quote: "…"]]`); a text without date and source takes the anchor claim's.
pub fn statement(text: &str, number: &str, anchor: &Claim) -> Result<Claim, String> {
    let line = |body: &str| parse(&format!("## Claims\n- {number}. {body}\n"));
    let given = line(text.trim());
    if let Some(claim) = given.claims.into_iter().next() {
        return Ok(claim);
    }
    let defaulted = line(&format!(
        "{} — {} — {}",
        text.trim(),
        anchor.date,
        anchor.source
    ));
    match (defaulted.claims.into_iter().next(), given.malformed.first()) {
        (Some(claim), _) => Ok(claim),
        (None, Some(m)) => Err(m.reason.clone()),
        (None, None) => Err("text is not a claim".into()),
    }
}

/// `split`: exactly two statements separated by ` || `.
pub fn split_statements(
    text: &str,
    numbers: [&str; 2],
    anchor: &Claim,
) -> Result<[Claim; 2], String> {
    let parts: Vec<&str> = text.split(" || ").collect();
    let [first, second] = parts.as_slice() else {
        return Err("split needs exactly two claims separated by ` || `".into());
    };
    Ok([
        statement(first, numbers[0], anchor)?,
        statement(second, numbers[1], anchor)?,
    ])
}

/// `coin`: `<word>: <definition> || <negative space>`.
#[derive(Debug, PartialEq)]
pub struct Coinage {
    pub word: String,
    pub definition: String,
    pub negative_space: String,
}

pub fn coinage(text: &str) -> Result<Coinage, String> {
    let usage = "coin needs `<word>: <definition> || <what it is NOT>`";
    let (word, rest) = text.split_once(':').ok_or(usage)?;
    let (definition, negative) = rest.split_once(" || ").ok_or(usage)?;
    let word = word
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    let (definition, negative) = (definition.trim(), negative.trim());
    if word.is_empty() || definition.is_empty() || negative.is_empty() {
        return Err(usage.into());
    }
    Ok(Coinage {
        word,
        definition: definition.to_owned(),
        negative_space: negative.to_owned(),
    })
}

/// The term's two definition claims (SPEC §2.4: definition and negative space are claims).
pub fn term_claims(c: &Coinage, numbers: [&str; 2], anchor: &Claim) -> Result<[Claim; 2], String> {
    Ok([
        statement(
            &format!("{} | means | {}", c.word, c.definition),
            numbers[0],
            anchor,
        )?,
        statement(
            &format!("{} | differs from | {}", c.word, c.negative_space),
            numbers[1],
            anchor,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn anchor() -> Claim {
        parse("## Claims\n- C5. Fergus | posted | the reply — 2026-09-22 — 2026-09-21-6cf0aec0f2f4.md\n")
            .claims
            .remove(0)
    }
    #[test]
    fn every_verb_improves_an_axis_and_round_trips() {
        for verb in VERBS {
            assert!(!verb.axis().is_empty());
            assert_eq!(Verb::parse(verb.name()), Some(verb));
        }
        assert_eq!(Verb::parse("undo"), None);
    }
    #[test]
    fn only_a_person_closes_a_reversal() {
        for verb in [
            Verb::Confirm,
            Verb::Contradict,
            Verb::Retract,
            Verb::Split,
            Verb::Expire,
        ] {
            assert!(verb.decides_for("reversal", "callum"));
            assert!(verb.decides_for("reversal", "fergus"));
            assert!(!verb.decides_for("reversal", "rule:auto"));
            assert!(!verb.decides_for("reversal", "judge:sonnet"));
        }
        assert!(!Verb::Weaken.decides_for("reversal", "callum"));
        assert!(!Verb::Add.decides_for("reversal", "fergus"));
        assert!(Verb::Confirm.decides_for("stale", "rule:recheck"));
        assert!(!Verb::Retract.decides_for("stale", "callum"));
    }
    #[test]
    fn who_may_act() {
        for ok in ["callum", "fergus", "rule:auto", "judge:sonnet"] {
            assert!(valid_by(ok), "{ok}");
        }
        for ok in [
            "rule:auto.v2",
            "rule:re-check_1",
            "judge:claude-sonnet-5:high",
        ] {
            assert!(valid_by(ok), "{ok}");
        }
        for bad in [
            "",
            "rule:",
            "judge: ",
            "bob",
            "Callum",
            "rule:x\n\nTags: no-compress",
            "rule:a b",
            "rule:A",
            "rule:x:y",
            "judge:m#1",
            "judge:**bold**",
        ] {
            assert!(!valid_by(bad), "{bad}");
        }
    }
    #[test]
    fn grounding_is_reported_by_rules_and_people() {
        for ok in ["rule:auto", "fergus", "callum"] {
            assert!(may_report_grounding(ok), "{ok}");
        }
        assert!(!may_report_grounding("judge:sonnet"));
    }
    #[test]
    fn rules_do_not_undo_people() {
        for person in ["callum", "fergus"] {
            assert!(may_undo(person, "callum", true).is_ok());
            assert!(may_undo(person, "rule:auto", false).is_ok());
        }
        for rule in ["rule:auto", "judge:sonnet"] {
            assert!(
                may_undo(rule, "fergus", false)
                    .unwrap_err()
                    .contains("fergus")
            );
            assert!(may_undo(rule, "callum", false).is_err());
            assert!(may_undo(rule, "rule:auto", true).is_err());
            assert!(may_undo(rule, "rule:auto", false).is_ok());
            assert!(may_undo(rule, "judge:sonnet", false).is_ok());
        }
    }
    #[test]
    fn asserting_verbs_are_person_only() {
        for verb in [Verb::Contradict, Verb::Split] {
            for by in ["rule:auto", "judge:sonnet"] {
                assert!(may_assert(verb, by, true).unwrap_err().contains(by));
            }
            assert!(may_assert(verb, "fergus", false).is_ok());
        }
        assert!(may_assert(Verb::Add, "rule:auto", false).is_err());
        assert!(may_assert(Verb::Add, "judge:sonnet", true).is_ok());
        assert!(may_assert(Verb::Add, "callum", false).is_ok());
        assert!(may_assert(Verb::Coin, "rule:auto", false).is_ok());
        let regrounded = statement(
            "Fergus | posted | the reply — 2026-09-22 — other.md",
            "C9",
            &anchor(),
        )
        .unwrap();
        assert!(same_fact(&anchor(), &regrounded));
        let changed = statement("Fergus | posted | a reply", "C9", &anchor()).unwrap();
        assert!(!same_fact(&anchor(), &changed));
    }
    #[test]
    fn only_a_person_settles_a_disputed_claim() {
        for verb in [Verb::Confirm, Verb::Expire, Verb::Retract] {
            let refused = may_settle(verb, "rule:auto", true).unwrap_err();
            assert!(refused.contains("rule:auto") && refused.contains(verb.name()));
            assert!(may_settle(verb, "judge:sonnet", true).is_err());
            assert!(may_settle(verb, "fergus", true).is_ok());
            assert!(may_settle(verb, "rule:auto", false).is_ok());
        }
        assert!(
            may_settle(Verb::Weaken, "rule:auto", true).is_ok(),
            "only lowers"
        );
    }
    #[test]
    fn free_text_is_one_line() {
        assert!(one_line("Callum | posted | it"));
        assert!(!one_line("a\n## Void supersedes"));
        assert!(!one_line("a\rb"));
    }
    #[test]
    fn statements_take_the_anchor_date_and_source() {
        let c = statement("Callum | posted | the reply himself", "C15", &anchor()).unwrap();
        assert_eq!((c.number.as_str(), c.date.as_str()), ("C15", "2026-09-22"));
        assert_eq!(c.source, "2026-09-21-6cf0aec0f2f4.md");
        let full = statement(
            "Callum | posted | it — 2026-09-23 — 2026-09-21-c0ad0aed1741.md — quote: \"posted\"",
            "C16",
            &anchor(),
        )
        .unwrap();
        assert_eq!(full.source, "2026-09-21-c0ad0aed1741.md");
        assert!(statement("no pipes here", "C17", &anchor()).is_err());
        assert!(statement("Fergus | corrected by | Callum", "C17", &anchor()).is_err());
        assert!(split_statements("A | b | c", ["C1", "C2"], &anchor()).is_err());
        let coin = coinage("Anchor Word: a definition || not the other").unwrap();
        assert_eq!(coin.word, "anchor_word");
        let [def, neg] = term_claims(&coin, ["C1", "C2"], &anchor()).unwrap();
        assert_eq!(
            (def.predicate.as_str(), neg.predicate.as_str()),
            ("means", "differs_from")
        );
    }
}
