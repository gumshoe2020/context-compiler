//! The corrections digest (M4d-2d item 3; SPEC §5.1 revised point 3: "Callum sees corrections in
//! the digest afterwards and can undo any of them"), pure. At most `max` lines: one per
//! correction by anyone other than the person (with the claim, what changed and `undo e_…`), then
//! ONE summary line for rule and judge events, then one line of open counts. Nothing at all when
//! the window holds no correction and no rule event. Terms (`coin`, adoption, veto) and canaries
//! are never in it: terms have their own monthly digest.
use std::collections::BTreeMap;

/// The default and usual `--max`.
pub const DIGEST_MAX: usize = 10;
/// A claim statement in a digest line is clipped to this many characters.
const STATEMENT_MAX: usize = 80;

/// One event in the digest window, as the store read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    /// `e_…`, or for an escalation the ask's token `#f_…`.
    pub id: String,
    pub by: String,
    /// The verb, or `escalate`.
    pub verb: String,
    /// `<rollup date> Cn`.
    pub label: String,
    pub statement: String,
    /// The claim's status before and after the event (equal when it did not change).
    pub status: (String, String),
    /// The successor's statement, or an escalation's proposal.
    pub successor: Option<String>,
}

/// Whether the event belongs in the digest: not the person's own (they made it), not a term event,
/// and not a `rule:recheck` survival (log-only, SPEC §2.3 amendment a).
pub fn included(by: &str, verb: &str) -> bool {
    let term = matches!(verb, "coin" | "adopt" | "veto") || by == "rule:adopt";
    by != crate::names::person() && by != "rule:recheck" && !term
}

/// A person's event (not a rule's or judge's): its own line.
pub fn is_person(by: &str) -> bool {
    !by.starts_with("rule:") && !by.starts_with("judge:")
}

/// `fergus contradict 2026-09-22 C5 "Fergus posted …": disputed → superseded; now "…" — undo e_…`;
/// an escalation: `fergus escalate … "…": proposes "…" — asked you #f_…`.
pub fn person_line(c: &Correction) -> String {
    let (by, verb, label) = (&c.by, &c.verb, &c.label);
    let statement = clip(&c.statement);
    if c.verb == "escalate" {
        let proposal = clip(c.successor.as_deref().unwrap_or_default());
        let id = &c.id;
        return format!(
            "{by} {verb} {label} \"{statement}\": proposes \"{proposal}\" — asked you {id}"
        );
    }
    let (before, after) = &c.status;
    let mut changed = if before == after {
        format!("stays {after}")
    } else {
        format!("{before} → {after}")
    };
    if let Some(successor) = &c.successor {
        changed.push_str(&format!("; now \"{}\"", clip(successor)));
    }
    format!(
        "{by} {verb} {label} \"{statement}\": {changed} — undo {}",
        c.id
    )
}

/// The outcome word a rule event is counted under.
pub fn outcome(c: &Correction) -> String {
    match c.verb.as_str() {
        "weaken" if c.status.1 == "disputed" => "disputed".into(),
        "weaken" => "weakened".into(),
        "add" => "re-grounded".into(),
        "expire" => "expired".into(),
        "confirm" => "confirmed".into(),
        "retract" => "retracted".into(),
        "undo" => "undone".into(),
        other => other.into(),
    }
}

/// The order outcomes are listed in (PLAN's example first); anything else follows.
const OUTCOMES: [&str; 7] = [
    "weakened",
    "expired",
    "re-grounded",
    "disputed",
    "confirmed",
    "retracted",
    "undone",
];

/// `judge:x: 1 re-grounded; rule:auto: 3 weakened, 1 disputed` (actors by name, outcomes in
/// `OUTCOMES` order, so the line does not depend on event order), or `None`.
pub fn rule_summary(rules: &[&Correction]) -> Option<String> {
    let mut actors: BTreeMap<&str, BTreeMap<(usize, String), usize>> = BTreeMap::new();
    for c in rules {
        let word = outcome(c);
        let rank = OUTCOMES
            .iter()
            .position(|o| *o == word)
            .unwrap_or(OUTCOMES.len());
        *actors
            .entry(&c.by)
            .or_default()
            .entry((rank, word))
            .or_default() += 1;
    }
    let parts: Vec<String> = actors
        .iter()
        .map(|(by, counts)| format!("{by}: {}", counted(counts)))
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn counted(counts: &BTreeMap<(usize, String), usize>) -> String {
    let words: Vec<String> = counts
        .iter()
        .map(|((_, w), n)| format!("{n} {w}"))
        .collect();
    words.join(", ")
}

/// The open counts the digest ends with: Fergus's queue, open asks, and open asks not yet sent
/// (M4d-2e item 2: an ask a failed run recorded but nobody delivered shows here).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Open {
    pub queue: usize,
    pub asks: usize,
    pub unsent: usize,
}

/// `queue=… asks=… unsent=…`.
pub fn open_line(open: Open) -> String {
    format!(
        "queue={} asks={} unsent={}",
        open.queue, open.asks, open.unsent
    )
}

/// `--since`: `YYYY-MM-DD`, optionally followed by `THH:MM[:SS…]` (compared with event times as
/// text, which sort chronologically).
pub fn valid_since(since: &str) -> bool {
    let b = since.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.into_iter().all(|i| b[i].is_ascii_digit());
    let date = b.len() >= 10 && digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-';
    let time = b.len() == 10 || (b.len() >= 16 && b[10] == b'T' && digits(11..13) && b[13] == b':');
    date && digits(8..10)
        && time
        && since
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":-.T".contains(c))
}

/// The digest's lines, at most `max`; empty when there is nothing to report.
pub fn lines(events: &[Correction], open: Open, max: usize) -> Vec<String> {
    let kept: Vec<&Correction> = events.iter().filter(|c| included(&c.by, &c.verb)).collect();
    let (people, rules): (Vec<&Correction>, Vec<&Correction>) =
        kept.into_iter().partition(|c| is_person(&c.by));
    if people.is_empty() && rules.is_empty() {
        return Vec::new();
    }
    let tail: Vec<String> = rule_summary(&rules)
        .into_iter()
        .chain([open_line(open)])
        .collect();
    let mut out = person_lines(&people, max.saturating_sub(tail.len()));
    out.extend(tail);
    out.truncate(max);
    out
}

/// At most `room` lines; when clipped, the last says how many more there are.
fn person_lines(people: &[&Correction], room: usize) -> Vec<String> {
    if people.len() <= room {
        return people.iter().map(|c| person_line(c)).collect();
    }
    let shown = room.saturating_sub(1);
    let mut out: Vec<String> = people[..shown].iter().map(|c| person_line(c)).collect();
    if room > 0 {
        let more = people.len() - shown;
        out.push(format!(
            "+{more} more corrections: palace improve --digest --max <n>"
        ));
    }
    out
}

fn clip(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= STATEMENT_MAX {
        return flat;
    }
    let cut: String = flat.chars().take(STATEMENT_MAX - 1).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(by: &str, verb: &str, status: (&str, &str)) -> Correction {
        Correction {
            id: "e_0123456789ab".into(),
            by: by.into(),
            verb: verb.into(),
            label: "2026-09-22 C5".into(),
            statement: "Fergus posted the reply".into(),
            status: (status.0.into(), status.1.into()),
            successor: None,
        }
    }
    #[test]
    fn since_is_a_date_or_a_time() {
        for ok in [
            "2026-09-27",
            "2026-09-27T20:12",
            "2026-09-27T20:12:00.000000Z",
        ] {
            assert!(valid_since(ok), "{ok}");
        }
        for bad in [
            "",
            "yesterday",
            "2026-9-27",
            "2026-09-27 20:12",
            "2026-09-27T",
            "2026-09-27';--",
        ] {
            assert!(!valid_since(bad), "{bad}");
        }
    }
    #[test]
    fn nothing_when_there_is_nothing() {
        assert!(
            lines(
                &[],
                Open {
                    queue: 1,
                    asks: 1,
                    unsent: 1
                },
                DIGEST_MAX
            )
            .is_empty()
        );
        let only_terms_and_the_person = [
            event("rule:auto", "coin", ("live", "live")),
            event("callum", "contradict", ("live", "superseded")),
            event("rule:recheck", "confirm", ("live", "live")),
        ];
        assert!(lines(&only_terms_and_the_person, Open::default(), DIGEST_MAX).is_empty());
    }
    #[test]
    fn people_first_then_one_rule_line_then_open_counts() {
        let mut fix = event("fergus", "contradict", ("disputed", "superseded"));
        fix.successor = Some("Callum posted the reply himself".into());
        let events = [
            event("rule:auto", "weaken", ("live", "live")),
            event("rule:auto", "weaken", ("live", "disputed")),
            fix,
            event("rule:auto", "expire", ("live", "expired")),
            event("judge:x", "add", ("live", "superseded")),
            event("rule:auto", "weaken", ("live", "live")),
        ];
        let open = Open {
            queue: 1,
            asks: 2,
            unsent: 1,
        };
        let out = lines(&events, open, DIGEST_MAX);
        assert_eq!(
            out,
            [
                "fergus contradict 2026-09-22 C5 \"Fergus posted the reply\": disputed → superseded; now \"Callum posted the reply himself\" — undo e_0123456789ab",
                "judge:x: 1 re-grounded; rule:auto: 2 weakened, 1 expired, 1 disputed",
                "queue=1 asks=2 unsent=1",
            ]
        );
    }
    #[test]
    fn at_most_max_lines() {
        let events: Vec<Correction> = (0..12)
            .map(|_| event("fergus", "confirm", ("disputed", "live")))
            .chain([event("rule:auto", "weaken", ("live", "live"))])
            .collect();
        let out = lines(&events, Open::default(), DIGEST_MAX);
        assert_eq!(out.len(), DIGEST_MAX);
        assert!(out[7].starts_with("+5 more corrections"), "{out:?}");
        assert!(out[8].starts_with("rule:auto: 1 weakened"));
        assert_eq!(lines(&events, Open::default(), 2).len(), 2);
    }
    #[test]
    fn an_escalation_names_its_ask_and_long_statements_are_clipped() {
        let mut up = event("fergus", "escalate", ("disputed", "disputed"));
        up.id = "#f_0123abcd".into();
        up.successor = Some("x".repeat(200));
        let line = person_line(&up);
        assert!(line.contains("\": proposes \"xxx"), "{line}");
        assert!(line.ends_with("— asked you #f_0123abcd"), "{line}");
        assert!(line.contains('…') && line.chars().count() < 200, "{line}");
    }
}
