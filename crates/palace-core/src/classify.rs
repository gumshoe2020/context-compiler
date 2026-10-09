//! `palace improve --classify` (M5c item 3), pure: which of the eight verbs a correction said in
//! conversation calls for (M5-CUTOVER-PLAN, M5c decision 3, the verbs table), from cue words,
//! with the claim's open findings breaking a tie. A suggestion for Fergus; nothing is applied.
//! Overlapping cues of different verbs resolve to the longer one ("true then, not now" is an
//! expiry, not a reversal's "…, not …"); two verbs left is `ambiguous` (every verb, in order of
//! appearance), none is `none`. It does not guess (M5c.1 item 7, M5c.2 item 5): a wrong `none`
//! costs one question, a wrong verb corrupts a claim. A `?` anywhere gives `none`. A cue counts
//! only as a word or phrase of its own (not inside `backwards-compatible`, `reversed.md`,
//! `x_confirmed`) and outside double or single quotation marks. A confirm cue counts only when
//! it opens the message (M5c.3 item 2): nothing before it but punctuation, symbols, emoji and
//! the words `yes`, `yep`, `yeah`, `yup`, `ok`, `okay`; elsewhere it is simply not a hit. A
//! negation word (`not`, any `n't` with or without its apostrophe, `never`, `no`, `nothing`,
//! `nobody`, `none`, `neither`, `nor`, `without`, `hardly`, `barely`, `far from`) or a word of
//! doubt, report or deferral (`HEARSAY`: `whether`, `if`, `before`, `said`, …) earlier in the
//! same sentence cancels a cue of any verb but confirm; a confirm cue is cancelled by a negation
//! anywhere in the message, or by a word of doubt or contrast (`DOUBT`). A message-opening
//! answer (`No,`, `Nope —`, `Not quite,`, `That's not true,`: `ANSWERS` and a comma or dash)
//! answers the claim and cancels no cue of a verb other than confirm (M5c.3 item 4); any other
//! negation after it still does (`No, don't drop it.` is `none`). A cancelled cue gives `none`. A negation inside any cue (`never
//! happened`, `, not`) negates nothing. Sarcasm, idiom and other languages are out of scope.
use std::fmt;

/// The verbs table: each verb and the cue phrases (lowercase) that call for it.
pub const CUES: [(&str, &[&str]); 8] = [
    (
        "confirm",
        &[
            "that is right",
            "that's right",
            "that is correct",
            "that's correct",
            "that is exactly right",
            "that's exactly right",
            "exactly right",
            "that is spot on",
            "that's spot on",
            "spot on",
            "confirmed",
        ],
    ),
    (
        "contradict",
        &[
            "backwards",
            "the other way around",
            "the other way round",
            "the opposite",
            "reversed",
            "wrong way round",
            ", not",
        ],
    ),
    (
        "add",
        &[
            "you missed",
            "you forgot",
            "you left out",
            "left out",
            "don't forget",
            "there is also",
            "there's also",
        ],
    ),
    (
        "weaken",
        &[
            "overstated",
            "only partly",
            "partly true",
            "partially",
            "exaggerated",
            "exaggerating",
            "exaggeration",
            "too strong",
            "not entirely",
            "not always",
        ],
    ),
    (
        "retract",
        &[
            "never happened",
            "didn't happen",
            "did not happen",
            "drop it",
            "drop that",
            "made that up",
            "made it up",
            "remove it",
            "delete it",
        ],
    ),
    (
        "split",
        &[
            "two different things",
            "different things",
            "two things",
            "not the same",
            "separate things",
            "conflated",
            "conflating",
            "conflation",
            "mixed up",
            "mixing up",
        ],
    ),
    (
        "expire",
        &[
            "true then",
            ", not now",
            "no longer",
            "not anymore",
            "not any more",
            "used to be",
            "out of date",
            "outdated",
            "not true anymore",
            "not true any more",
            "isn't true anymore",
            "isn't true any more",
        ],
    ),
    (
        "coin",
        &[
            "a word for",
            "a term for",
            "a name for",
            "need a word",
            "new word",
            "coin a",
        ],
    ),
];

/// The verb of a printed suggestion: the text before ` (cue`, so `none` and `ambiguous: …` whole
/// (how `expected/m5c/classify.json` compares a case with `expect_verb`).
pub fn verb_of(line: &str) -> &str {
    line.split(" (cue").next().unwrap_or(line)
}

/// The verb an open finding of `kind` proposes (SPEC §3): its tie-breaking weight.
pub fn verbs_for_finding(kind: &str) -> &'static [&'static str] {
    match kind {
        "reversal" => &["contradict"],
        "omission" => &["add"],
        "unsupported" => &["weaken", "retract"],
        "conflation" => &["split"],
        "stale" => &["expire"],
        "vocabulary_gap" => &["coin"],
        _ => &[],
    }
}

#[derive(Debug, PartialEq)]
pub enum Suggestion {
    Verb {
        verb: &'static str,
        cue: &'static str,
        finding: Option<String>,
    },
    /// Every verb found, in order of appearance.
    Ambiguous(Vec<&'static str>),
    None,
}

impl fmt::Display for Suggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Suggestion::Verb { verb, cue, finding } => {
                write!(f, "{verb} (cue: \"{cue}\"")?;
                match finding {
                    Some(kind) => write!(f, ", open {kind} finding)"),
                    None => write!(f, ")"),
                }
            }
            Suggestion::Ambiguous(verbs) => write!(f, "ambiguous: {}", verbs.join(" | ")),
            Suggestion::None => write!(f, "none"),
        }
    }
}

/// One cue found in the text: the verb, the cue and its byte span.
struct Hit {
    verb: &'static str,
    cue: &'static str,
    start: usize,
    end: usize,
}

/// The suggested verb for `text`, given the kinds of the claim's open findings.
pub fn classify(text: &str, open_findings: &[String]) -> Suggestion {
    let text = normalise(text);
    let hits = surviving(cue_hits(&text));
    if text.contains('?') || hits.iter().any(|h| doubted(&text, h, &hits)) {
        return Suggestion::None;
    }
    let verbs = in_order(&hits);
    let cue = |verb: &str| hits.iter().find(|h| h.verb == verb).map_or("", |h| h.cue);
    if verbs.len() < 2 {
        return verbs
            .first()
            .map_or(Suggestion::None, |verb| Suggestion::Verb {
                verb,
                cue: cue(verb),
                finding: None,
            });
    }
    match backed(&verbs, open_findings) {
        Some((verb, kind)) => Suggestion::Verb {
            verb,
            cue: cue(verb),
            finding: Some(kind),
        },
        None => Suggestion::Ambiguous(verbs),
    }
}

/// Each verb of `hits` once, in order of appearance.
fn in_order(hits: &[Hit]) -> Vec<&'static str> {
    let mut verbs: Vec<&'static str> = Vec::new();
    for hit in hits {
        if !verbs.contains(&hit.verb) {
            verbs.push(hit.verb);
        }
    }
    verbs
}

/// Single quotation marks and apostrophes (’ ‘ ʼ ＇ ‛ ‚), made `'`.
const SINGLE: [char; 6] = [
    '\u{2019}', '\u{2018}', '\u{02bc}', '\u{ff07}', '\u{201b}', '\u{201a}',
];
/// Double quotation marks (“ ” „ « » ＂ ‟ ‹ › 「 」 『 』), made `"`.
const DOUBLE: [char; 13] = [
    '\u{201c}', '\u{201d}', '\u{201e}', '\u{ab}', '\u{bb}', '\u{ff02}', '\u{201f}', '\u{2039}',
    '\u{203a}', '\u{300c}', '\u{300d}', '\u{300e}', '\u{300f}',
];
/// Question marks (？ ؟ ¿ ⸮), made `?`.
const QUESTION: [char; 4] = ['\u{ff1f}', '\u{061f}', '\u{bf}', '\u{2e2e}'];

/// Lowercase; `SINGLE` made `'`, `DOUBLE` made `"`, `QUESTION` made `?`; whitespace runs made
/// single spaces; trimmed.
fn normalise(text: &str) -> String {
    let straight = text
        .to_lowercase()
        .replace(SINGLE, "'")
        .replace(DOUBLE, "\"")
        .replace(QUESTION, "?");
    straight.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Each cue that stands as a word or phrase of its own (`bounded`) and is not quoted, in order
/// of position.
fn cue_hits(text: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (verb, cues) in CUES {
        for cue in cues {
            for (start, _) in text.match_indices(cue) {
                let end = start + cue.len();
                let placed = verb != "confirm" || opens(text, start);
                if placed && bounded(text, start, end) && !quoted(text, start) {
                    hits.push(Hit {
                        verb,
                        cue,
                        start,
                        end,
                    });
                }
            }
        }
    }
    hits.sort_by_key(|h| h.start);
    hits
}

/// Words that may come before a confirm cue that opens the message.
const AFFIRMATIONS: [&str; 6] = ["yes", "yep", "yeah", "yup", "ok", "okay"];

/// Whether byte `at` opens the message: before it only punctuation, symbols, emoji and
/// `AFFIRMATIONS` (`Yes, that's right`, `🎩yes spot on`).
fn opens(text: &str, at: usize) -> bool {
    words(&text[..at])
        .iter()
        .all(|(_, word)| AFFIRMATIONS.contains(word))
}

/// True when `text[start..end]` is not part of a larger token on either side.
fn bounded(text: &str, start: usize, end: usize) -> bool {
    let inner = &text[start..end];
    let mut before = text[..start].chars().rev();
    let mut after = text[end..].chars();
    !joined(inner.chars().next(), before.next(), before.next())
        && !joined(inner.chars().next_back(), after.next(), after.next())
}

/// Whether a cue's end character `edge` is joined outward to `beside` (then `beyond`): a letter
/// or digit touching another, or touching one `-`, `_`, `/` or `.` that touches one
/// (`backwards-compatible`, `reversed.md`, `x.confirmed`).
fn joined(edge: Option<char>, beside: Option<char>, beyond: Option<char>) -> bool {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let linker = matches!(beside, Some('-' | '_' | '/' | '.'));
    word(edge) && (word(beside) || (linker && word(beyond)))
}

/// Whether byte `at` lies inside double or inside single quotation marks: an odd number of
/// either before it. A `'` between two letters or digits is an apostrophe, not a mark.
fn quoted(text: &str, at: usize) -> bool {
    let chars: Vec<char> = text[..at].chars().collect();
    let next = text[at..].chars().next();
    let mut marks = (0, 0);
    for (i, &c) in chars.iter().enumerate() {
        let after = chars.get(i + 1).copied().or(next);
        let apostrophe =
            i > 0 && chars[i - 1].is_alphanumeric() && after.is_some_and(char::is_alphanumeric);
        match c {
            '"' => marks.0 += 1,
            '\'' if !apostrophe => marks.1 += 1,
            _ => {}
        }
    }
    marks.0 % 2 == 1 || marks.1 % 2 == 1
}

/// A cue its message casts doubt on. A confirm cue: any negation, doubt or contrast word in the
/// whole message. Any other cue: a negation or `HEARSAY` word earlier in its sentence, outside
/// every cue (a negation that is part of a cue, as in `never happened` or `, not`, negates
/// nothing), and outside a message-opening answer (`answered`).
fn doubted(text: &str, hit: &Hit, hits: &[Hit]) -> bool {
    if hit.verb == "confirm" {
        let doubt = words(text).iter().any(|(_, word)| DOUBT.contains(word));
        return doubt || !negations(text).is_empty();
    }
    let answer = answered(text);
    let exempt =
        |(s, e): (usize, usize)| e <= answer || hits.iter().any(|o| o.start <= s && e <= o.end);
    cancellers(text).into_iter().any(|span| {
        sentence_start(text, hit.start) <= span.0 && span.1 <= hit.start && !exempt(span)
    })
}

/// Openers that answer the claim when a comma or a dash follows them (M5c.3 item 4).
const ANSWERS: [&str; 7] = [
    "no",
    "nope",
    "not quite",
    "that's not true",
    "that's not right",
    "that is not true",
    "that is not right",
];

/// The byte where a message-opening answer ends (`No,` → 2), after leading punctuation; 0 when
/// the message does not open with one. A hyphen counts as a dash only before a space (`No-one`).
fn answered(text: &str) -> usize {
    let lead = text.len()
        - text
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .len();
    let answer = |opener: &&str| {
        let rest = text[lead..].strip_prefix(*opener)?.trim_start();
        let dash = rest.starts_with(['\u{2013}', '\u{2014}']) || rest.starts_with("- ");
        (rest.starts_with(',') || dash).then_some(lead + opener.len())
    };
    ANSWERS.iter().find_map(answer).unwrap_or(0)
}

/// Words of doubt, condition, exception or contrast that cancel a confirm cue anywhere in the
/// message (M5c.3 item 2).
const DOUBT: [&str; 14] = [
    "doubt", "maybe", "perhaps", "if", "whether", "unsure", "wonder", "except", "but", "however",
    "though", "although", "wrong", "mistaken",
];

/// Words of doubt, report or deferral that cancel a cue of a verb other than confirm later in
/// the same sentence (M5c.3 item 3: `Stop saying it never happened.`, `Hold off before you
/// delete it.`).
const HEARSAY: [&str; 6] = ["whether", "if", "before", "said", "saying", "say"];

/// The spans that cancel a cue of a verb other than confirm: the negations and `HEARSAY`.
fn cancellers(text: &str) -> Vec<(usize, usize)> {
    let hearsay = words(text)
        .into_iter()
        .filter(|(_, word)| HEARSAY.contains(word))
        .map(|(start, word)| (start, start + word.len()));
    let mut spans = negations(text);
    spans.extend(hearsay);
    spans
}

/// The byte where the sentence holding byte `at` begins: after the last `.`, `!`, `?` or `;`
/// before it that is followed by a space.
fn sentence_start(text: &str, at: usize) -> usize {
    let head = &text[..at];
    head.char_indices()
        .rev()
        .find(|&(i, c)| matches!(c, '.' | '!' | '?' | ';') && head[i + 1..].starts_with(' '))
        .map_or(0, |(i, _)| i + 2)
}

/// The byte spans of the negation words: `NEGATIONS`, `far from`, and every `n't` form, with an
/// apostrophe or without one (`isnt`, `dont`, …).
fn negations(text: &str) -> Vec<(usize, usize)> {
    let words = words(text);
    let mut spans = Vec::new();
    for (i, &(start, word)) in words.iter().enumerate() {
        let next = words.get(i + 1).map(|&(s, w)| (s + w.len(), w));
        match next {
            Some((end, "from")) if word == "far" => spans.push((start, end)),
            _ if negation(word) => spans.push((start, start + word.len())),
            _ => {}
        }
    }
    spans
}

/// The words of `text` (letters, digits and inner apostrophes), each with its byte offset.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        let part = c.is_alphanumeric() || (c == '\'' && start.is_some());
        match (part, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                words.push((s, text[s..i].trim_end_matches('\'')));
                start = None;
            }
            _ => {}
        }
    }
    words
}

/// Apostrophe-less `n't` forms, as typed on a phone.
const BARE_NT: [&str; 18] = [
    "isnt", "arent", "wasnt", "werent", "dont", "doesnt", "didnt", "wont", "wouldnt", "cant",
    "couldnt", "shouldnt", "aint", "hasnt", "havent", "hadnt", "mustnt", "neednt",
];

/// Negation words besides the `n't` forms (M5c.3 item 3 added `nobody` to `barely`).
const NEGATIONS: [&str; 12] = [
    "not", "never", "no", "nothing", "hardly", "cannot", "nobody", "none", "neither", "nor",
    "without", "barely",
];

fn negation(word: &str) -> bool {
    NEGATIONS.contains(&word) || word.ends_with("n't") || BARE_NT.contains(&word)
}

/// Hits not overlapped by a longer hit of another verb.
fn surviving(hits: Vec<Hit>) -> Vec<Hit> {
    let beaten = |h: &Hit| {
        hits.iter().any(|o| {
            o.verb != h.verb
                && o.start < h.end
                && h.start < o.end
                && o.end - o.start > h.end - h.start
        })
    };
    let keep: Vec<bool> = hits.iter().map(|h| !beaten(h)).collect();
    hits.into_iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(h, _)| h)
        .collect()
}

/// The one verb among `verbs` an open finding proposes, with that finding's kind.
fn backed(verbs: &[&'static str], open_findings: &[String]) -> Option<(&'static str, String)> {
    let backed: Vec<(&'static str, &String)> = verbs
        .iter()
        .filter_map(|v| {
            let kind = open_findings
                .iter()
                .find(|k| verbs_for_finding(k).contains(v))?;
            Some((*v, kind))
        })
        .collect();
    match backed.as_slice() {
        [(verb, kind)] => Some((verb, (*kind).clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan's table: one phrasing per verb.
    pub const TABLE: [(&str, &str); 8] = [
        ("Yes, that is right.", "confirm"),
        ("That is backwards, it was X, not Y.", "contradict"),
        ("You missed that the 09-24 entry also chose Valkey.", "add"),
        ("That is overstated, only partly true.", "weaken"),
        ("That never happened, drop it.", "retract"),
        ("Those are two different things.", "split"),
        ("That was true then, not now.", "expire"),
        ("We need a word for this.", "coin"),
    ];

    #[test]
    fn each_phrasing_of_the_table_names_its_verb() {
        for (text, verb) in TABLE {
            match classify(text, &[]) {
                Suggestion::Verb { verb: got, .. } => assert_eq!(got, verb, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
        assert_eq!(
            classify("That was true then, not now.", &[]).to_string(),
            "expire (cue: \"true then\")"
        );
    }

    #[test]
    fn two_verbs_are_ambiguous_unless_an_open_finding_backs_one() {
        let text = "That is backwards, and anyway it never happened.";
        assert_eq!(
            classify(text, &[]).to_string(),
            "ambiguous: contradict | retract"
        );
        assert_eq!(
            classify(text, &["reversal".into()]).to_string(),
            "contradict (cue: \"backwards\", open reversal finding)"
        );
        assert_eq!(
            classify("Hmm, interesting.", &["reversal".into()]),
            Suggestion::None
        );
    }

    #[test]
    fn it_does_not_guess() {
        let none = |text: &str| classify(text, &["reversal".into()]).to_string();
        for text in [
            "Not exactly right.",
            "I wouldn't say that is right.",
            "That's unconfirmed.",
            "He said \u{201c}that never happened\u{201d} but it did.",
            "The team is made up of three people.",
            "That claim was reversed?",
            "Not now, I'm busy.",
            "That is not overstated.",
        ] {
            assert_eq!(none(text), "none", "{text}");
        }
        assert_eq!(
            none("It's Valkey, not Redis."),
            "contradict (cue: \", not\")"
        );
        assert_eq!(
            none("Two things: yes, that is right, and you missed the Vault part."),
            "ambiguous: split | add"
        );
        assert_eq!(
            none("That never happened, drop it."),
            "retract (cue: \"never happened\")"
        );
    }

    #[test]
    fn no_text_makes_it_panic() {
        for text in [
            "«Valkey» is spot on",
            "It isn＇t exactly right.",
            "Hmm —honestly, spot on.",
            "🎩yes spot on",
        ] {
            let _ = classify(text, &["reversal".into()]);
        }
        let pieces = [
            "a", "é", "🎩", "—", "–", "«", "»", "\"", "“", "”", "'", "’", "ʼ", "＇", " ", ",", ".",
            "?", "-", "not ", "n't", "e\u{301}", "日本",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) as usize % n
        };
        let cues = CUES.iter().flat_map(|(_, cues)| cues.iter());
        for cue in cues {
            for _ in 0..8 {
                let before: String = (0..next(4)).map(|_| pieces[next(pieces.len())]).collect();
                let after: String = (0..next(4)).map(|_| pieces[next(pieces.len())]).collect();
                let _ = classify(&format!("{before}{cue}{after}"), &[]);
                let _ = classify(&format!("{before}{}{after}", cue.to_uppercase()), &[]);
            }
        }
    }

    #[test]
    fn the_second_rounds_rules() {
        let got = |text: &str| classify(text, &[]).to_string();
        for text in [
            "Keep x/backwards and backwards_compat.",
            "Load file.reversed first.",
            "It is x.confirmed there.",
            "That is right. It never happened before.",
            "Far from it, that is backwards.",
            "Is it spot on or not",
            "Nothing changed, they conflated nothing.",
            "Is it backwards? I think so.",
        ] {
            assert_eq!(got(text), "none", "{text}");
        }
        assert_eq!(
            got("Nothing changed. That is backwards."),
            "contradict (cue: \"backwards\")"
        );
        assert_eq!(
            got("It is backwards. Nothing else."),
            "contradict (cue: \"backwards\")"
        );
        assert_eq!(
            got("Yes, that is right; the log says 'confirmed' too."),
            "confirm (cue: \"that is right\")"
        );
        assert_eq!(
            got("That’s spot on, Callum’s note too."),
            "confirm (cue: \"that's spot on\")"
        );
        assert_eq!(
            got("Backwards-compatible, but backwards."),
            "contradict (cue: \"backwards\")"
        );
    }

    #[test]
    fn confirm_opens_the_message() {
        let got = |text: &str| classify(text, &[]).to_string();
        for (text, cue) in [
            ("Yes, that is right.", "that is right"),
            ("Yeah — spot on.", "spot on"),
            ("ok ok, that's right", "that's right"),
            ("That's exactly right.", "that's exactly right"),
        ] {
            assert_eq!(got(text), format!("confirm (cue: \"{cue}\")"), "{text}");
        }
        for text in [
            "Valkey is spot on.",
            "So yes, that's right.",
            "Yes, that's right, but the date is off.",
            "Spot on, though I wonder.",
            "That is correct, I was mistaken.",
        ] {
            assert_eq!(got(text), "none", "{text}");
        }
        assert_eq!(
            got("I confirmed it: you forgot the backup."),
            "add (cue: \"you forgot\")"
        );
    }

    #[test]
    fn other_question_and_quotation_marks() {
        let got = |text: &str| classify(text, &[]).to_string();
        for text in [
            "That's right\u{ff1f}",
            "That's right\u{061f}",
            "\u{bf}That's right",
            "Spot on\u{2e2e}",
            "He wrote \u{ff02}spot on\u{ff02} in the draft",
            "He wrote \u{201f}drop it\u{201d} in the draft",
            "He wrote \u{2039}drop it\u{203a} in the draft",
            "He wrote \u{300c}drop it\u{300d} in the draft",
            "He wrote \u{300e}drop it\u{300f} in the draft",
            "He wrote \u{201b}drop it\u{2019} in the draft",
            "He wrote \u{201a}drop it\u{2019} in the draft",
        ] {
            assert_eq!(got(text), "none", "{text}");
        }
    }

    #[test]
    fn an_opening_no_answers_the_claim() {
        let got = |text: &str| classify(text, &[]).to_string();
        for (text, verb) in [
            ("No, it's backwards.", "contradict"),
            ("Nope \u{2014} you missed the Vault part.", "add"),
            (
                "That is not right, those are two different things.",
                "split",
            ),
            ("\u{1f3a9}No - that never happened.", "retract"),
            ("It isn't true anymore.", "expire"),
        ] {
            assert_eq!(verb_of(&got(text)), verb, "{text}");
        }
        for text in [
            "No, don't drop it.",
            "No-one mixed up anything.",
            "No backwards logic here.",
            "Not quite right, it's backwards.",
            "So no, it's backwards.",
        ] {
            assert_eq!(got(text), "none", "{text}");
        }
    }

    #[test]
    fn negation_and_hearsay_cancel_other_verbs() {
        let got = |text: &str| classify(text, &[]).to_string();
        for text in [
            "Nobody mixed up anything.",
            "Without being overstated, it's fine.",
            "Barely overstated, keep it.",
            "Stop saying it never happened.",
            "If it is backwards, say so.",
            "Hold off before you delete it.",
            "You said it was backwards; it's fine as it is.",
            "None of it is outdated.",
        ] {
            assert_eq!(got(text), "none", "{text}");
        }
        assert_eq!(
            got("You said so. It is backwards."),
            "contradict (cue: \"backwards\")"
        );
        assert_eq!(
            got("Delete it before Friday."),
            "retract (cue: \"delete it\")"
        );
    }
}
