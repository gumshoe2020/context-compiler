//! SETTLE-1b (spec §3–§4): the distill prompt, and the model's structured reply checked against
//! the palace copy it was given. Pure: no file, model or store here.
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// One line of the palace copy the model saw: its 1-based line number and turn.
#[derive(Clone, Debug)]
pub struct CopyLine {
    pub line: usize,
    pub role: String,
    pub text: String,
}

/// The prompt: the policy (prompts/distill.md), then the new text, one copy line per turn.
pub fn prompt(policy: &str, session: &str, lines: &[CopyLine]) -> String {
    let mut text = format!(
        "{}\n\n## Conversation (session {session})\n\n",
        policy.trim()
    );
    for l in lines {
        text.push_str(&format!("L{} [{}] {}\n", l.line, l.role, l.text));
    }
    text
}

#[derive(Clone, Debug, PartialEq)]
pub struct DistilledEntry {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DistilledClaim {
    pub kind: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub date: String,
    pub quote: Option<String>,
    pub line: usize,
    /// For a demoted `specific` (spec §4.4): why it was written as a `concept`.
    pub demoted: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DistilledRelation {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub line: usize,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub entries: Vec<DistilledEntry>,
    pub claims: Vec<DistilledClaim>,
    pub relations: Vec<DistilledRelation>,
}

/// The reply checked: what passed, the shape problems (any one fails the reply: retried once),
/// and the claims marked `specific` whose quote is missing or not in the copy (spec §4.4).
#[derive(Debug, Default)]
pub struct Checked {
    pub reply: Reply,
    pub problems: Vec<String>,
    pub unquoted_specifics: Vec<usize>,
}
impl Checked {
    pub fn passed(&self) -> bool {
        self.problems.is_empty() && self.unquoted_specifics.is_empty()
    }
}

/// The first JSON object in the model's answer (a fenced or prefaced answer is accepted).
pub fn json_of(answer: &str) -> Option<Value> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    (start < end)
        .then(|| serde_json::from_str(&answer[start..=end]).ok())
        .flatten()
}

/// Parse and check one answer against the copy lines the model was given. A claim or relation
/// line outside them is a problem; a quote found on another of those lines moves the claim's
/// line there (the locator must name a line that holds the quote, spec test 14a).
pub fn check(answer: &str, lines: &[CopyLine]) -> Checked {
    let mut checked = Checked::default();
    let Some(value) = json_of(answer) else {
        checked
            .problems
            .push("the answer holds no JSON object".into());
        return checked;
    };
    let by_line: BTreeMap<usize, &str> = lines.iter().map(|l| (l.line, l.text.as_str())).collect();
    for (i, e) in items(&value, "entries").iter().enumerate() {
        match entry(e) {
            Ok(entry) => checked.reply.entries.push(entry),
            Err(why) => checked.problems.push(format!("entry {i}: {why}")),
        }
    }
    for (i, c) in items(&value, "claims").iter().enumerate() {
        match claim(c, &by_line) {
            Ok(claim) => {
                if claim.kind == "specific" && claim.quote.is_none() {
                    checked.unquoted_specifics.push(checked.reply.claims.len());
                }
                checked.reply.claims.push(claim);
            }
            Err(why) => checked.problems.push(format!("claim {i}: {why}")),
        }
    }
    for (i, r) in items(&value, "relations").iter().enumerate() {
        match relation(r, &by_line) {
            Ok(relation) => checked.reply.relations.push(relation),
            Err(why) => checked.problems.push(format!("relation {i}: {why}")),
        }
    }
    let stated = !checked.reply.claims.is_empty() || !checked.reply.relations.is_empty();
    if stated && checked.reply.entries.is_empty() {
        checked
            .problems
            .push("claims or relations without an entry to state them".into());
    }
    checked
}

fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}
fn text(value: &Value, key: &str) -> Result<String, String> {
    match value[key].as_str().map(str::trim) {
        Some(text) if !text.is_empty() => Ok(text.to_owned()),
        _ => Err(format!("{key} missing or empty")),
    }
}
fn entry(value: &Value) -> Result<DistilledEntry, String> {
    let tags = value["tags"]
        .as_array()
        .ok_or("tags missing (a list)")?
        .iter()
        .filter_map(Value::as_str)
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty() && !t.contains([',', '.', '\n']))
        .collect();
    Ok(DistilledEntry {
        title: text(value, "title")?,
        body: text(value, "body")?,
        tags,
    })
}
fn line_of(value: &Value, by_line: &BTreeMap<usize, &str>) -> Result<usize, String> {
    let line = value["line"]
        .as_u64()
        .and_then(|l| usize::try_from(l).ok())
        .ok_or("line missing (a number)")?;
    match by_line.contains_key(&line) {
        true => Ok(line),
        false => Err(format!("line {line} is not a line of this conversation")),
    }
}
fn claim(value: &Value, by_line: &BTreeMap<usize, &str>) -> Result<DistilledClaim, String> {
    let kind = text(value, "kind")?;
    if kind != "specific" && kind != "concept" {
        return Err(format!("kind {kind:?} is neither specific nor concept"));
    }
    let date = text(value, "date")?;
    if !is_date(&date) {
        return Err(format!("date {date:?} is not YYYY-MM-DD"));
    }
    let mut line = line_of(value, by_line)?;
    let quote = value["quote"]
        .as_str()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .and_then(|q| locate(q, line, by_line).map(|found| (q.to_owned(), found)));
    if let Some((_, found)) = &quote {
        line = *found;
    }
    Ok(DistilledClaim {
        kind,
        subject: text(value, "subject")?,
        predicate: text(value, "predicate")?,
        object: text(value, "object")?,
        date,
        quote: quote.map(|(q, _)| q),
        line,
        demoted: None,
    })
}
/// The line holding `quote` verbatim: the given line first, else the first that does.
fn locate(quote: &str, line: usize, by_line: &BTreeMap<usize, &str>) -> Option<usize> {
    if by_line.get(&line).is_some_and(|t| t.contains(quote)) {
        return Some(line);
    }
    by_line
        .iter()
        .find(|(_, t)| t.contains(quote))
        .map(|(l, _)| *l)
}
fn relation(value: &Value, by_line: &BTreeMap<usize, &str>) -> Result<DistilledRelation, String> {
    let confidence = value["confidence"]
        .as_f64()
        .filter(|c| (0.0..=1.0).contains(c))
        .ok_or("confidence missing or outside 0..1")?;
    Ok(DistilledRelation {
        subject: text(value, "subject")?,
        predicate: text(value, "predicate")?,
        object: text(value, "object")?,
        confidence,
        line: line_of(value, by_line)?,
    })
}
fn is_date(date: &str) -> bool {
    let b = date.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Spec §4.4: after the retry, a `specific` still without a verbatim quote from the copy is
/// written as a `concept`, the reason kept for its finding.
pub fn demote_unquoted(checked: &mut Checked) {
    for index in std::mem::take(&mut checked.unquoted_specifics) {
        let claim = &mut checked.reply.claims[index];
        claim.kind = "concept".into();
        claim.demoted =
            Some("marked specific without a verbatim quote found in the transcript".into());
    }
}

/// The reply as logged (the write log keeps it, so a resumed distill writes the same entries).
pub fn to_json(reply: &Reply) -> Value {
    json!({
        "entries": reply.entries.iter().map(|e| json!({"title":e.title,"body":e.body,"tags":e.tags})).collect::<Vec<_>>(),
        "claims": reply.claims.iter().map(|c| json!({"kind":c.kind,"subject":c.subject,"predicate":c.predicate,"object":c.object,"date":c.date,"quote":c.quote,"line":c.line,"demoted":c.demoted})).collect::<Vec<_>>(),
        "relations": reply.relations.iter().map(|r| json!({"subject":r.subject,"predicate":r.predicate,"object":r.object,"confidence":r.confidence,"line":r.line})).collect::<Vec<_>>(),
    })
}
/// The logged reply read back (already checked when it was logged).
pub fn from_json(value: &Value) -> Reply {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    let line = |v: &Value| v["line"].as_u64().unwrap_or(0) as usize;
    Reply {
        entries: items(value, "entries")
            .iter()
            .map(|e| DistilledEntry {
                title: s(e, "title"),
                body: s(e, "body"),
                tags: items(e, "tags")
                    .iter()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect(),
            })
            .collect(),
        claims: items(value, "claims")
            .iter()
            .map(|c| DistilledClaim {
                kind: s(c, "kind"),
                subject: s(c, "subject"),
                predicate: s(c, "predicate"),
                object: s(c, "object"),
                date: s(c, "date"),
                quote: c["quote"].as_str().map(str::to_owned),
                line: line(c),
                demoted: c["demoted"].as_str().map(str::to_owned),
            })
            .collect(),
        relations: items(value, "relations")
            .iter()
            .map(|r| DistilledRelation {
                subject: s(r, "subject"),
                predicate: s(r, "predicate"),
                object: s(r, "object"),
                confidence: r["confidence"].as_f64().unwrap_or(0.0),
                line: line(r),
            })
            .collect(),
    }
}

/// The diary text of an entry: its body, the transcript lines it came from, its tags.
pub fn entry_text(entry: &DistilledEntry, locator: &str) -> String {
    let mut text = format!("{}\n\nTranscript: {locator}", entry.body.trim());
    if !entry.tags.is_empty() {
        text.push_str(&format!("\n\nTags: {}.", entry.tags.join(", ")));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn copy() -> Vec<CopyLine> {
        [
            (4, "user", "Ravi swaps with Theo on the 24th."),
            (5, "assistant", "Noted: the 24th."),
        ]
        .into_iter()
        .map(|(line, role, text)| CopyLine {
            line,
            role: role.into(),
            text: text.into(),
        })
        .collect()
    }
    #[test]
    fn a_good_reply_passes_and_a_quote_moves_its_line() {
        let answer = r#"Here: {"entries":[{"title":"Breakfast swap","body":"Ravi and Theo swap.","tags":["Breakfast"]}],
            "claims":[{"kind":"specific","subject":"Ravi","predicate":"swaps with","object":"Theo on the 24th","date":"2026-10-06","quote":"Ravi swaps with Theo on the 24th","line":5}],
            "relations":[{"subject":"Ravi","predicate":"swaps_with","object":"Theo","confidence":0.9,"line":4}]}"#;
        let checked = check(answer, &copy());
        assert!(checked.passed(), "{:?}", checked.problems);
        assert_eq!(checked.reply.claims[0].line, 4);
        assert_eq!(checked.reply.entries[0].tags, ["breakfast"]);
        assert_eq!(from_json(&to_json(&checked.reply)), checked.reply);
    }
    #[test]
    fn shape_problems_and_unquoted_specifics() {
        let answer = r#"{"entries":[{"title":"","body":"b","tags":[]}],
            "claims":[{"kind":"specific","subject":"Ravi","predicate":"swaps","object":"x","date":"2026-10-06","quote":"not there","line":4},
                      {"kind":"fact","subject":"a","predicate":"b","object":"c","date":"2026-10-06","line":4},
                      {"kind":"concept","subject":"a","predicate":"b","object":"c","date":"2026-10-06","line":9}]}"#;
        let mut checked = check(answer, &copy());
        assert_eq!(checked.problems.len(), 4, "{:?}", checked.problems);
        assert_eq!(checked.unquoted_specifics, [0]);
        demote_unquoted(&mut checked);
        assert_eq!(checked.reply.claims[0].kind, "concept");
        assert!(checked.reply.claims[0].demoted.is_some());
        assert!(!check("no json", &copy()).passed());
        assert!(check(r#"{"entries":[],"claims":[],"relations":[]}"#, &copy()).passed());
    }
}
