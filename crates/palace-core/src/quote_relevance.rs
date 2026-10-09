//! M4b.1 S-2 / M4b.2 N-1 / M4d-1 0a: does a verbatim quote support its claim? Pure; shared by the
//! rollup validator and the claims check. Matching is on whole words: a word is a maximal run of
//! Unicode alphanumeric characters (everything else is a boundary) after a possessive `'s` / `’s`
//! is stripped, compared lowercased; an anchor matches only as a contiguous run of whole words
//! ("port" never matches inside "report").
use std::collections::HashSet;

use crate::claim::Claim;

/// Leading words that never anchor anything: articles and personal pronouns.
const LEADING: &[&str] = &["a", "an", "the", "i", "we", "he", "she", "it", "they"];
/// Words too common to anchor an object, in any capitalisation.
const STOPWORDS: &[&str] = &[
    "about", "after", "all", "also", "and", "any", "are", "been", "before", "but", "can", "did",
    "for", "from", "had", "has", "have", "her", "his", "into", "its", "may", "not", "now", "our",
    "over", "rather", "than", "that", "their", "them", "then", "there", "they", "this", "was",
    "were", "what", "when", "which", "while", "who", "will", "with", "you", "your",
];

/// Lowercased whole words of `text`, possessive `'s` stripped first.
pub fn words(text: &str) -> Vec<String> {
    strip_possessive(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `text` without a possessive `'s` / `’s` at the end of a word ("Callum's" → "Callum").
fn strip_possessive(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let apostrophe = matches!(chars[i], '\'' | '’');
        let ends = chars.get(i + 2).is_none_or(|c| !c.is_alphanumeric());
        if apostrophe && matches!(chars.get(i + 1), Some('s' | 'S')) && ends && i > 0 {
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn is_number(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
}

/// A single word that can never anchor: a stopword or leading word in any case, or a word shorter
/// than 3 characters that is not a number.
fn is_weak(word: &str) -> bool {
    LEADING.contains(&word)
        || STOPWORDS.contains(&word)
        || (word.chars().count() < 3 && !is_number(word))
}

/// `anchor` as words with leading articles/pronouns dropped; `None` when nothing is left or only a
/// single weak word.
pub fn anchor_words(anchor: &str) -> Option<Vec<String>> {
    let words = words(anchor);
    let skip = words
        .iter()
        .take_while(|w| LEADING.contains(&w.as_str()))
        .count();
    let rest = words[skip..].to_vec();
    let weak = rest.len() == 1 && is_weak(&rest[0]);
    (!rest.is_empty() && !weak).then_some(rest)
}

/// True when `needle` occurs in `haystack` as a contiguous run of whole words.
pub fn contains_words(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|run| run == needle)
}

/// The object's distinctive anchors, as word runs, never containing a word of the subject:
/// proper nouns and number-like tokens (`31:12`, `300-year`) when the object has any; otherwise
/// its content words (≥ 4 characters, not a stopword); and when excluding the subject's words
/// leaves nothing, the object's own content words (M4d-2a item 10, review L-7).
pub fn distinctive_anchors(claim: &Claim) -> Vec<Vec<String>> {
    anchors_of(claim).0
}

/// The distinctive anchors, and whether they are the fallback (the object's own content words,
/// every one of them a subject word or weak).
fn anchors_of(claim: &Claim) -> (Vec<Vec<String>>, bool) {
    let subject: HashSet<String> = words(&claim.subject).into_iter().collect();
    let proper = proper_nouns(claim);
    let mut strong = Vec::new();
    let mut content = Vec::new();
    for token in claim.object.split_whitespace() {
        let own: Vec<String> = words(token)
            .into_iter()
            .filter(|w| !subject.contains(w))
            .collect();
        if is_strong(&own, &proper) {
            strong.push(own);
        } else {
            content.extend(own.into_iter().filter(|w| is_content(w)).map(|w| vec![w]));
        }
    }
    match (strong.is_empty(), content.is_empty()) {
        (false, _) => (strong, false),
        (true, false) => (content, false),
        (true, true) => (object_content(claim), true),
    }
}

fn object_content(claim: &Claim) -> Vec<Vec<String>> {
    words(&claim.object)
        .into_iter()
        .filter(|w| is_content(w))
        .map(|w| vec![w])
        .collect()
}

/// Words capitalised somewhere in the claim's statement (`subject predicate object`, one
/// sentence) at a position where only a proper noun takes a capital (L-7): not sentence-initial,
/// not a clause, title or list start (`title_position`). A word capitalised only at such starts
/// is not a proper noun unless it is capitalised elsewhere in the claim (M4d-2a.1 item 6).
fn proper_nouns(claim: &Claim) -> HashSet<String> {
    let statement = format!(
        "{} {} {}",
        claim.subject,
        claim.predicate.replace('_', " "),
        claim.object
    );
    let tokens: Vec<&str> = statement.split_whitespace().collect();
    let mut proper = HashSet::new();
    for (i, token) in tokens.iter().enumerate().skip(1) {
        let capital = token
            .chars()
            .find(|c| c.is_alphanumeric())
            .is_some_and(char::is_uppercase);
        if capital && !title_position(tokens[i - 1], token) {
            proper.extend(words(token));
        }
    }
    proper
}

/// A capital that may start a clause, a title or a list item rather than name something: after
/// `,` `.` `;` `:` `!` `?` or a dash (M4d-2a item 10), and after `(`, a quote mark, `/`, `and`
/// or `or` (M4d-2a.1 item 6, review L-2: `(Replies)`, `"Report"`, `/ Report`, `and Report`).
fn title_position(previous: &str, token: &str) -> bool {
    previous.ends_with([',', '.', ';', ':', '!', '?', '—', '–', '-', '/'])
        || matches!(previous.to_lowercase().as_str(), "and" | "or")
        || token.starts_with(['(', '[', '"', '“', '\'', '‘', '/'])
}

/// A number-like token, or a proper-noun token, that is not a single weak word.
fn is_strong(own: &[String], proper: &HashSet<String>) -> bool {
    if own.is_empty() || (own.len() == 1 && is_weak(&own[0])) {
        return false;
    }
    own.iter().any(|w| is_number(w) || proper.contains(w))
}

fn is_content(word: &str) -> bool {
    word.chars().count() >= 4 && !STOPWORDS.contains(&word)
}

/// A verbatim `quote` supports `claim` only if the quote itself contains a distinctive object
/// anchor, or the subject and a distinctive object anchor both occur within 600 characters of the
/// quote's position in `source`. The subject alone is never enough (it names who, not what).
/// `false` when the quote is not in the source.
pub fn quote_supports(claim: &Claim, quote: &str, source: &str) -> bool {
    let Some(at) = source.find(quote) else {
        return false;
    };
    let (distinctive, fallback) = anchors_of(claim);
    let found = |text: &[String]| {
        distinctive.iter().any(|a| contains_words(text, a)) && (!fallback || acts(claim, text))
    };
    if found(&words(quote)) {
        return true;
    }
    let near = words(window(source, at, quote.len(), 600));
    anchor_words(&claim.subject).is_some_and(|s| contains_words(&near, &s)) && found(&near)
}

/// When the object's anchors are the fallback (only subject words), a quote naming them must also
/// carry a content word of the predicate: `Callum | quoted | Callum` is not supported by any
/// quote that names Callum (M4d-2a.1 item 6, review L-2).
fn acts(claim: &Claim, text: &[String]) -> bool {
    words(&claim.predicate.replace('_', " "))
        .iter()
        .filter(|w| is_content(w))
        .any(|w| text.contains(w))
}

/// `text[start..start + len]` widened by `margin` characters on each side.
fn window(text: &str, start: usize, len: usize, margin: usize) -> &str {
    let from = text[..start]
        .char_indices()
        .rev()
        .nth(margin.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    let end = start + len;
    let to = text[end..]
        .char_indices()
        .nth(margin)
        .map_or(text.len(), |(i, _)| end + i);
    &text[from..to]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        claim::{Claim, check, parse},
        rollup::Entry,
        traits::DiaryReader,
    };
    fn claim(line: &str) -> Claim {
        parse(&format!("## Claims\n{line}\n")).claims.remove(0)
    }
    pub const SOURCE: &str =
        "Callum replied under the Halloran post. Callum will report if brackenridge replies.";
    #[test]
    fn quote_with_an_object_anchor_supports() {
        let c = claim(
            "- C1. Callum | replied under | the Halloran post — 2026-09-21 — s.md — quote: \"under the Halloran post\"",
        );
        assert!(quote_supports(&c, c.quote.as_deref().unwrap(), SOURCE));
    }
    #[test]
    fn subject_alone_in_the_quote_does_not_support() {
        let c = claim(
            "- C6. Callum | quoted | Almanack 31:12 in his round-one comment — 2026-09-21 — s.md — quote: \"Callum will report if brackenridge replies\"",
        );
        assert!(!quote_supports(&c, c.quote.as_deref().unwrap(), SOURCE));
    }
    #[test]
    fn subject_and_object_near_the_quote_support() {
        let c =
            claim("- C2. Callum | follows | Halloran — 2026-09-21 — s.md — quote: \"will report\"");
        assert!(quote_supports(&c, "will report", SOURCE));
        let far = format!("Callum likes Halloran.{}will report", " x".repeat(400));
        assert!(!quote_supports(&c, "will report", &far));
    }
    #[test]
    fn irrelevant_verbatim_quote_is_unsupported_at_base_060() {
        struct One;
        impl DiaryReader for One {
            type Error = String;
            fn header(&self, _: &str) -> Result<String, String> {
                Ok(String::new())
            }
            fn names(&self) -> Result<Vec<String>, String> {
                Ok(vec!["s".into()])
            }
            fn read(&self, _: &str) -> Result<Option<String>, String> {
                Ok(Some(format!("# s\n{SOURCE}")))
            }
        }
        let entry = Entry::parse("r", String::new());
        let objects = ["Almanack 31:12 in his round-one comment"]
            .into_iter()
            .chain(BYPASSES);
        for object in objects {
            let c = claim(&format!(
                "- C6. Callum | quoted | {object} — 2026-09-21 — s.md — quote: \"Callum will report if brackenridge replies\""
            ));
            let rows = check::<_, crate::test_support::Preserving>(&entry, &[c], &One, None);
            assert_eq!(rows[0]["status"], "unsupported", "{object}");
            assert_eq!(rows[0]["confidence_base"], 0.6, "{object}");
        }
    }
    /// The M4b.1 re-review's three one-word bypasses of the original C6 (N-1), and the M4d-1
    /// review's post-comma and post-dash capitals (L-7).
    pub const BYPASSES: [&str; 6] = [
        "A verse of Almanack 31:12 in his round-one comment",
        "Almanack 31:12, I recall, in his round-one comment",
        "Almanack 31:12 in his round-one comment on the port",
        "Almanack 31:12 in his round-one comment, Replies",
        "Almanack 31:12 in his round-one comment, Report",
        "Almanack 31:12 in his round-one comment — Will's report",
    ];
    /// L-7: an object made only of subject words still anchors on its own content words; a
    /// mid-phrase capital is still a proper noun.
    #[test]
    fn residual_anchors() {
        let c = claim(
            "- C1. MemPalace team | pinned | MemPalace — 2026-09-21 — s.md — quote: \"we pinned MemPalace\"",
        );
        assert!(quote_supports(
            &c,
            "we pinned MemPalace",
            "so we pinned MemPalace today"
        ));
        let c = claim(
            "- C1. Callum | replied under | the post by Halloran — 2026-09-21 — s.md — quote: \"Halloran\"",
        );
        assert_eq!(distinctive_anchors(&c), [vec!["halloran".to_owned()]]);
    }
    #[test]
    fn the_three_bypasses_are_unsupported() {
        for object in BYPASSES {
            let c = claim(&format!(
                "- C6. Callum | quoted | {object} — 2026-09-21 — s.md — quote: \"Callum will report if brackenridge replies\""
            ));
            assert!(
                !quote_supports(&c, c.quote.as_deref().unwrap(), SOURCE),
                "{object}"
            );
        }
    }
    #[test]
    fn anchors_match_whole_words_case_insensitively() {
        let q = |s: &str| words(s);
        assert!(contains_words(&q("Callum will REPORT back"), &q("report")));
        assert!(!contains_words(&q("Callum will report back"), &q("port")));
        assert!(contains_words(
            &q("see Almanack 31:12."),
            &q("almanack 31:12")
        ));
        assert!(!contains_words(&q("Zürichsee"), &q("Zürich")));
        assert!(contains_words(&q("in ZÜRICH, then"), &q("zürich")));
        assert_eq!(anchor_words("A"), None);
        assert_eq!(anchor_words("the"), None);
        assert_eq!(anchor_words("x"), None);
        assert_eq!(anchor_words("The Halloran post"), Some(q("halloran post")));
        assert_eq!(anchor_words("It Works"), Some(q("works")));
    }
    #[test]
    fn a_leading_article_does_not_stop_a_real_anchor() {
        let c = claim(
            "- C1. Callum | replied under | The Halloran post — 2026-09-21 — s.md — quote: \"the halloran post\"",
        );
        let source = "Callum replied under the halloran post.";
        assert!(quote_supports(&c, "the halloran post", source));
    }
    const C6: &str = "Almanack 31:12 in his round-one comment";
    const IRRELEVANT: &str = "Callum will report if brackenridge replies";
    fn supports_c6(object: &str) -> bool {
        let c = claim(&format!(
            "- C6. Callum | quoted | {object} — 2026-09-21 — s.md — quote: \"{IRRELEVANT}\""
        ));
        quote_supports(&c, IRRELEVANT, SOURCE)
    }
    /// M4b.2 review S-1 / question 5: every variant of C6 with the same irrelevant quote.
    #[test]
    fn the_m4b2_review_variants_are_unsupported() {
        let objects = [
            format!("{C6} on reports"),
            format!("{C6} in his re-port"),
            format!("{C6}, Of note"),
            format!("{C6}, In"),
            "Almanack 31:12 in Callum's round-one comment".to_owned(),
            "Almanack 31:12 in Callum’s round-one comment".to_owned(),
            format!("{C6}, CALLUM"),
            format!("{C6}, An Callum"),
            format!("{C6}, If memory serves"),
            format!("{C6}, If"),
            format!("{C6} — The Will"),
            format!("{C6}, Will"),
            format!("{C6} on the report"),
            format!("{C6}, replies pending"),
        ];
        for object in objects {
            assert!(!supports_c6(&object), "{object}");
        }
    }
    /// A proper noun genuinely shared by object and quote is support (the lexical rule's limit).
    #[test]
    fn a_shared_proper_noun_still_supports() {
        assert!(supports_c6(&format!("{C6} on the Brackenridge thread")));
    }
    /// M4d-2a.1 item 6 (review L-2): a capital after `(`, a quote mark, `/`, `and` or `or` is a
    /// title or list start, not a proper noun — so `(Brackenridge thread)` no longer anchors
    /// either — unless the word is capitalised elsewhere in the claim or has a digit.
    pub const TITLE_STARTS: [&str; 7] = [
        " (Replies)",
        " \"Report\"",
        " “Report”",
        " / Report",
        " and Report",
        " or Replies",
        " (Brackenridge thread)",
    ];
    #[test]
    fn title_starts_are_not_proper_nouns() {
        for suffix in TITLE_STARTS {
            assert!(!supports_c6(&format!("{C6}{suffix}")), "{suffix}");
        }
        assert!(supports_c6(
            "Almanack 31:12 in his Replies comment (Replies)"
        ));
        let c = claim(
            "- C1. Callum | quoted | the verse (Psalm 23) — 2026-09-21 — s.md — quote: \"q\"",
        );
        let anchors = distinctive_anchors(&c);
        assert!(anchors.contains(&words("23")) && !anchors.contains(&words("psalm")));
    }
    /// M4d-2a.1 item 6 (review L-2): an object of subject words alone needs the predicate too.
    #[test]
    fn subject_words_alone_never_support() {
        for object in ["Callum", "Callum's", "the Callum"] {
            assert!(!supports_c6(object), "{object}");
        }
        let c = claim("- C1. Callum | quoted | Callum — 2026-09-21 — s.md — quote: \"q\"");
        assert!(quote_supports(
            &c,
            "Callum quoted Callum",
            "Callum quoted Callum"
        ));
    }
    #[test]
    fn capitalised_stopwords_and_short_words_never_anchor() {
        for weak in [
            "If", "Will", "So", "My", "No", "At", "By", "Or", "AI", "The", "And",
        ] {
            assert_eq!(anchor_words(weak), None, "{weak}");
        }
        assert_eq!(anchor_words("8"), Some(words("8")));
        assert_eq!(anchor_words("31:12"), Some(words("31 12")));
    }
    #[test]
    fn number_like_anchors_rank_first() {
        let c = claim(
            "- C1. Callum | quoted | Almanack 31:12 about iron sharpening — 2026-09-21 — s.md — quote: \"iron sharpening\"",
        );
        assert!(!quote_supports(
            &c,
            "iron sharpening",
            "iron sharpening iron"
        ));
        assert!(quote_supports(&c, "31:12 says", "31:12 says iron"));
    }
    #[test]
    fn subject_words_inside_the_object_never_anchor() {
        let c = claim(
            "- C1. Fergus | read | Fergus's Almanac — 2026-09-21 — s.md — quote: \"Fergus read it\"",
        );
        assert!(!quote_supports(&c, "Fergus read it", "Fergus read it"));
        assert!(distinctive_anchors(&c).contains(&words("almanac")));
    }
    #[test]
    fn possessives_hyphens_non_ascii_and_two_letter_words() {
        assert_eq!(words("Callum's and Callum’s"), words("callum and callum"));
        assert_eq!(words("it's"), words("it"));
        let c = claim(
            "- C1. Fergus | met | Jean-Luc in Zürich about AI — 2026-09-21 — s.md — quote: \"q\"",
        );
        assert!(distinctive_anchors(&c).contains(&words("jean luc")));
        assert!(!distinctive_anchors(&c).contains(&words("ai")));
        let supports = |quote: &str| quote_supports(&c, quote, quote);
        assert!(supports("jean luc arrived"));
        assert!(supports("in ZÜRICH, then"));
        assert!(!supports("the Zürichsee shore"));
        assert!(!supports("AI all day"));
        assert!(!supports("Jeanluc arrived"));
    }
}
