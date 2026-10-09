use crate::{
    canary::Question,
    ids,
    rollup::Entry,
    traits::{DiaryReader, Judge},
};
use serde_json::{Value, json};
#[derive(Clone, Debug, Default)]
pub struct Claim {
    pub number: String,
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub statement: String,
    pub date: String,
    pub source: String,
    pub quote: Option<String>,
    /// The claim that replaces this one, from `(superseded by Cn)` or `(corrected by Cn)`
    /// (empty for a bare `(superseded)`).
    pub superseded_by: Option<String>,
    /// `(corrected by Cn)`: this claim was WRONG when written (a reversal), not merely replaced.
    pub corrected: bool,
}
/// A `## Claims` line that could not be parsed; reported as a finding, never a run failure.
#[derive(Clone, Debug)]
pub struct Malformed {
    pub line: usize,
    pub raw: String,
    pub reason: String,
}
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub claims: Vec<Claim>,
    pub malformed: Vec<Malformed>,
}
pub fn parse(text: &str) -> Parsed {
    let mut active = false;
    let mut parsed = Parsed::default();
    for (index, line) in text.lines().enumerate() {
        if line.starts_with("## ") {
            active = is_claims_heading(line);
            continue;
        }
        if !active || line.trim().is_empty() {
            continue;
        }
        let result = parse_line(line.trim()).and_then(|claim| {
            if parsed.claims.iter().any(|c| c.number == claim.number) {
                Err(format!("duplicate claim {}", claim.number))
            } else {
                Ok(claim)
            }
        });
        match result {
            Ok(claim) => parsed.claims.push(claim),
            Err(reason) => parsed.malformed.push(Malformed {
                line: index + 1,
                raw: line.to_owned(),
                reason,
            }),
        }
    }
    parsed
}
/// `## Claims`, optionally followed by a parenthesised count such as `## Claims (12)`.
pub fn is_claims_heading(line: &str) -> bool {
    line.trim()
        .strip_prefix("## Claims")
        .is_some_and(|rest| rest.is_empty() || (rest.starts_with(" (") && rest.ends_with(')')))
}
pub fn malformed_rows(entry: &Entry, malformed: &[Malformed]) -> Vec<Value> {
    malformed
        .iter()
        .map(|m| {
            json!({"check":"claim","rollup":entry.name,"status":"malformed","line":m.line,
                "finding":{"kind":"malformed_claim","raw":m.raw,"reason":m.reason},
                "evidence":{"source":entry.name,"passage":m.raw,"locator":format!("L{}-L{}", m.line, m.line)},
                "proposal":crate::finding::proposal("malformed_claim", &Claim::default(), &[]),
                "checker":"rule:grammar"})
        })
        .collect()
}
/// Pinned grammar (M4b P2):
/// `- C<n>. [(superseded by Cm) |(corrected by Cm) ]<subject> | <predicate> | <object> — <YYYY-MM-DD> — <source> [— quote: "…"]`.
/// Trailing fields split from the right on ` — `, ` – ` or ` - `; quotes may be straight or curly.
fn parse_line(line: &str) -> Result<Claim, String> {
    let bad = |why: &str| format!("malformed claim ({why}): {line}");
    let (number, rest) = line
        .strip_prefix("- ")
        .and_then(|s| s.split_once(". "))
        .ok_or_else(|| bad("expected `- C<n>. `"))?;
    if !is_claim_number(number) {
        return Err(bad("claim number must be C<n>"));
    }
    let (superseded_by, corrected, rest) = marker(rest).map_err(&bad)?;
    let (rest, quote) = split_quote(rest).map_err(&bad)?;
    let (rest, source) = split_last_field(rest).ok_or_else(|| bad("missing source"))?;
    let (statement, date) = split_last_field(rest).ok_or_else(|| bad("missing date"))?;
    let fields = statement_fields(statement).map_err(&bad)?;
    let predicate = active_predicate(number, fields[1])?;
    validate_date(date).map_err(&bad)?;
    validate_source(source).map_err(&bad)?;
    check_quote(number, quote.as_deref())?;
    Ok(Claim {
        quote,
        superseded_by,
        corrected,
        ..assemble(number, &fields, predicate, (date, source))
    })
}
fn active_predicate(number: &str, predicate: &str) -> Result<String, String> {
    let predicate = normalise_predicate(predicate);
    if is_passive(&predicate) {
        return Err(format!("claim {number}: passive predicate is forbidden"));
    }
    Ok(predicate)
}
fn check_quote(number: &str, quote: Option<&str>) -> Result<(), String> {
    if quote.is_some_and(|s| s.split_whitespace().count() > 20) {
        return Err(format!("claim {number}: quote exceeds 20 words"));
    }
    Ok(())
}
/// The claim of checked fields (no quote, no marker yet).
fn assemble(
    number: &str,
    fields: &[&str],
    predicate: String,
    (date, source): (&str, &str),
) -> Claim {
    let (subject, object) = (fields[0].to_owned(), fields[2].to_owned());
    Claim {
        number: number.into(),
        id: claim_id(&subject, &predicate, &object, date, source),
        statement: format!("{subject} {} {object}", predicate.replace('_', " ")),
        subject,
        predicate,
        object,
        date: date.into(),
        source: source.into(),
        quote: None,
        superseded_by: None,
        corrected: false,
    }
}
/// `subject | predicate | object`, each non-empty, the subject free of stray markers.
fn statement_fields(statement: &str) -> Result<Vec<&str>, &'static str> {
    let fields: Vec<_> = statement.split(" | ").map(str::trim).collect();
    if fields.len() != 3 || fields.iter().any(|f| f.is_empty()) {
        return Err("expected `subject | predicate | object`");
    }
    if stray_marker(fields[0]) {
        return Err(STRAY);
    }
    Ok(fields)
}
/// `c_` + sha256(lp(subject, predicate, object, date, source))[:12] over normalised fields.
pub fn claim_id(subject: &str, predicate: &str, object: &str, date: &str, source: &str) -> String {
    let fields = [
        normalise_entity(subject),
        normalise_predicate(predicate),
        normalise_entity(object),
        date.trim().to_owned(),
        source.trim().to_owned(),
    ];
    let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
    format!(
        "c_{}",
        &ids::sha256(ids::length_prefixed(&refs).as_bytes())[..12]
    )
}
pub fn normalise_entity(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
pub fn normalise_predicate(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase()
}
/// Passive voice (M4b review L3, M4d-2a item 8): an auxiliary first and a `by` anywhere, or a
/// past participle followed by `by` anywhere in the predicate (`corrected by callum`,
/// `corrected-by`, `approved_by_callum`); words split on `_` and `-`. Allowed: the relation
/// names in `ALLOWED_BY` and active phrasal verbs (`stands by`, `went by`, `stopped by`).
fn is_passive(predicate: &str) -> bool {
    const ALLOWED_BY: &[&str] = &["bounded_by"];
    let words: Vec<&str> = predicate
        .split(['_', '-'])
        .filter(|w| !w.is_empty())
        .collect();
    if ALLOWED_BY.contains(&words.join("_").as_str()) {
        return false;
    }
    let auxiliary = matches!(
        words.first(),
        Some(&("was" | "were" | "been" | "being" | "is" | "are" | "got" | "gets" | "get"))
    ) && words.contains(&"by");
    auxiliary
        || words
            .windows(2)
            .any(|w| w[1] == "by" && is_participle(w[0]))
}
/// Irregular past participles (`<word> by` is passive).
const IRREGULAR: &[&str] = &[
    "written",
    "taken",
    "given",
    "made",
    "done",
    "known",
    "seen",
    "held",
    "led",
    "built",
    "sent",
    "told",
    "paid",
    "driven",
    "chosen",
    "spoken",
    "broken",
    "shown",
    "drawn",
    "beaten",
    "bought",
    "caught",
    "taught",
    "brought",
    "found",
    "kept",
    "lost",
    "sold",
    "won",
    "eaten",
    "ridden",
    "run",
    "hit",
    "set",
    "put",
    "read",
    "overseen",
    "said",
    "heard",
    "meant",
    "understood",
    "forgotten",
    "begun",
    "fed",
    "met",
    "thought",
    "struck",
    "stolen",
    "sworn",
    "torn",
    "worn",
    "withdrawn",
    "undertaken",
    "overtaken",
    "mistaken",
    "forbidden",
    "hidden",
    "bitten",
    "blown",
    "flown",
    "grown",
    "thrown",
    "shaken",
    "hung",
    "dug",
    "upheld",
    "withheld",
    "misled",
    "cut",
    "shut",
    "spent",
    "lent",
    "dealt",
    "sought",
    "fought",
];
/// Past tenses in `-ed` that are active phrasal verbs before `by`.
const ACTIVE_PHRASAL: &[&str] = &["stopped", "dropped", "passed", "popped", "abided", "lived"];
/// A past participle that makes `<word> by` passive (not an active phrasal verb). Without a
/// following `by` none of these is passive: `set`, `hit`, `led`, `built`, `paid`, `bought`,
/// `held` and `kept` stay active past tense (M4d-2a.1 item 5, review L-1).
fn is_participle(word: &str) -> bool {
    (word.ends_with("ed") && !ACTIVE_PHRASAL.contains(&word)) || IRREGULAR.contains(&word)
}
const STRAY: &str = "a (corrected by …) or (superseded by …) outside the single leading marker";
/// A bracketed `corrected by` / `correction by` / `superseded by` anywhere in `text` — any
/// bracket (full-width and square included), any spacing, `-`/`_`/`:` joiners — which only the
/// single leading marker may carry (M4d-2a.1 item 5, review L-1): otherwise a second marker,
/// an indented one or a misspelt one would silently become subject text and escape pairing.
fn stray_marker(text: &str) -> bool {
    const OPEN: [char; 5] = ['(', '[', '{', '（', '［'];
    const MARKERS: [&str; 4] = [
        "correctedby",
        "correctionby",
        "supersededby",
        "supersessionby",
    ];
    let lower = text.to_lowercase();
    lower.match_indices(OPEN).any(|(i, bracket)| {
        let squeezed: String = lower[i + bracket.len()..]
            .chars()
            .filter(|c| !(c.is_whitespace() || "-_:".contains(*c)))
            .take(16)
            .collect();
        MARKERS.iter().any(|m| squeezed.starts_with(m))
    })
}
const MARKER: &str =
    "a leading (corrected… or (superseded… must be exactly (corrected by Cn) or (superseded by Cn)";
/// `(superseded by Cn) ` or `(corrected by Cn) ` → (target, corrected, rest). Any other leading
/// parenthetical that begins `(corrected` or `(superseded` is malformed (M4d-2a item 8, L-5), so
/// a reversal marker is never silently absorbed into the subject.
fn marker(rest: &str) -> Result<(Option<String>, bool, &str), &'static str> {
    let head = rest.to_lowercase();
    if !(head.starts_with("(corrected") || head.starts_with("(superseded")) {
        return Ok((None, false, rest));
    }
    let (inside, tail) = rest
        .strip_prefix('(')
        .and_then(|marker| marker.split_once(") "))
        .ok_or(MARKER)?;
    let lower = inside.to_lowercase();
    let (target, corrected) = match (
        lower.strip_prefix("superseded by "),
        lower.strip_prefix("corrected by "),
    ) {
        (Some(target), _) => (target, false),
        (_, Some(target)) => (target, true),
        _ => return Err(MARKER),
    };
    if !is_claim_number(&target.to_uppercase()) {
        return Err("marker must name exactly one claim, e.g. (superseded by C7)");
    }
    Ok((Some(target.to_uppercase()), corrected, tail))
}
/// `C<digits>`.
pub fn is_claim_number(text: &str) -> bool {
    text.len() >= 2 && text.starts_with('C') && text[1..].bytes().all(|b| b.is_ascii_digit())
}
const SEPARATORS: [&str; 3] = [" — ", " – ", " - "];
fn split_last_field(text: &str) -> Option<(&str, &str)> {
    SEPARATORS
        .iter()
        .filter_map(|sep| text.rfind(sep).map(|i| (i, sep.len())))
        .max_by_key(|(i, _)| *i)
        .map(|(i, len)| (text[..i].trim_end(), text[i + len..].trim()))
}
fn split_quote(text: &str) -> Result<(&str, Option<String>), &'static str> {
    let found = SEPARATORS
        .iter()
        .filter_map(|sep| {
            text.rfind(&format!("{sep}quote:"))
                .map(|i| (i, sep.len() + "quote:".len()))
        })
        .max_by_key(|(i, _)| *i);
    let Some((i, len)) = found else {
        return Ok((text, None));
    };
    let raw = text[i + len..].trim();
    let quote = [('"', '"'), ('“', '”')]
        .iter()
        .find_map(|(open, close)| {
            raw.strip_prefix(*open)
                .and_then(|s| s.strip_suffix(*close))
                .filter(|s| !s.is_empty())
        })
        .ok_or("quote must be wrapped in quotation marks")?;
    Ok((&text[..i], Some(quote.to_owned())))
}
fn validate_date(date: &str) -> Result<(), &'static str> {
    let shaped = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if shaped {
        Ok(())
    } else {
        Err("date must be YYYY-MM-DD")
    }
}
fn validate_source(source: &str) -> Result<(), &'static str> {
    if !source.ends_with(".md") || source.contains(['/', '\\']) || source.starts_with('.') {
        return Err("source must be a diary filename");
    }
    Ok(())
}
pub fn confidence(base: f64, survived: usize, findings: usize) -> f64 {
    ((base + survived as f64 * 0.05 - findings as f64 * 0.15).clamp(0.05, 0.99) * 100.0).round()
        / 100.0
}
/// Object anchors: the whole object and each capitalised name (joined with a following capitalised
/// or numeric word), lowercased; anchors that are only a leading article/pronoun or one letter are
/// dropped (M4b.2 N-1, closing M4a L3's single-capital anchors).
pub(crate) fn anchors(object: &str) -> Vec<String> {
    let mut result = anchors_raw(object);
    result.retain(|a| crate::quote_relevance::anchor_words(a).is_some());
    result
}
fn anchors_raw(object: &str) -> Vec<String> {
    let mut result = vec![object.trim_matches('"').to_lowercase()];
    let words: Vec<_> = object.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        let w = w.trim_matches(['"', '\'', '.', ',']);
        if w.chars().next().is_some_and(char::is_uppercase) {
            let mut anchor = w.to_owned();
            if let Some(next) = words.get(i + 1).filter(|n| {
                n.chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
            }) {
                anchor.push(' ');
                anchor.push_str(next);
            }
            result.push(anchor.trim_end_matches("'s").to_lowercase());
        }
    }
    result
}
fn entities_near(claim: &Claim, source: &str) -> bool {
    let text = source.to_lowercase();
    let subject = claim.subject.to_lowercase();
    let objects = anchors(&claim.object);
    text.match_indices(&subject).any(|(s, _)| {
        objects.iter().any(|object| {
            text.match_indices(object).any(|(o, _)| {
                let start = s.min(o);
                let end = (s + subject.len()).max(o + object.len());
                text[start..end].chars().count() <= 600
            })
        })
    })
}
/// M4d-2b grounding: whether `text` (a candidate source) carries the claim — its quote verbatim
/// and relevant, or, without a quote, its subject and object within 600 characters.
pub fn locates(c: &Claim, text: &str) -> bool {
    match c.quote.as_deref().filter(|q| !q.is_empty()) {
        Some(quote) => text.contains(quote) && quote_supports(c, quote, text),
        None => entities_near(c, text),
    }
}
/// The `unsupported` test on a source read now (the auto loop's second check): an irrelevant
/// verbatim quote, or neither a supporting quote nor the entities near each other.
pub fn unsupported_by(c: &Claim, text: &str) -> bool {
    let grounding = Grounding::of(c, Some(text));
    grounding == Grounding::IrrelevantQuote
        || (grounding != Grounding::Quoted && !entities_near(c, text))
}
/// Loose: the subject and one object anchor anywhere in `text` (who the judge is asked about).
pub fn mentions(c: &Claim, text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains(&c.subject.to_lowercase())
        && anchors(&c.object)
            .iter()
            .any(|a| lower.contains(a.as_str()))
}
pub fn check<D: DiaryReader, J: Judge + ?Sized>(
    entry: &Entry,
    claims: &[Claim],
    diary: &D,
    judge: Option<&J>,
) -> Vec<Value>
where
    D::Error: std::fmt::Display,
    J::Error: std::fmt::Display,
{
    check_against(entry, claims, claims, diary, judge)
}
pub fn check_against<D: DiaryReader, J: Judge + ?Sized>(
    entry: &Entry,
    claims: &[Claim],
    context: &[Claim],
    diary: &D,
    judge: Option<&J>,
) -> Vec<Value>
where
    D::Error: std::fmt::Display,
    J::Error: std::fmt::Display,
{
    claims
        .iter()
        .map(|claim| check_one(entry, claim, context, diary, judge))
        .collect()
}
fn check_one<D: DiaryReader, J: Judge + ?Sized>(
    entry: &Entry,
    c: &Claim,
    claims: &[Claim],
    diary: &D,
    judge: Option<&J>,
) -> Value
where
    D::Error: std::fmt::Display,
    J::Error: std::fmt::Display,
{
    let mut row = json!({"check":"claim","rollup":entry.name,"claim":c.number,"claim_id":c.id,"source":c.source,"status":"ok","reversal":"unchecked"});
    let source = diary.read(&c.source);
    let text = match &source {
        Ok(text) => text.as_deref(),
        Err(error) => {
            row["reason"] = json!(error.to_string());
            None
        }
    };
    if let Some(text) = text {
        row["source_sha256"] = json!(crate::ids::sha256(text.as_bytes()));
    }
    let grounding = Grounding::of(c, text);
    let (status, survived) = match (&source, text) {
        _ if c.superseded_by.is_some() => (route_pair(entry, c, claims, text, judge, &mut row), 0),
        (Err(_), _) => ("error", 0),
        (Ok(_), None) => ("absent-source", 0),
        (Ok(_), Some(text)) => status(entry, c, claims, text, grounding, judge, &mut row),
    };
    let base = base_confidence(grounding);
    let findings = usize::from(matches!(
        status,
        "unsupported" | "stale" | "absent-source" | "reversal"
    ));
    row["status"] = json!(status);
    row["confidence_base"] = json!(base);
    row["confidence"] = json!(confidence(base, survived, findings));
    if let Some(kind) = crate::finding::kind_of(status) {
        describe_finding(&mut row, kind, c, claims, text);
    }
    row
}
/// SPEC §2.2 (M4d item 1): the finding's source passage + locator, proposal and checker. A
/// reversal's checker is `judge`; the store names the model (`judge:<model>`).
fn describe_finding(row: &mut Value, kind: &str, c: &Claim, claims: &[Claim], text: Option<&str>) {
    let passage = text.map(|t| crate::finding::passage(t, c));
    row["evidence"] = json!({
        "source": c.source,
        "passage": passage.as_ref().map_or("", |p| p.text.as_str()),
        "locator": passage.map(|p| p.locator),
    });
    row["proposal"] = crate::finding::proposal(kind, c, claims);
    row["checker"] = json!(match row["status"].as_str() {
        Some("reversal") => "judge",
        Some("stale") => "rule:stale",
        Some("absent-source") => "rule:source-absent",
        _ => "rule:claims-check",
    });
}
/// How the source grounds a claim, before any judge is asked.
#[derive(Clone, Copy, PartialEq)]
enum Grounding {
    /// The source is itself a rollup (legacy only).
    Legacy,
    /// A verbatim quote that supports the claim (M4b.1 S-2).
    Quoted,
    /// A verbatim quote that says nothing about the claim.
    IrrelevantQuote,
    /// No verbatim quote.
    Unquoted,
}
impl Grounding {
    fn of(c: &Claim, text: Option<&str>) -> Self {
        let Some(text) = text else {
            return Self::Unquoted;
        };
        if Entry::parse(&c.source, text.into()).is_rollup {
            return Self::Legacy;
        }
        match c
            .quote
            .as_deref()
            .filter(|q| !q.is_empty() && text.contains(q))
        {
            None => Self::Unquoted,
            Some(quote) if quote_supports(c, quote, text) => Self::Quoted,
            Some(_) => Self::IrrelevantQuote,
        }
    }
}
fn base_confidence(grounding: Grounding) -> f64 {
    match grounding {
        Grounding::Legacy => 0.30,
        Grounding::Quoted => 0.80,
        Grounding::IrrelevantQuote | Grounding::Unquoted => 0.60,
    }
}
/// Status of a live claim whose source was read: stale, unsupported, or the judge's verdict.
fn status<J: Judge + ?Sized>(
    entry: &Entry,
    c: &Claim,
    claims: &[Claim],
    text: &str,
    grounding: Grounding,
    judge: Option<&J>,
    row: &mut Value,
) -> (&'static str, usize)
where
    J::Error: std::fmt::Display,
{
    let quoted = grounding == Grounding::Quoted;
    if is_stale(c, claims) {
        ("stale", 0)
    } else if grounding == Grounding::IrrelevantQuote {
        ("unsupported", 0)
    } else if !entities_near(c, text) {
        // P3: only a passage carrying both subject and object can be judged for direction.
        (if quoted { "ok" } else { "unsupported" }, 0)
    } else if let Some(judge) = judge {
        judged(entry, c, text, judge, row)
    } else {
        (if quoted { "ok" } else { "unchecked" }, 0)
    }
}
/// M4d item 9: a marked claim is not live and raises no finding. Its pair is routed: a
/// `(corrected by Cn)` pair to the judge as a reversal candidate (the verdict is recorded, the
/// correction already resolved it), a `(superseded by Cn)` pair to the stale test.
fn route_pair<J: Judge + ?Sized>(
    entry: &Entry,
    c: &Claim,
    claims: &[Claim],
    text: Option<&str>,
    judge: Option<&J>,
    row: &mut Value,
) -> &'static str
where
    J::Error: std::fmt::Display,
{
    let by = c.superseded_by.clone().unwrap_or_default();
    if !c.corrected {
        let stale = claims
            .iter()
            .find(|n| n.number == by)
            .is_some_and(|n| replaces(c, n));
        row["pair"] = json!({"kind":"stale","by":by,"stale":stale});
        return "superseded";
    }
    row["pair"] = json!({"kind":"reversal","by":by});
    if let (Some(text), Some(judge)) = (text, judge)
        && entities_near(c, text)
    {
        judged(entry, c, text, judge, row);
    }
    "corrected"
}
/// `new` says the same subject and predicate as `old` with a different object.
fn replaces(old: &Claim, new: &Claim) -> bool {
    normalise_entity(&new.subject) == normalise_entity(&old.subject)
        && new.predicate == old.predicate
        && normalise_entity(&new.object) != normalise_entity(&old.object)
}
/// A later live claim with the same subject and predicate and a different object.
fn is_stale(c: &Claim, claims: &[Claim]) -> bool {
    claims
        .iter()
        .any(|later| later.superseded_by.is_none() && replaces(c, later) && later.date > c.date)
}
fn judged<J: Judge + ?Sized>(
    entry: &Entry,
    c: &Claim,
    text: &str,
    judge: &J,
    row: &mut Value,
) -> (&'static str, usize)
where
    J::Error: std::fmt::Display,
{
    let question = Question {
        rollup: entry.name.clone(),
        id: c.number.clone(),
        statement: c.statement.clone(),
        passage: text.chars().take(12000).collect(),
    };
    match judge.judge(&question) {
        Ok(verdict) => {
            row["reversal"] = verdict["verdict"].clone();
            row["quote"] = verdict["quote"].clone();
            match verdict["verdict"].as_str() {
                Some("reversed") => ("reversal", 0),
                Some("preserved") => ("preserved", 1),
                _ => ("unchecked", 0),
            }
        }
        Err(e) => {
            row["reason"] = json!(e.to_string());
            ("error", 0)
        }
    }
}
pub use crate::quote_relevance::quote_supports;
