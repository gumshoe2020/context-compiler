//! Rollup stage 2: the Model's constrained thread choice among Rust-built candidates.
use crate::rollup::{Entry, rollup_thread};
use crate::rollup_prep::{Candidate, Recent};
use serde_json::Value;

pub const SELECT_MARKER: &str = "=== TASK: SELECT ONE THREAD ===";

#[derive(Debug, PartialEq)]
pub enum Selection {
    Thread {
        thread: String,
        reason: String,
        candidate_ids: Vec<usize>,
    },
    Bail {
        reason: String,
    },
}
impl Selection {
    /// Tags of the chosen candidates, in candidate order.
    pub fn tags(&self, candidates: &[Candidate]) -> Vec<String> {
        let Selection::Thread { candidate_ids, .. } = self else {
            return Vec::new();
        };
        candidates
            .iter()
            .filter(|c| candidate_ids.contains(&c.id))
            .map(|c| c.tag.clone())
            .collect()
    }
    /// The tags of the chosen candidates' entries, less every chosen candidate's grouping tag (a
    /// tag the whole group shares names no thread, M5b-11 item 3); sorted, each once.
    pub fn entry_tags(&self, candidates: &[Candidate], recent: &Recent) -> Vec<String> {
        let grouping = self.tags(candidates);
        let Selection::Thread { candidate_ids, .. } = self else {
            return Vec::new();
        };
        let chosen: Vec<&String> = candidates
            .iter()
            .filter(|c| candidate_ids.contains(&c.id))
            .flat_map(|c| &c.entries)
            .collect();
        let mut tags: Vec<String> = recent
            .entries
            .iter()
            .filter(|e| chosen.contains(&&e.name))
            .flat_map(|e| e.tags.iter().cloned())
            .filter(|t| !grouping.contains(t))
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }
}
/// The select prompt; `threads` are the threads the diary's rollups already carry (M5b-10 item 5),
/// of which only those `parse` accepts are listed (M5b-11 item 1).
pub fn prompt(
    policy: &str,
    recent: &Recent,
    candidates: &[Candidate],
    threads: &[String],
) -> String {
    let mut text = format!(
        "{policy}\n\n{SELECT_MARKER}\nAs of {}. {} recent entries, {} substantive. Candidate threads (tag, entries, distinct days):\n",
        recent.as_of,
        recent.entries.len(),
        recent.substantive.len()
    );
    for c in candidates {
        text.push_str(&candidate_line(c, recent));
    }
    let listed: Vec<&str> = threads
        .iter()
        .map(String::as_str)
        .filter(|t| valid_thread(t))
        .collect();
    if !listed.is_empty() {
        text.push_str(&format!(
            "\nThreads that already have a rollup (newest first): {}. When the entries you choose continue one of them, name the thread exactly as it is written there.\n",
            listed.join("; ")
        ));
    }
    text.push_str("\nChoose ONE thread per Step 1. Reply with one JSON object only: {\"thread\": \"<thread name: one of the listed names, or a new short kebab-case name>\", \"reason\": \"<one sentence>\", \"candidate_ids\": [<ids of the candidates it covers>]} or {\"bail\": true, \"reason\": \"<one sentence>\"}.\n");
    text
}
fn candidate_line(c: &Candidate, recent: &Recent) -> String {
    let titles: Vec<String> = c
        .entries
        .iter()
        .filter_map(|name| recent.entries.iter().find(|e| &e.name == name))
        .map(|e| format!("{} {}", e.name, e.title))
        .collect();
    format!(
        "[{}] tag: {} — {} entries over {} days\n    {}\n",
        c.id,
        c.tag,
        c.entries.len(),
        c.days,
        titles.join("\n    ")
    )
}
/// First JSON object in the reply; the choice must name valid candidates.
pub fn parse(raw: &str, candidates: &[Candidate]) -> Result<Selection, String> {
    let value = first_object(raw).ok_or("select: no JSON object in the reply")?;
    let reason = value["reason"].as_str().unwrap_or_default().to_owned();
    if value["bail"] == true {
        return Ok(Selection::Bail { reason });
    }
    let thread = value["thread"]
        .as_str()
        .map(str::trim)
        .filter(|t| valid_thread(t))
        .ok_or("select: thread must be a short single-line name")?
        .to_owned();
    let candidate_ids: Vec<usize> = value["candidate_ids"]
        .as_array()
        .ok_or("select: candidate_ids must be an array")?
        .iter()
        .map(|v| v.as_u64().map(|n| n as usize))
        .collect::<Option<_>>()
        .ok_or("select: candidate_ids must be integers")?;
    if candidate_ids.is_empty()
        || candidate_ids
            .iter()
            .any(|id| !candidates.iter().any(|c| c.id == *id))
    {
        return Err("select: candidate_ids must name offered candidates".into());
    }
    Ok(Selection::Thread {
        thread,
        reason,
        candidate_ids,
    })
}
/// M5b-11 item 3: a prior rollup's thread replaces the model's name only on equality: the thread
/// of the newest prior that the model's name is a tag of, or whose thread is one of `entry_tags`
/// (the chosen entries' tags less their grouping tags), in any case. `None` (the model's name
/// stands) when a prior's thread is the model's name, when no prior is continued by an equal
/// name (a substring of a title renames nothing), or when that prior's thread is not valid.
pub fn kept_thread(model: &str, priors: &[Entry], entry_tags: &[String]) -> Option<String> {
    let model = model.to_lowercase();
    let thread = |p: &Entry| rollup_thread(p).map(|t| t.to_lowercase());
    if priors.iter().any(|p| thread(p) == Some(model.clone())) {
        return None;
    }
    let tagged = |t: String| entry_tags.iter().any(|e| e.to_lowercase() == t);
    let continued = |p: &&Entry| p.tags.contains(&model) || thread(p).is_some_and(tagged);
    let newest = priors
        .iter()
        .filter(continued)
        .max_by(|a, b| a.name.cmp(&b.name))?;
    rollup_thread(newest).filter(|t| valid_thread(t))
}
/// Whether a thread name is acceptable: the one rule for the select prompt's list, the select
/// reply and the kept name (M5b-11 item 1).
pub fn valid_thread(thread: &str) -> bool {
    !thread.is_empty()
        && thread.chars().count() <= 100
        && !thread.contains(['\n', '\r', '/', '\\'])
        && !thread.contains(" — ")
}
/// The first `{` whose balanced JSON object parses.
pub fn first_object(raw: &str) -> Option<Value> {
    raw.char_indices()
        .filter(|(_, c)| *c == '{')
        .find_map(|(i, _)| {
            serde_json::Deserializer::from_str(&raw[i..])
                .into_iter::<Value>()
                .next()
                .and_then(Result::ok)
                .filter(Value::is_object)
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn candidates() -> Vec<Candidate> {
        vec![Candidate {
            id: 1,
            tag: "halloran".into(),
            entries: vec![],
            days: 1,
        }]
    }
    fn rollup(name: &str, thread: &str, tags: &str) -> Entry {
        let text = format!("# ROLLUP: {thread} — state as of 2026-09-21\n\nTags: {tags}.\n");
        Entry::parse(name, text)
    }
    #[test]
    fn a_thread_keeps_its_prior_rollups_name_only_on_equality() {
        let old = rollup(
            "2026-09-20-aaaaaaaaaaaa",
            "m5-cutover",
            "m5-cutover, rollup",
        );
        let new = rollup(
            "2026-09-21-bbbbbbbbbbbb",
            "context-compiler-m5-cutover",
            "cutover-work, rollup",
        );
        let priors = [old.clone(), new];
        let kept = Some("context-compiler-m5-cutover");
        assert_eq!(kept_thread("m5", &priors, &[]), None);
        assert_eq!(kept_thread("Cutover-Work", &priors, &[]).as_deref(), kept);
        let tagged = ["context-compiler-m5-cutover".to_owned()];
        assert_eq!(kept_thread("m5", &priors, &tagged).as_deref(), kept);
        assert_eq!(kept_thread("m5-cutover", &priors, &tagged), None);
        assert_eq!(kept_thread("M5-Cutover", &[old], &[]), None);
        assert_eq!(kept_thread("anything", &[], &tagged), None);
        let slashed = rollup("2026-09-21-cccccccccccc", "a / b", "a, rollup");
        assert_eq!(kept_thread("a", &[slashed], &[]), None);
    }
    #[test]
    fn a_grouping_tag_is_not_a_continuing_tag() {
        let entry =
            |name: &str, tags: &str| Entry::parse(name, format!("# {name}\nTags: {tags}.\n"));
        let recent = Recent {
            as_of: "2026-09-22".into(),
            entries: vec![
                entry("2026-09-21-aaaaaaaaaaaa", "callum, judges-cycle"),
                entry("2026-09-22-bbbbbbbbbbbb", "callum, gaddis"),
                entry("2026-09-22-cccccccccccc", "fergus, other"),
            ],
            substantive: vec![],
        };
        let group = |id: usize, tag: &str, names: &[&str]| Candidate {
            id,
            tag: tag.into(),
            entries: names.iter().map(|n| (*n).to_owned()).collect(),
            days: 2,
        };
        let candidates = [
            group(
                1,
                "callum",
                &["2026-09-21-aaaaaaaaaaaa", "2026-09-22-bbbbbbbbbbbb"],
            ),
            group(2, "fergus", &["2026-09-22-cccccccccccc"]),
        ];
        let chosen = Selection::Thread {
            thread: "x".into(),
            reason: String::new(),
            candidate_ids: vec![1],
        };
        assert_eq!(
            chosen.entry_tags(&candidates, &recent),
            ["gaddis", "judges-cycle"]
        );
        let broad = rollup("2026-09-20-dddddddddddd", "callum", "callum, rollup");
        let tags = chosen.entry_tags(&candidates, &recent);
        assert_eq!(kept_thread("backup-plan", &[broad], &tags), None);
    }
    #[test]
    fn the_prompt_names_the_existing_threads() {
        let recent = Recent {
            as_of: "2026-09-22".into(),
            entries: vec![],
            substantive: vec![],
        };
        let text = prompt(
            "p",
            &recent,
            &candidates(),
            &["m5-cutover".into(), "halloran".into()],
        );
        assert!(text.contains("already have a rollup (newest first): m5-cutover; halloran."));
        assert!(text.contains("one of the listed names, or a new short kebab-case name"));
        assert!(!prompt("p", &recent, &candidates(), &[]).contains("already have a rollup"));
    }
    #[test]
    fn the_prompt_lists_only_names_the_reply_may_carry() {
        let recent = Recent {
            as_of: "2026-09-22".into(),
            entries: vec![],
            substantive: vec![],
        };
        let refused = ["a / b".to_owned(), "alpha — beta".into(), "x".repeat(101)];
        let mut threads = refused.to_vec();
        threads.push("halloran".into());
        let text = prompt("p", &recent, &candidates(), &threads);
        assert!(text.contains("already have a rollup (newest first): halloran."));
        for name in &refused {
            assert!(!text.contains(name.as_str()), "{name}");
            let reply = format!("{{\"thread\":{},\"candidate_ids\":[1]}}", json!(name));
            assert!(parse(&reply, &candidates()).is_err(), "{name}");
        }
        assert!(!prompt("p", &recent, &candidates(), &refused).contains("already have a rollup"));
    }
    #[test]
    fn constrained_choice() {
        let ok = parse(
            "sure: {\"thread\":\"halloran-thread\",\"reason\":\"r\",\"candidate_ids\":[1]} done",
            &candidates(),
        )
        .unwrap();
        assert_eq!(ok.tags(&candidates()), ["halloran"]);
        assert!(parse("{\"thread\":\"x\",\"candidate_ids\":[2]}", &candidates()).is_err());
        assert!(parse("{\"thread\":\"a/b\",\"candidate_ids\":[1]}", &candidates()).is_err());
        assert!(parse("no json", &candidates()).is_err());
        assert_eq!(
            parse("{\"bail\":true,\"reason\":\"quiet\"}", &candidates()).unwrap(),
            Selection::Bail {
                reason: "quiet".into()
            }
        );
    }
}
