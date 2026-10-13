//! SETTLE-1f §2: thread clusters stay on thread. Pure.
//!
//! **Why.** Before, every MemPalace hit for the thread's name and for each tag the select step
//! chose joined the cluster, whatever it said. The select step chooses among the tags the recent
//! entries share, which are often a person's name (a real run chose three tags, two of them
//! people's names, and its search hits brought 30 entries in, some with no tag at all).
//!
//! **The rule.** Besides the thread's lineage (the prior rollups' superseded and referenced
//! entries, which always stay), an entry joins only through thread-specific evidence ([`joins`]):
//! - a `continuing-thread:<thread>` tag (or `ongoing-thread:<thread>`, its other spelling);
//! - a tag that is the thread's name;
//! - a title that names the thread: the rule `prior_rollups` already applies to a rollup's title,
//!   here word by word, with `-` and `_` read as spaces (`kg-precision-audit` is named by the
//!   title "KG precision audit 2026-09-20"; `settle-1` by "SETTLE-1 live switch", not by
//!   "SETTLE-1d accepted").
//!
//! Tags and names compare word by word in the same way. A thread named by a generic tag
//! ([`GenericTags`], prompts/rollup-generic-tags.json) gets nothing through its tag or title, and
//! the select step's tags are never evidence: they are only searched, and a generic one not even
//! that.
use crate::rollup::Entry;
use serde_json::Value;
use std::collections::BTreeSet;

/// Tags shared across threads (prompts/rollup-generic-tags.json): never evidence.
#[derive(Clone, Debug, Default)]
pub struct GenericTags(BTreeSet<String>);
impl GenericTags {
    /// Every list of the JSON object (keys starting `_` are notes), `{person}` and `{assistant}`
    /// read as the names given.
    pub fn parse(json: &str, person: &str, assistant: &str) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(json).map_err(|e| format!("rollup generic tags: {e}"))?;
        let object = value
            .as_object()
            .ok_or("rollup generic tags: not a JSON object")?;
        let mut tags = BTreeSet::new();
        for (key, list) in object.iter().filter(|(k, _)| !k.starts_with('_')) {
            let list = list
                .as_array()
                .ok_or_else(|| format!("rollup generic tags: {key} is not a list"))?;
            for tag in list.iter().filter_map(Value::as_str) {
                let tag = tag
                    .replace("{person}", person)
                    .replace("{assistant}", assistant);
                tags.insert(words(&tag).join(" "));
            }
        }
        Ok(Self(tags))
    }
    pub fn contains(&self, tag: &str) -> bool {
        self.0.contains(&words(tag).join(" "))
    }
}

/// Why `entry` joins `thread`'s cluster (see the module notes), or `None`.
pub fn joins(entry: &Entry, thread: &str, generic: &GenericTags) -> Option<String> {
    let name = words(thread);
    if name.is_empty() {
        return None;
    }
    let named = |tag: &str| words(tag) == name;
    for tag in &entry.tags {
        let marked = ["continuing-thread:", "ongoing-thread:"]
            .iter()
            .find_map(|marker| tag.strip_prefix(marker));
        if marked.is_some_and(named) {
            return Some(format!("tag {tag}"));
        }
    }
    if generic.contains(thread) {
        return None;
    }
    if let Some(tag) = entry.tags.iter().find(|t| named(t)) {
        return Some(format!("tag {tag}"));
    }
    let title = words(&entry.title);
    title
        .windows(name.len())
        .any(|w| w == name.as_slice())
        .then(|| "title".to_owned())
}

/// Lowercase words: letters and digits, everything else a break.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generic() -> GenericTags {
        GenericTags::parse(
            r#"{"_about": "x", "people": ["{person}", "{assistant}"], "kinds": ["decision", "Continuing-Thread"]}"#,
            "callum",
            "fergus",
        )
        .unwrap()
    }
    fn entry(title: &str, tags: &str) -> Entry {
        Entry::parse(
            "2031-04-02-aaaaaaaaaaaa",
            format!("# {title}\nTags: {tags}.\nBody."),
        )
    }
    #[test]
    fn the_generic_list_reads_every_group_and_the_names() {
        let g = generic();
        for tag in [
            "callum",
            "Fergus",
            "decision",
            "continuing-thread",
            "continuing thread",
        ] {
            assert!(g.contains(tag), "{tag}");
        }
        assert!(!g.contains("garden-plan"));
        assert!(GenericTags::parse("[]", "a", "b").is_err());
        assert!(GenericTags::parse(r#"{"x": "y"}"#, "a", "b").is_err());
    }
    #[test]
    fn only_thread_specific_evidence_joins() {
        let g = generic();
        let joined = |title: &str, tags: &str| joins(&entry(title, tags), "garden-plan", &g);
        assert_eq!(
            joined("Beds", "callum, garden-plan"),
            Some("tag garden-plan".into())
        );
        assert_eq!(
            joined("Beds", "Garden_Plan"),
            Some("tag garden_plan".into())
        );
        assert_eq!(
            joined("Beds", "continuing-thread:garden-plan"),
            Some("tag continuing-thread:garden-plan".into())
        );
        assert_eq!(
            joined("Beds", "ongoing-thread: garden-plan"),
            Some("tag ongoing-thread: garden-plan".into())
        );
        assert_eq!(
            joined("Garden plan: the beds", "callum"),
            Some("title".into())
        );
        for (title, tags) in [
            ("Beds", "callum, fergus, decision, continuing-thread"),
            ("Beds", "continuing-thread:garden-plans, garden-plan-b"),
            ("Garden planning", "garden"),
            ("", ""),
        ] {
            assert_eq!(joined(title, tags), None, "{title} / {tags}");
        }
    }
    #[test]
    fn a_step_tag_or_title_is_not_the_thread() {
        let g = generic();
        let joined = |title: &str, tags: &str| joins(&entry(title, tags), "settle-1", &g);
        assert_eq!(joined("SETTLE-1d accepted", "settle-1d"), None);
        assert_eq!(joined("SETTLE-1 live switch", "x"), Some("title".into()));
    }
    #[test]
    fn a_generic_thread_joins_only_by_its_marker() {
        let g = generic();
        let joined = |title: &str, tags: &str| joins(&entry(title, tags), "fergus", &g);
        assert_eq!(joined("Fergus notes", "fergus"), None);
        assert_eq!(
            joined("x", "continuing-thread:fergus"),
            Some("tag continuing-thread:fergus".into())
        );
    }
}
