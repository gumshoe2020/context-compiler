use crate::traits::DiaryReader;
use std::collections::{BTreeMap, BTreeSet};

/// Tag of an append-only correction entry that voids earlier supersessions (M4d item 10).
pub const VOID_TAG: &str = "supersession-void";

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub title: String,
    pub tags: BTreeSet<String>,
    pub is_rollup: bool,
    pub text: String,
}
impl Entry {
    pub fn parse(name: &str, text: String) -> Self {
        let title = text.split('\n').next().unwrap_or("").trim().to_owned();
        let lower = text.to_lowercase();
        let tags = lower
            .match_indices("tags:")
            .find(|(i, _)| *i == 0 || !word(lower[..*i].chars().next_back().unwrap()))
            .map(|(i, _)| {
                lower[i + 5..]
                    .trim_start()
                    .split(['\n', '.'])
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let is_rollup = title
            .strip_prefix('#')
            .is_some_and(|t| t.trim_start().to_uppercase().starts_with("ROLLUP:"));
        Self {
            name: name.strip_suffix(".md").unwrap_or(name).into(),
            title,
            tags,
            is_rollup,
            text,
        }
    }
}
/// `(thread, as_of)` of a `ROLLUP: <thread> — state as of <YYYY-MM-DD>` title; the as-of falls
/// back to the entry name's date prefix, the thread to the name.
pub fn title_parts(entry: &Entry) -> (String, String) {
    let title = entry.title.trim_start_matches('#').trim();
    let rest = title
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("ROLLUP:"))
        .map_or(title, |_| title[7..].trim());
    match rest.rsplit_once(" — state as of ") {
        Some((thread, date)) => (thread.trim().to_owned(), date.trim().to_owned()),
        None => (
            entry.name.clone(),
            entry.name.get(..10).unwrap_or_default().to_owned(),
        ),
    }
}
/// The thread of a rollup titled `ROLLUP: <thread> — state as of <date>` (M5b-10 item 5); `None`
/// for any other entry.
pub fn rollup_thread(entry: &Entry) -> Option<String> {
    let titled = entry.is_rollup && entry.title.contains(" — state as of ");
    titled.then(|| title_parts(entry).0)
}
pub fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
#[derive(Default, Debug)]
pub struct SourceRefs {
    pub resolved: Vec<String>,
    pub dangling: Vec<String>,
    pub referenced_only: Vec<String>,
}
/// The label `prompts/rollup.md` pins for referenced-only entries (M5b item 0).
pub const REFERENCED_ONLY_LABEL: &str = "Referenced only (not superseded):";
/// Superseded sources are the names before the first referenced-only label; the names from each
/// label to the end of its paragraph or list are referenced-only, never superseded.
pub fn superseded_sources(entry: &Entry, names: &[String]) -> SourceRefs {
    let section = supersedes_section(&entry.text);
    let excluded: BTreeSet<String> = referenced_regions(&section)
        .into_iter()
        .flat_map(|region| resolve_tokens(region, names, &BTreeSet::new()).0)
        .collect();
    let before = ref_label(&section).map_or(section.as_str(), |(start, _)| &section[..start]);
    let (resolved, dangling) = resolve_tokens(before, names, &excluded);
    SourceRefs {
        resolved,
        dangling,
        referenced_only: excluded.into_iter().collect(),
    }
}
/// Diary files named in the Supersedes section that are neither superseded (named before the
/// first referenced-only label) nor in a referenced-only region (M5b-2 item 8, review S-e):
/// `watch` reports them as unclassified, never drops them silently.
pub fn unclassified(entry: &Entry, names: &[String]) -> Vec<String> {
    let section = supersedes_section(&entry.text);
    let refs = superseded_sources(entry, names);
    resolve_tokens(&section, names, &BTreeSet::new())
        .0
        .into_iter()
        .filter(|name| !refs.resolved.contains(name) && !refs.referenced_only.contains(name))
        .collect()
}
/// Paragraphs of a Supersedes section after its first that name diary files without a
/// referenced-only label, each with the names (M5b item 0): such a paragraph would otherwise be
/// read as superseded sources, or silently dropped after a label.
pub fn unlabelled_paragraphs(entry: &Entry, names: &[String]) -> Vec<Vec<String>> {
    let section = supersedes_section(&entry.text);
    paragraphs(&section)
        .skip(1)
        .filter(|paragraph| ref_label(paragraph).is_none())
        .map(|paragraph| resolve_tokens(paragraph, names, &BTreeSet::new()).0)
        .filter(|named| !named.is_empty())
        .collect()
}
/// Every referenced-only label in the Supersedes section other than the pinned one at the very
/// start of a paragraph, as written: a NEW rollup may use only that one (M5b-1 item 4).
pub fn unpinned_labels(entry: &Entry) -> Vec<String> {
    let section = supersedes_section(&entry.text);
    let mut found = Vec::new();
    for paragraph in paragraphs(&section) {
        let mut offset = 0;
        while let Some((start, end)) = ref_label(&paragraph[offset..]) {
            let label = &paragraph[offset + start..offset + end];
            if offset + start > 0 || label != REFERENCED_ONLY_LABEL {
                found.push(label.to_owned());
            }
            offset += end;
        }
    }
    found
}
/// Non-blank blocks separated by blank lines.
fn paragraphs(section: &str) -> impl Iterator<Item = &str> {
    let mut rest = section;
    std::iter::from_fn(move || {
        let start = rest.len() - rest.trim_start_matches([' ', '\n']).len();
        rest = &rest[start..];
        if rest.is_empty() {
            return None;
        }
        let end = paragraph_end(rest);
        let paragraph = &rest[..end];
        rest = &rest[end..];
        Some(paragraph)
    })
}
/// Each referenced-only label's region: from the label to the end of its paragraph (a list with
/// no blank line inside is one paragraph); a later label in it starts nothing new.
fn referenced_regions(section: &str) -> Vec<&str> {
    let mut regions = Vec::new();
    let mut rest = section;
    while let Some((_, end)) = ref_label(rest) {
        let region = &rest[end..];
        let stop = paragraph_end(region);
        regions.push(&region[..stop]);
        rest = &region[stop..];
    }
    regions
}
/// Byte offset of the first blank line after the text's first line, or its length.
fn paragraph_end(text: &str) -> usize {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if offset > 0 && blank(line) {
            return offset;
        }
        offset += line.len();
    }
    text.len()
}
/// The entries a `supersession-void` correction entry names under `## Void supersedes`: they are
/// NOT superseded, whichever rollup listed them. Empty for any other entry.
pub fn void_listed(entry: &Entry, names: &[String]) -> Vec<String> {
    if !entry.tags.contains(VOID_TAG) {
        return Vec::new();
    }
    resolve_tokens(&void_section(&entry.text), names, &BTreeSet::new()).0
}
/// Voided entry → the file name of the latest dated `supersession-void` note naming it (M4d-2a
/// item 9; the name, not only its date, since M4d-2a.1 item 4 orders same-day files).
pub type Voids = BTreeMap<String, String>;
/// Every entry voided by a dated `supersession-void` note in the diary. A note whose file name
/// does not start with a valid date voids nothing (M4d-2a.1 item 4, review L-3; see
/// `undated_void_notes`).
pub fn voided<D: DiaryReader>(diary: &D, names: &[String]) -> Result<Voids, String>
where
    D::Error: std::fmt::Display,
{
    let mut voids = Voids::new();
    for (note, listed) in void_notes(diary, names)? {
        if crate::rollup_prep::entry_date(&note).is_none() {
            continue;
        }
        for entry in listed {
            let latest = voids.entry(entry).or_default();
            if *latest < note {
                latest.clone_from(&note);
            }
        }
    }
    Ok(voids)
}
/// `supersession-void` notes that list entries but have no valid date in their file name: they
/// void nothing, and `watch` reports them.
pub fn undated_void_notes<D: DiaryReader>(
    diary: &D,
    names: &[String],
) -> Result<Vec<String>, String>
where
    D::Error: std::fmt::Display,
{
    Ok(void_notes(diary, names)?
        .into_iter()
        .filter(|(note, _)| crate::rollup_prep::entry_date(note).is_none())
        .map(|(note, _)| note)
        .collect())
}
/// Each `supersession-void` note naming at least one entry, with the entries.
fn void_notes<D: DiaryReader>(
    diary: &D,
    names: &[String],
) -> Result<Vec<(String, Vec<String>)>, String>
where
    D::Error: std::fmt::Display,
{
    let mut notes = Vec::new();
    for name in names {
        let Some(text) = diary.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        let listed = void_listed(&Entry::parse(name, text), names);
        if !listed.is_empty() {
            notes.push((name.clone(), listed));
        }
    }
    Ok(notes)
}
/// Whether `rollup`'s supersession of `name` is voided: a note names it and the rollup precedes
/// the note — an earlier date, or the same date with a file name that sorts first (both names
/// start with their date). An undated rollup name counts as voided.
pub fn is_voided(voids: &Voids, name: &str, rollup: &str) -> bool {
    voids.get(name).is_some_and(|note| {
        crate::rollup_prep::entry_date(rollup).is_none() || rollup < note.as_str()
    })
}
/// `superseded_sources(entry).resolved` without the entries whose supersession by it is voided.
pub fn superseded_live(entry: &Entry, names: &[String], voids: &Voids) -> Vec<String> {
    let mut resolved = superseded_sources(entry, names).resolved;
    resolved.retain(|n| !is_voided(voids, n, &entry.name));
    resolved
}
/// Lines under a `## Void supersedes` heading, up to the next heading.
fn void_section(text: &str) -> String {
    let mut lines = text.lines().skip_while(|line| {
        !line
            .trim_start_matches('#')
            .trim()
            .to_lowercase()
            .starts_with("void supersedes")
            || !line.starts_with('#')
    });
    lines.next();
    lines
        .take_while(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}
fn supersedes_section(text: &str) -> String {
    let lines: Vec<_> = text.split('\n').collect();
    let Some(start) = lines.iter().position(|line| {
        let line = line.trim();
        let count = line.chars().take_while(|c| *c == '#').count();
        if count > 3 {
            return false;
        }
        let rest = line[count..].trim_start().to_uppercase();
        ["SUPERSEDES", "SUPERSEDE"].iter().any(|prefix| {
            rest.strip_prefix(prefix)
                .is_some_and(|s| s.chars().next().is_none_or(|c| !word(c)))
        })
    }) else {
        return String::new();
    };
    let first = lines[start];
    if !first
        .trim_start_matches('#')
        .trim()
        .to_uppercase()
        .starts_with("SUPERSEDES")
    {
        return String::new();
    }
    let inline = first.contains(':') && !first.starts_with("##");
    let mut result = Vec::new();
    if inline {
        result.push(first.split_once(':').unwrap().1);
    }
    for line in &lines[start + 1..] {
        if line.starts_with('#') || (inline && line.trim().is_empty()) {
            break;
        }
        result.push(line);
    }
    result.join("\n")
}
/// A referenced-only label (M5b-2 item 8, M5b-3 F-2: one rule shared with the Python watcher,
/// whose regular expression spells whitespace as a literal space): after "referenced" (any case),
/// `[- ]only` or ` *,? *(but +)?not +superseded`, then an optional one-level parenthetical
/// (` *\([^()\n]*\)`), spaces, an optional `**`, then a colon; standing at the start of a line
/// (after space indentation, a `-`, `*` or `+` marker with spaces, and `**`) or after the end of
/// a sentence (`.` or `;`, spaces, an optional `**`). Tabs, NBSP and other Unicode spaces are
/// never whitespace here. Returns the label's start and the offset just past its colon.
fn ref_label(text: &str) -> Option<(usize, usize)> {
    // Labels are ASCII; use original byte offsets to preserve Unicode passages.
    let lower = text.to_ascii_lowercase();
    lower.match_indices("referenced").find_map(|(start, _)| {
        let end = label_end(&lower[start..])?;
        label_position(&lower[..start]).then_some((start, start + end))
    })
}
/// The offset just past the colon of a label that begins `rest`.
fn label_end(rest: &str) -> Option<usize> {
    let mut tail = phrase_tail(rest.strip_prefix("referenced")?)?;
    let spaced = tail.trim_start_matches(' ');
    if let Some(inner) = spaced.strip_prefix('(') {
        let close = inner.find([')', '(', '\n'])?;
        tail = inner[close..].strip_prefix(')')?;
    }
    tail = tail.trim_start_matches(' ');
    tail = tail.strip_prefix("**").unwrap_or(tail);
    tail = tail.strip_prefix(':')?;
    Some(rest.len() - tail.len())
}
/// What follows the phrase after "referenced": `[- ]only`, or ` *,? *(but +)?not +superseded`.
fn phrase_tail(after: &str) -> Option<&str> {
    let only = after
        .strip_prefix(['-', ' '])
        .and_then(|t| t.strip_prefix("only"));
    if only.is_some() {
        return only;
    }
    let t = after.trim_start_matches(' ');
    let t = t.strip_prefix(',').unwrap_or(t).trim_start_matches(' ');
    let t = t
        .strip_prefix("but ")
        .map_or(t, |b| b.trim_start_matches(' '));
    t.strip_prefix("not ")?
        .trim_start_matches(' ')
        .strip_prefix("superseded")
}
/// Whether a label may stand after `before` (the text preceding it).
fn label_position(before: &str) -> bool {
    let line = before.rsplit('\n').next().unwrap_or_default();
    let bare = line.strip_suffix("**").unwrap_or(line);
    let sentence = bare.trim_end_matches(' ');
    let after_sentence =
        sentence.len() < bare.len() && (sentence.ends_with('.') || sentence.ends_with(';'));
    after_sentence || line_start(bare.trim_start_matches(' '))
}
/// Nothing, or a `-`, `*` or `+` list marker followed only by spaces (a numbered item's "1. " is
/// a sentence end).
fn line_start(s: &str) -> bool {
    let marked = s.strip_prefix(['-', '*', '+']);
    s.is_empty() || marked.is_some_and(|r| !r.is_empty() && blank(r))
}
/// A line holding nothing but spaces (the Python watcher's `[ ]*`).
fn blank(line: &str) -> bool {
    line.trim_end_matches('\n').bytes().all(|b| b == b' ')
}
fn full_name(bytes: &[u8]) -> bool {
    bytes.len() >= 23
        && bytes[..23].iter().enumerate().all(|(i, b)| match i {
            4 | 7 | 10 => *b == b'-',
            0..=9 => b.is_ascii_digit(),
            _ => b.is_ascii_hexdigit(),
        })
}
fn resolve_tokens(
    text: &str,
    names: &[String],
    exclude: &BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut resolved = Vec::new();
    let mut dangling = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if full_name(&bytes[i..]) {
            let name = &text[i..i + 23];
            if !exclude.contains(name) && !resolved.iter().any(|n| n == name) {
                resolved.push(name.to_owned());
            }
            let end = i
                + 23
                + if text[i + 23..].to_ascii_lowercase().starts_with(".md") {
                    3
                } else {
                    0
                };
            spans.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }
    for i in 0..bytes.len().saturating_sub(11) {
        if !bytes[i..i + 12].iter().all(u8::is_ascii_hexdigit)
            || (i > 0 && (bytes[i - 1].is_ascii_hexdigit() || bytes[i - 1] == b'-'))
            || bytes.get(i + 12).is_some_and(u8::is_ascii_hexdigit)
            || spans.iter().any(|(a, b)| *a <= i && i < *b)
        {
            continue;
        }
        let hash = text[i..i + 12].to_ascii_lowercase();
        if let Some(name) = names.iter().find(|n| n.ends_with(&format!("-{hash}"))) {
            if !exclude.contains(name) && !resolved.contains(name) {
                resolved.push(name.clone());
            }
        } else if !dangling.contains(&hash) {
            dangling.push(hash);
        }
    }
    (resolved, dangling)
}
#[cfg(test)]
mod label_tests {
    use super::*;

    const SUPERSEDED: &str = "2026-09-20-aaaaaaaaaaaa";
    const KEPT: &str = "2026-09-20-bbbbbbbbbbbb";
    const ALSO_KEPT: &str = "2026-09-20-cccccccccccc";

    fn refs(section: &str) -> SourceRefs {
        let text = format!("# ROLLUP: t — state as of 2026-09-23\nTags: t.\n{section}");
        superseded_sources(&Entry::parse("r", text), &[])
    }
    fn section(label: &str) -> String {
        format!("## Supersedes\n- {SUPERSEDED}.md\n\n{label} {KEPT}.md (why), {ALSO_KEPT}.md\n")
    }
    /// M5b-1 item 4: the label is a bounded closed set, at the start of a line or after the end
    /// of a sentence; prose and parentheticals that merely mention the word are not labels.
    #[test]
    fn the_label_is_a_closed_set_at_a_line_or_sentence_start() {
        let cases = [
            ("Referenced only:", true),
            ("Referenced-only:", true),
            ("referenced only (no-compress):", true),
            ("Referenced, NOT superseded:", true),
            (
                "Referenced, not superseded (no-compress or separate thread):",
                true,
            ),
            ("Referenced but not superseded:", true),
            ("**Referenced only (not superseded):**", true),
            ("- Referenced only (not superseded):", true),
            (
                "None are no-compress. Referenced-only (not superseded):",
                true,
            ),
            ("All others superseded; referenced only (mail infra):", true),
            ("+ Referenced only:", true),
            ("Referenced not superseded:", true),
            ("Referenced, but not superseded:", true),
            ("A. **Referenced only:**", true),
            ("1. Referenced only:", true),
            ("* **Referenced only**:", true),
            ("REFERENCED:", false),
            ("Referenced in passing, this one is superseded too:", false),
            ("- A.md (Referenced material: see C2)", false),
            ("Also for the thread, referenced only:", false),
            ("1) Referenced only:", false),
            ("> Referenced only:", false),
        ];
        for (label, is_label) in cases {
            let r = refs(&section(label));
            if is_label {
                assert_eq!(r.resolved, [SUPERSEDED], "{label}");
                assert_eq!(r.referenced_only, [KEPT, ALSO_KEPT], "{label}");
            } else {
                assert_eq!(r.resolved, [SUPERSEDED, KEPT, ALSO_KEPT], "{label}");
                assert!(r.referenced_only.is_empty(), "{label}");
            }
        }
        let parenthetical =
            format!("## Supersedes\n- {SUPERSEDED}.md (Referenced only: see C2)\n- {KEPT}.md\n");
        assert_eq!(refs(&parenthetical).resolved, [SUPERSEDED, KEPT]);
    }
    /// M5b-3 F-2: one whitespace rule with the Python watcher (whitespace is a literal space;
    /// no `(` inside the parenthetical; a blank line holds only spaces). The eleven cases of
    /// REVIEW-opus55-m5b2 on which the two differed, numbered as there (NOTES.md lists them).
    #[test]
    fn the_eleven_whitespace_cases_follow_one_rule() {
        let cases = [
            ("01", "-\tReferenced only:", false),
            ("02", "A.\tReferenced only:", false),
            ("03", "Referenced,  not  superseded:", true),
            ("04", "Referenced,not superseded:", true),
            ("24", "Referenced ,not superseded:", true),
            ("05", "Referenced\u{a0}only:", false),
            ("07", "Referenced only\t:", false),
            ("09", "Referenced\nonly:", false),
            ("06", "Referenced only (see (x):", false),
            ("16", "\u{2003}Referenced only:", false),
        ];
        for (case, label, is_label) in cases {
            let r = refs(&section(label));
            let expected: &[&str] = if is_label {
                &[SUPERSEDED]
            } else {
                &[SUPERSEDED, KEPT, ALSO_KEPT]
            };
            assert_eq!(r.resolved, expected, "case {case}");
        }
        let nbsp_line = format!(
            "## Supersedes\n- {SUPERSEDED}.md\n\n{REFERENCED_ONLY_LABEL}\n- {KEPT}.md\n\u{a0}\n- {ALSO_KEPT}.md\n"
        );
        let r = refs(&nbsp_line);
        assert_eq!(
            (r.resolved, r.referenced_only),
            (
                vec![SUPERSEDED.to_owned()],
                vec![KEPT.to_owned(), ALSO_KEPT.to_owned()]
            ),
            "case 23"
        );
    }
    /// A NEW rollup may use only the pinned label, at the start of a paragraph.
    #[test]
    fn only_the_pinned_label_at_a_paragraph_start_is_pinned() {
        let parse = |s: &str| Entry::parse("r", format!("# ROLLUP: t\n{s}"));
        assert!(unpinned_labels(&parse(&section(REFERENCED_ONLY_LABEL))).is_empty());
        for label in [
            "Referenced, NOT superseded:",
            "- Referenced only (not superseded):",
        ] {
            let found = unpinned_labels(&parse(&section(label)));
            assert_eq!(found.len(), 1, "{label}");
        }
        let after =
            format!("## Supersedes\n- {SUPERSEDED}.md. {REFERENCED_ONLY_LABEL} {KEPT}.md\n");
        assert_eq!(unpinned_labels(&parse(&after)), [REFERENCED_ONLY_LABEL]);
    }
    #[test]
    fn the_label_runs_to_the_end_of_its_paragraph_or_list() {
        let list = format!(
            "## Supersedes\n- {SUPERSEDED}.md\n{REFERENCED_ONLY_LABEL}\n- {KEPT}.md\n- {ALSO_KEPT}.md\n\nNo INVALIDATE CANDIDATE.\n"
        );
        let r = refs(&list);
        assert_eq!(r.resolved, [SUPERSEDED]);
        assert_eq!(r.referenced_only, [KEPT, ALSO_KEPT]);
        let after = format!(
            "## Supersedes\n- {SUPERSEDED}.md\n\nReferenced only: {KEPT}.md\n\nLater: {ALSO_KEPT}.md\n"
        );
        let r = refs(&after);
        assert_eq!(
            r.resolved,
            [SUPERSEDED],
            "nothing after a label is superseded"
        );
        assert_eq!(r.referenced_only, [KEPT]);
    }
    #[test]
    fn the_word_without_a_colon_on_its_line_is_not_a_label() {
        for sentence in [
            format!("- {KEPT}.md — referenced in the essay\n- {ALSO_KEPT}.md: kept"),
            format!("- {KEPT}.md, referenced at 18:15\n- {ALSO_KEPT}.md"),
            format!("- {KEPT}.md — unreferenced: yes\n- {ALSO_KEPT}.md"),
        ] {
            let r = refs(&format!("## Supersedes\n- {SUPERSEDED}.md\n{sentence}\n"));
            assert_eq!(r.resolved, [SUPERSEDED, KEPT, ALSO_KEPT], "{sentence}");
            assert!(r.referenced_only.is_empty(), "{sentence}");
        }
    }
    /// M5b-2 item 8 (review S-e, case 21): a name after the label's paragraph, with no label of
    /// its own, is neither superseded nor referenced-only: it is unclassified, not dropped.
    #[test]
    fn a_name_outside_both_lists_is_unclassified() {
        let names = [SUPERSEDED, KEPT, ALSO_KEPT].map(String::from);
        let parse = |s: &str| Entry::parse("r", format!("# ROLLUP: t\n{s}"));
        let later = format!(
            "## Supersedes\n- {SUPERSEDED}.md\n\n{REFERENCED_ONLY_LABEL}\n- {KEPT}.md\n\n- {ALSO_KEPT}.md\n"
        );
        assert_eq!(unclassified(&parse(&later), &names), [ALSO_KEPT]);
        assert!(unclassified(&parse(&section(REFERENCED_ONLY_LABEL)), &names).is_empty());
        let plain = format!("## Supersedes\n- {SUPERSEDED}.md\n- {KEPT}.md\n");
        assert!(unclassified(&parse(&plain), &names).is_empty());
    }
    #[test]
    fn a_later_paragraph_without_a_label_is_reported() {
        let names = [SUPERSEDED, KEPT, ALSO_KEPT].map(String::from);
        let parse = |s: &str| Entry::parse("r", format!("# ROLLUP: t\n{s}"));
        let labelled = section(REFERENCED_ONLY_LABEL);
        assert!(unlabelled_paragraphs(&parse(&labelled), &names).is_empty());
        let bare = format!("## Supersedes\n- {SUPERSEDED}.md\n\nKept (no-compress): {KEPT}.md\n");
        assert_eq!(unlabelled_paragraphs(&parse(&bare), &names), [[KEPT]]);
        let prose = format!("## Supersedes\n- {SUPERSEDED}.md\n\nNo INVALIDATE CANDIDATE.\n");
        assert!(unlabelled_paragraphs(&parse(&prose), &names).is_empty());
    }
}
#[cfg(test)]
mod void_tests {
    use super::*;
    use crate::{fidelity, rollup_cluster, rollup_prep, test_support::Memory, traits::Searcher};

    const ROLLUP: &str = "2026-09-22-aaaaaaaaaaaa";
    const VOIDED: &str = "2026-09-21-bbbbbbbbbbbb";
    const NOTE: &str = "2026-09-26-cccccccccccc";

    fn diary(with_note: bool) -> Memory {
        let mut d = Memory::default();
        let entries = [
            (
                ROLLUP,
                format!(
                    "# ROLLUP: t — state as of 2026-09-23\nTags: t.\n## Supersedes\n- {VOIDED}.md\n- 2026-09-21-dddddddddddd.md\n"
                ),
            ),
            (VOIDED, "# B\nTags: t, no-compress.\nbody".to_owned()),
            ("2026-09-21-dddddddddddd", "# D\nTags: t.\nbody".to_owned()),
            ("2026-09-21-eeeeeeeeeeee", "# E\nTags: t.\nbody".to_owned()),
            ("2026-09-21-ffffffffffff", "# F\nTags: t.\nbody".to_owned()),
        ];
        for (name, text) in entries {
            d.0.insert(name.into(), text);
        }
        if with_note {
            d.0.insert(
                NOTE.into(),
                format!("# Void\nTags: supersession-void, t.\nThe rollup {ROLLUP}.md is not voided.\n## Void supersedes\n- {VOIDED}.md — reason\n## After\n- 2026-09-21-dddddddddddd.md\n"),
            );
        }
        d
    }
    fn names(d: &Memory) -> Vec<String> {
        d.0.keys().cloned().collect()
    }
    #[test]
    fn only_the_void_section_of_a_tagged_entry_counts() {
        let d = diary(true);
        assert_eq!(
            voided(&d, &names(&d)).unwrap(),
            Voids::from([(VOIDED.to_owned(), NOTE.to_owned())])
        );
        let untagged = Entry::parse(NOTE, d.0[NOTE].replace("supersession-void, ", ""));
        assert!(void_listed(&untagged, &names(&d)).is_empty());
    }
    #[test]
    fn floor_and_depth_skip_voided_supersessions() {
        for (note, violations, share) in [(false, 1, 1.0), (true, 0, 1.0)] {
            let d = diary(note);
            let rollup = Entry::parse(ROLLUP, d.0[ROLLUP].clone());
            let floor = fidelity::floor(&rollup, &d);
            assert_eq!(floor["violations"].as_array().unwrap().len(), violations);
            assert_eq!(fidelity::depth(&rollup, &d)["primary_share"], share);
        }
    }
    #[test]
    fn a_voided_entry_is_substantive_again_for_prep() {
        let mut d = diary(true);
        let plain = d.0[VOIDED].replace(", no-compress", "");
        d.0.insert(VOIDED.into(), plain.clone());
        let recent = rollup_prep::recent(&d, "2026-09-23", 3).unwrap();
        assert!(recent.substantive.contains(&VOIDED.to_owned()));
        d.0.remove(NOTE);
        let recent = rollup_prep::recent(&d, "2026-09-23", 3).unwrap();
        assert!(!recent.substantive.contains(&VOIDED.to_owned()));
    }
    struct NoHits;
    impl Searcher for NoHits {
        type Error = String;
        fn search(
            &self,
            _: &str,
            _: usize,
            _: f64,
        ) -> Result<Vec<crate::search_hit::SearchHit>, String> {
            Ok(Vec::new())
        }
    }
    #[test]
    fn cluster_follows_a_voided_entry_as_an_uncompressed_primary() {
        let mut d = diary(true);
        let plain = d.0[VOIDED].replace(", no-compress", "");
        d.0.insert(VOIDED.into(), plain);
        let request = rollup_cluster::Request {
            thread: "t",
            tags: &[],
            as_of: "2026-09-26",
        };
        let cluster = rollup_cluster::pull(&d, &NoHits, &request).unwrap();
        assert_eq!(cluster.voided, [VOIDED]);
        assert_eq!(cluster.member(VOIDED).unwrap().label(), "primary");
        let prompt = crate::rollup_validate::write_prompt(
            "policy",
            &crate::rollup_validate::Target {
                thread: "t",
                as_of: "2026-09-26",
            },
            &cluster,
            None,
        );
        assert!(prompt.contains(&format!(
            "{VOIDED}.md [PRIMARY (an earlier rollup's supersession of it was voided"
        )));
    }
    /// M5b item 0: a prior rollup's referenced-only entries are pulled into the cluster; M5b-1
    /// item 5 (review S-1): as reference-only, even without `no-compress`, so the write prompt
    /// labels them so and the validator refuses to supersede them.
    #[test]
    fn cluster_pulls_a_prior_rollups_referenced_only_entries() {
        let mut d = diary(false);
        let referenced = "2026-09-21-eeeeeeeeeeee";
        let text = format!(
            "{}\nReferenced, NOT superseded: {referenced}.md\n",
            d.0[ROLLUP]
        );
        d.0.insert(ROLLUP.into(), text);
        let request = rollup_cluster::Request {
            thread: "t",
            tags: &[],
            as_of: "2026-09-26",
        };
        let cluster = rollup_cluster::pull(&d, &NoHits, &request).unwrap();
        let member = cluster.member(referenced).unwrap();
        assert!(!member.entry.tags.contains("no-compress"));
        assert_eq!(
            (member.label(), member.supersedable),
            ("reference-only", false)
        );
        let target = crate::rollup_validate::Target {
            thread: "t",
            as_of: "2026-09-26",
        };
        let prompt = crate::rollup_validate::write_prompt("policy", &target, &cluster, None);
        assert!(prompt.contains(&format!(
            "{referenced}.md [REFERENCE ONLY (an earlier rollup referenced it without superseding"
        )));
        let body = format!(
            "# ROLLUP: t — state as of 2026-09-26\nTags: t.\n## Supersedes\n- 2026-09-21-dddddddddddd.md\n- {referenced}.md\n"
        );
        let v = crate::rollup_validate::validate(&body, &target, &cluster, &names(&d));
        assert!(
            v.problems
                .iter()
                .any(|p| p.starts_with(&format!("Supersedes {referenced}: not a"))),
            "{:?}",
            v.problems
        );
    }
    /// M4d-2a item 9 (L-6): a rollup dated after the note may supersede the entry again.
    #[test]
    fn a_void_does_not_reach_a_later_rollup() {
        let mut d = diary(true);
        let plain = d.0[VOIDED].replace(", no-compress", "");
        d.0.insert(VOIDED.into(), plain);
        let later = "2026-09-28-aaaaaaaaaaaa";
        let text = d.0[ROLLUP].replace("2026-09-22", "2026-09-28");
        d.0.insert(later.into(), text);
        let voids = voided(&d, &names(&d)).unwrap();
        assert!(is_voided(&voids, VOIDED, ROLLUP));
        assert!(!is_voided(&voids, VOIDED, later));
        assert!(is_voided(&voids, VOIDED, "synthetic-rollup"));
        let recent = rollup_prep::recent(&d, "2026-09-28", 7).unwrap();
        assert!(
            !recent.substantive.contains(&VOIDED.to_owned()),
            "compressed again by the later rollup"
        );
        let recent = rollup_prep::recent(&d, "2026-09-26", 7).unwrap();
        assert!(
            recent.substantive.contains(&VOIDED.to_owned()),
            "as of the note, only the voided rollup"
        );
        let request = rollup_cluster::Request {
            thread: "t",
            tags: &[],
            as_of: "2026-09-28",
        };
        let cluster = rollup_cluster::pull(&d, &NoHits, &request).unwrap();
        assert!(cluster.voided.is_empty(), "{:?}", cluster.voided);
    }
    /// M4d-2a.1 item 4 (review L-3): an undated or short-named note voids nothing and is
    /// reported; a same-day note voids a rollup only when its file sorts after the rollup's.
    #[test]
    fn a_void_note_needs_a_date_and_orders_same_day_files() {
        for undated in ["void-note", "short", "2026-9-26-cccc"] {
            let mut d = diary(false);
            let note = diary(true).0[NOTE].clone();
            d.0.insert(undated.into(), note);
            assert!(voided(&d, &names(&d)).unwrap().is_empty(), "{undated}");
            assert_eq!(undated_void_notes(&d, &names(&d)).unwrap(), [undated]);
        }
        assert!(
            undated_void_notes(&diary(true), &names(&diary(true)))
                .unwrap()
                .is_empty()
        );
        let voids = Voids::from([(VOIDED.to_owned(), NOTE.to_owned())]);
        assert!(
            is_voided(&voids, VOIDED, "2026-09-26-000000000000"),
            "same day, earlier file"
        );
        assert!(
            !is_voided(&voids, VOIDED, "2026-09-26-ffffffffffff"),
            "same day, later file"
        );
        assert!(!is_voided(&voids, VOIDED, "2026-09-27-000000000000"));
    }
    #[test]
    fn depth_does_not_descend_into_a_voided_rollup_source() {
        let old = "2026-09-20-999999999999";
        for (note, depth) in [(false, 2), (true, 1)] {
            let mut d = diary(note);
            d.0.insert(old.into(), "# ROLLUP: t — state as of 2026-09-20\nTags: t.\n## Supersedes\n- 2026-09-21-eeeeeeeeeeee.md\n".into());
            let text = d.0[ROLLUP].replace(VOIDED, old);
            d.0.insert(ROLLUP.into(), text.clone());
            if note {
                let void = d.0[NOTE].replace(VOIDED, old);
                d.0.insert(NOTE.into(), void);
            }
            let report = fidelity::depth(&Entry::parse(ROLLUP, text), &d);
            assert_eq!(report["depth"], depth, "note={note}");
        }
    }
}
