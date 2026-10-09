use crate::{
    rollup::{Entry, word},
    traits::Judge,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Canary {
    #[serde(default)]
    pub id: String,
    pub statement: String,
    #[serde(default)]
    pub entities: Vec<Vec<String>>,
    #[serde(default)]
    pub topic_terms: Vec<String>,
    /// M4d item 7 (a canary from the person's correction): the question asked, the expected verdict,
    /// the successor's source and the contradict event it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// M4d-2c item 2: who made the correction (the person or the assistant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Whether an event becomes a canary (SPEC §5.1 revised, point 5): every `contradict` by the person or
/// the assistant; never a rule's or a judge's.
pub fn makes_canary(verb: &str, by: &str) -> bool {
    verb == "contradict" && crate::names::is_person(by)
}

/// Error → canary (M4d item 7, M4d-2c item 2): a person's `contradict` of `old` by `new` (event
/// `origin`, by `author`) becomes a canary whose statement is the successor's direction (expected
/// `preserved`), in scope where a rollup names the successor's subject and one of its object's
/// words.
pub fn from_correction(
    old: &crate::claim::Claim,
    new: &crate::claim::Claim,
    (origin, author): (&str, &str),
) -> Canary {
    Canary {
        id: format!("correction-{origin}"),
        statement: new.statement.clone(),
        entities: vec![vec![new.subject.clone()]],
        topic_terms: topic_terms(&new.object),
        question: Some(format!(
            "Does the rollup say \"{}\" (corrected from \"{}\")?",
            new.statement, old.statement
        )),
        expected: Some("preserved".into()),
        source: Some(new.source.clone()),
        origin: Some(origin.to_owned()),
        author: Some(author.to_owned()),
    }
}

/// An object's content words (four letters or more, lowercased, first seen first).
fn topic_terms(object: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in object.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.chars().count() >= 4
            && !["that", "this", "with", "from"].contains(&word.as_str())
            && !terms.contains(&word)
        {
            terms.push(word);
        }
    }
    terms
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Question {
    pub rollup: String,
    pub id: String,
    pub statement: String,
    pub passage: String,
}
pub fn word_position(term: &str, text: &str) -> Option<usize> {
    text.match_indices(term)
        .find(|(i, _)| {
            let before = text[..*i].chars().next_back().is_some_and(word);
            let after = text[*i + term.len()..].chars().next().is_some_and(word);
            !before && !after
        })
        .map(|(i, _)| text[..i].chars().count())
}
pub fn in_scope(text: &str, canary: &Canary) -> bool {
    let text = text.to_lowercase();
    canary.entities.iter().all(|group| {
        group
            .iter()
            .any(|alias| word_position(&alias.to_lowercase(), &text).is_some())
    }) && canary
        .topic_terms
        .iter()
        .any(|term| word_position(&term.to_lowercase(), &text).is_some())
}
pub fn extract_passage(text: &str, canary: &Canary) -> String {
    let lower = text.to_lowercase();
    let hit = canary
        .entities
        .iter()
        .flatten()
        .filter_map(|alias| word_position(&alias.to_lowercase(), &lower))
        .min()
        .unwrap_or(text.chars().count());
    let start = hit.saturating_sub(6000);
    text.chars().skip(start).take(hit + 6000 - start).collect()
}
pub fn check<J: Judge + ?Sized>(entry: &Entry, canary: &Canary, judge: &J) -> Value
where
    J::Error: std::fmt::Display,
{
    let mut row =
        json!({"check":"canary","rollup":entry.name,"canary_id":canary.id,"status":"absent"});
    if !in_scope(&entry.text, canary) {
        return row;
    }
    let question = Question {
        rollup: entry.name.clone(),
        id: canary.id.clone(),
        statement: canary.statement.clone(),
        passage: extract_passage(&entry.text, canary),
    };
    match judge.judge(&question) {
        Err(error) => {
            row["status"] = json!("warn");
            row["reason"] = json!(error.to_string());
        }
        Ok(verdict) => {
            row["status"] = json!(match verdict["verdict"].as_str() {
                Some("preserved") => "ok",
                Some("reversed") => "fail",
                _ => "warn",
            });
            row["verdict"] = verdict.get("verdict").cloned().unwrap_or(json!("unclear"));
            row["quote"] = verdict.get("quote").cloned().unwrap_or(json!(""));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_correction_becomes_a_canary_the_rollup_is_asked() {
        let parse = |line: &str| {
            crate::claim::parse(&format!("## Claims\n{line}\n"))
                .claims
                .remove(0)
        };
        let old = parse(
            "- C5. Fergus | posted | the Lighthouse 8 reply on Callum's behalf — 2026-09-22 — a.md",
        );
        let new =
            parse("- C15. Callum | posted | the Lighthouse 8 reply himself — 2026-09-22 — a.md");
        let c = from_correction(&old, &new, ("e_1", "fergus"));
        assert_eq!(c.author.as_deref(), Some("fergus"));
        assert_eq!(
            (c.id.as_str(), c.origin.as_deref()),
            ("correction-e_1", Some("e_1"))
        );
        assert_eq!(c.statement, "Callum posted the Lighthouse 8 reply himself");
        assert_eq!(c.expected.as_deref(), Some("preserved"));
        assert_eq!(c.source.as_deref(), Some("a.md"));
        assert_eq!(c.topic_terms, ["lighthouse", "reply", "himself"]);
        assert!(in_scope("Callum answered with Lighthouse 8.", &c));
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["origin"], "e_1");
        let bare: Canary =
            serde_json::from_str(r#"{"id":"x","statement":"s","primary":"p.md"}"#).unwrap();
        assert!(serde_json::to_value(bare).unwrap().get("origin").is_none());
    }
    #[test]
    fn only_a_persons_contradict_makes_a_canary() {
        assert!(makes_canary("contradict", "callum"));
        assert!(makes_canary("contradict", "fergus"));
        for by in ["rule:auto", "judge:sonnet", "Callum"] {
            assert!(!makes_canary("contradict", by), "{by}");
        }
        assert!(!makes_canary("retract", "callum"));
    }
}
