//! The automatic loop's decisions (M4d item 4; SPEC §5.1 as revised 2026-09-27), pure: what one
//! open finding gets this run — a mechanical verb, a dispute (a reversal: lowered, marked
//! `disputed`, queued for Fergus), a grounding attempt, an ask, a recorded proposal, or nothing —
//! and the per-run rate limits. The CLI gathers the facts and acts.
use crate::{claim::Claim, improve::Verb};

/// Below this an ungrounded claim becomes an ask (SPEC §5.1 row 3).
pub const ASK_FLOOR: f64 = 0.40;
/// Above this a second check must agree before any automatic verb (SPEC §5.1 row 1).
pub const RECHECK_ABOVE: f64 = 0.80;
/// Default `--max-verbs`: automatic non-weaken verbs per run.
pub const MAX_VERBS: usize = 5;
/// Every automatic event's author.
pub const BY: &str = "rule:auto";
/// The text of the `weaken` that disputes a reversal (its finding stays open for Fergus).
pub const DISPUTE_REASON: &str = "reversal flagged by judge";
/// Nothing below this, and nothing disputed, may be promoted to a keystone (SPEC §5).
pub const KEYSTONE_FLOOR: f64 = 0.60;

/// What the loop knows about one open finding at the start of the run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts<'a> {
    pub kind: &'a str,
    /// The claim is live or disputed (not ended by a verb).
    pub claim_live: bool,
    /// The claim is `disputed`: a reversal was lowered and waits in Fergus's queue.
    pub disputed: bool,
    pub confidence: f64,
    /// The claim's source file was found (the finding's evidence is not `absent-source`).
    pub source_present: bool,
    /// Re-read now (review S-3): the claim's OWN cited source exists and locates the claim, so the
    /// next watch grounds it — no attempt, no ask.
    pub own_source_found: bool,
    /// An ask for this finding is open (asked, not answered): never repeated.
    pub asked: bool,
    /// An identical `add` was refused after an operator's undo: never retried.
    pub vetoed: bool,
    /// the person or the assistant acted on this claim: the loop does not override them.
    pub person_acted: bool,
    /// The finding's proposal text is a coinage (`word: definition || not …`).
    pub coinable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Apply a mechanical verb to the finding.
    Apply(Verb),
    /// A reversal: `weaken` by rule:auto (no second decrement), the claim marked `disputed`, the
    /// finding left open in Fergus's queue. Never an ask (the source exists: provenance).
    Dispute,
    /// One grounding attempt (success `add --supersede`, failure `weaken --grounding-failed`).
    Ground,
    /// One Signal ask: `ungrounded` (an `escalated` ask is Fergus's, never the loop's).
    Ask(&'static str),
    /// Record the finding's proposal (`split`, `add`, `coin`) without applying it.
    Propose(&'static str),
    Skip(&'static str),
}

/// SPEC §5.1 for one finding. A judge may not assert: a reversal is only lowered and disputed.
pub fn plan(f: &Facts) -> Plan {
    if !f.claim_live {
        return Plan::Skip("claim not live");
    }
    if f.asked {
        return Plan::Skip("ask open");
    }
    if f.vetoed {
        return Plan::Skip("vetoed");
    }
    let planned = by_kind(f);
    // A grounding attempt still runs for a claim a person acted on: `after_ground` decides.
    if matches!(planned, Plan::Apply(_) | Plan::Dispute) && f.person_acted {
        return Plan::Skip("people first");
    }
    planned
}

/// A disputed claim is ended only by a person (M4d-2e item 1, review M-2): every finding on it is
/// skipped except an `unsupported` weaken, which only lowers.
fn by_kind(f: &Facts) -> Plan {
    match live_kind(f) {
        Plan::Apply(Verb::Weaken) if f.disputed && f.kind == "unsupported" => {
            Plan::Apply(Verb::Weaken)
        }
        _ if f.disputed => Plan::Skip(disputed()),
        planned => planned,
    }
}

fn live_kind(f: &Facts) -> Plan {
    match f.kind {
        "reversal" => Plan::Dispute,
        "unsupported" if f.source_present => Plan::Apply(Verb::Weaken),
        "unsupported" if f.own_source_found => Plan::Skip(OWN_SOURCE_FOUND),
        "unsupported" if f.confidence >= ASK_FLOOR => Plan::Ground,
        "unsupported" => Plan::Ask("ungrounded"),
        "stale" => Plan::Apply(Verb::Expire),
        "conflation" => Plan::Propose("split"),
        "omission" => Plan::Propose("add"),
        "vocabulary_gap" if f.coinable => Plan::Apply(Verb::Coin),
        "vocabulary_gap" => Plan::Propose("coin"),
        _ => Plan::Skip("no automatic handling"),
    }
}

/// "disputed: in <assistant>'s queue", as the `&'static str` a `Plan::Skip` carries.
fn disputed() -> &'static str {
    static REASON: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    REASON.get_or_init(|| format!("disputed: in {}'s queue", crate::names::assistant()))
}
const OWN_SOURCE_FOUND: &str = "own source now present: the next watch grounds it";

/// A claim-store row (`lookup::claim` JSON) as a `Claim`, for the pure checks.
pub fn claim_of(row: &serde_json::Value) -> Claim {
    let text = |k: &str| row[k].as_str().unwrap_or_default().to_owned();
    Claim {
        number: text("number"),
        id: text("id"),
        statement: format!(
            "{} {} {}",
            text("subject"),
            text("predicate").replace('_', " "),
            text("object")
        ),
        subject: text("subject"),
        predicate: text("predicate"),
        object: text("object"),
        date: text("date"),
        source: text("source"),
        quote: row["quote"].as_str().map(str::to_owned),
        superseded_by: None,
        corrected: false,
    }
}

/// The successor a successful grounding adds: the same statement and date citing `source`, with
/// the quote kept only when it is verbatim in the found text.
pub fn grounded_statement(c: &Claim, source: &str, text: &str) -> String {
    let base = format!(
        "{} | {} | {} — {} — {source}",
        c.subject,
        c.predicate.replace('_', " "),
        c.object,
        c.date
    );
    match c
        .quote
        .as_deref()
        .filter(|q| !q.is_empty() && text.contains(*q))
    {
        Some(quote) => format!("{base} — quote: \"{quote}\""),
        None => base,
    }
}

/// What a grounding attempt leads to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grounded {
    /// `add --supersede` citing the found source.
    Add,
    /// The identical add exists already (an operator undid it): vetoed, never retried.
    Veto,
    /// `weaken --grounding-failed`.
    Weaken,
    /// the person or the assistant acted on the claim: nothing automatic.
    Skip,
}

/// After the search: a found source whose add was refused before is a veto (M4d-2a L-4) —
/// recorded even for a claim a person acted on, since the undo that caused it is theirs;
/// otherwise a person's claim is left alone.
pub fn after_ground(found: bool, add_exists: bool, person_acted: bool) -> Grounded {
    match (found, add_exists, person_acted) {
        (true, true, _) => Grounded::Veto,
        (_, _, true) => Grounded::Skip,
        (true, false, false) => Grounded::Add,
        (false, _, false) => Grounded::Weaken,
    }
}

/// Whether a claim may be promoted to a keystone: confident enough and not disputed.
pub fn keystone_eligible(confidence: f64, status: &str) -> bool {
    confidence >= KEYSTONE_FLOOR && status == "live"
}

/// Whether a claim this confident needs a second, agreeing check before an automatic verb.
pub fn needs_second_check(confidence: f64) -> bool {
    confidence > RECHECK_ABOVE
}

/// Per-run limits: at most `max_verbs` automatic non-weaken verbs, of which at most one `coin`.
/// `weaken` (the safe direction) is never limited.
#[derive(Debug)]
pub struct Budget {
    max_verbs: usize,
    verbs: usize,
    coins: usize,
}

impl Budget {
    pub fn new(max_verbs: usize) -> Self {
        Self {
            max_verbs,
            verbs: 0,
            coins: 0,
        }
    }
    /// Takes one slot for `verb`; `false` when the run's limit is reached.
    pub fn admit(&mut self, verb: Verb) -> bool {
        if verb == Verb::Weaken {
            return true;
        }
        if self.verbs >= self.max_verbs || (verb == Verb::Coin && self.coins >= 1) {
            return false;
        }
        self.verbs += 1;
        self.coins += usize::from(verb == Verb::Coin);
        true
    }
}

/// The claim as it was right after a `rule:auto` event (M5b-5 item 1, review M-1).
#[derive(Clone, Copy, Debug)]
pub struct AfterEvent<'a> {
    /// The event's undo has been applied.
    pub undone: bool,
    pub status: &'a str,
    pub confidence: f64,
}

/// A finding that recurs on a (claim, kind) last decided by a `rule:auto` event repeats that
/// event (no new finding, no new event) while the event stands and the claim's status and
/// confidence are what they were right after it; otherwise it is a new finding.
pub fn is_repeat(after: &AfterEvent, status: &str, confidence: f64) -> bool {
    !after.undone && after.status == status && (after.confidence - confidence).abs() < 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts(kind: &str, confidence: f64, source_present: bool) -> Facts<'_> {
        Facts {
            kind,
            claim_live: true,
            confidence,
            source_present,
            ..Default::default()
        }
    }
    #[test]
    fn spec_5_1_by_kind() {
        assert_eq!(plan(&facts("reversal", 0.95, true)), Plan::Dispute);
        let mut disputed = facts("reversal", 0.45, true);
        disputed.disputed = true;
        assert_eq!(plan(&disputed), Plan::Skip("disputed: in fergus's queue"));
        assert_eq!(
            plan(&facts("unsupported", 0.45, true)),
            Plan::Apply(Verb::Weaken)
        );
        assert_eq!(plan(&facts("unsupported", 0.45, false)), Plan::Ground);
        assert_eq!(plan(&facts("unsupported", 0.40, false)), Plan::Ground);
        assert_eq!(
            plan(&facts("unsupported", 0.30, false)),
            Plan::Ask("ungrounded")
        );
        assert_eq!(plan(&facts("stale", 0.65, true)), Plan::Apply(Verb::Expire));
        assert_eq!(
            plan(&facts("conflation", 0.6, true)),
            Plan::Propose("split")
        );
        assert_eq!(plan(&facts("omission", 0.6, true)), Plan::Propose("add"));
        assert_eq!(
            plan(&facts("vocabulary_gap", 0.6, true)),
            Plan::Propose("coin")
        );
        let mut coin = facts("vocabulary_gap", 0.6, true);
        coin.coinable = true;
        assert_eq!(plan(&coin), Plan::Apply(Verb::Coin));
    }
    #[test]
    fn asks_are_not_repeated_and_people_come_first() {
        let mut f = facts("reversal", 0.45, true);
        f.asked = true;
        assert_eq!(plan(&f), Plan::Skip("ask open"));
        let mut f = facts("stale", 0.65, true);
        f.person_acted = true;
        assert_eq!(plan(&f), Plan::Skip("people first"));
        let mut f = facts("reversal", 0.45, true);
        f.person_acted = true;
        assert_eq!(plan(&f), Plan::Skip("people first"));
        let mut f = facts("unsupported", 0.45, false);
        f.person_acted = true;
        assert_eq!(plan(&f), Plan::Ground, "after_ground decides");
        assert_eq!(after_ground(true, true, true), Grounded::Veto);
        assert_eq!(after_ground(true, false, true), Grounded::Skip);
        assert_eq!(after_ground(false, false, true), Grounded::Skip);
        assert_eq!(after_ground(true, false, false), Grounded::Add);
        assert_eq!(after_ground(true, true, false), Grounded::Veto);
        assert_eq!(after_ground(false, false, false), Grounded::Weaken);
        f.confidence = 0.30;
        assert_eq!(plan(&f), Plan::Ask("ungrounded"), "asking overrides nobody");
        let mut f = facts("unsupported", 0.45, false);
        f.vetoed = true;
        assert_eq!(plan(&f), Plan::Skip("vetoed"));
        let mut f = facts("stale", 0.65, true);
        f.claim_live = false;
        assert_eq!(plan(&f), Plan::Skip("claim not live"));
    }
    /// M4d-2e item 1 (review M-2): on a disputed claim only the `unsupported` weaken runs; a
    /// stale expire, a grounding attempt, an ask, a proposal or a coin waits for a person.
    #[test]
    fn a_disputed_claim_is_ended_only_by_a_person() {
        let on_disputed = |kind, confidence, source_present, coinable| {
            let mut f = facts(kind, confidence, source_present);
            f.disputed = true;
            f.coinable = coinable;
            plan(&f)
        };
        assert_eq!(
            on_disputed("unsupported", 0.45, true, false),
            Plan::Apply(Verb::Weaken)
        );
        for (kind, confidence, source, coinable) in [
            ("stale", 0.45, true, false),
            ("reversal", 0.45, true, false),
            ("unsupported", 0.45, false, false),
            ("unsupported", 0.30, false, false),
            ("conflation", 0.45, true, false),
            ("omission", 0.45, true, false),
            ("vocabulary_gap", 0.45, true, true),
            ("vocabulary_gap", 0.45, true, false),
        ] {
            assert_eq!(
                on_disputed(kind, confidence, source, coinable),
                Plan::Skip(disputed()),
                "{kind} {confidence} {source}"
            );
        }
    }
    /// Review S-3: a source that appeared after the watch is neither searched for nor asked about.
    #[test]
    fn an_own_source_found_now_skips_grounding_and_asking() {
        for confidence in [0.45, 0.30] {
            let mut f = facts("unsupported", confidence, false);
            f.own_source_found = true;
            assert_eq!(plan(&f), Plan::Skip(OWN_SOURCE_FOUND), "{confidence}");
        }
    }
    #[test]
    fn rate_limits() {
        let mut b = Budget::new(2);
        assert!(b.admit(Verb::Coin));
        assert!(!b.admit(Verb::Coin), "one coin per run");
        assert!(b.admit(Verb::Expire));
        assert!(!b.admit(Verb::Add), "max verbs");
        assert!(b.admit(Verb::Weaken), "weaken is never limited");
        assert!(needs_second_check(0.85) && !needs_second_check(0.80));
    }
    #[test]
    fn a_disputed_claim_is_not_a_keystone() {
        assert!(keystone_eligible(0.60, "live"));
        assert!(!keystone_eligible(0.59, "live"));
        assert!(!keystone_eligible(0.95, "disputed"));
    }

    #[test]
    fn a_recurrence_repeats_only_a_standing_event_on_an_unchanged_claim() {
        let after = AfterEvent {
            undone: false,
            status: "live",
            confidence: 0.45,
        };
        assert!(is_repeat(&after, "live", 0.45));
        assert!(!is_repeat(&after, "disputed", 0.45), "status changed");
        assert!(!is_repeat(&after, "live", 0.30), "confidence changed");
        let undone = AfterEvent {
            undone: true,
            ..after
        };
        assert!(!is_repeat(&undone, "live", 0.45), "the event was undone");
    }
}
