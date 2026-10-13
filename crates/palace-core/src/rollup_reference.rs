//! SETTLE-1d §6: a rollup supersedes only what it keeps. Pure and deterministic: no model.
//!
//! **The rule.** An entry the rollup's `## Supersedes` list names (`rollup::superseded_sources`)
//! stays superseded when every live specific it states is kept or cited by the rollup's claims:
//! the `## Claims` section holds the specific's value (its object) or its id. An entry with any
//! live specific the claims neither keep nor cite is **referenced** instead.
//!
//! **Where it is stored.** Where supersession is: the rollup's text. A referenced entry leaves
//! the superseded list (a line naming only referenced entries is dropped) and is named in the
//! same section in a paragraph of its own, last, under [`LIST_LABEL`] (SETTLE-1e §4): it opens
//! with the pinned label `Referenced only (not superseded):`, which every reader of supersession
//! already honours (the context's thread routing, rollup prep, the watch, fidelity), and a blank
//! line keeps it apart from the superseded list and from any referenced-only paragraph the model
//! wrote. The rollup also gains a `## Referenced entries` section, right after `## Supersedes`,
//! naming each with the claims it left uncited. [`layout`] rebuilds the paragraph from that
//! section, so a rollup marked the SETTLE-1d way is rewritten to this layout, once.
use crate::rollup::{Entry, REFERENCED_ONLY_LABEL, named_entries, superseded_sources};
use std::collections::BTreeSet;

/// The heading of the list of referenced entries.
pub const HEADING: &str = "## Referenced entries";
/// The label line of the referenced entries' paragraph in `## Supersedes`.
pub const LIST_LABEL: &str = "Referenced only (not superseded): entries whose live specifics the claims neither keep nor cite:";

/// A live specific, with the entry that states it.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveSpecific {
    pub id: String,
    pub object: String,
    pub entry: String,
}

/// A superseded source that must be referenced instead: its name and the live specifics the
/// rollup's claims neither keep nor cite.
#[derive(Clone, Debug, PartialEq)]
pub struct Uncited {
    pub entry: String,
    pub claims: Vec<String>,
}

/// The sources of `entry` to reference instead of supersede (see the module notes), in the order
/// the Supersedes list names them.
pub fn referenced(entry: &Entry, names: &[String], specifics: &[LiveSpecific]) -> Vec<Uncited> {
    let kept = |s: &LiveSpecific| claims_keep(&entry.text, &s.object, &s.id);
    superseded_sources(entry, names)
        .resolved
        .into_iter()
        .filter_map(|source| {
            let uncited: Vec<String> = specifics
                .iter()
                .filter(|s| bare(&s.entry) == source && !kept(s))
                .map(|s| s.id.clone())
                .collect();
            (!uncited.is_empty()).then_some(Uncited {
                entry: source,
                claims: uncited,
            })
        })
        .collect()
}
/// Whether the rollup's `## Claims` section keeps a specific's value or cites its id.
pub fn claims_keep(rollup: &str, value: &str, id: &str) -> bool {
    let claims = section(rollup, "## Claims");
    let value = value.trim();
    (!value.is_empty() && claims.contains(value)) || claims.contains(id)
}
fn bare(name: &str) -> &str {
    name.strip_suffix(".md").unwrap_or(name)
}

/// The lines under `heading` (a line starting with it), up to the next `## ` heading.
fn section(text: &str, heading: &str) -> String {
    let mut lines = text
        .lines()
        .skip_while(|l| !l.trim_end().starts_with(heading));
    lines.next();
    lines
        .take_while(|l| !l.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The entries named under `## Referenced entries` (none when the rollup has no such section).
pub fn referenced_entries(entry: &Entry, names: &[String]) -> Vec<String> {
    named_entries(&section(&entry.text, HEADING), names)
}

/// The rollup text with `uncited`'s entries moved from superseded to referenced (see the module
/// notes); unchanged when there is nothing to move or no `## Supersedes` heading.
pub fn mark(text: &str, names: &[String], uncited: &[Uncited]) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(start) = lines.iter().position(|l| is_supersedes_heading(l)) else {
        return text.to_owned();
    };
    if uncited.is_empty() {
        return text.to_owned();
    }
    let end = (start + 1..lines.len())
        .find(|&i| lines[i].starts_with('#'))
        .unwrap_or(lines.len());
    let moved: BTreeSet<&str> = uncited.iter().map(|u| u.entry.as_str()).collect();
    let section = supersedes_lines(&lines[start + 1..end], names, &moved);
    let mut out: Vec<String> = lines[..=start].iter().map(|l| (*l).to_owned()).collect();
    out.extend(section);
    let mut rest: Vec<String> = lines[end..].iter().map(|l| (*l).to_owned()).collect();
    let items = uncited.iter().map(|u| {
        format!(
            "- {}.md: live specifics its claims neither keep nor cite: {}",
            u.entry,
            u.claims.join(", ")
        )
    });
    match rest.iter().position(|l| l.trim_end() == HEADING) {
        Some(at) => {
            let close = (at + 1..rest.len())
                .find(|&i| rest[i].starts_with('#'))
                .unwrap_or(rest.len());
            let last = (at + 1..close)
                .rev()
                .find(|&i| !rest[i].trim().is_empty())
                .unwrap_or(at);
            rest.splice(last + 1..last + 1, items);
        }
        None => {
            let block: Vec<String> = [HEADING.to_owned()]
                .into_iter()
                .chain(items)
                .chain([String::new()])
                .collect();
            out.extend(block);
        }
    }
    out.extend(rest);
    layout(&out.join("\n"), names)
}
/// The rollup text with the entries named under `## Referenced entries` listed in their own
/// paragraph, last in `## Supersedes` (see the module notes); their lines elsewhere in the
/// section (a `- <entry>.md` line, or a label line naming only them) are dropped. Unchanged when
/// no entry is referenced or there is no `## Supersedes` heading; a second pass changes nothing.
pub fn layout(text: &str, names: &[String]) -> String {
    let referenced = referenced_entries(&Entry::parse("rollup", text.to_owned()), names);
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(start) = lines.iter().position(|l| is_supersedes_heading(l)) else {
        return text.to_owned();
    };
    if referenced.is_empty() {
        return text.to_owned();
    }
    let end = (start + 1..lines.len())
        .find(|&i| lines[i].starts_with('#'))
        .unwrap_or(lines.len());
    let files: Vec<String> = referenced.iter().map(|e| format!("{e}.md")).collect();
    let mut section: Vec<String> = Vec::new();
    for line in &lines[start + 1..end] {
        let blank = line.trim().is_empty();
        if names_only_referenced(line, &files)
            || (blank && section.last().is_some_and(|l| l.trim().is_empty()))
        {
            continue;
        }
        section.push((*line).to_owned());
    }
    while section.last().is_some_and(|l| l.trim().is_empty()) {
        section.pop();
    }
    section.extend([String::new(), LIST_LABEL.to_owned()]);
    section.extend(files.iter().map(|f| format!("- {f}")));
    if end < lines.len() {
        section.push(String::new());
    }
    let mut out: Vec<String> = lines[..=start].iter().map(|l| (*l).to_owned()).collect();
    out.extend(section);
    out.extend(lines[end..].iter().map(|l| (*l).to_owned()));
    out.join("\n")
}
/// A line the referenced entries' paragraph owns: its label, `- <file>` for a referenced file, or
/// the SETTLE-1d paragraph `Referenced only (not superseded): <files>` naming only referenced ones.
fn names_only_referenced(line: &str, files: &[String]) -> bool {
    let line = line.trim();
    if line == LIST_LABEL {
        return true;
    }
    if let Some(file) = line.strip_prefix("- ") {
        return files.iter().any(|f| f == file);
    }
    line.strip_prefix(REFERENCED_ONLY_LABEL)
        .is_some_and(|rest| {
            let listed: Vec<&str> = rest.split(',').map(str::trim).collect();
            !rest.trim().is_empty() && listed.iter().all(|l| files.iter().any(|f| f == l))
        })
}
pub(crate) fn is_supersedes_heading(line: &str) -> bool {
    let line = line.trim();
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=3).contains(&hashes)
        && line[hashes..]
            .trim_start()
            .to_uppercase()
            .starts_with("SUPERSEDES")
}
/// The section's lines without those of the superseded list that name only moved entries.
fn supersedes_lines(section: &[&str], names: &[String], moved: &BTreeSet<&str>) -> Vec<String> {
    let first_label = section
        .iter()
        .position(|l| starts_with_label(l))
        .unwrap_or(section.len());
    section
        .iter()
        .enumerate()
        .filter(|(i, line)| {
            let named = named_entries(line, names);
            *i >= first_label
                || named.is_empty()
                || !named.iter().all(|n| moved.contains(n.as_str()))
        })
        .map(|(_, l)| (*l).to_owned())
        .collect()
}
fn starts_with_label(line: &str) -> bool {
    let line = line.trim_start().trim_start_matches(['-', '*', '+', ' ']);
    line.trim_start_matches("**")
        .starts_with(REFERENCED_ONLY_LABEL)
}
#[cfg(test)]
mod tests {
    use super::*;
    const KEPT: &str = "2031-04-02-aaaaaaaaaaaa";
    const LOOSE: &str = "2031-04-02-bbbbbbbbbbbb";
    const OTHER: &str = "2031-04-01-cccccccccccc";
    fn names() -> Vec<String> {
        [KEPT, LOOSE, OTHER].map(str::to_owned).to_vec()
    }
    fn rollup() -> String {
        format!(
            "# ROLLUP: Larkspur ferry — state as of 2031-04-04\n\n## Claims\n- C1. Larkspur ferry inspection | is scheduled on | May 21 — 2031-04-03 — {KEPT}.md\n\n## Current consolidated truth\nThe inspection is on May 21.\n\n## Supersedes\n- {KEPT}.md\n- {LOOSE}.md\n\n## Graph relations (proposed)\n```json relations\n[]\n```\n"
        )
    }
    fn specifics() -> Vec<LiveSpecific> {
        let s = |id: &str, object: &str, entry: &str| LiveSpecific {
            id: id.into(),
            object: object.into(),
            entry: format!("{entry}.md"),
        };
        vec![
            s("c_kept", "May 21", KEPT),
            s("c_crew", "3,200 crowns", LOOSE),
            s("c_cited", "Wenna Ostrow", LOOSE),
            s("c_elsewhere", "June 2", OTHER),
        ]
    }
    #[test]
    fn an_entry_with_an_uncited_specific_is_referenced() {
        let entry = Entry::parse("r", rollup());
        let found = referenced(&entry, &names(), &specifics());
        assert_eq!(
            found,
            [Uncited {
                entry: LOOSE.into(),
                claims: vec!["c_crew".into(), "c_cited".into()]
            }]
        );
        let cited = rollup().replace("— 2031-04-03 —", "— 2031-04-03 — c_crew c_cited —");
        assert!(referenced(&Entry::parse("r", cited), &names(), &specifics()).is_empty());
    }
    #[test]
    fn mark_moves_the_entry_and_lists_it() {
        let found = referenced(&Entry::parse("r", rollup()), &names(), &specifics());
        let marked = mark(&rollup(), &names(), &found);
        let entry = Entry::parse("r", marked.clone());
        let refs = superseded_sources(&entry, &names());
        assert_eq!(refs.resolved, [KEPT]);
        assert_eq!(refs.referenced_only, [LOOSE]);
        assert!(marked.contains(&format!(
            "- {KEPT}.md\n\n{LIST_LABEL}\n- {LOOSE}.md\n\n{HEADING}\n- {LOOSE}.md: live specifics its claims neither keep nor cite: c_crew, c_cited\n\n## Graph relations"
        )), "{marked}");
        assert_eq!(layout(&marked, &names()), marked);
        assert_eq!(referenced_entries(&entry, &names()), [LOOSE]);
        assert!(
            referenced(&entry, &names(), &specifics()).is_empty(),
            "a rescan finds nothing"
        );
        assert_eq!(mark(&marked, &names(), &[]), marked);
        assert!(crate::rollup::unclassified(&entry, &names()).is_empty());
        assert!(crate::rollup::unpinned_labels(&entry).is_empty());
    }
    #[test]
    fn an_existing_label_paragraph_and_list_are_extended() {
        let text = rollup().replace(
            &format!("- {LOOSE}.md\n\n## Graph"),
            &format!("- {LOOSE}.md\n\n{REFERENCED_ONLY_LABEL} {OTHER}.md (a separate thread)\n\n{HEADING}\n- earlier.md: x\n\n## Graph"),
        );
        let found = referenced(&Entry::parse("r", text.clone()), &names(), &specifics());
        let marked = mark(&text, &names(), &found);
        assert!(
            marked.contains(&format!(
                "{REFERENCED_ONLY_LABEL} {OTHER}.md (a separate thread)\n\n{LIST_LABEL}\n- {LOOSE}.md\n\n{HEADING}"
            )),
            "{marked}"
        );
        assert!(marked.contains(&format!("- earlier.md: x\n- {LOOSE}.md: live specifics")));
        let refs = superseded_sources(&Entry::parse("r", marked), &names());
        assert_eq!(refs.resolved, [KEPT]);
    }
    #[test]
    fn a_rollup_marked_the_settle1d_way_is_laid_out_once() {
        let listed = format!(
            "{HEADING}\n- {LOOSE}.md: live specifics its claims neither keep nor cite: c_crew\n\n## Graph"
        );
        let appended = rollup().replace(
            &format!("- {LOOSE}.md\n\n## Graph"),
            &format!(
                "\n{REFERENCED_ONLY_LABEL} {OTHER}.md (a separate thread)\n- {LOOSE}.md\n\n{listed}"
            ),
        );
        let paragraph = rollup().replace(
            &format!("- {LOOSE}.md\n\n## Graph"),
            &format!("\n{REFERENCED_ONLY_LABEL} {LOOSE}.md\n\n{listed}"),
        );
        for (old, model) in [
            (
                appended,
                format!("{REFERENCED_ONLY_LABEL} {OTHER}.md (a separate thread)\n\n"),
            ),
            (paragraph, String::new()),
        ] {
            let laid = layout(&old, &names());
            let want = format!(
                "## Supersedes\n- {KEPT}.md\n\n{model}{LIST_LABEL}\n- {LOOSE}.md\n\n{HEADING}"
            );
            assert!(laid.contains(&want), "{laid}");
            assert_eq!(
                layout(&laid, &names()),
                laid,
                "a second pass changes nothing"
            );
            let entry = Entry::parse("r", laid);
            let refs = superseded_sources(&entry, &names());
            assert_eq!(refs.resolved, [KEPT]);
            assert!(refs.referenced_only.contains(&LOOSE.to_owned()));
            assert!(crate::rollup::unclassified(&entry, &names()).is_empty());
            assert!(crate::rollup::unpinned_labels(&entry).is_empty());
            assert!(crate::rollup::unlabelled_paragraphs(&entry, &names()).is_empty());
        }
        assert_eq!(layout(&rollup(), &names()), rollup(), "nothing referenced");
    }
    #[test]
    fn the_thread_routing_sends_only_superseded_hits_to_the_rollup() {
        use crate::context_thread::ThreadDiary;
        let found = referenced(&Entry::parse("r", rollup()), &names(), &specifics());
        let rollup_name = "2031-04-04-dddddddddddd";
        let mut all = names();
        all.push(rollup_name.into());
        let marked = mark(&rollup(), &all, &found);
        let entries = vec![
            Entry::parse(KEPT, "# Larkspur ferry inspection\n\nMay 21.".into()),
            Entry::parse(LOOSE, "# Larkspur crew\n\n3,200 crowns.".into()),
            Entry::parse(rollup_name, marked),
        ];
        let diary = ThreadDiary::new(entries, &Default::default());
        assert_eq!(
            diary.thread_of(KEPT).map(|(e, _)| e.name.clone()),
            Some(rollup_name.into())
        );
        assert!(
            diary.thread_of(LOOSE).is_none(),
            "a referenced entry is its own hit"
        );
    }
}
