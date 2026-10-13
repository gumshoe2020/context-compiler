//! M5b-10 item 2: a rollup whose only problems are problems of claim lines is repaired, not
//! rewritten. Pure: which attempt comes next, the repair prompt, and the splice of the corrected
//! lines into the first body (every other line unchanged, byte for byte).
/// SETTLE-1h §1: a cited live specific's missing value, quoted after its handle before
/// re-validation.
pub use crate::rollup_cited::{Mend, mend as mend_cited_values};
/// SETTLE-1f §1: a near-miss of the referenced entries' layout, mended before validation.
pub use crate::rollup_layout::mend as mend_layout;
use crate::{
    claim::{self, Claim},
    quote_relevance::distinctive_anchors,
    rollup_cluster::Cluster,
};
use serde_json::{Value, json};

pub const REPAIR_MARKER: &str = "=== TASK: REPAIR CLAIM LINES ===";
/// Each claim line asked for starts with this in the repair prompt (the replay model echoes them).
pub const LINE_PREFIX: &str = "Line as written: ";
/// The first write and at most two more attempts.
pub const MAX_ATTEMPTS: usize = 3;
pub const MAX_REPAIRS: usize = 2;
const REFUSED: &str = "repair: ";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Write,
    Rewrite,
    Repair,
}
impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Write => "write",
            Mode::Rewrite => "rewrite",
            Mode::Repair => "repair",
        }
    }
}

/// One problem of one claim line: the line as written, the problem, the words of which its quote
/// must carry one (the rule's own anchors for that claim).
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimProblem {
    pub number: String,
    pub problem: String,
    pub line: String,
    pub source: String,
    pub anchors: Vec<String>,
}
impl ClaimProblem {
    pub fn to_json(&self) -> Value {
        json!({"claim": self.number, "problem": self.problem, "line": self.line,
            "source": self.source, "anchors": self.anchors})
    }
}

/// The claim number of a claim-line problem (`Cn: quote does not support claim`, `Cn: quote not
/// verbatim in …`), else `None`.
pub fn claim_of(problem: &str) -> Option<&str> {
    let (number, rest) = problem.split_once(": ")?;
    let line_problem =
        rest == "quote does not support claim" || rest.starts_with("quote not verbatim in ");
    (claim::is_claim_number(number) && line_problem).then_some(number)
}

/// The claim-line problems of `body`, when every problem is one (or a refused repair's); else
/// `None`.
pub fn claim_problems(body: &str, problems: &[String]) -> Option<Vec<ClaimProblem>> {
    let refused = |p: &String| p.starts_with(REFUSED);
    if problems.is_empty() || !problems.iter().all(|p| claim_of(p).is_some() || refused(p)) {
        return None;
    }
    let found = claim_problem_rows(body, problems);
    (!found.is_empty()).then_some(found)
}

/// Every claim-line problem among `problems`, whatever else there is (the kept report, item 3).
pub fn claim_problem_rows(body: &str, problems: &[String]) -> Vec<ClaimProblem> {
    let claims = claim::parse(body).claims;
    problems
        .iter()
        .filter_map(|p| {
            let claim = claims
                .iter()
                .find(|c| Some(c.number.as_str()) == claim_of(p))?;
            Some(problem_of(body, claim, p))
        })
        .collect()
}

fn problem_of(body: &str, claim: &Claim, problem: &str) -> ClaimProblem {
    ClaimProblem {
        number: claim.number.clone(),
        problem: problem.to_owned(),
        line: claim_line(body, &claim.number)
            .unwrap_or_default()
            .to_owned(),
        source: claim.source.clone(),
        anchors: distinctive_anchors(claim)
            .iter()
            .map(|a| a.join(" "))
            .collect(),
    }
}

/// Which attempt follows a failed one: a repair while every problem is a claim line's and fewer
/// than two repairs ran, else one rewrite of the whole; `None` after the last attempt.
pub fn next_mode(
    repairable: bool,
    attempt: usize,
    repairs: usize,
    rewrites: usize,
) -> Option<Mode> {
    if attempt >= MAX_ATTEMPTS {
        return None;
    }
    match (repairable && repairs < MAX_REPAIRS, rewrites) {
        (true, _) => Some(Mode::Repair),
        (false, 0) => Some(Mode::Rewrite),
        _ => None,
    }
}

/// The line of the `## Claims` section that holds claim `number`, trimmed.
pub fn claim_line<'b>(body: &'b str, number: &str) -> Option<&'b str> {
    claims_section_lines(body)
        .map(|(_, line)| line.trim())
        .find(|line| number_of(line) == Some(number))
}

/// `(byte offset, line with its ending)` of every line inside a `## Claims` section.
fn claims_section_lines(body: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut active = false;
    let mut offset = 0;
    body.split_inclusive('\n').filter_map(move |line| {
        let at = offset;
        offset += line.len();
        if line.starts_with("## ") {
            active = claim::is_claims_heading(line);
            return None;
        }
        active.then_some((at, line))
    })
}

/// `C7` of `- C7. …`.
fn number_of(line: &str) -> Option<&str> {
    let (number, _) = line.trim().strip_prefix("- ")?.split_once(". ")?;
    claim::is_claim_number(number).then_some(number)
}

/// The repair prompt: the policy, then for each claim the line as written, the problem, the words
/// its quote must carry one of, and its source; then each source entry once; `refused` names why
/// the previous repair was not accepted.
pub fn prompt(
    policy: &str,
    problems: &[ClaimProblem],
    cluster: &Cluster,
    refused: &[String],
) -> String {
    let mut text = format!(
        "{policy}\n\n{REPAIR_MARKER}\nYour rollup failed validation only on the claim lines below; every other line of it stands. Return ONLY the corrected lines, one per line, each `- Cn. …` in the pinned claim grammar, nothing else. Change the quote and nothing else: keep each line's number, subject, predicate, object, date, source and marker (if any) as written, and give a quote copied verbatim from that source entry that carries at least one of the words listed (the quote rule in Step 3b). A line that changes anything but its quote is refused. If no quote of that source carries one of those words, return the line as written: the rollup is then not written, and a person reads the claim.\n"
    );
    for p in problems {
        text.push_str(&format!(
            "\n--- {} ---\n{LINE_PREFIX}{}\nProblem: {}\nThe quote must carry one of: {}\nSource entry: {}\n",
            p.number,
            p.line,
            p.problem,
            p.anchors.join(", "),
            p.source
        ));
    }
    if !refused.is_empty() {
        text.push_str(&format!(
            "\nYour previous repair was refused: {}\n",
            refused.join("; ")
        ));
    }
    push_sources(&mut text, problems, cluster);
    text
}

fn push_sources(text: &mut String, problems: &[ClaimProblem], cluster: &Cluster) {
    let mut named: Vec<&str> = Vec::new();
    text.push_str("\n=== SOURCE ENTRIES ===\n");
    for name in problems.iter().map(|p| p.source.as_str()) {
        if named.contains(&name) {
            continue;
        }
        named.push(name);
        if let Some(member) = cluster.member(name) {
            text.push_str(&format!("\n--- {name} ---\n{}\n", member.entry.text));
        }
    }
}

/// The claim lines a repair prompt asks for, as written (what the replay model answers with).
pub fn asked_lines(prompt: &str) -> Vec<&str> {
    let task = prompt.find(REPAIR_MARKER).map_or("", |at| &prompt[at..]);
    task.lines()
        .filter_map(|l| l.strip_prefix(LINE_PREFIX))
        .collect()
}

/// `body` with the corrected lines of `reply` in place of the claims `asked`, every other byte
/// unchanged; or the named problems of the reply (a line not asked for, one that drops its quote,
/// changes anything but its quote, does not parse, or an asked claim not returned).
pub fn splice(body: &str, reply: &str, asked: &[String]) -> Result<(String, usize), Vec<String>> {
    let original = claim::parse(body).claims;
    let mut problems = Vec::new();
    let mut lines: Vec<(String, String)> = Vec::new();
    for line in reply.lines().map(str::trim).filter(|l| l.starts_with("- ")) {
        match corrected(line, asked, &original) {
            Ok(number) if lines.iter().any(|(n, _)| *n == number) => {
                problems.push(format!("{REFUSED}{number} returned twice"));
            }
            Ok(number) => lines.push((number, line.to_owned())),
            Err(problem) => problems.push(format!("{REFUSED}{problem}")),
        }
    }
    for number in asked.iter().filter(|n| !lines.iter().any(|(m, _)| m == *n)) {
        problems.push(format!("{REFUSED}{number} was not returned"));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok((replace_lines(body, &lines), lines.len()))
}

/// The number of a corrected line that keeps a quote and every field of its claim but the quote.
fn corrected(line: &str, asked: &[String], original: &[Claim]) -> Result<String, String> {
    let parsed = claim::parse(&format!("## Claims\n{line}\n"));
    let new = match (parsed.claims.first(), parsed.malformed.first()) {
        (Some(c), None) => c,
        (_, Some(m)) => return Err(m.reason.clone()),
        _ => return Err(format!("not a claim line: {line}")),
    };
    let number = new.number.clone();
    let old = original
        .iter()
        .find(|c| c.number == number)
        .filter(|_| asked.contains(&number))
        .ok_or(format!("{number} was not asked for"))?;
    if new.quote.is_none() {
        return Err(format!("{number} dropped its quote"));
    }
    match changed_field(old, new) {
        None => Ok(number),
        Some("source") => Err(format!(
            "{number} changed its source from {} to {}",
            old.source, new.source
        )),
        Some(field) => Err(format!("{number} changed its {field}")),
    }
}

/// The first field of the corrected claim `new` that differs from the claim asked for `old`
/// (subject, predicate, object, date, source, then the marker; trimmed, any case): a repair may
/// change the quote and nothing else (M5b-11 item 2).
pub fn changed_field(old: &Claim, new: &Claim) -> Option<&'static str> {
    let same = |a: &str, b: &str| a.trim().to_lowercase() == b.trim().to_lowercase();
    let fields = [
        ("subject", &old.subject, &new.subject),
        ("predicate", &old.predicate, &new.predicate),
        ("object", &old.object, &new.object),
        ("date", &old.date, &new.date),
        ("source", &old.source, &new.source),
    ];
    if let Some((field, ..)) = fields.iter().find(|(_, a, b)| !same(a, b)) {
        return Some(field);
    }
    let marker = |c: &Claim| {
        (
            c.superseded_by.as_deref().map(str::to_lowercase),
            c.corrected,
        )
    };
    (marker(old) != marker(new)).then_some("marker")
}

/// Each claim line of `lines` (by number) put in place of the first line of the Claims section with
/// that number, keeping the old line's leading whitespace and line ending.
fn replace_lines(body: &str, lines: &[(String, String)]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut copied = 0;
    let mut done: Vec<&str> = Vec::new();
    for (at, old) in claims_section_lines(body) {
        let Some((number, new)) = lines
            .iter()
            .find(|(n, _)| number_of(old) == Some(n.as_str()) && !done.contains(&n.as_str()))
        else {
            continue;
        };
        done.push(number);
        let indent = &old[..old.len() - old.trim_start().len()];
        let ending = &old[old.trim_end().len()..];
        out.push_str(&body[copied..at]);
        out.push_str(&format!("{indent}{new}{ending}"));
        copied = at + old.len();
    }
    out.push_str(&body[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const BODY: &str = "# ROLLUP: t — state as of 2026-09-23\n\nIntro  with two spaces \n## Claims\n- C1. Alice | likes | Rust — 2026-09-20 — a.md — quote: \"weather\"\n- C10. Alice | uses | Cargo daily — 2026-09-20 — a.md — quote: \"uses Cargo\"\n- C2. (superseded by C1) Alice | liked | Go — 2026-09-19 — b.md — quote: \"Go\"\r\n## Current consolidated truth\n- C1. prose that looks like a claim\n";
    fn asked(numbers: &[&str]) -> Vec<String> {
        numbers.iter().map(|n| (*n).to_owned()).collect()
    }
    #[test]
    fn only_the_asked_line_changes() {
        let fixed = "- C1. Alice | likes | Rust — 2026-09-20 — a.md — quote: \"likes Rust\"";
        let (body, n) = splice(BODY, &format!("Here:\n{fixed}\n"), &asked(&["C1"])).unwrap();
        assert_eq!(n, 1);
        let old = "- C1. Alice | likes | Rust — 2026-09-20 — a.md — quote: \"weather\"";
        assert_eq!(body, BODY.replacen(old, fixed, 1));
        let fixed2 =
            "- C2. (superseded by C1) Alice | liked | Go — 2026-09-19 — b.md — quote: \"liked Go\"";
        let (body, _) = splice(BODY, fixed2, &asked(&["C2"])).unwrap();
        assert!(body.contains(&format!("{fixed2}\r\n## Current")), "{body}");
    }
    #[test]
    fn a_repair_that_breaks_the_line_is_refused() {
        let cases = [
            (
                "- C1. Alice | likes | Rust — 2026-09-20 — a.md",
                "repair: C1 dropped its quote",
            ),
            (
                "- C1. Alice | likes | Rust — 2026-09-20 — b.md — quote: \"x\"",
                "repair: C1 changed its source from a.md to b.md",
            ),
            (
                "- C10. Alice | uses | Cargo — 2026-09-20 — a.md — quote: \"x\"",
                "repair: C10 was not asked for",
            ),
            (
                "- C2. Alice | liked | Go — 2026-09-19 — b.md — quote: \"Go\"",
                "repair: C2 changed its marker",
            ),
        ];
        for (line, expected) in cases {
            let number = if line.contains("C2.") { "C2" } else { "C1" };
            let err = splice(BODY, line, &asked(&[number])).unwrap_err();
            assert!(err.contains(&expected.to_owned()), "{line}: {err:?}");
        }
        assert_eq!(
            splice(BODY, "nothing", &asked(&["C1"])).unwrap_err(),
            ["repair: C1 was not returned"]
        );
    }
    #[test]
    fn a_repair_changes_the_quote_and_nothing_else() {
        let asked_c1 = asked(&["C1"]);
        let line = |fields: &str| format!("- C1. {fields} — quote: \"likes Rust\"");
        let cases = [
            ("Bob | likes | Rust — 2026-09-20 — a.md", "subject"),
            ("Alice | hates | Rust — 2026-09-20 — a.md", "predicate"),
            ("Alice | likes | Rust and Go — 2026-09-20 — a.md", "object"),
            ("Alice | likes | Rust — 2026-09-21 — a.md", "date"),
        ];
        for (fields, field) in cases {
            let err = splice(BODY, &line(fields), &asked_c1).unwrap_err();
            assert_eq!(
                err[0],
                format!("repair: C1 changed its {field}"),
                "{fields}"
            );
        }
        let cased = line(" alice | LIKES | rust — 2026-09-20 — A.md");
        assert_eq!(splice(BODY, &cased, &asked_c1).map(|(_, n)| n), Ok(1));
        let text = prompt("policy", &[], &Cluster::default(), &[]);
        assert!(text.contains("Change the quote and nothing else"));
        assert!(!text.contains("wording of the object"));
    }
    #[test]
    fn modes_follow_the_problems() {
        assert_eq!(next_mode(true, 1, 0, 0), Some(Mode::Repair));
        assert_eq!(next_mode(true, 2, 1, 0), Some(Mode::Repair));
        assert_eq!(next_mode(true, 3, 2, 0), None);
        assert_eq!(next_mode(false, 1, 0, 0), Some(Mode::Rewrite));
        assert_eq!(next_mode(false, 2, 0, 1), None);
        assert_eq!(next_mode(true, 2, 0, 1), Some(Mode::Repair));
        let quote = "C1: quote does not support claim".to_owned();
        let verbatim = "C10: quote not verbatim in a.md".to_owned();
        let found = claim_problems(BODY, &[quote.clone(), verbatim]).unwrap();
        assert_eq!(found[0].anchors, ["rust"]);
        assert_eq!(found[1].line, claim_line(BODY, "C10").unwrap());
        assert!(claim_problems(BODY, &[quote, "missing section `## Open items`".into()]).is_none());
    }
    #[test]
    fn the_prompt_names_line_problem_words_and_source() {
        let found = claim_problems(BODY, &["C1: quote does not support claim".into()]).unwrap();
        let text = prompt("policy", &found, &Cluster::default(), &[]);
        assert_eq!(asked_lines(&text), [claim_line(BODY, "C1").unwrap()]);
        assert!(text.contains("The quote must carry one of: rust\nSource entry: a.md\n"));
    }
}
