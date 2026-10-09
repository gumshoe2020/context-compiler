//! Terms, minimal (M4d item 8; SPEC §2.4, §4), pure: the `vocabulary_gap` check against an
//! editable table of predicate → accepted quote verbs (prompts/terms.json), the adoption
//! survival bar (claims in ≥ 3 rollups over ≥ 14 days, no open vocabulary_gap or conflation
//! finding, the definition claim survived a check) and the monthly digest's line.
use crate::claim::Claim;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MIN_ROLLUPS: usize = 3;
pub const MIN_DAYS: i64 = 14;
pub const DIGEST_MAX: usize = 10;

/// predicate → the verbs a quote may use for it (lowercase phrases).
#[derive(Debug, Default)]
pub struct Table(BTreeMap<String, Vec<String>>);

impl Table {
    /// `{"predicates": {"supports": ["supports", "backs", …], …}}`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let map = value["predicates"]
            .as_object()
            .ok_or("terms table: `predicates` must be an object")?;
        let mut table = BTreeMap::new();
        for (predicate, verbs) in map {
            let verbs = verbs
                .as_array()
                .ok_or_else(|| format!("terms table: {predicate} must list verbs"))?;
            let verbs = verbs
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_lowercase);
            table.insert(predicate.clone(), verbs.collect());
        }
        Ok(Self(table))
    }
}

/// SPEC §4: a claim whose predicate the table knows, with a quote that uses none of that
/// predicate's accepted verbs (nor the predicate itself), is a vocabulary gap.
pub fn vocabulary_gap(c: &Claim, table: &Table) -> bool {
    let (Some(verbs), Some(quote)) = (table.0.get(&c.predicate), c.quote.as_deref()) else {
        return false;
    };
    let words = format!(" {} ", crate::quote_relevance::words(quote).join(" "));
    let own = c.predicate.replace('_', " ");
    !verbs
        .iter()
        .chain([&own])
        .any(|v| words.contains(&format!(" {v} ")))
}

/// The watch row a gap raises (check `vocabulary`; kind `vocabulary_gap`; proposal `coin`).
pub fn gap_row(rollup: &str, c: &Claim) -> Value {
    let quote = c.quote.clone().unwrap_or_default();
    json!({"check":"vocabulary","rollup":rollup,"claim":c.number,"claim_id":c.id,"status":"vocabulary_gap",
        "evidence":{"source":c.source,"passage":quote,"locator":null},
        "proposal":{"verb":"coin","text":format!("coin a term fine enough for \"{}\": the quote says \"{quote}\", not `{}`", c.statement, c.predicate)},
        "checker":"rule:terms"})
}

/// How a term is used, for the survival bar.
#[derive(Debug, Default)]
pub struct Usage {
    pub claims: usize,
    pub rollups: usize,
    /// Earliest and latest as-of date of those rollups (YYYY-MM-DD).
    pub first: Option<String>,
    pub last: Option<String>,
    /// Open vocabulary_gap or conflation findings against the term or its claims.
    pub blocking: usize,
    pub definition_survived: bool,
}

/// `Ok` when a proposed term meets SPEC §2.4's survival bar; else why not.
pub fn survival(u: &Usage) -> Result<(), String> {
    let days = match (&u.first, &u.last) {
        (Some(first), Some(last)) => days_between(first, last).unwrap_or(0),
        _ => 0,
    };
    let why = [
        (
            u.rollups < MIN_ROLLUPS,
            format!("used in {} rollup(s) (< {MIN_ROLLUPS})", u.rollups),
        ),
        (
            days < MIN_DAYS,
            format!("over {days} day(s) (< {MIN_DAYS})"),
        ),
        (
            u.blocking > 0,
            format!("{} open vocabulary_gap/conflation finding(s)", u.blocking),
        ),
        (
            !u.definition_survived,
            "definition claim has survived no check".to_owned(),
        ),
    ];
    let failed: Vec<String> = why
        .into_iter()
        .filter(|(bad, _)| *bad)
        .map(|(_, w)| w)
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// Whole days from `a` to `b` (YYYY-MM-DD), or None when either is not a date.
pub fn days_between(a: &str, b: &str) -> Option<i64> {
    Some(day_number(b)? - day_number(a)?)
}

fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    Some(era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy)
}

/// One digest line: `🎩 word <w>: <definition> — <n> claim(s). Reply veto <w> to retire it.`
pub fn digest_line(word: &str, definition: &str, claims: usize) -> String {
    format!("🎩 word {word}: {definition} — {claims} claim(s). Reply veto {word} to retire it.")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn claim(line: &str) -> Claim {
        crate::claim::parse(&format!("## Claims\n{line}\n"))
            .claims
            .remove(0)
    }
    fn table() -> Table {
        Table::from_json(&json!({"predicates":{"depends_on":["relies on","requires"],"supports":["supports","backs"]}})).unwrap()
    }
    #[test]
    fn a_quote_verb_outside_the_predicate_is_a_gap() {
        let gap = claim(
            "- C2. Callum | supports | the tour — 2026-09-21 — s.md — quote: \"Callum booked a zeppelin tour\"",
        );
        let fine = claim(
            "- C1. Callum | depends on | the schedule — 2026-09-21 — s.md — quote: \"Callum relies on the zeppelin schedule\"",
        );
        let own = claim(
            "- C3. Callum | supports | it — 2026-09-21 — s.md — quote: \"Callum supports it\"",
        );
        let unknown = claim(
            "- C4. Callum | booked | the tour — 2026-09-21 — s.md — quote: \"Callum chose it\"",
        );
        let bare = claim("- C5. Callum | supports | the tour — 2026-09-21 — s.md");
        assert!(vocabulary_gap(&gap, &table()));
        for c in [&fine, &own, &unknown, &bare] {
            assert!(!vocabulary_gap(c, &table()), "{}", c.number);
        }
        let row = gap_row("r", &gap);
        assert_eq!(
            (row["status"].as_str(), row["proposal"]["verb"].as_str()),
            (Some("vocabulary_gap"), Some("coin"))
        );
    }
    #[test]
    fn the_survival_bar() {
        let mut u = Usage {
            claims: 3,
            rollups: 3,
            first: Some("2026-09-01".into()),
            last: Some("2026-09-22".into()),
            blocking: 0,
            definition_survived: true,
        };
        assert_eq!(survival(&u), Ok(()));
        u.last = Some("2026-09-14".into());
        assert!(survival(&u).unwrap_err().contains("13 day(s)"));
        u.last = Some("2026-09-15".into());
        u.rollups = 2;
        u.blocking = 1;
        u.definition_survived = false;
        assert_eq!(survival(&u).unwrap_err().matches("; ").count(), 2);
        assert_eq!(days_between("2024-02-28", "2024-03-01"), Some(2));
        assert_eq!(days_between("2025-12-31", "2026-01-01"), Some(1));
        assert!(
            digest_line("recurs_in", "a pattern", 3)
                .ends_with("Reply veto recurs_in to retire it.")
        );
    }
}
