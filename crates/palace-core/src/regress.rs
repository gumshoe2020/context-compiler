//! M4c: compare MemPalace's current top-5 answers with the frozen ones (pure, no IO).
//! A case's `actual` is `None` when the search for it failed.
use std::collections::HashMap;

pub const TOP: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub query: String,
    pub expected: Vec<String>,
    pub actual: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub queries: usize,
    pub answered: usize,
    pub top1: usize,
    pub top5: f64,
    pub exact: usize,
    pub errors: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gates {
    pub min_top1: usize,
    pub min_top5: f64,
    pub min_exact: usize,
    pub max_errors: usize,
}
impl Default for Gates {
    fn default() -> Self {
        Self {
            min_top1: 45,
            min_top5: 0.90,
            min_exact: 45,
            max_errors: 0,
        }
    }
}

/// Queries whose first hit equals the frozen first hit (an empty list never matches).
pub fn top1(cases: &[Case]) -> usize {
    cases
        .iter()
        .filter(|case| match (&case.actual, case.expected.first()) {
            (Some(actual), Some(expected)) => actual.first() == Some(expected),
            _ => false,
        })
        .count()
}

/// Mean multiset overlap |expected ∩ actual| / 5 over every case (failed searches score 0).
pub fn top5(cases: &[Case]) -> f64 {
    if cases.is_empty() {
        return 0.0;
    }
    let total: usize = cases
        .iter()
        .map(|case| overlap(&case.expected, case.actual.as_deref().unwrap_or_default()))
        .sum();
    total as f64 / (TOP * cases.len()) as f64
}

/// Queries whose ordered answer is identical to the frozen one.
pub fn exact(cases: &[Case]) -> usize {
    cases
        .iter()
        .filter(|case| case.actual.as_ref() == Some(&case.expected))
        .count()
}

pub fn errors(cases: &[Case]) -> usize {
    cases.iter().filter(|case| case.actual.is_none()).count()
}

pub fn metrics(cases: &[Case]) -> Metrics {
    Metrics {
        queries: cases.len(),
        answered: cases.len() - errors(cases),
        top1: top1(cases),
        top5: top5(cases),
        exact: exact(cases),
        errors: errors(cases),
    }
}

/// Cases whose answer is not identical to the frozen one (failed searches included).
pub fn differing(cases: &[Case]) -> Vec<&Case> {
    cases
        .iter()
        .filter(|case| case.actual.as_ref() != Some(&case.expected))
        .collect()
}

/// The names of the gates `metrics` misses; empty = pass.
pub fn failures(metrics: &Metrics, gates: &Gates) -> Vec<String> {
    let mut failed = Vec::new();
    if metrics.top1 < gates.min_top1 {
        failed.push(format!("top1<{}", gates.min_top1));
    }
    if metrics.top5 < gates.min_top5 {
        failed.push(format!("top5<{:.2}", gates.min_top5));
    }
    if metrics.exact < gates.min_exact {
        failed.push(format!("exact<{}", gates.min_exact));
    }
    if metrics.errors > gates.max_errors {
        failed.push(format!("errors>{}", gates.max_errors));
    }
    failed
}

/// The worst value of each metric across variants (what the one-line summary reports).
pub fn worst(variants: &[Metrics]) -> Option<Metrics> {
    let first = variants.first()?.clone();
    Some(variants.iter().fold(first, |worst, m| Metrics {
        queries: worst.queries.max(m.queries),
        answered: worst.answered.min(m.answered),
        top1: worst.top1.min(m.top1),
        top5: worst.top5.min(m.top5),
        exact: worst.exact.min(m.exact),
        errors: worst.errors.max(m.errors),
    }))
}

/// `top1=<n>/<q> top5=<0.xx> exact=<n>/<q> errors=<n>` preceded by the version and counts.
pub fn summary(version: &str, variants: usize, m: &Metrics) -> String {
    format!(
        "mempalace={version} variants={variants} queries={}/{} top1={}/{} top5={:.2} exact={}/{} errors={}",
        m.answered, m.queries, m.top1, m.queries, m.top5, m.exact, m.queries, m.errors
    )
}

/// The value of the script's one `PIN=` assignment (quotes stripped). No such line, an empty
/// value, or more than one `PIN=` line (M4d-2a item 7, review L-13) is an error.
pub fn parse_pin(script: &str) -> Result<String, String> {
    let values: Vec<&str> = script
        .lines()
        .filter_map(|line| line.trim().strip_prefix("PIN="))
        .map(|value| {
            let value = value.split('#').next().unwrap_or_default().trim();
            value.trim_matches(|c| c == '"' || c == '\'')
        })
        .collect();
    match values.as_slice() {
        [] => Err("no PIN= line".into()),
        [""] => Err("an empty PIN= line".into()),
        [value] => Ok((*value).to_owned()),
        more => Err(format!("{} PIN= lines (exactly one allowed)", more.len())),
    }
}

fn overlap(expected: &[String], actual: &[String]) -> usize {
    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for name in expected {
        *remaining.entry(name).or_default() += 1;
    }
    actual
        .iter()
        .filter(|name| match remaining.get_mut(name.as_str()) {
            Some(count) if *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }
    fn case(expected: &[&str], actual: Option<&[&str]>) -> Case {
        Case {
            query: "q".into(),
            expected: names(expected),
            actual: actual.map(names),
        }
    }
    const FIVE: [&str; 5] = ["a", "b", "c", "d", "e"];

    #[test]
    fn identical() {
        let cases = vec![case(&FIVE, Some(&FIVE)), case(&FIVE, Some(&FIVE))];
        let m = metrics(&cases);
        assert_eq!((m.top1, m.top5, m.exact, m.errors), (2, 1.0, 2, 0));
        assert!(differing(&cases).is_empty());
        assert!(
            failures(
                &m,
                &Gates {
                    min_top1: 2,
                    min_exact: 2,
                    ..Gates::default()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn one_swap_keeps_overlap_but_loses_top1_and_exact() {
        let cases = vec![case(&FIVE, Some(&["b", "a", "c", "d", "e"]))];
        let m = metrics(&cases);
        assert_eq!((m.top1, m.top5, m.exact, m.errors), (0, 1.0, 0, 0));
        assert_eq!(differing(&cases).len(), 1);
    }

    #[test]
    fn one_missing_costs_a_fifth() {
        let cases = vec![case(&FIVE, Some(&["a", "b", "c", "d", "z"]))];
        let m = metrics(&cases);
        assert_eq!((m.top1, m.exact), (1, 0));
        assert!((m.top5 - 0.8).abs() < 1e-12);
    }

    #[test]
    fn empty_tops_and_errors() {
        let cases = vec![
            case(&[], Some(&[])),
            case(&FIVE, Some(&[])),
            case(&FIVE, None),
        ];
        let m = metrics(&cases);
        assert_eq!(
            (m.top1, m.top5, m.exact, m.errors, m.answered),
            (0, 0.0, 1, 1, 2)
        );
        assert_eq!(
            failures(&m, &Gates::default()),
            ["top1<45", "top5<0.90", "exact<45", "errors>0"]
        );
        assert_eq!(metrics(&[]).top5, 0.0);
    }

    /// 50 frozen rows; `wrong` of them altered the way the M4c review tampered its file.
    fn fifty(wrong: impl Fn(usize) -> Option<Vec<&'static str>>) -> Vec<Case> {
        (0..50)
            .map(|i| case(&FIVE, Some(&wrong(i).unwrap_or(FIVE.to_vec()))))
            .collect()
    }

    #[test]
    fn exact_is_gated_eleven_wrong_rows_fail_at_the_defaults() {
        // rows 0-9: hits 4 and 5 swapped; row 20: one hit missing (M4c review S-2).
        let m = metrics(&fifty(|i| match i {
            0..=9 => Some(vec!["a", "b", "c", "e", "d"]),
            20 => Some(vec!["a", "b", "c", "d"]),
            _ => None,
        }));
        assert_eq!((m.top1, m.exact), (50, 39));
        assert!(m.top5 >= 0.90);
        assert_eq!(failures(&m, &Gates::default()), ["exact<45"]);
    }

    #[test]
    fn a_deleted_diary_entry_fails_the_strict_exact_gate() {
        // The review's measurement with one entry's drawers deleted: top1 48, exact 47.
        let m = metrics(&fifty(|i| match i {
            0 | 1 => Some(vec!["b", "c", "d", "e", "f"]),
            2 => Some(vec!["a", "c", "d", "e", "f"]),
            _ => None,
        }));
        assert_eq!((m.top1, m.exact), (48, 47));
        let strict = Gates {
            min_top1: 50,
            min_exact: 50,
            ..Gates::default()
        };
        assert!(failures(&m, &Gates::default()).is_empty());
        assert_eq!(failures(&m, &strict), ["top1<50", "exact<50"]);
    }

    #[test]
    fn duplicates_count_as_a_multiset() {
        let cases = vec![case(
            &["a", "a", "b", "c", "d"],
            Some(&["a", "b", "b", "c", "d"]),
        )];
        assert!((top5(&cases) - 0.8).abs() < 1e-12);
    }

    #[test]
    fn worst_across_variants_and_summary() {
        let a = metrics(&[case(&FIVE, Some(&FIVE))]);
        let b = metrics(&[case(&FIVE, None)]);
        let w = worst(&[a, b]).unwrap();
        assert_eq!(
            summary("3.5.0", 2, &w),
            "mempalace=3.5.0 variants=2 queries=0/1 top1=0/1 top5=0.00 exact=0/1 errors=1"
        );
        assert!(worst(&[]).is_none());
    }

    #[test]
    fn pin_parsing() {
        assert_eq!(
            parse_pin("set -u\nPIN=\"3.5.0\"\nX=1").as_deref(),
            Ok("3.5.0")
        );
        assert_eq!(parse_pin("PIN='3.6.1'  # bumped").as_deref(), Ok("3.6.1"));
        assert_eq!(parse_pin("PIN=3.7.0").as_deref(), Ok("3.7.0"));
        assert!(parse_pin("echo $PIN\nPIN=\"\"").is_err());
        assert_eq!(parse_pin("no pin here"), Err("no PIN= line".into()));
        assert_eq!(
            parse_pin("PIN=3.5.0\nPIN=3.6.0"),
            Err("2 PIN= lines (exactly one allowed)".into())
        );
    }
}
