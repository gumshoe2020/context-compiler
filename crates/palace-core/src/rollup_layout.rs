//! SETTLE-1f §1: one layout for the entries a rollup references without superseding. Pure.
//!
//! **The layout** (prompts/rollup.md, "SUPERSEDES LAYOUT", pinned to this module by a test):
//! `## Supersedes` lists the superseded entries, one `- <file>` line each; each paragraph of
//! referenced entries follows, a blank line before it, opening with a label line that begins with
//! the pinned label and names no entry, then one `- <file>` line per entry (a reason may follow the
//! name). The commit step's marking (`rollup_reference::layout`) writes its own paragraph that
//! way, last. No other paragraph of the section names an entry.
//!
//! **The mend** ([`mend`]), mechanical, before a reply is validated: a referenced-only label that
//! opens a line in another wording becomes the pinned one; a label line that does not open a
//! paragraph gets a blank line before it; entries named on a label line (the older single-label
//! form) move to their own `- <file>` lines under it. Which entries are superseded and which are
//! referenced is the same before and after, for every reader of supersession. A paragraph that
//! names entries without a label is not mended: the validator rejects it.
use crate::rollup::{REFERENCED_ONLY_LABEL, entry_spans, opening_label};
use crate::rollup_reference::is_supersedes_heading;

/// The pinned example of the layout in prompts/rollup.md opens with this line.
pub const OPEN: &str = "<!-- supersedes-layout";
pub const CLOSE: &str = "<!-- /supersedes-layout -->";

/// The pinned example's lines, between its markers.
pub fn example(policy: &str) -> Option<&str> {
    let start = policy.find(OPEN)?;
    let body = &policy[start..];
    let body = &body[body.find('\n')? + 1..];
    Some(&body[..body.find(CLOSE)?])
}

/// `body` with its `## Supersedes` section mended (see the module notes), and one line per mend.
pub fn mend(body: &str, names: &[String]) -> (String, Vec<String>) {
    let lines: Vec<&str> = body.split('\n').collect();
    let Some(start) = lines.iter().position(|l| is_supersedes_heading(l)) else {
        return (body.to_owned(), Vec::new());
    };
    let end = (start + 1..lines.len())
        .find(|&i| lines[i].starts_with('#'))
        .unwrap_or(lines.len());
    let mut out: Vec<String> = lines[..=start].iter().map(|l| (*l).to_owned()).collect();
    let mut mends = Vec::new();
    let mut after_blank = true;
    for line in &lines[start + 1..end] {
        match opening_label(line) {
            Some(label) => {
                if !after_blank {
                    out.push(String::new());
                    mends.push("a blank line before the referenced-only label".to_owned());
                }
                out.extend(label_paragraph(line, label, names, &mut mends));
            }
            None => out.push((*line).to_owned()),
        }
        after_blank = line.trim().is_empty();
    }
    out.extend(lines[end..].iter().map(|l| (*l).to_owned()));
    (out.join("\n"), mends)
}

/// A line opening with a referenced-only label, laid out: the pinned label line, then one item
/// per entry it names.
fn label_paragraph(
    line: &str,
    (start, end): (usize, usize),
    names: &[String],
    mends: &mut Vec<String>,
) -> Vec<String> {
    let label = &line[start..end];
    if label != REFERENCED_ONLY_LABEL {
        mends.push(format!(
            "label \"{label}\" replaced by \"{REFERENCED_ONLY_LABEL}\""
        ));
    }
    let rest = line[end..]
        .trim_start_matches(' ')
        .trim_start_matches("**")
        .trim();
    let spans = entry_spans(rest, names);
    let Some(first) = spans.first() else {
        return vec![with_text(rest)];
    };
    let mut paragraph = vec![with_text(note(&rest[..first.0]))];
    for (i, (_, stop, name)) in spans.iter().enumerate() {
        let next = spans.get(i + 1).map_or(rest.len(), |s| s.0);
        let reason = note(&rest[*stop..next]);
        let gap = if reason.is_empty() { "" } else { " " };
        paragraph.push(format!("- {name}.md{gap}{reason}"));
    }
    let listed: Vec<&str> = spans.iter().map(|s| s.2.as_str()).collect();
    mends.push(format!(
        "{} listed one per line under the label",
        listed.join(", ")
    ));
    paragraph
}

/// The pinned label, then `text` after a space when there is any.
fn with_text(text: &str) -> String {
    match text {
        "" => REFERENCED_ONLY_LABEL.to_owned(),
        text => format!("{REFERENCED_ONLY_LABEL} {text}"),
    }
}

/// The text between two names, without the separators of a list (`,` `;` `.` `and` `or`).
fn note(text: &str) -> &str {
    let mut text = text.trim().trim_start_matches([',', ';']).trim();
    loop {
        let shorter = text
            .strip_suffix(" and")
            .or_else(|| text.strip_suffix(" or"))
            .or_else(|| text.strip_suffix([',', ';', '.']))
            .map(str::trim_end);
        match shorter {
            Some(s) if s != text => text = s,
            _ => break,
        }
    }
    if text == "and" || text == "or" {
        ""
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollup::{
        Entry, superseded_sources, unclassified, unlabelled_paragraphs, unpinned_labels,
    };
    use crate::rollup_reference::{HEADING, LIST_LABEL, layout};

    const A: &str = "2026-09-20-aaaaaaaaaaaa";
    const B: &str = "2026-09-21-bbbbbbbbbbbb";
    const C: &str = "2026-09-19-cccccccccccc";
    const D: &str = "2026-09-21-dddddddddddd";

    fn names() -> Vec<String> {
        [A, B, C, D].map(str::to_owned).to_vec()
    }
    fn policy() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/rollup.md"
        ))
        .unwrap()
    }
    fn rollup(section: &str) -> String {
        format!(
            "# ROLLUP: t — state as of 2031-04-04\n\n## Claims\n- C1. x | y | z — 2026-09-20 — {A}.md\n\n{section}\n## Open items\n- none\n"
        )
    }
    /// (superseded, referenced) as every reader of supersession reads them.
    fn read(text: &str) -> (Vec<String>, Vec<String>) {
        let refs = superseded_sources(&Entry::parse("r", text.to_owned()), &names());
        (refs.resolved, refs.referenced_only)
    }
    fn clean(text: &str) -> bool {
        let entry = Entry::parse("r", text.to_owned());
        unlabelled_paragraphs(&entry, &names()).is_empty()
            && unpinned_labels(&entry).is_empty()
            && unclassified(&entry, &names()).is_empty()
    }

    /// The prompt's pinned example is the layout: every reader reads it as written, the mend
    /// leaves it alone, and the commit step's marking adds its own paragraph in the same layout.
    #[test]
    fn the_prompts_example_is_the_layout() {
        let policy = policy();
        let example = example(&policy).expect("the pinned example");
        let text = rollup(example);
        assert!(clean(&text), "{text}");
        assert_eq!(
            read(&text),
            (vec![A.into(), B.into()], vec![C.into(), D.into()])
        );
        assert_eq!(mend(&text, &names()), (text.clone(), vec![]));
        let marked = text.replacen(
            &format!("- {B}.md\n"),
            "",
            1,
        ).replace("## Open items", &format!("{HEADING}\n- {B}.md: live specifics its claims neither keep nor cite: c_x\n\n## Open items"));
        let laid = layout(&marked, &names());
        assert!(
            laid.contains(&format!(
                "{D}.md — a separate thread\n\n{LIST_LABEL}\n- {B}.md\n\n{HEADING}"
            )),
            "{laid}"
        );
        assert!(clean(&laid), "{laid}");
        assert_eq!(mend(&laid, &names()), (laid.clone(), vec![]));
    }

    /// The older single-label form, near-miss labels and a label straight after the list are
    /// mended into the layout; what is superseded and what is referenced does not change.
    #[test]
    fn near_misses_are_mended_and_read_the_same() {
        let cases = [
            format!("## Supersedes\n- {A}.md\n\n{REFERENCED_ONLY_LABEL} {C}.md, {D}.md\n"),
            format!(
                "## Supersedes\n- {A}.md\n\nReferenced only (no-compress): {C}.md (the record), and {D}.md.\n"
            ),
            format!(
                "## Supersedes\n- {A}.md\n- **Referenced, NOT superseded:** {C}.md; {D}.md (a separate thread)\n\nNo reversal resolved.\n"
            ),
        ];
        let wants = [
            format!("## Supersedes\n- {A}.md\n\n{REFERENCED_ONLY_LABEL}\n- {C}.md\n- {D}.md\n"),
            format!(
                "## Supersedes\n- {A}.md\n\n{REFERENCED_ONLY_LABEL}\n- {C}.md (the record)\n- {D}.md\n"
            ),
            format!(
                "## Supersedes\n- {A}.md\n\n{REFERENCED_ONLY_LABEL}\n- {C}.md\n- {D}.md (a separate thread)\n\nNo reversal resolved.\n"
            ),
        ];
        for (case, want) in cases.iter().zip(&wants) {
            let before = rollup(case);
            let (after, mends) = mend(&before, &names());
            assert_eq!(after, rollup(want), "{case}");
            assert!(!mends.is_empty());
            assert_eq!(read(&before), read(&after), "{case}");
            assert!(clean(&after), "{after}");
            assert_eq!(
                mend(&after, &names()).1,
                Vec::<String>::new(),
                "a second mend changes nothing"
            );
        }
    }

    /// A paragraph that names an entry without a label is not mended: the validator rejects it.
    #[test]
    fn an_unlabelled_name_is_left_for_the_validator() {
        let text = rollup(&format!(
            "## Supersedes\n- {A}.md\n\n{REFERENCED_ONLY_LABEL}\n- {C}.md\n\nThe entry {D}.md is referenced rather than superseded.\n"
        ));
        assert_eq!(mend(&text, &names()), (text.clone(), vec![]));
        let entry = Entry::parse("r", text);
        assert_eq!(
            unlabelled_paragraphs(&entry, &names()),
            [vec![D.to_owned()]]
        );
    }

    #[test]
    fn notes_lose_list_separators_only() {
        assert_eq!(note(", "), "");
        assert_eq!(note(" (the record), and "), "(the record)");
        assert_eq!(note(" — a separate thread."), "— a separate thread");
        assert_eq!(note(" or "), "");
    }
}
