//! When a message asks for a specific, and which live specifics it names (CTX-1 §1 block 1, §2),
//! pure and deterministic: no model.
//!
//! **Asks** (`asks`): the message has one of `prompts/specific-triggers.json`'s phrases anywhere
//! (`…` inside a phrase stands for one to six words), a question word as the first word of a
//! sentence or in a message holding a `?`, or a check phrase ("was it") with a date or number in
//! the message (a digit, a month or a weekday). Words are lowercase runs of letters, digits and
//! apostrophes (curly ones made straight).
//!
//! **Names**: a specific is named when one of
//! the message's canonical tokens (`entity_match::canon_tokens`: three or more characters, the
//! stop-list dropped, a trailing `s` folded) is a canonical token of its subject or object.
//! Value tokens never name an entity: a token holding a digit ("24th", "2031"), a month or a
//! weekday matches nothing, so "was it the 24th?" alone names no specific.
use crate::entity_match::canon_tokens;
use serde::Deserialize;
use std::collections::BTreeSet;

/// The longest run of words a `…` in a phrase stands for.
const MAX_GAP: usize = 6;

/// The word lists of `prompts/specific-triggers.json`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Triggers {
    pub phrases: Vec<String>,
    pub question_words: Vec<String>,
    pub check_phrases: Vec<String>,
}

impl Triggers {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("specific triggers: {e}"))
    }
}

/// Whether the message asks for a specific (see the module notes).
pub fn asks(message: &str, t: &Triggers) -> bool {
    let words = words(message);
    let plain: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let has = |phrase: &String| contains(&plain, &words_of(phrase));
    if t.phrases.iter().any(has) {
        return true;
    }
    let question = message.contains('?');
    let asked = |q: &String| {
        words
            .iter()
            .any(|w| w.text == q.to_lowercase() && (question || w.starts_sentence))
    };
    if t.question_words.iter().any(asked) {
        return true;
    }
    plain.iter().any(|w| is_value(w)) && t.check_phrases.iter().any(has)
}

struct Word {
    text: String,
    starts_sentence: bool,
}

/// The message's words, each marked when it opens a sentence (the first, or after `.`, `!`, `?`).
fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let (mut current, mut opening, mut next_opens) = (String::new(), true, true);
    for c in text
        .chars()
        .map(|c| if matches!(c, '’' | '‘') { '\'' } else { c })
    {
        if c.is_alphanumeric() || c == '\'' {
            if current.is_empty() {
                opening = next_opens;
                next_opens = false;
            }
            current.extend(c.to_lowercase());
            continue;
        }
        push_word(&mut out, &mut current, opening);
        if matches!(c, '.' | '!' | '?') {
            next_opens = true;
        }
    }
    push_word(&mut out, &mut current, opening);
    out
}

fn push_word(out: &mut Vec<Word>, current: &mut String, opening: bool) {
    let text = current.trim_matches('\'').to_owned();
    current.clear();
    if !text.is_empty() {
        out.push(Word {
            text,
            starts_sentence: opening,
        });
    }
}

/// A phrase's words, `…` kept as its own word.
fn words_of(phrase: &str) -> Vec<String> {
    phrase
        .split_whitespace()
        .flat_map(|w| match w {
            "…" | "..." => vec!["…".to_owned()],
            w => words(w).into_iter().map(|w| w.text).collect(),
        })
        .collect()
}

fn contains(words: &[&str], phrase: &[String]) -> bool {
    !phrase.is_empty() && (0..words.len()).any(|i| matches_at(&words[i..], phrase))
}

fn matches_at(words: &[&str], phrase: &[String]) -> bool {
    let Some((first, rest)) = phrase.split_first() else {
        return true;
    };
    if first == "…" {
        return (1..=MAX_GAP.min(words.len())).any(|gap| matches_at(&words[gap..], rest));
    }
    words.first().is_some_and(|w| w == first) && matches_at(&words[1..], rest)
}

/// A date or number word: one holding a digit, a month or a weekday.
fn is_value(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
        || CALENDAR.contains(&word)
        || word
            .strip_suffix('s')
            .is_some_and(|w| CALENDAR.contains(&w))
}

/// Month and weekday names (folded as `canon_tokens` folds them), never entity tokens.
const CALENDAR: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "jan",
    "feb",
    "mar",
    "apr",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "mon",
    "tue",
    "tues",
    "wed",
    "thu",
    "thur",
    "thurs",
    "fri",
    "sat",
    "sun",
];

/// The tokens of `text` that can name an entity.
pub fn entity_tokens(text: &str) -> BTreeSet<String> {
    canon_tokens(text)
        .into_iter()
        .filter(|t| !t.chars().any(|c| c.is_ascii_digit()))
        .filter(|t| !CALENDAR.contains(&t.as_str()))
        .collect()
}

/// Whether the message (its entity tokens) names the specific's subject or object.
pub fn names_specific(message: &BTreeSet<String>, subject: &str, object: &str) -> bool {
    let named = |text: &str| entity_tokens(text).iter().any(|t| message.contains(t));
    named(subject) || named(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> Triggers {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/specific-triggers.json"
        );
        Triggers::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn value_questions_and_recall_asks_trigger() {
        let t = shipped();
        for message in [
            "When does the Tern harbour survey boat leave?",
            "what time is the Mossgate lease signing",
            "How much does the boat hire cost us",
            "Remind me what Petra agreed for the bakery rent",
            "What did Bram Feld say about the keys",
            "What did we finally decide about the survey, Corin?",
            "Did we fix the Mossgate rent already",
            "who checks the soundings? I forget",
            "Where is the survey boat moored? Kelbrook or Tern?",
            "Thanks. Who signs the Mossgate lease",
            "the boat leaves Kelbrook, was it the 19th?",
            "Is it Tuesday that Corin checks the soundings",
            "What’s the address of the Mossgate bakery",
            "I need the survey date. What was it again",
        ] {
            assert!(asks(message, &t), "{message}");
        }
    }

    #[test]
    fn chatty_messages_never_trigger() {
        let t = shipped();
        for message in [
            "Thanks, that's great work on the Tern harbour survey",
            "ok sounds good, let's carry on with the Mossgate bakery lease",
            "Love it. The survey boat from Kelbrook is a good call",
            "Let me think about the quince press and come back to you",
            "Nice — the bakery lease looks done, well played",
            "I'll ask Corin Dask whether the soundings look sane",
            "we should tidy the Tern harbour notes when we have time",
            "Good night, talk tomorrow about the survey",
            "The boat leaves on the 19th, as planned",
            "Nobody knows where the time goes, but the survey is fine",
        ] {
            assert!(!asks(message, &t), "{message}");
        }
    }

    #[test]
    fn a_gap_stands_for_one_to_six_words() {
        let t = Triggers {
            phrases: vec!["what did … say".into()],
            ..Default::default()
        };
        assert!(asks("what did Bram say", &t));
        assert!(asks("what did the landlord of the old unit say", &t));
        assert!(!asks("what did say", &t));
        assert!(!asks("what did one two three four five six seven say", &t));
    }

    #[test]
    fn a_specific_is_named_by_its_subject_or_object_never_by_a_value() {
        let m = entity_tokens("When does Ravi swap with Theo at the orchard?");
        assert!(names_specific(&m, "Ravi", "swaps with Theo on the 24th"));
        assert!(names_specific(&m, "orchard rota", "the 24th"));
        assert!(!names_specific(&m, "ferry inspection", "May 14"));
        let values = entity_tokens("was it the 24th of May, or Tuesday 2031?");
        assert!(values.is_empty(), "{values:?}");
        assert!(!names_specific(&values, "inspection", "the 24th of May"));
    }
}
