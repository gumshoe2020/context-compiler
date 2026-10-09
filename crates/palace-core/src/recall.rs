//! SETTLE-1b (spec §8): recall at both compression points — the pure parts. Seeded sampling,
//! the sentences of a transcript that carry values (dates, names, numbers) and the exact-match
//! test for specifics, the judge's question and verdicts for concepts, and the digest line.
use crate::ids::{length_prefixed, sha256};
use serde_json::Value;

/// `FIDELITY_SAMPLE`: items sampled per check.
pub const SAMPLE: usize = 10;
/// `FIDELITY_FLOOR`: concept recall below it pages.
pub const FLOOR: f64 = 0.7;
/// `DISTILL_RECALL_MIN_WORDS` (SETTLE-1c §1): a distill-point concept sample draws only from
/// messages with at least this many words.
pub const DISTILL_MIN_WORDS: usize = 8;

/// The words of a message once markdown and emoji are stripped: link targets (`](…)`) dropped,
/// then every whitespace-separated token holding a letter or digit (so `**`, `-`, `#`, `|` and
/// emoji are not words, and `**bold**` is one).
pub fn words(text: &str) -> usize {
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("](") {
        stripped.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        rest = rest.find(')').map_or("", |end| &rest[end + 1..]);
        stripped.push(' ');
    }
    stripped.push_str(rest);
    stripped
        .split_whitespace()
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .count()
}

/// Up to `n` of `items`, chosen by the hash of the seed (a transcript or rollup file name) and
/// each item's key, so a rerun samples the same items; returned in their original order.
pub fn sample<T: Clone>(seed: &str, items: &[T], key: impl Fn(&T) -> String, n: usize) -> Vec<T> {
    let mut ranked: Vec<(String, usize)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (sha256(length_prefixed(&[seed, &key(item)]).as_bytes()), i))
        .collect();
    ranked.sort();
    let mut chosen: Vec<usize> = ranked.into_iter().take(n).map(|(_, i)| i).collect();
    chosen.sort_unstable();
    chosen.into_iter().map(|i| items[i].clone()).collect()
}

/// The sentences of a text (split after `.`, `!`, `?` before a space, and at line breaks).
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut start = 0;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for (k, (i, c)) in chars.iter().enumerate() {
            let next_is_space = chars.get(k + 1).is_none_or(|(_, n)| n.is_whitespace());
            if matches!(c, '.' | '!' | '?') && next_is_space {
                push_sentence(&mut out, &line[start..i + c.len_utf8()]);
                start = i + c.len_utf8();
            }
        }
        push_sentence(&mut out, &line[start..]);
    }
    out
}
fn push_sentence(out: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        out.push(text.to_owned());
    }
}

/// The values a sentence carries: numbers (any word with a digit, e.g. `14`, `3,200`,
/// `2031-04-02`) and names (capitalized words not first in the sentence, month names included),
/// each trimmed of surrounding punctuation.
pub fn values(sentence: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, raw) in sentence.split_whitespace().enumerate() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
        let word = word.strip_suffix("'s").unwrap_or(word);
        if word.is_empty() {
            continue;
        }
        let number = word.chars().any(|c| c.is_ascii_digit());
        let name = index > 0 && word != "I" && word.chars().next().is_some_and(char::is_uppercase);
        if (number || name) && !found.iter().any(|f| f == word) {
            found.push(word.to_owned());
        }
    }
    found
}

/// A sampled sentence is `supported` when every value it carries is verbatim in the text of the
/// specifics (their subjects, objects and quotes), else `missing`. No judge.
pub fn specific_verdict(sentence: &str, specifics_text: &str) -> &'static str {
    match values(sentence)
        .iter()
        .all(|v| specifics_text.contains(v.as_str()))
    {
        true => "supported",
        false => "missing",
    }
}

/// A judge's answer as a recall verdict: `supported`, `missing` or `contradicted` (the watch's
/// `preserved`/`reversed`/`unclear` read as the first, third and second).
pub fn verdict(answer: &str) -> &'static str {
    match answer.trim().to_lowercase().as_str() {
        "supported" | "preserved" => "supported",
        "contradicted" | "reversed" => "contradicted",
        _ => "missing",
    }
}

/// The judge's question for one concept: does `text` capture `statement`?
pub fn question(statement: &str, text: &str) -> String {
    format!(
        "Recall check. Does the text below capture this statement?\n\nStatement: {statement}\n\nText:\n{}\n\nAnswer with one JSON object only: {{\"verdict\": \"supported\"|\"missing\"|\"contradicted\"}}. `supported`: the text states it or something that clearly implies it. `missing`: the text does not say it. `contradicted`: the text says the opposite or reverses who did what.",
        text.chars().take(12000).collect::<String>()
    )
}
/// The verdict of a model's answer to [`question`] (`None` when it gives none).
pub fn parse_answer(answer: &str) -> Option<&'static str> {
    let value = crate::distill::json_of(answer)?;
    let said = value["verdict"].as_str()?;
    ["supported", "missing", "contradicted"]
        .contains(&said)
        .then(|| verdict(said))
}

/// `DISTILL_RECALL_FOLLOWING` (SETTLE-1c §2): the messages after a sampled one that the
/// distill-point question carries.
pub const DISTILL_FOLLOWING: usize = 6;
/// The cap, in characters, on those following messages.
pub const FOLLOWING_CHARS: usize = 4000;
/// The distill-point verdict of a statement with no lasting content: kept in the row's items,
/// left out of `sampled`.
pub const NOT_APPLICABLE: &str = "not_applicable";

/// The messages after the sampled one as the distill-point question shows them: up to `n`, one
/// per line, at most [`FOLLOWING_CHARS`] characters in all.
pub fn following(messages: &[String], n: usize) -> String {
    let lines: Vec<String> = messages.iter().take(n).map(|m| format!("- {m}")).collect();
    lines.join("\n").chars().take(FOLLOWING_CHARS).collect()
}

/// The distill-point question (SETTLE-1c §2): does `text` (the run's entries) capture
/// `statement` as the conversation finally left it, given the `following` messages?
pub fn distill_question(statement: &str, following: &str, text: &str) -> String {
    let following = if following.is_empty() {
        "(none)"
    } else {
        following
    };
    format!(
        "Recall check. A conversation was distilled into the text below. Does the text capture this statement from the conversation, as the conversation finally left it?\n\nStatement: {statement}\n\nThe messages after it in the conversation, in order:\n{following}\n\nText:\n{}\n\n{}",
        text.chars().take(12000).collect::<String>(),
        DISTILL_RULES
    )
}
const DISTILL_RULES: &str = "Judge the statement as the conversation finally left it: if a later message updates, completes or replaces it, judge that final state, and answer `supported` when the text captures the final state. Hedged wording (\"should be\", \"I think\", \"probably\") that the text states definitively is `supported`, unless the text contradicts the substance.\n\nAnswer with one JSON object only: {\"verdict\": \"supported\"|\"missing\"|\"contradicted\"|\"not_applicable\"}. `supported`: the text states the statement's final state or something that clearly implies it. `missing`: the text does not say it. `contradicted`: only when the text asserts the opposite of the statement's final state, or reverses who did what. `not_applicable`: the statement has no lasting content (a greeting, an acknowledgement, a question with no answer in these messages, or procedural instructions).";

/// A distill-point verdict: [`verdict`]'s, or `not_applicable`.
pub fn distill_verdict(answer: &str) -> &'static str {
    match answer.trim().to_lowercase().as_str() {
        "not_applicable" | "not applicable" | "n/a" => NOT_APPLICABLE,
        other => verdict(other),
    }
}
/// The verdict of a model's answer to [`distill_question`] (`None` when it gives none).
pub fn parse_distill_answer(answer: &str) -> Option<&'static str> {
    let value = crate::distill::json_of(answer)?;
    let said = value["verdict"].as_str()?;
    ["supported", "missing", "contradicted", NOT_APPLICABLE]
        .contains(&said)
        .then(|| distill_verdict(said))
}

/// One `fidelity` row's counts (spec §8: sampled, supported, missing, contradicted; unjudged when
/// no judge ran).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub sampled: usize,
    pub supported: usize,
    pub missing: usize,
    pub contradicted: usize,
    pub unjudged: usize,
}
impl Counts {
    pub fn add(&mut self, verdict: Option<&str>) {
        self.sampled += 1;
        match verdict {
            Some("supported") => self.supported += 1,
            Some("contradicted") => self.contradicted += 1,
            Some(_) => self.missing += 1,
            None => self.unjudged += 1,
        }
    }
    /// Supported over judged, `None` when nothing was judged.
    pub fn recall(&self) -> Option<f64> {
        let judged = self.sampled - self.unjudged;
        (judged > 0).then(|| self.supported as f64 / judged as f64)
    }
}

/// Why a check pages (spec §8), if it does: a missing specific at consolidation, any
/// contradiction, or concept recall below `floor` (`FIDELITY_FLOOR`).
pub fn page_reason(point: &str, kind: &str, counts: &Counts, floor: f64) -> Option<String> {
    if point == "consolidation" && kind == "specific" && counts.missing > 0 {
        return Some(format!("{} specific(s) missing", counts.missing));
    }
    if counts.contradicted > 0 {
        return Some(format!("{} {kind}(s) contradicted", counts.contradicted));
    }
    match counts.recall() {
        Some(recall) if kind == "concept" && recall < floor => Some(format!(
            "concept recall {}/{} below {floor}",
            counts.supported,
            counts.sampled - counts.unjudged
        )),
        _ => None,
    }
}

/// The digest's line, `F: specifics 100%, concepts 9/10 (30-day 0.87)`, from the rows of the
/// digest's window and of the last 30 days (`type`, `sampled`, `supported`, `unjudged` each).
pub fn digest_line(window: &[Value], month: &[Value]) -> Option<String> {
    if window.is_empty() {
        return None;
    }
    let sum = |rows: &[Value], kind: &str, field: &str| -> u64 {
        rows.iter()
            .filter(|r| r["type"] == kind)
            .map(|r| r[field].as_u64().unwrap_or(0))
            .sum()
    };
    let judged =
        |rows: &[Value]| sum(rows, "concept", "sampled") - sum(rows, "concept", "unjudged");
    let specifics = match sum(window, "specific", "sampled") {
        0 => "-".to_owned(),
        n => format!("{}%", sum(window, "specific", "supported") * 100 / n),
    };
    let concepts = match judged(window) {
        0 => "unjudged".to_owned(),
        n => format!("{}/{n}", sum(window, "concept", "supported")),
    };
    let thirty = match judged(month) {
        0 => "-".to_owned(),
        n => format!(
            "{:.2}",
            sum(month, "concept", "supported") as f64 / n as f64
        ),
    };
    Some(format!(
        "F: specifics {specifics}, concepts {concepts} (30-day {thirty})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sampling_is_seeded_and_stable() {
        let items: Vec<u32> = (0..30).collect();
        let a = sample("r1.md", &items, |i| i.to_string(), 10);
        assert_eq!(a, sample("r1.md", &items, |i| i.to_string(), 10));
        assert_ne!(a, sample("r2.md", &items, |i| i.to_string(), 10));
        assert_eq!(a.len(), 10);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(sample("s", &items[..3], |i| i.to_string(), 10).len(), 3);
    }
    #[test]
    fn words_strip_markdown_and_emoji() {
        assert_eq!(words("Not yet. Let me do that now"), 7);
        assert_eq!(words("That should be good to go"), 6);
        assert_eq!(words("night night 🌙"), 2);
        assert_eq!(words("ok thanks 👍 👍 👍 👍 👍 👍 👍 👍"), 2);
        assert_eq!(
            words("- **Done** — see [the notes](https://example.org/a b c) | ✅"),
            4
        );
        assert_eq!(
            words("I haven't sent the invoice to Corvin Dale yet."),
            DISTILL_MIN_WORDS + 1
        );
    }
    #[test]
    fn the_distill_question_carries_the_final_state() {
        let later: Vec<String> = (1..=8).map(|i| format!("[user] message {i}")).collect();
        let shown = following(&later, DISTILL_FOLLOWING);
        assert!(shown.contains("- [user] message 6") && !shown.contains("message 7"));
        let long = vec!["x".repeat(3000), "y".repeat(3000)];
        assert_eq!(
            following(&long, DISTILL_FOLLOWING).chars().count(),
            FOLLOWING_CHARS
        );
        let q = distill_question("[user] I haven't sent it yet.", &shown, "Sent.");
        assert!(q.contains("Statement: [user] I haven't sent it yet."));
        assert!(q.contains("- [user] message 1\n"));
        assert!(q.contains("as the conversation finally left it"));
        assert!(q.contains("Hedged wording"));
        assert!(q.contains("\"not_applicable\""));
        assert!(distill_question("s", "", "t").contains("in order:\n(none)\n"));
        assert_eq!(
            parse_distill_answer("{\"verdict\":\"not_applicable\"}"),
            Some(NOT_APPLICABLE)
        );
        assert_eq!(parse_distill_answer("{\"verdict\":\"unclear\"}"), None);
        assert_eq!(distill_verdict("N/A"), NOT_APPLICABLE);
        assert_eq!(distill_verdict("reversed"), "contradicted");
        assert_eq!(parse_answer("{\"verdict\":\"not_applicable\"}"), None);
    }
    #[test]
    fn value_sentences_and_exact_match() {
        let text = "Tobiah Quell moved the Larkspur ferry inspection to May 14. Good!\nThe crew budget is 3,200 crowns.";
        let s = sentences(text);
        assert_eq!(s.len(), 3);
        assert_eq!(values(&s[0]), ["Quell", "Larkspur", "May", "14"]);
        assert!(values("Good!").is_empty());
        assert_eq!(values(&s[2]), ["3,200"]);
        let specifics = "Larkspur ferry inspection May 14 Tobiah Quell moved";
        assert_eq!(specific_verdict(&s[0], specifics), "supported");
        assert_eq!(specific_verdict(&s[2], specifics), "missing");
    }
    #[test]
    fn verdicts_pages_and_the_digest_line() {
        assert_eq!(verdict("preserved"), "supported");
        assert_eq!(verdict("reversed"), "contradicted");
        assert_eq!(verdict("unclear"), "missing");
        assert_eq!(parse_answer("x {\"verdict\":\"missing\"}"), Some("missing"));
        assert_eq!(parse_answer("no"), None);
        let mut c = Counts::default();
        for v in ["supported"; 9].into_iter().chain(["contradicted"]) {
            c.add(Some(v));
        }
        assert_eq!(
            page_reason("consolidation", "concept", &c, FLOOR).unwrap(),
            "1 concept(s) contradicted"
        );
        let mut low = Counts::default();
        for v in ["supported", "missing", "missing"] {
            low.add(Some(v));
        }
        assert!(
            page_reason("distill", "concept", &low, FLOOR)
                .unwrap()
                .contains("below 0.7")
        );
        let mut gone = Counts::default();
        gone.add(Some("missing"));
        assert!(page_reason("consolidation", "specific", &gone, FLOOR).is_some());
        assert!(page_reason("distill", "specific", &gone, FLOOR).is_none());
        let mut none = Counts::default();
        none.add(None);
        assert_eq!(page_reason("consolidation", "concept", &none, FLOOR), None);
        let rows = [
            json!({"type":"specific","sampled":4,"supported":4,"unjudged":0}),
            json!({"type":"concept","sampled":10,"supported":9,"unjudged":0}),
        ];
        let month = [json!({"type":"concept","sampled":20,"supported":17,"unjudged":0})];
        assert_eq!(
            digest_line(&rows, &month).unwrap(),
            "F: specifics 100%, concepts 9/10 (30-day 0.85)"
        );
        assert_eq!(digest_line(&[], &month), None);
    }
}
