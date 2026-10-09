//! Signal replies (M4d item 5; SPEC §6 and §5.1 as revised 2026-09-27), pure: what a reply says
//! and which verb it becomes. Replies name actions: `keep` → confirm; `accept` → the proposal
//! (Fergus's escalated claim as a contradict, else the finding's own text-free verb); `drop` →
//! retract; `fix: <text>` → contradict with that text as the successor; `source: <file>` → add
//! citing the file (ungrounded asks); `veto <word>` → retire a proposed term; `undo e_<id>` →
//! undo that event (the person, from the corrections digest; M4d-2d item 3). `ok` and `no` are
//! refused (they were ambiguous: "looks REVERSED — ok"). Anything else is a usage error.
use crate::improve::Verb;

pub const USAGE: &str = "reply with: keep | accept | fix: <subject> | <predicate> | <object> | source: <file.md> | drop | veto <word> | undo e_<id>";
/// `ok` and `no` no longer answer any ask; the accepted words are listed.
pub const OBSOLETE: &str = "`ok` and `no` are no longer replies: say keep / accept / fix: <text> (a line with a source) or source: <file> / drop / fix: <text> (no source found)";

#[derive(Debug, PartialEq)]
pub enum Reply {
    Keep,
    Accept,
    Drop,
    Fix(String),
    Source(String),
    Veto(String),
    Undo(String),
}

/// Parses a reply (keywords case-insensitive; `fix:`/`source:` keep their text as written).
pub fn parse(text: &str) -> Result<Reply, &'static str> {
    let text = normalise(text);
    let lower = text.to_lowercase();
    match lower.as_str() {
        "ok" | "no" => return Err(OBSOLETE),
        "keep" => return Ok(Reply::Keep),
        "accept" => return Ok(Reply::Accept),
        "drop" => return Ok(Reply::Drop),
        _ => {}
    }
    let rest = |prefix: &str| {
        lower
            .starts_with(prefix)
            .then(|| text[prefix.len()..].trim().to_owned())
            .filter(|r| !r.is_empty())
    };
    if let Some(fix) = rest("fix:") {
        return Ok(Reply::Fix(fix));
    }
    if let Some(file) = rest("source:").filter(|f| is_entry_name(f)) {
        return Ok(Reply::Source(file));
    }
    if let Some(event) = rest("undo ").filter(|e| is_event_id(e)) {
        return Ok(Reply::Undo(event));
    }
    match rest("veto ") {
        Some(word) if !word.contains(char::is_whitespace) => Ok(Reply::Veto(word.to_lowercase())),
        _ => Err(USAGE),
    }
}

/// One line, runs of whitespace collapsed, trimmed: the idempotency key's text.
pub fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An improvement event id: `e_` and twelve lowercase hex digits.
fn is_event_id(id: &str) -> bool {
    id.strip_prefix("e_").is_some_and(|hex| {
        hex.len() == 12
            && hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    })
}

/// A diary entry's file name: no path, ends `.md`.
fn is_entry_name(file: &str) -> bool {
    file.ends_with(".md") && !file.contains(['/', '\\']) && !file.starts_with('.')
}

/// Whether `reply` answers this target (M4d-2c item 3, review S-2): an ungrounded claim (no
/// source found) takes `source:` / `drop` / `fix:` — `keep` would confirm a claim with no source;
/// any other finding takes `keep` / `accept` / `fix:`. `veto` has no target.
pub fn allowed(reply: &Reply, ungrounded: bool) -> Result<(), String> {
    let fits = match reply {
        Reply::Keep | Reply::Accept => !ungrounded,
        Reply::Source(_) | Reply::Drop => ungrounded,
        Reply::Fix(_) | Reply::Veto(_) | Reply::Undo(_) => true,
    };
    if fits {
        return Ok(());
    }
    let (what, words) = if ungrounded {
        ("no source was found for this claim", UNGROUNDED_WORDS)
    } else {
        ("this line has a source", WORDS)
    };
    Err(format!("{what}: reply {words}"))
}

const WORDS: &str = "keep / accept / fix: <text>";
const UNGROUNDED_WORDS: &str = "source: <file> / drop / fix: <text>";

/// The refusal of a reply to a finding already decided (M4d-2c item 3, review S-2).
pub fn already_answered(answer: &str, event: &str) -> String {
    format!("already answered \"{answer}\" (event {event}); use palace improve undo {event} first")
}

/// A proposal or `fix:` text never carries a reply token: two `#f_` in one Signal line would
/// make the reply ambiguous (M4d-2d item 4, review L-7).
pub fn tokenless(text: &str) -> Result<(), String> {
    if text.to_lowercase().contains("#f_") {
        return Err("a proposal may not contain a reply token (#f_…)".into());
    }
    Ok(())
}

/// Only a reversal is escalated (M4d-2d item 2, review S-2): `accept` on an escalation means
/// contradict-with-the-proposal, which is wrong for a conflation (`split`) or an omission (`add`);
/// those stay in Fergus's queue for him to correct. `Err` says why and what to do instead.
pub fn escalatable(kind: &str) -> Result<(), String> {
    if kind == "reversal" {
        return Ok(());
    }
    Err(format!(
        "escalate is for reversals only; this is a {kind} finding: correct it with split / add --by {}",
        crate::names::assistant()
    ))
}

/// What `accept` applies: Fergus's escalated claim (a contradict with it as the successor), else
/// the finding's own proposal verb when it needs no text (stale → expire, unsupported → weaken).
/// A reversal without an escalation has nothing to accept.
pub fn accepted(
    kind: &str,
    proposal_verb: &str,
    escalated: Option<&str>,
) -> Result<(Verb, Option<String>), String> {
    if let Some(claim) = escalated.filter(|_| escalatable(kind).is_ok()) {
        return Ok((Verb::Contradict, Some(claim.to_owned())));
    }
    match Verb::parse(proposal_verb) {
        Some(verb) if kind != "reversal" && !verb.needs_text() => Ok((verb, None)),
        _ => Err(format!(
            "`accept` has no proposal to apply to this {kind} finding; reply fix: <text>"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_parse() {
        assert_eq!(parse(" KEEP "), Ok(Reply::Keep));
        assert_eq!(parse("accept"), Ok(Reply::Accept));
        assert_eq!(parse("Drop"), Ok(Reply::Drop));
        assert_eq!(
            parse("fix:  Callum | posted | it himself"),
            Ok(Reply::Fix("Callum | posted | it himself".into()))
        );
        assert_eq!(
            parse("Source: 2026-09-21-c0ad0aed1741.md"),
            Ok(Reply::Source("2026-09-21-c0ad0aed1741.md".into()))
        );
        assert_eq!(parse("veto Recurs_In"), Ok(Reply::Veto("recurs_in".into())));
        assert_eq!(
            parse(" Undo  e_0123456789ab "),
            Ok(Reply::Undo("e_0123456789ab".into()))
        );
        for bad in [
            "",
            "okay",
            "fix:",
            "source: ../x.md",
            "source: notes.txt",
            "veto two words",
            "undo",
            "undo e_123",
            "undo e_0123456789AB",
            "undo e_0123456789ab extra",
            "undo f_0123456789ab",
            "maybe",
        ] {
            assert_eq!(parse(bad), Err(USAGE), "{bad:?}");
        }
        assert_eq!(normalise("fix:  a |\tb | c "), "fix: a | b | c");
    }
    #[test]
    fn ok_and_no_are_refused_with_the_accepted_words() {
        for word in ["ok", " OK ", "no", "No"] {
            let refused = parse(word).unwrap_err();
            assert_eq!(refused, OBSOLETE);
            for accepted in ["keep", "accept", "fix:", "source:", "drop"] {
                assert!(refused.contains(accepted), "{accepted}");
            }
        }
    }
    #[test]
    fn keep_never_confirms_an_ungrounded_claim() {
        let refused = allowed(&Reply::Keep, true).unwrap_err();
        assert!(
            refused.contains("source: <file> / drop / fix: <text>"),
            "{refused}"
        );
        assert!(allowed(&Reply::Accept, true).is_err());
        assert!(allowed(&Reply::Drop, false).is_err());
        assert!(allowed(&Reply::Source("a.md".into()), false).is_err());
        for ok in [
            Reply::Drop,
            Reply::Source("a.md".into()),
            Reply::Fix("x".into()),
        ] {
            assert_eq!(allowed(&ok, true), Ok(()));
        }
        for ok in [Reply::Keep, Reply::Accept, Reply::Fix("x".into())] {
            assert_eq!(allowed(&ok, false), Ok(()));
        }
        assert_eq!(
            already_answered("keep", "e_1"),
            "already answered \"keep\" (event e_1); use palace improve undo e_1 first"
        );
    }
    #[test]
    fn accept_applies_the_proposal() {
        let escalated = accepted("reversal", "contradict", Some("A | b | c"));
        assert_eq!(escalated, Ok((Verb::Contradict, Some("A | b | c".into()))));
        assert_eq!(accepted("stale", "expire", None), Ok((Verb::Expire, None)));
        assert_eq!(
            accepted("unsupported", "weaken", None),
            Ok((Verb::Weaken, None))
        );
        assert!(accepted("reversal", "contradict", None).is_err());
        assert!(accepted("conflation", "split", None).is_err());
        assert!(accepted("conflation", "split", Some("A | b | c")).is_err());
        assert!(accepted("omission", "add", Some("A | b | c")).is_err());
    }
    #[test]
    fn proposals_carry_no_token() {
        assert!(tokenless("Callum | posted | it").is_ok());
        assert!(tokenless("Callum | posted | #f_0123abcd").is_err());
        assert!(tokenless("Callum | posted | #F_0123ABCD").is_err());
    }
    #[test]
    fn only_a_reversal_is_escalated() {
        assert_eq!(escalatable("reversal"), Ok(()));
        for kind in ["conflation", "omission", "unsupported", "stale"] {
            let refused = escalatable(kind).unwrap_err();
            assert!(
                refused.contains(kind) && refused.contains("reversals only"),
                "{refused}"
            );
        }
    }
}
