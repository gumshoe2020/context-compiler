//! SETTLE-1d §1: the values a sentence carries, and whether the specifics keep each one. Pure and
//! deterministic: no model.
//!
//! **Values.** Code goes first, content and all (SETTLE-1e §1): fenced code blocks ([`prose`],
//! applied to a whole message before it is split into sentences) and inline code spans. Then
//! markdown is stripped (`*`, a leading numbered-list marker; the other markers, `#`s, bullets,
//! `_` and emoji are not words), so the first word left is the sentence start. A value is a word
//! holding a digit (a number) or a capitalized word that is not the sentence start and not on the
//! common word list (`prompts/recall-common-words.json`: a name). Words are trimmed of surrounding
//! punctuation and a trailing `'s`; a value is listed once (case-insensitive). Never a value: a
//! URL or bare domain (`https://…`, `backblaze.com`), a path or handle (a token holding `/` and a
//! letter, or starting with `~`, `./` or `/`), a command-line flag (a token starting with `-`)
//! and a single character. Dates (`10/17`), longer numbers and hyphenated names (`CTX-1`) stay.
//!
//! **Kept.** A value is kept when the specifics' text holds it as a whole token, case-insensitively
//! (SETTLE-1e §2): tokens break at anything but letters, digits and the joiners `-`, `.`, `/`,
//! `:`, `'` between two of them, so `Morrow` is not in `tomorrow` nor `CTX` in `CTX-1`. A trailing
//! `'s` ends a token, as it does a value.
//! A value that is part of a date in the sentence (`10/17`, `2026-10-17`, `Oct 17`, `17 October`,
//! `Saturday the 17th`, a weekday before any of them; no year: the run's) is also kept when a
//! specific's `date`, or a date in its text, is that calendar date.
use crate::diary_file::{civil_from_days, days_from_civil};
use serde_json::Value;
use std::collections::BTreeSet;

/// The common word list: capitalized words that are never names.
#[derive(Clone, Debug, Default)]
pub struct CommonWords(BTreeSet<String>);

impl CommonWords {
    /// Every key of the JSON object not starting with `_` holds a list of words; all are merged.
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(json).map_err(|e| format!("recall common words: {e}"))?;
        let object = value
            .as_object()
            .ok_or("recall common words: not a JSON object")?;
        let mut words = BTreeSet::new();
        for (key, list) in object.iter().filter(|(k, _)| !k.starts_with('_')) {
            let list = list
                .as_array()
                .ok_or_else(|| format!("recall common words: {key} is not a list"))?;
            words.extend(list.iter().filter_map(Value::as_str).map(fold));
        }
        Ok(Self(words))
    }
    fn has(&self, word: &str) -> bool {
        self.0.contains(&fold(word))
    }
}

/// A word as the common list and the matching compare it: lowercase, straight apostrophes.
fn fold(word: &str) -> String {
    word.replace(['\u{2019}', '\u{2018}'], "'").to_lowercase()
}

/// The values of a sentence (see the module notes), in order.
pub fn values(sentence: &str, common: &CommonWords) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (index, (raw, word)) in tokens(&prose(sentence)).into_iter().enumerate() {
        if word.chars().count() < 2 || is_code_like(&raw) {
            continue;
        }
        let number = word.chars().any(|c| c.is_ascii_digit());
        let capital = word.chars().next().is_some_and(char::is_uppercase);
        let name = index > 0 && capital && !common.has(&word);
        if (number || name) && !found.iter().any(|f| fold(f) == fold(&word)) {
            found.push(word);
        }
    }
    found
}

/// The text without its code: each fenced code block (a line opening with ```` ``` ```` or `~~~`
/// up to the closing fence, or the end) becomes an empty line, and each inline code span a space;
/// a backtick run that closes nothing is dropped.
pub fn prose(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<String> = None;
    for line in text.split('\n') {
        let opener = fence_marker(line);
        match (&fence, opener) {
            (Some(open), Some(close)) if close.starts_with(open.as_str()) => fence = None,
            (Some(_), _) => {}
            (None, Some(open)) => fence = Some(open),
            (None, None) => {
                out.push(without_code_spans(line));
                continue;
            }
        }
        out.push(String::new());
    }
    out.join("\n")
}
/// The fence a line opens or closes with (its run of backticks or tildes), when it is one.
fn fence_marker(line: &str) -> Option<String> {
    let trimmed = line.trim_start_matches(' ');
    let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run: String = trimmed.chars().take_while(|c| *c == mark).collect();
    (line.len() - trimmed.len() <= 3 && run.len() >= 3).then_some(run)
}
/// The line with each inline code span (a backtick run up to the next run of the same length)
/// replaced by a space.
fn without_code_spans(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let run_at = |i: usize| chars[i..].iter().take_while(|c| **c == '`').count();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let n = run_at(i);
        let close = (i + n..chars.len())
            .find(|&j| chars[j] == '`' && chars[j - 1] != '`' && run_at(j) == n);
        match close {
            Some(j) => {
                out.push(' ');
                i = j + n;
            }
            None => i += n,
        }
    }
    out
}
/// A URL, bare domain, path, handle or command-line flag (SETTLE-1e §1), judged on the token as
/// written, less any brackets, quotes and trailing punctuation around it.
fn is_code_like(raw: &str) -> bool {
    let token = raw
        .trim_start_matches(['(', '[', '{', '<', '"', '\'', '\u{201C}', '\u{2018}'])
        .trim_end_matches([
            ')', ']', '}', '>', '"', '\'', '\u{201D}', '\u{2019}', '.', ',', ';', ':', '!', '?',
        ]);
    let lower = token.to_lowercase();
    let letter = token.chars().any(char::is_alphabetic);
    token.starts_with('-')
        || token.starts_with(['~', '/'])
        || token.starts_with("./")
        || (token.contains('/') && letter)
        || lower.contains("://")
        || lower.starts_with("www.")
        || is_domain(&lower)
}
/// Top-level domains a bare domain is recognised by.
const DOMAINS: [&str; 30] = [
    "com", "org", "net", "io", "ai", "dev", "app", "co", "edu", "gov", "me", "us", "uk", "de",
    "fr", "ca", "au", "eu", "info", "biz", "xyz", "sh", "so", "gg", "tv", "ly", "fm", "cloud",
    "tech", "site",
];
/// `backblaze.com`, `docs.example.io`: dot-separated labels of letters, digits and `-`, the last
/// a known top-level domain.
fn is_domain(lower: &str) -> bool {
    let labels: Vec<&str> = lower.split('.').collect();
    let label = |l: &&str| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-');
    labels.len() >= 2
        && labels.iter().all(label)
        && labels.last().is_some_and(|tld| DOMAINS.contains(tld))
}

/// The words of a sentence once markdown is stripped, each trimmed of surrounding punctuation and
/// a trailing `'s`; tokens with no letter or digit (bullets, `#`s, emoji) are not words.
fn words(sentence: &str) -> Vec<String> {
    tokens(sentence).into_iter().map(|(_, word)| word).collect()
}
/// Each word of the sentence (see [`words`]) with the token it was written as.
fn tokens(sentence: &str) -> Vec<(String, String)> {
    let plain: String = sentence
        .chars()
        .filter(|c| !matches!(c, '*' | '`'))
        .collect();
    let mut tokens: Vec<&str> = plain.split_whitespace().collect();
    let marker = tokens
        .iter()
        .position(|t| t.chars().any(char::is_alphanumeric));
    if let Some(first) = marker.filter(|&i| is_list_number(tokens[i])) {
        tokens.remove(first);
    }
    tokens
        .into_iter()
        .map(|t| {
            let w = t.trim_matches(|c: char| !c.is_alphanumeric());
            let w = w
                .strip_suffix("'s")
                .or_else(|| w.strip_suffix("\u{2019}s"))
                .unwrap_or(w);
            (t.to_owned(), w.to_owned())
        })
        .filter(|(_, w)| !w.is_empty())
        .collect()
}
/// `1.` or `12)`: a numbered-list marker.
fn is_list_number(token: &str) -> bool {
    let digits = token.trim_end_matches(['.', ')']);
    digits.len() < token.len()
        && token.len() - digits.len() == 1
        && !digits.is_empty()
        && digits.len() <= 3
        && digits.bytes().all(|b| b.is_ascii_digit())
}

/// A calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    pub year: i64,
    pub month: i64,
    pub day: i64,
}
impl Day {
    /// `YYYY-MM-DD` (the rest ignored), when it is a real date.
    pub fn parse(text: &str) -> Option<Self> {
        let b = text.as_bytes();
        let digits = [0, 1, 2, 3, 5, 6, 8, 9];
        let shaped = b.len() >= 10 && b[4] == b'-' && b[7] == b'-';
        if !shaped || !digits.iter().all(|&i| b[i].is_ascii_digit()) {
            return None;
        }
        let n = |r: std::ops::Range<usize>| text[r].parse::<i64>().ok();
        let day = Self {
            year: n(0..4)?,
            month: n(5..7)?,
            day: n(8..10)?,
        };
        day.is_real().then_some(day)
    }
    fn is_real(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && civil_from_days(self.days()) == (self.year, self.month, self.day)
    }
    fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }
    /// Sunday = 0.
    fn weekday(&self) -> i64 {
        (self.days() + 4).rem_euclid(7)
    }
}

const MONTHS: [&str; 12] = [
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
];
const WEEKDAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
/// A month's number from its name or a 3-letter (or `sept`) abbreviation.
fn month(word: &str) -> Option<i64> {
    let w = word.to_lowercase();
    let w = w.trim_end_matches('.');
    MONTHS
        .iter()
        .position(|m| {
            *m == w || (w.len() >= 3 && m.starts_with(w) && (w.len() == 3 || w == "sept"))
        })
        .map(|i| i as i64 + 1)
}
fn weekday(word: &str) -> Option<i64> {
    let w = word.to_lowercase();
    let w = w.trim_end_matches('.');
    WEEKDAYS
        .iter()
        .position(|d| *d == w || (w.len() == 3 && d.starts_with(w)))
        .map(|i| i as i64)
}
/// `17`, `17th`, `1st`, `2nd`, `3rd`: a day of the month.
fn day_number(word: &str) -> Option<i64> {
    let lower = word.to_lowercase();
    let digits = ["st", "nd", "rd", "th"]
        .iter()
        .find_map(|s| lower.strip_suffix(s))
        .unwrap_or(&lower);
    let n: i64 = digits.parse().ok().filter(|_| digits.len() <= 2)?;
    (1..=31).contains(&n).then_some(n)
}
fn ordinal(word: &str) -> bool {
    word.len() > 2 && day_number(word).is_some() && !word.bytes().all(|b| b.is_ascii_digit())
}
/// `10/17`, `10/17/26`, `10/17/2026`.
fn slashed(word: &str, run: Day) -> Option<Day> {
    let parts: Vec<&str> = word.split('/').collect();
    let n = |s: &str| {
        s.parse::<i64>()
            .ok()
            .filter(|_| s.bytes().all(|b| b.is_ascii_digit()))
    };
    let year = match parts.get(2) {
        None => run.year,
        Some(y) if y.len() == 2 => 2000 + n(y)?,
        Some(y) if y.len() == 4 => n(y)?,
        Some(_) => return None,
    };
    if !(2..=3).contains(&parts.len()) || parts[0].len() > 2 || parts[1].len() > 2 {
        return None;
    }
    let day = Day {
        year,
        month: n(parts[0])?,
        day: n(parts[1])?,
    };
    day.is_real().then_some(day)
}
fn dated(year: i64, month: i64, day: i64) -> Option<Day> {
    let d = Day { year, month, day };
    d.is_real().then_some(d)
}
/// A 4-digit year word.
fn year(word: Option<&String>) -> Option<i64> {
    let w = word?;
    (w.len() == 4 && w.bytes().all(|b| b.is_ascii_digit()))
        .then(|| w.parse().ok())
        .flatten()
}
/// The month nearest the run date (within three months either side) whose `day` falls on
/// `weekday`.
fn nearest_weekday(day: i64, weekday: i64, run: Day) -> Option<Day> {
    (-3i64..=3)
        .filter_map(|shift| {
            let index = run.year * 12 + run.month - 1 + shift;
            dated(index.div_euclid(12), index.rem_euclid(12) + 1, day)
        })
        .filter(|d| d.weekday() == weekday)
        .min_by_key(|d| (d.days() - run.days()).abs())
}

/// The dates of a text, each with the (folded) words that state it.
pub fn dates(text: &str, run: Day) -> Vec<(Day, Vec<String>)> {
    let words = words(text);
    let mut found: Vec<(Day, Vec<String>)> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let (date, used) = date_at(&words, i, run);
        let Some(date) = date else {
            i += 1;
            continue;
        };
        let mut said: Vec<String> = words[i..i + used].iter().map(|w| fold(w)).collect();
        let before = i.checked_sub(1).map(|b| &words[b]);
        if let Some(before) = before.filter(|b| weekday(b) == Some(date.weekday())) {
            said.push(fold(before)); // "Saturday, 10/17"
        }
        found.push((date, said));
        i += used;
    }
    found
}
/// The date starting at word `i`, and how many words state it.
fn date_at(words: &[String], i: usize, run: Day) -> (Option<Day>, usize) {
    let w = &words[i];
    let next = words.get(i + 1);
    if let Some(day) = Day::parse(w).filter(|_| w.len() == 10) {
        return (Some(day), 1);
    }
    if let Some(day) = slashed(w, run) {
        return (Some(day), 1);
    }
    if let (Some(m), Some(d)) = (month(w), next.and_then(|n| day_number(n))) {
        let given = year(words.get(i + 2));
        let date = dated(given.unwrap_or(run.year), m, d);
        return (date, if given.is_some() { 3 } else { 2 });
    }
    if let (Some(d), Some(m)) = (day_number(w), next.and_then(|n| month(n))) {
        let given = year(words.get(i + 2));
        let date = dated(given.unwrap_or(run.year), m, d);
        return (date, if given.is_some() { 3 } else { 2 });
    }
    if let Some(wd) = weekday(w) {
        let the = next.is_some_and(|n| n.eq_ignore_ascii_case("the"));
        let at = if the { i + 2 } else { i + 1 };
        if let Some(d) = words
            .get(at)
            .filter(|n| ordinal(n))
            .and_then(|n| day_number(n))
        {
            return (nearest_weekday(d, wd, run), at - i + 1);
        }
    }
    (None, 1)
}

/// What the specifics of a check state: their text (lowercase) and every date they name.
#[derive(Clone, Debug, Default)]
pub struct Specifics {
    text: String,
    dates: BTreeSet<Day>,
}
impl Specifics {
    /// From each specific's text (subject, object, quote) and `date` (`YYYY-MM-DD`); dates in the
    /// text without a year take the specific's year, else the run's.
    pub fn new<'a>(items: impl IntoIterator<Item = (&'a str, &'a str)>, run: Day) -> Self {
        let mut specifics = Self::default();
        for (text, date) in items {
            let own = Day::parse(date);
            specifics.dates.extend(own);
            let base = Day {
                year: own.map_or(run.year, |d| d.year),
                ..run
            };
            specifics
                .dates
                .extend(dates(text, base).into_iter().map(|(d, _)| d));
            specifics.text.push_str(&fold(text));
            specifics.text.push('\n');
        }
        specifics
    }
}

/// Each value of the sentence and whether the specifics keep it (see the module notes).
pub fn kept_values(
    sentence: &str,
    specifics: &Specifics,
    common: &CommonWords,
    run: Day,
) -> Vec<(String, bool)> {
    let dated = dates(sentence, run);
    values(sentence, common)
        .into_iter()
        .map(|value| {
            let folded = fold(&value);
            let by_date = dated
                .iter()
                .any(|(d, said)| said.contains(&folded) && specifics.dates.contains(d));
            let kept = by_date || has_token(&specifics.text, &folded);
            (value, kept)
        })
        .collect()
}

/// Whether `text` holds `value` as a whole token (see the module notes); both already folded.
fn has_token(text: &str, value: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let wanted: Vec<char> = value.chars().collect();
    let n = wanted.len();
    !wanted.is_empty()
        && (0..=chars.len().saturating_sub(n)).any(|start| {
            chars.len() >= n
                && chars[start..start + n] == wanted[..]
                && (start == 0 || !in_token(&chars, start - 1))
                && (start + n == chars.len()
                    || !in_token(&chars, start + n)
                    || possessive(&chars, start + n))
        })
}
/// Whether the character at `i` belongs to a token: a letter or digit, or a joiner between two.
fn in_token(chars: &[char], i: usize) -> bool {
    let alnum = |j: Option<usize>| {
        j.and_then(|j| chars.get(j))
            .is_some_and(|c| c.is_alphanumeric())
    };
    alnum(Some(i))
        || (matches!(chars[i], '-' | '.' | '/' | ':' | '\'')
            && alnum(i.checked_sub(1))
            && alnum(Some(i + 1)))
}
/// `'s` closing a token at `i`.
fn possessive(chars: &[char], i: usize) -> bool {
    chars.get(i) == Some(&'\'')
        && chars.get(i + 1) == Some(&'s')
        && (i + 2 == chars.len() || !in_token(chars, i + 2))
}

/// A sampled sentence is `supported` when the specifics keep every value it carries, else
/// `missing`. No judge.
pub fn specific_verdict(
    sentence: &str,
    specifics: &Specifics,
    common: &CommonWords,
    run: Day,
) -> &'static str {
    match kept_values(sentence, specifics, common, run)
        .iter()
        .all(|(_, kept)| *kept)
    {
        true => "supported",
        false => "missing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn common() -> CommonWords {
        CommonWords::parse(include_str!("../../../prompts/recall-common-words.json")).unwrap()
    }
    const RUN: Day = Day {
        year: 2026,
        month: 10,
        day: 10,
    };
    #[test]
    fn chatter_and_markdown_carry_no_values() {
        let c = common();
        for none in [
            "If you're up with her, I'm here.",
            "If you end up awake with her for a bit, I'm here.",
            "- **Wenna goes in first** and the rest wait.",
            "1. **Offer** the spare room.",
            "## Next steps",
            "I'll check. She said That's fine. Same again? Offer it. Prove it.",
            "\u{1F319} Tobiah sleeps early tonight.",
            "> _Tobiah_ said hello.",
            "`Wenna` is fine.",
            "Ok, I\u{2019}ll do it.",
        ] {
            assert!(
                values(none, &c).is_empty(),
                "{none}: {:?}",
                values(none, &c)
            );
        }
    }
    #[test]
    fn numbers_names_months_and_weekdays_are_values() {
        let c = common();
        assert_eq!(
            values("Ravi and Theo swap: Ravi takes the 24th.", &c),
            ["Theo", "Ravi", "24th"]
        );
        assert_eq!(
            values(
                "The budget is 3,200 crowns, due 10/17 or 2031-04-02 (v5.5).",
                &c
            ),
            ["3,200", "10/17", "2031-04-02", "v5.5"]
        );
        assert_eq!(
            values("We meet on Saturday in May at Larkspur's dock.", &c),
            ["Saturday", "May", "Larkspur"]
        );
        assert_eq!(
            values("then Wenna met Quell, WENNA and quell", &c),
            ["Wenna", "Quell"]
        );
    }
    #[test]
    fn code_urls_paths_flags_and_single_characters_are_not_values() {
        let c = common();
        let message = "Run this:\n```sh\nssh -U Tomas@Larkspur ~/.ssh/config\nTerminal Allow\n```\nThen `git push Origin` from ./Build, see https://Quell.example/x or backblaze.com, Wenna/nix-demo and /opt/Ravi. Use --dry-run or -U. So Priya moved the review to 10/17.";
        let found: Vec<String> = crate::recall::sentences(&prose(message))
            .iter()
            .flat_map(|s| values(s, &c))
            .collect();
        assert_eq!(found, ["Priya", "10/17"]);
        assert!(values("So Ravi saw F, U, 4 and 2.", &c) == ["Ravi"]);
        assert_eq!(
            values("So Quell shipped CTX-1 for x86-64, 12 days early.", &c),
            ["Quell", "CTX-1", "x86-64", "12"]
        );
        assert_eq!(values("Ask Wenna: `Tobiah` ``a ` b`` done", &c), ["Wenna"]);
        assert_eq!(prose("a\n~~~~\nB ```\n~~~\nC\n~~~~\nd"), "a\n\n\n\n\n\nd");
        assert_eq!(prose("``` open\nNever closed"), "\n");
    }
    #[test]
    fn dates_are_read_with_the_run_year() {
        let d = |y, m, day| Day {
            year: y,
            month: m,
            day,
        };
        let found =
            |text: &str| -> Vec<Day> { dates(text, RUN).into_iter().map(|(day, _)| day).collect() };
        assert_eq!(found("on 10/17"), [d(2026, 10, 17)]);
        assert_eq!(
            found("on Oct 17, then 17 October 2027"),
            [d(2026, 10, 17), d(2027, 10, 17)]
        );
        assert_eq!(found("on Oct. 17th 2027"), [d(2027, 10, 17)]);
        assert_eq!(found("Saturday the 17th"), [d(2026, 10, 17)]);
        assert_eq!(found("on Friday the 13th"), [d(2026, 11, 13)]);
        assert_eq!(found("2031-04-02 and 2/30 and 13/1"), [d(2031, 4, 2)]);
        let (_, said) = &dates("We meet Saturday, 10/17.", RUN)[0];
        assert_eq!(said, &["10/17", "saturday"]);
    }
    #[test]
    fn values_are_kept_by_text_case_insensitively_or_by_date() {
        let c = common();
        let sentence = "Ravi and Theo swap: Ravi takes the 24th.";
        let none = Specifics::new([], RUN);
        assert_eq!(
            kept_values(sentence, &none, &c, RUN)
                .iter()
                .filter(|(_, k)| !k)
                .count(),
            3
        );
        assert_eq!(specific_verdict(sentence, &none, &c, RUN), "missing");
        let filed = Specifics::new(
            [("ravi swaps with theo; ravi takes the 24th", "2026-10-10")],
            RUN,
        );
        assert_eq!(specific_verdict(sentence, &filed, &c, RUN), "supported");
        let dated = Specifics::new([("Larkspur ferry inspection is on", "2026-10-17")], RUN);
        for sentence in [
            "We meet on Saturday, 10/17.",
            "Larkspur: Saturday the 17th.",
            "Moved to Oct 17.",
        ] {
            assert_eq!(
                specific_verdict(sentence, &dated, &c, RUN),
                "supported",
                "{sentence}"
            );
        }
        let in_text = Specifics::new([("the inspection moved to October 17", "2026-10-09")], RUN);
        assert_eq!(
            specific_verdict("Inspection on 10/17.", &in_text, &c, RUN),
            "supported"
        );
        assert_eq!(
            specific_verdict("Inspection on 10/18.", &in_text, &c, RUN),
            "missing"
        );
        assert_eq!(
            specific_verdict("No values here.", &none, &c, RUN),
            "supported"
        );
    }
    #[test]
    fn values_are_kept_only_as_whole_tokens() {
        let c = common();
        let fragments = Specifics::new(
            [("Tomorrow the news desk files on 2026-04-14", "2026-04-13")],
            RUN,
        );
        let kept = kept_values("So Morrow read the New list, item 4.", &fragments, &c, RUN);
        assert_eq!(kept, [("Morrow".into(), false), ("New".into(), false)]);
        let said = Specifics::new([("Ravi ships CTX-1 on x86-64 with O'Brien", "")], RUN);
        let only = Specifics::new([("Ravi ships CTX on x 86 with Brien", "")], RUN);
        let sentence = "So CTX-1 runs on x86-64, says O\u{2019}Brien.";
        let all = |s: &Specifics| kept_values(sentence, s, &c, RUN);
        assert!(all(&said).iter().all(|(_, k)| *k), "{:?}", all(&said));
        assert!(all(&only).iter().all(|(_, k)| !*k), "{:?}", all(&only));
        let possessive = Specifics::new([("Wenna's crew: 3,200 crowns.", "")], RUN);
        assert_eq!(
            specific_verdict("So Wenna pays 3,200.", &possessive, &c, RUN),
            "supported"
        );
        assert!(!has_token("ctx-1", "ctx") && has_token("ctx-1.", "ctx-1"));
        assert!(has_token("(tom)", "tom") && !has_token("atom", "tom"));
    }
    #[test]
    fn the_word_list_parses_and_refuses_bad_shapes() {
        let c = CommonWords::parse(r#"{"_about":"x","a":["She","I’m"]}"#).unwrap();
        assert!(c.has("she") && c.has("I'm") && !c.has("Wenna"));
        assert!(CommonWords::parse("[]").is_err());
        assert!(CommonWords::parse(r#"{"a":"She"}"#).is_err());
        let real = common();
        for word in ["May", "Saturday", "October"] {
            assert!(!real.has(word), "{word} must stay a value");
        }
    }
}
