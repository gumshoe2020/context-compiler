//! SPEC §2.2 Finding shape (M4d item 1): which claim status is a finding, the ≤ 600-character
//! SOURCE passage with its line locator, and the proposal every kind carries. Pure.
use crate::claim::Claim;
use serde_json::{Value, json};

pub const PASSAGE_CHARS: usize = 600;

/// The SPEC §2.2 kind a claim-check status raises, if any (a missing source is `unsupported`).
pub fn kind_of(status: &str) -> Option<&'static str> {
    match status {
        "reversal" => Some("reversal"),
        "unsupported" | "absent-source" => Some("unsupported"),
        "stale" => Some("stale"),
        _ => None,
    }
}

/// A verbatim excerpt of a source and the 1-based line range it spans (`L3-L5`).
#[derive(Debug, PartialEq)]
pub struct Passage {
    pub text: String,
    pub locator: String,
}

/// ≤ 600 characters of `source` centred on where the claim is found: its quote, else its subject
/// (ASCII case-insensitive), else the start of the source.
pub fn passage(source: &str, claim: &Claim) -> Passage {
    let at = claim
        .quote
        .as_deref()
        .and_then(|q| source.find(q))
        .or_else(|| {
            source
                .to_ascii_lowercase()
                .find(&claim.subject.to_ascii_lowercase())
        })
        .unwrap_or(0);
    let (from, to) = centred(source, at, PASSAGE_CHARS);
    let text = &source[from..to];
    let first = source[..from].matches('\n').count() + 1;
    let last = first + text.trim_end_matches('\n').matches('\n').count();
    Passage {
        text: text.to_owned(),
        locator: format!("L{first}-L{last}"),
    }
}

/// Byte range of at most `chars` characters of `text` around byte offset `at`.
fn centred(text: &str, at: usize, chars: usize) -> (usize, usize) {
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    let centre = boundaries.partition_point(|&i| i < at);
    let start = centre
        .saturating_sub(chars / 2)
        .min(boundaries.len().saturating_sub(chars + 1));
    let end = (start + chars).min(boundaries.len() - 1);
    (boundaries[start], boundaries[end])
}

/// What the checker would do (SPEC §3): `{verb, text}` for each kind.
pub fn proposal(kind: &str, claim: &Claim, context: &[Claim]) -> Value {
    let (verb, text) = match kind {
        "stale" => ("expire", stale_text(claim, context)),
        _ => proposal_text(kind, claim),
    };
    json!({"verb": verb, "text": text})
}

/// The verb and sentence for every kind but `stale` (`{s}` is the statement, `{source}` the
/// claim's source).
fn proposal_text(kind: &str, claim: &Claim) -> (&'static str, String) {
    let (verb, template) = PROPOSALS.iter().find(|(k, _, _)| *k == kind).map_or(
        ("none", "ask: rewrite the line in the pinned claim grammar"),
        |(_, v, t)| (*v, *t),
    );
    let text = template
        .replace("{s}", &claim.statement)
        .replace("{source}", &claim.source);
    (verb, text)
}

const PROPOSALS: [(&str, &str, &str); 5] = [
    (
        "reversal",
        "contradict",
        "the source reverses \"{s}\"; replace it with the direction the source states",
    ),
    (
        "unsupported",
        "weaken",
        "narrow \"{s}\" to what {source} supports (−0.15); retract if the next check still finds no support",
    ),
    (
        "conflation",
        "split",
        "split \"{s}\" into claims with distinct subjects/objects",
    ),
    (
        "omission",
        "add",
        "add the claim the sources carry beside \"{s}\"",
    ),
    (
        "vocabulary_gap",
        "coin",
        "coin a term fine enough for \"{s}\"",
    ),
];

fn stale_text(claim: &Claim, context: &[Claim]) -> String {
    let later = context.iter().find(|l| {
        l.superseded_by.is_none()
            && l.number != claim.number
            && l.subject.eq_ignore_ascii_case(&claim.subject)
            && l.predicate == claim.predicate
            && l.date > claim.date
    });
    match later {
        Some(l) => format!(
            "\"{}\" is replaced by {} \"{}\"",
            claim.statement, l.number, l.statement
        ),
        None => format!("\"{}\" is no longer current", claim.statement),
    }
}

/// The short reply token a Signal line carries (`#f_ab12cd34`), from a finding id `f_…`.
pub fn reply_token(finding_id: &str) -> String {
    let hex = finding_id.strip_prefix("f_").unwrap_or(finding_id);
    format!("#f_{}", &hex[..hex.len().min(8)])
}

/// Replies to a line with a source (an escalated ask, or any finding shown by `--signal`):
/// `keep` (the claim stands), `accept` (apply the proposal), `fix: <text>` (SPEC §5.1, revised
/// 2026-09-27: action words only; `ok` and `no` are refused).
pub const REPLIES: &str = "keep / accept / fix: <text>";
/// Replies to an ungrounded ask (no source found): `source: <file>`, `drop` (retract), `fix:`.
pub const UNGROUNDED_REPLIES: &str = "source: <file> / drop / fix: <text>";

/// What one SPEC §6 Signal line says about one finding.
pub struct Signal<'a> {
    pub number: &'a str,
    /// The rollup's as-of date (`2026-09-25`); the line shows `09-25`.
    pub rollup_date: &'a str,
    pub statement: &'a str,
    /// The source excerpt; `None` when no source was found (the §5.1 ask form).
    pub excerpt: Option<&'a str>,
    pub kind: &'a str,
    /// The judge's verdict for a reversal (`reversed`, …); otherwise the kind speaks.
    pub verdict: Option<&'a str>,
    /// Fergus's proposed claim on an escalated ask (`palace improve escalate`).
    pub proposal: Option<&'a str>,
    pub token: &'a str,
}

/// `🎩 C7 in ROLLUP 09-25: claim "…" vs source "…" — looks STALE. Reply: keep / accept / fix: <text> #f_…`;
/// escalated, `… — judge says REVERSED; Fergus proposes "…". Reply: keep / accept / fix: <text> #f_…`;
/// with no source, `… — no source found. Reply: source: <file> / drop / fix: <text> #f_…`.
pub fn signal_line(s: &Signal) -> String {
    let date = s.rollup_date.get(5..).unwrap_or(s.rollup_date);
    let head = format!(
        "🎩 {} in ROLLUP {date}: claim \"{}\"",
        s.number,
        clip(s.statement, 80)
    );
    let Some(excerpt) = s.excerpt else {
        return format!(
            "{head} — no source found. Reply: {UNGROUNDED_REPLIES} {}",
            s.token
        );
    };
    let verdict = match s.proposal {
        Some(p) => format!(
            "judge says {}; {} proposes \"{}\"",
            looks(s.kind, s.verdict),
            crate::names::display(crate::names::assistant()),
            clip(p, 120)
        ),
        None => format!("looks {}", looks(s.kind, s.verdict)),
    };
    format!(
        "{head} vs source \"{}\" — {verdict}. Reply: {REPLIES} {}",
        clip(excerpt, 100),
        s.token
    )
}

fn looks(kind: &str, verdict: Option<&str>) -> String {
    let word = match (kind, verdict) {
        ("reversal", Some(v)) => v,
        ("unsupported", _) => "unsupported",
        ("malformed_claim", _) => "malformed",
        (other, _) => other,
    };
    word.to_uppercase()
}

/// `text` on one line, cut to `max` characters with `…`.
fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max - 1).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::parse;
    fn claim(line: &str) -> Claim {
        parse(&format!("## Claims\n{line}\n")).claims.remove(0)
    }
    #[test]
    fn passage_is_at_most_600_chars_of_the_source_with_its_lines() {
        let source = format!(
            "# t\n{}\nCallum chose Rust today.\n{}",
            "a\n".repeat(400),
            "b".repeat(900)
        );
        let c = claim("- C1. Callum | chose | Rust — 2026-09-21 — s.md — quote: \"chose Rust\"");
        let p = passage(&source, &c);
        assert!(p.text.chars().count() <= PASSAGE_CHARS);
        assert!(source.contains(&p.text) && p.text.contains("chose Rust"));
        let first = source[..source.find(&p.text).unwrap()]
            .matches('\n')
            .count()
            + 1;
        let last = first + p.text.matches('\n').count();
        assert_eq!(p.locator, format!("L{first}-L{last}"));
    }
    #[test]
    fn short_sources_are_whole_and_unicode_safe() {
        let c = claim("- C1. Zoë | likes | Zürich — 2026-09-21 — s.md");
        let p = passage("é\nzoë likes Zürich", &c);
        assert_eq!(
            p,
            Passage {
                text: "é\nzoë likes Zürich".into(),
                locator: "L1-L2".into()
            }
        );
    }
    #[test]
    fn every_kind_has_its_proposal_verb() {
        let c = claim("- C9. X | concerned | cycles — 2026-09-21 — s.md");
        let later = claim("- C10. X | concerned | success — 2026-09-22 — s.md");
        let context = [c.clone(), later];
        for (kind, verb) in [
            ("reversal", "contradict"),
            ("unsupported", "weaken"),
            ("stale", "expire"),
            ("conflation", "split"),
            ("omission", "add"),
            ("vocabulary_gap", "coin"),
            ("malformed_claim", "none"),
        ] {
            assert_eq!(proposal(kind, &c, &context)["verb"], verb, "{kind}");
        }
        assert!(
            proposal("stale", &c, &context)["text"]
                .as_str()
                .unwrap()
                .contains("C10")
        );
        assert_eq!(reply_token("f_ab12cd34ef56"), "#f_ab12cd34");
    }
    #[test]
    fn signal_lines_follow_spec_6_and_the_ask_form() {
        let mut s = Signal {
            number: "C5",
            rollup_date: "2026-09-22",
            statement: "Fergus posted the Lighthouse 8 reply on Callum's behalf",
            excerpt: Some("Callum chose #1 and posted\nit himself"),
            kind: "reversal",
            verdict: Some("reversed"),
            proposal: None,
            token: "#f_ab12cd34",
        };
        assert_eq!(
            signal_line(&s),
            "🎩 C5 in ROLLUP 09-22: claim \"Fergus posted the Lighthouse 8 reply on Callum's behalf\" vs source \"Callum chose #1 and posted it himself\" — looks REVERSED. Reply: keep / accept / fix: <text> #f_ab12cd34"
        );
        s.proposal = Some("Callum | posted | the Lighthouse 8 reply himself");
        assert!(signal_line(&s).ends_with(
            "— judge says REVERSED; Fergus proposes \"Callum | posted | the Lighthouse 8 reply himself\". Reply: keep / accept / fix: <text> #f_ab12cd34"
        ));
        s.excerpt = None;
        assert!(signal_line(&s).ends_with(
            "— no source found. Reply: source: <file> / drop / fix: <text> #f_ab12cd34"
        ));
        assert_eq!(clip(&"x".repeat(90), 80).chars().count(), 80);
    }
}
