//! `palace findings [--open] [--rollup <file>] [--claim <id|"<date> C7">] [--json|--signal]
//! [--queue <assistant>]` (M4d item 2): the claim store's findings as a list, JSON, or one SPEC §6
//! Signal line each, every line carrying its reply token `#f_<8 hex>`. `--queue <assistant>` (M4d-2c
//! item 1): the open reversal, conflation and omission findings Fergus corrects, each with its
//! passage and proposal. Read-only; prints, never sends.
use clap::Args;
use palace_core::finding::{Signal, reply_token, signal_line};
use palace_store::{Error, claims::ClaimStore, lookup::FindingFilter};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Args)]
pub struct Options {
    /// The claim store (compiler.sqlite).
    #[arg(long)]
    pub store: PathBuf,
    /// Only findings without a decision.
    #[arg(long)]
    pub open: bool,
    /// Only findings raised on this rollup (file name, `.md` optional).
    #[arg(long)]
    pub rollup: Option<String>,
    /// Only findings on this claim: `c_…`, `f_…`, `#f_…` or "<rollup date> C7".
    #[arg(long)]
    pub claim: Option<String>,
    #[arg(long, conflicts_with = "signal")]
    pub json: bool,
    /// SPEC §6 Signal lines; implies `--open` (a decided finding is never asked again).
    #[arg(long)]
    pub signal: bool,
    /// The assistant's name (`palace --assistant`): the open findings waiting for the assistant
    /// (reversal, conflation, omission), with passage and proposal; implies `--open`.
    #[arg(long, conflicts_with = "signal")]
    pub queue: Option<String>,
}

/// The kinds a person corrects (SPEC §5.1 revised): the loop only lowers or records them.
pub const QUEUE_KINDS: [&str; 3] = ["reversal", "conflation", "omission"];

/// Exit 0 printed, 1 refused (unknown claim), 2 broken.
pub fn run(o: Options) -> i32 {
    match print(&o) {
        Ok(()) => 0,
        Err(e) => crate::improve::fail("palace findings", e),
    }
}

fn print(o: &Options) -> Result<(), Error> {
    let assistant = palace_core::names::assistant();
    if o.queue.as_deref().is_some_and(|queue| queue != assistant) {
        return Err(
            format!("--queue: the only queue is {assistant}'s (--queue {assistant})").into(),
        );
    }
    let store = ClaimStore::open(&crate::paths::input(&o.store)?)?;
    let mut rows = described(&store, &filter(&store, o)?)?;
    if o.queue.is_some() {
        rows.retain(|r| QUEUE_KINDS.contains(&r["kind"].as_str().unwrap_or_default()));
    }
    if o.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    for row in &rows {
        let line = match (o.signal, o.queue.is_some()) {
            (true, _) => signal(row, None),
            (_, true) => queued(row),
            _ => summary(row),
        };
        println!("{line}");
    }
    Ok(())
}

fn filter(store: &ClaimStore, o: &Options) -> Result<FindingFilter, Error> {
    Ok(FindingFilter {
        open: o.open || o.signal || o.queue.is_some(),
        claim: o
            .claim
            .as_deref()
            .map(|t| store.resolve(t))
            .transpose()?
            .map(|r| r.claim_id),
        rollup: o.rollup.as_deref().map(|r| {
            let name = r.rsplit('/').next().unwrap_or(r);
            name.strip_suffix(".md").unwrap_or(name).to_owned()
        }),
    })
}

/// Each finding with its reply token and, for a claim finding, the claim's number and statement.
fn described(store: &ClaimStore, filter: &FindingFilter) -> Result<Vec<Value>, Error> {
    let mut rows = store.findings(filter)?;
    for row in &mut rows {
        row["token"] = json!(reply_token(row["id"].as_str().unwrap_or_default()));
        let claim = row["claim"]
            .as_str()
            .map(|id| store.claim(id))
            .transpose()?
            .flatten();
        if let Some(claim) = claim {
            describe(row, &claim);
        }
    }
    Ok(rows)
}

/// The claim's number, rollup date, statement, quote and confidence on a finding row.
fn describe(row: &mut Value, claim: &Value) {
    let listed = claim["numbers"]
        .as_array()
        .and_then(|n| n.iter().find(|n| n["rollup"] == row["rollup"]).cloned());
    row["number"] = listed
        .as_ref()
        .map_or(claim["number"].clone(), |n| n["number"].clone());
    row["rollup_date"] = listed.map_or(json!(""), |n| n["date"].clone());
    row["statement"] = json!(format!(
        "{} {} {}",
        claim["subject"].as_str().unwrap_or(""),
        claim["predicate"].as_str().unwrap_or("").replace('_', " "),
        claim["object"].as_str().unwrap_or("")
    ));
    row["quote"] = claim["quote"].clone();
    row["confidence"] = claim["confidence"].clone();
}

/// The SPEC §6 line (or the ask form) for one finding on `claim` (M4d-2b: the auto loop's asks);
/// with Fergus's `proposal`, the escalated ask (M4d-2c item 1).
pub fn signal_for(finding: &Value, claim: &Value, proposal: Option<&str>) -> String {
    let mut row = finding.clone();
    row["token"] = json!(reply_token(finding["id"].as_str().unwrap_or_default()));
    describe(&mut row, claim);
    signal(&row, proposal)
}

fn signal(row: &Value, proposal: Option<&str>) -> String {
    let passage = row["evidence"]["passage"].as_str().unwrap_or_default();
    let excerpt = excerpt(passage, row["quote"].as_str());
    let malformed = row["claim"].is_null();
    let text = |key: &str| row[key].as_str().unwrap_or_default().to_owned();
    let (number, statement) = if malformed {
        (
            text_of(&row["evidence"]["locator"]),
            text_of(&row["evidence"]["raw"]),
        )
    } else {
        (text("number"), text("statement"))
    };
    let kind = text("kind");
    signal_line(&Signal {
        number: &number,
        rollup_date: row["rollup_date"]
            .as_str()
            .unwrap_or(row["rollup"].as_str().unwrap_or("")),
        statement: &statement,
        excerpt: (!passage.is_empty() || malformed).then_some(excerpt.as_str()),
        kind: &kind,
        verdict: (kind == "reversal").then_some("reversed"),
        proposal,
        token: row["token"].as_str().unwrap_or_default(),
    })
}

fn text_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// The claim's quote when the passage carries it, else the middle 100 characters of the passage.
fn excerpt(passage: &str, quote: Option<&str>) -> String {
    if let Some(quote) = quote.filter(|q| passage.contains(*q)) {
        return quote.to_owned();
    }
    let chars: Vec<char> = passage.chars().collect();
    let start = chars.len().saturating_sub(100) / 2;
    chars[start..(start + 100).min(chars.len())]
        .iter()
        .collect()
}

/// `QUEUE <token> <kind> <date> Cn claim "…" passage "…" proposal=<verb>: <text>`; the passage
/// clipped as on the Signal line (M4d-2d item 4, review L-5).
fn queued(row: &Value) -> String {
    let passage = row["evidence"]["passage"].as_str().unwrap_or_default();
    let passage = excerpt(passage, row["quote"].as_str());
    let flat = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "QUEUE {} {} {} {} claim \"{}\" passage \"{}\" proposal={}: {}",
        row["token"].as_str().unwrap_or_default(),
        row["kind"].as_str().unwrap_or_default(),
        row["rollup_date"].as_str().unwrap_or_default(),
        row["number"].as_str().unwrap_or("-"),
        row["statement"].as_str().unwrap_or_default(),
        flat(&passage),
        row["proposal"]["verb"].as_str().unwrap_or("-"),
        flat(row["proposal"]["text"].as_str().unwrap_or_default()),
    )
}

fn summary(row: &Value) -> String {
    format!(
        "{} {} {} {} {} proposal={} seen={} {}",
        row["token"].as_str().unwrap_or_default(),
        row["kind"].as_str().unwrap_or_default(),
        row["rollup"].as_str().unwrap_or_default(),
        row["number"].as_str().unwrap_or("-"),
        if row["decision"].is_null() {
            "open"
        } else {
            "decided"
        },
        row["proposal"]["verb"].as_str().unwrap_or("-"),
        row["seen_count"],
        row["checker"].as_str().unwrap_or_default(),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpt_prefers_the_quote_else_the_passage_middle() {
        assert_eq!(excerpt("a b quoted c", Some("quoted")), "quoted");
        let passage = format!("{}MIDDLE{}", "x".repeat(200), "y".repeat(200));
        let middle = excerpt(&passage, Some("absent"));
        assert_eq!(middle.chars().count(), 100);
        assert!(middle.contains("MIDDLE"));
    }
    #[test]
    fn the_queue_line_clips_its_passage() {
        let passage = format!("{} civilization {}", "a ".repeat(200), "b ".repeat(200));
        let row = json!({"token":"#f_0123abcd","kind":"reversal","rollup_date":"2026-09-22","number":"C5","statement":"s","quote":null,"evidence":{"passage":passage},"proposal":{"verb":"contradict","text":""}});
        let line = queued(&row);
        let quoted = line
            .split("passage \"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(quoted.chars().count() <= 100, "{line}");
        assert!(quoted.contains("civilization"), "{line}");
    }
}
