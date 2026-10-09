//! Rollup stages 1–2 (Rust side): recent entries, the quiet-period bail, thread candidates.
use crate::{
    diary_file::{civil_from_days, days_from_civil},
    rollup::{Entry, superseded_live, voided},
    traits::DiaryReader,
};
use std::collections::{BTreeMap, BTreeSet};

pub const QUIET: &str = "Palace compression: quiet period, nothing to compress";
pub const MIN_SUBSTANTIVE: usize = 3;
pub const MAX_CANDIDATES: usize = 8;
/// Tags that describe an entry's kind rather than a thread.
const META_TAGS: &[&str] = &[
    "no-compress",
    "rollup",
    "ongoing-thread",
    "continuing-thread",
];

#[derive(Debug)]
pub struct Recent {
    pub as_of: String,
    pub entries: Vec<Entry>,
    /// Names of the entries that count toward the bail check (see `is_substantive`).
    pub substantive: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: usize,
    pub tag: String,
    pub entries: Vec<String>,
    pub days: usize,
}

/// Entries dated in `[as_of - since_days, as_of]`, ROLLUP entries excluded.
pub fn recent<D: DiaryReader>(diary: &D, as_of: &str, since_days: i64) -> Result<Recent, String>
where
    D::Error: std::fmt::Display,
{
    let start = shift_date(as_of, -since_days)?;
    let names = diary.names().map_err(|e| e.to_string())?;
    let compressed = already_compressed(diary, &names, as_of)?;
    let mut recent = Recent {
        as_of: as_of.into(),
        entries: Vec::new(),
        substantive: Vec::new(),
    };
    for name in names.iter().filter(|n| in_window(n, &start, as_of)) {
        let Some(text) = diary.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        let entry = Entry::parse(name, text);
        if entry.is_rollup {
            continue;
        }
        if is_substantive(&entry, &compressed) {
            recent.substantive.push(entry.name.clone());
        }
        recent.entries.push(entry);
    }
    Ok(recent)
}
/// Substantive = still compressible: not `no-compress`, not already superseded by a rollup
/// dated on or before as-of (a supersession voided by a `supersession-void` entry does not count).
fn is_substantive(entry: &Entry, compressed: &BTreeSet<String>) -> bool {
    !entry.tags.contains("no-compress") && !compressed.contains(&entry.name)
}
fn already_compressed<D: DiaryReader>(
    diary: &D,
    names: &[String],
    as_of: &str,
) -> Result<BTreeSet<String>, String>
where
    D::Error: std::fmt::Display,
{
    let voided = voided(diary, names)?;
    let mut compressed = BTreeSet::new();
    for name in names
        .iter()
        .filter(|n| entry_date(n).is_some_and(|d| d <= as_of))
    {
        if let Some(text) = diary.read(name).map_err(|e| e.to_string())? {
            let entry = Entry::parse(name, text);
            if entry.is_rollup {
                compressed.extend(superseded_live(&entry, names, &voided));
            }
        }
    }
    Ok(compressed)
}
pub fn should_bail(recent: &Recent) -> bool {
    recent.substantive.len() < MIN_SUBSTANTIVE
}
/// Tags shared by ≥ 2 recent entries, ranked by entry count × distinct days (ties by tag).
pub fn candidates(recent: &Recent) -> Vec<Candidate> {
    let mut by_tag: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for entry in &recent.entries {
        for tag in entry.tags.iter().filter(|t| !is_meta_tag(t)) {
            by_tag.entry(tag).or_default().push(entry);
        }
    }
    let mut ranked: Vec<Candidate> = by_tag
        .into_iter()
        .filter(|(_, entries)| entries.len() >= 2)
        .map(|(tag, entries)| Candidate {
            id: 0,
            tag: tag.into(),
            days: entries
                .iter()
                .filter_map(|e| entry_date(&e.name))
                .collect::<BTreeSet<_>>()
                .len(),
            entries: entries.iter().map(|e| e.name.clone()).collect(),
        })
        .collect();
    ranked.sort_by(|a, b| {
        (b.entries.len() * b.days)
            .cmp(&(a.entries.len() * a.days))
            .then_with(|| a.tag.cmp(&b.tag))
    });
    ranked.truncate(MAX_CANDIDATES);
    for (i, c) in ranked.iter_mut().enumerate() {
        c.id = i + 1;
    }
    ranked
}
fn is_meta_tag(tag: &str) -> bool {
    META_TAGS.contains(&tag)
}
pub fn entry_date(name: &str) -> Option<&str> {
    let date = name.get(..10)?;
    valid_date(date).then_some(date)
}
fn in_window(name: &str, start: &str, end: &str) -> bool {
    entry_date(name).is_some_and(|d| start <= d && d <= end)
}
pub fn valid_date(date: &str) -> bool {
    date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}
/// `YYYY-MM-DD` shifted by whole days.
pub fn shift_date(date: &str, days: i64) -> Result<String, String> {
    if !valid_date(date) {
        return Err(format!("date must be YYYY-MM-DD: {date}"));
    }
    let part = |range: std::ops::Range<usize>| date[range].parse::<i64>().unwrap_or(0);
    let (y, m, d) = civil_from_days(days_from_civil(part(0..4), part(5..7), part(8..10)) + days);
    Ok(format!("{y:04}-{m:02}-{d:02}"))
}
/// `3d` → 3.
pub fn parse_since(value: &str) -> Result<i64, String> {
    value
        .strip_suffix('d')
        .and_then(|n| n.parse::<i64>().ok())
        .filter(|n| *n >= 0)
        .ok_or_else(|| format!("--since must look like 3d, got {value}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Memory;
    fn diary() -> Memory {
        let mut d = Memory::default();
        for (name, text) in [
            ("2026-09-20-aaaaaaaaaaaa", "# A\nTags: alpha, beta."),
            ("2026-09-21-bbbbbbbbbbbb", "# B\nTags: alpha, no-compress."),
            ("2026-09-22-cccccccccccc", "# C\nTags: alpha, beta."),
            (
                "2026-09-22-dddddddddddd",
                "# ROLLUP: x — state as of 2026-09-23\nTags: alpha.\n## Supersedes\n- 2026-09-20-aaaaaaaaaaaa.md",
            ),
            ("2026-09-10-eeeeeeeeeeee", "# E\nTags: alpha."),
        ] {
            d.0.insert(name.into(), text.into());
        }
        d
    }
    #[test]
    fn window_bail_and_candidates() {
        let r = recent(&diary(), "2026-09-22", 3).unwrap();
        let names: Vec<_> = r.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "2026-09-20-aaaaaaaaaaaa",
                "2026-09-21-bbbbbbbbbbbb",
                "2026-09-22-cccccccccccc"
            ]
        );
        assert_eq!(r.substantive, ["2026-09-22-cccccccccccc"]);
        assert!(should_bail(&r));
        let c = candidates(&r);
        assert_eq!(c[0].tag, "alpha");
        assert_eq!((c[0].id, c[0].entries.len(), c[0].days), (1, 3, 3));
        assert_eq!(c[1].tag, "beta");
        assert_eq!(c.len(), 2);
    }
    #[test]
    fn dates() {
        assert_eq!(shift_date("2026-03-01", -1).unwrap(), "2026-02-28");
        assert_eq!(shift_date("2024-03-01", -1).unwrap(), "2024-02-29");
        assert_eq!(parse_since("3d").unwrap(), 3);
        assert!(parse_since("3h").is_err());
    }
}
