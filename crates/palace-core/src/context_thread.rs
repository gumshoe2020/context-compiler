//! Thread grouping for `palace context --layered` blocks 3 and 4 (CTX-1 §1, §4 item 3), pure and
//! deterministic: no model. It works on one diary directory (the one a search hit came from, so
//! a thread never crosses wings) with the supersession data the diary already holds and the
//! claim store's subjects.
//!
//! **The thread of a hit.** An entry that a live rollup supersedes (named in its `## Supersedes`
//! list before any referenced-only label, the supersession not voided by a `supersession-void`
//! note: `rollup::superseded_live`) is replaced by that rollup, the newest one when several list
//! it, repeatedly, so a hit on an old entry lands on the current consolidation. A titled rollup
//! (`ROLLUP: <thread> — state as of <date>`) stands for its thread; the thread's newest rollup
//! (the latest as-of date, then the name) is the one shown. Any other entry has no thread: it is
//! a top entry hit.
//!
//! **Newer entries of a thread.** Entries of the same diary that are not rollups, dated (by the
//! date prefix of their name) strictly after the rollup's as-of date and superseded by no live
//! rollup, that belong to the thread: an entry belongs when it shares a tag with the thread
//! (the rollup's tags and those of the entries it supersedes, directly or through an earlier
//! rollup, without `rollup`,
//! `ongoing-thread` and `no-compress`, plus the thread name itself, all lowercase), or when it
//! states a live claim whose subject (lowercase) is the subject of a live claim stated by the
//! rollup or by an entry it supersedes. Newest first: date, then name, descending.
use crate::rollup::{self, Entry, Voids};
use crate::rollup_prep::entry_date;
use std::collections::{BTreeMap, BTreeSet};

/// Tags every rollup carries: they name no thread.
const ROLLUP_TAGS: &[&str] = &["rollup", "ongoing-thread", "no-compress"];

/// One diary directory, its entries parsed, with who supersedes whom.
pub struct ThreadDiary {
    entries: BTreeMap<String, Entry>,
    /// Entry name → the live rollups that supersede it.
    superseded_by: BTreeMap<String, Vec<String>>,
}

impl ThreadDiary {
    pub fn new(entries: Vec<Entry>, voids: &Voids) -> Self {
        let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        let mut superseded_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in entries.iter().filter(|e| e.is_rollup) {
            for source in rollup::superseded_live(entry, &names, voids) {
                if source != entry.name {
                    superseded_by
                        .entry(source)
                        .or_default()
                        .push(entry.name.clone());
                }
            }
        }
        let entries = entries.into_iter().map(|e| (e.name.clone(), e)).collect();
        Self {
            entries,
            superseded_by,
        }
    }

    /// The rollup standing for the thread of entry `name` (with or without `.md`), if any.
    pub fn thread_of(&self, name: &str) -> Option<(&Entry, String)> {
        let mut current = name.strip_suffix(".md").unwrap_or(name).to_owned();
        for _ in 0..=self.entries.len() {
            match self
                .superseded_by
                .get(&current)
                .and_then(|by| by.iter().max())
            {
                Some(newer) => current = newer.clone(),
                None => break,
            }
        }
        let entry = self.entries.get(&current)?;
        let thread = rollup::rollup_thread(entry)?;
        Some(self.newest_rollup(entry, thread))
    }

    /// The newest titled rollup of the same thread (case-insensitive), by as-of date then name.
    fn newest_rollup<'a>(&'a self, entry: &'a Entry, thread: String) -> (&'a Entry, String) {
        let same = |e: &&Entry| {
            rollup::rollup_thread(e).is_some_and(|t| t.to_lowercase() == thread.to_lowercase())
        };
        let newest = self
            .entries
            .values()
            .filter(same)
            .max_by_key(|e| (rollup::title_parts(e).1, e.name.clone()))
            .unwrap_or(entry);
        let thread = rollup::rollup_thread(newest).unwrap_or(thread);
        (newest, thread)
    }

    /// The entries `rollup` supersedes (live), and theirs in turn when they are rollups.
    fn sources(&self, rollup: &Entry) -> Vec<&Entry> {
        let mut found: Vec<&Entry> = Vec::new();
        let mut pending = vec![rollup.name.clone()];
        while let Some(by) = pending.pop() {
            for (name, rollups) in &self.superseded_by {
                let Some(entry) = self.entries.get(name) else {
                    continue;
                };
                if rollups.contains(&by) && !found.iter().any(|e| e.name == *name) {
                    found.push(entry);
                    pending.push(name.clone());
                }
            }
        }
        found
    }

    /// The thread's tags: the rollup's and its sources', less the rollup tags, plus its name.
    pub fn thread_tags(&self, rollup: &Entry, thread: &str) -> BTreeSet<String> {
        let mut tags: BTreeSet<String> = std::iter::once(rollup)
            .chain(self.sources(rollup))
            .flat_map(|e| e.tags.iter().cloned())
            .filter(|t| !ROLLUP_TAGS.contains(&t.as_str()))
            .collect();
        tags.insert(thread.to_lowercase());
        tags
    }

    /// The thread's newer entries, newest first (see the module notes). `subjects` gives the
    /// lowercase subjects of the live claims an entry (name without `.md`) states.
    pub fn newer<F>(&self, rollup: &Entry, thread: &str, subjects: F) -> Vec<&Entry>
    where
        F: Fn(&str) -> BTreeSet<String>,
    {
        let as_of = rollup::title_parts(rollup).1;
        let tags = self.thread_tags(rollup, thread);
        let thread_subjects: BTreeSet<String> = std::iter::once(rollup)
            .chain(self.sources(rollup))
            .flat_map(|e| subjects(&e.name))
            .collect();
        let belongs = |e: &Entry| {
            !e.tags.is_disjoint(&tags) || !subjects(&e.name).is_disjoint(&thread_subjects)
        };
        let mut newer: Vec<&Entry> = self
            .entries
            .values()
            .filter(|e| !e.is_rollup && !self.superseded_by.contains_key(&e.name))
            .filter(|e| entry_date(&e.name).is_some_and(|d| d > as_of.as_str()))
            .filter(|e| belongs(e))
            .collect();
        newer.sort_by(|a, b| b.name.cmp(&a.name));
        newer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "2025-02-03-1a2b3c4d5e6f";
    const OLDER_ROLLUP: &str = "2025-02-04-aaaaaaaaaaaa";
    const ROLLUP: &str = "2025-02-10-3c4d5e6f7a8b";
    const NEW: &str = "2025-03-01-0a1b2c3d4e5f";
    const NEW_BY_CLAIM: &str = "2025-03-02-1b2c3d4e5f6a";
    const OTHER: &str = "2025-03-03-2c3d4e5f6a7b";
    const SAME_DAY: &str = "2025-02-10-4d5e6f7a8b9c";
    const LOOSE: &str = "2025-01-01-5e6f7a8b9c0d";

    fn entry(name: &str, text: &str) -> Entry {
        Entry::parse(&format!("{name}.md"), text.to_owned())
    }

    fn diary() -> ThreadDiary {
        ThreadDiary::new(
            vec![
                entry(OLD, "# kickoff\n\nbody\n\nTags: tern-harbour, survey."),
                entry(
                    OLDER_ROLLUP,
                    &format!(
                        "# ROLLUP: Tern harbour survey — state as of 2025-02-04\n\n## Supersedes\n- {OLD}.md\n\nTags: tern harbour survey, rollup."
                    ),
                ),
                entry(
                    ROLLUP,
                    &format!(
                        "# ROLLUP: Tern harbour survey — state as of 2025-02-10\n\n## Supersedes\n- {OLDER_ROLLUP}.md\n\nTags: tern harbour survey, rollup, ongoing-thread."
                    ),
                ),
                entry(NEW, "# date\n\nbody\n\nTags: survey, boats."),
                entry(NEW_BY_CLAIM, "# signing\n\nbody\n\nTags: paperwork."),
                entry(
                    OTHER,
                    "# quince\n\nbody\n\nTags: quince-press, ongoing-thread.",
                ),
                entry(SAME_DAY, "# same day\n\nbody\n\nTags: tern-harbour."),
                entry(LOOSE, "# loose\n\nbody\n\nTags: choir."),
            ],
            &Voids::new(),
        )
    }

    fn subjects(name: &str) -> BTreeSet<String> {
        match name {
            OLD | NEW_BY_CLAIM => BTreeSet::from(["corin dask".to_owned()]),
            OTHER => BTreeSet::from(["quince press".to_owned()]),
            _ => BTreeSet::new(),
        }
    }

    #[test]
    fn a_superseded_hit_lands_on_the_newest_rollup_of_its_thread() {
        let d = diary();
        for hit in [OLD, OLDER_ROLLUP, ROLLUP] {
            let (rollup, thread) = d.thread_of(&format!("{hit}.md")).unwrap();
            assert_eq!(
                (rollup.name.as_str(), thread.as_str()),
                (ROLLUP, "Tern harbour survey")
            );
        }
        assert!(d.thread_of(NEW).is_none() && d.thread_of(LOOSE).is_none());
        assert!(d.thread_of("2099-01-01-ffffffffffff").is_none());
    }

    #[test]
    fn newer_entries_share_a_tag_or_a_claim_subject_and_follow_the_as_of_date() {
        let d = diary();
        let (rollup, thread) = d.thread_of(OLD).unwrap();
        let tags = d.thread_tags(rollup, &thread);
        assert!(
            tags.contains("survey") && tags.contains("tern harbour survey"),
            "{tags:?}"
        );
        assert!(
            !tags.contains("rollup") && !tags.contains("ongoing-thread"),
            "{tags:?}"
        );
        let newer: Vec<&str> = d
            .newer(rollup, &thread, subjects)
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(newer, [NEW_BY_CLAIM, NEW]);
    }

    #[test]
    fn a_voided_supersession_does_not_route() {
        let d = ThreadDiary::new(
            vec![
                entry(OLD, "# kickoff\n\nTags: tern-harbour."),
                entry(
                    ROLLUP,
                    &format!(
                        "# ROLLUP: Tern harbour survey — state as of 2025-02-10\n\n## Supersedes\n- {OLD}.md"
                    ),
                ),
            ],
            &Voids::from([(OLD.to_owned(), "2025-02-11-bbbbbbbbbbbb".to_owned())]),
        );
        assert!(d.thread_of(OLD).is_none());
        assert_eq!(d.thread_of(ROLLUP).unwrap().0.name, ROLLUP);
    }
}
