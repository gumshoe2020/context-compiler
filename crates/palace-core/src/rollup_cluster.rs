//! Rollup stage 3: pull the thread's cluster — the entries a prior `ROLLUP: <thread>` superseded
//! or referenced (its lineage), then the diary entries with thread-specific evidence
//! (`rollup_membership`, SETTLE-1f §2): first those MemPalace finds for the thread and its
//! non-generic tags, in its order, then the rest, newest first. Rollups are never sources.
use crate::{
    evidence::is_bookkeeping,
    rollup::{Entry, rollup_thread, superseded_live, superseded_sources, voided},
    rollup_membership::{GenericTags, joins},
    rollup_prep::entry_date,
    traits::{DiaryReader, Searcher},
};
use std::collections::BTreeMap;

pub const HITS_PER_QUERY: usize = 10;
pub const MAX_CHARS: usize = 60_000;

/// Two independent facts about a member (M4b.1 S-1). Rollups are never members.
#[derive(Debug)]
pub struct Member {
    pub entry: Entry,
    /// A claim may cite it: not graph bookkeeping (the write gate's `is_bookkeeping`).
    pub citable: bool,
    /// Supersedes may name it: citable and not `no-compress`.
    pub supersedable: bool,
}
impl Member {
    pub fn new(entry: Entry) -> Self {
        let citable = !is_bookkeeping(&entry.text);
        let supersedable = citable && !entry.tags.contains("no-compress");
        Self {
            entry,
            citable,
            supersedable,
        }
    }
    /// A member a prior rollup referenced without superseding (M5b-1 item 5, review S-1): the
    /// prior deliberately kept it (another thread, or context), so it is never supersedable,
    /// whatever its tags.
    pub fn referenced(entry: Entry) -> Self {
        Self {
            supersedable: false,
            ..Self::new(entry)
        }
    }
    pub fn label(&self) -> &'static str {
        match (self.citable, self.supersedable) {
            (true, true) => "primary",
            (true, false) => "reference-only",
            _ => "bookkeeping",
        }
    }
}
#[derive(Debug, Default)]
pub struct Cluster {
    pub thread: String,
    pub prior_rollups: Vec<String>,
    pub members: Vec<Member>,
    /// Entries left out because the 60k-character budget was spent.
    pub dropped: Vec<String>,
    /// Entries a prior rollup listed under Supersedes whose supersession a `supersession-void`
    /// entry voided: followed as primaries that are NOT yet compressed (M4d item 10).
    pub voided: Vec<String>,
    /// Entries a prior rollup referenced without superseding and no prior superseded: members
    /// only as reference-only (M5b-1 item 5).
    pub referenced: Vec<String>,
    /// Why each member and dropped entry joined (SETTLE-1f §2): `lineage`, `tag <tag>` or `title`.
    pub joined_by: BTreeMap<String, String>,
}
impl Cluster {
    pub fn member(&self, name: &str) -> Option<&Member> {
        let name = name.strip_suffix(".md").unwrap_or(name);
        self.members.iter().find(|m| m.entry.name == name)
    }
    /// The entries that joined: the members and those the budget left out (SETTLE-1f §2's cap).
    pub fn size(&self) -> usize {
        self.members.len() + self.dropped.len()
    }
}
pub struct Request<'a> {
    pub thread: &'a str,
    pub tags: &'a [String],
    pub as_of: &'a str,
    /// Tags that are never evidence and never searched (SETTLE-1f §2).
    pub generic: &'a GenericTags,
}
/// `ROLLUP_MAX_CLUSTER`'s default: a thread whose cluster holds more entries is skipped.
pub const MAX_CLUSTER: usize = 25;
/// The thread's cluster: the prior rollups' superseded entries first (and which of them a void
/// made primaries again), then the search hits for the thread and its tags, within the budget.
pub fn pull<D, S>(diary: &D, searcher: &S, request: &Request) -> Result<Cluster, String>
where
    D: DiaryReader,
    S: Searcher,
    D::Error: std::fmt::Display,
    S::Error: std::fmt::Display,
{
    let names = diary.names().map_err(|e| e.to_string())?;
    let priors = prior_rollups(diary, &names, request)?;
    let (mut ordered, followed_voids, referenced) = prior_sources(diary, &names, &priors)?;
    let lineage = ordered.len();
    push_unique(&mut ordered, search_hits(searcher, &names, request)?);
    let mut newest_first = names.clone();
    newest_first.sort_by(|a, b| b.cmp(a));
    push_unique(&mut ordered, newest_first);
    let mut cluster = Cluster {
        thread: request.thread.into(),
        prior_rollups: priors.iter().map(|p| p.name.clone()).collect(),
        voided: followed_voids,
        referenced,
        ..Default::default()
    };
    let (lineage, candidates) = ordered.split_at(lineage);
    fill(&mut cluster, diary, (lineage, candidates), request)?;
    Ok(cluster)
}

/// Prior sources in order, the voided ones followed as primaries, the referenced-only ones.
type PriorSources = (Vec<String>, Vec<String>, Vec<String>);
/// Every entry the prior rollups superseded or referenced without superseding (M5b item 0), in
/// order; those among them whose supersession was voided by every prior that lists them
/// (followed as uncompressed primaries); and those only referenced, never superseded, by a prior
/// (reference-only members, M5b-1 item 5).
fn prior_sources<D: DiaryReader>(
    diary: &D,
    names: &[String],
    priors: &[Entry],
) -> Result<PriorSources, String>
where
    D::Error: std::fmt::Display,
{
    let void = voided(diary, names)?;
    let (mut ordered, mut superseded, mut referenced) = (Vec::new(), Vec::new(), Vec::new());
    for prior in priors {
        let refs = superseded_sources(prior, names);
        push_unique(&mut ordered, refs.resolved.clone());
        push_unique(&mut superseded, refs.resolved);
        push_unique(&mut ordered, refs.referenced_only.clone());
        push_unique(&mut referenced, refs.referenced_only);
    }
    referenced.retain(|n| !superseded.contains(n));
    let compressed: Vec<String> = priors
        .iter()
        .flat_map(|prior| superseded_live(prior, names, &void))
        .collect();
    let followed = ordered
        .iter()
        .filter(|n| void.contains_key(*n) && !compressed.contains(n))
        .cloned()
        .collect();
    Ok((ordered, followed, referenced))
}

/// Diary entries MemPalace returns for the thread and each of its tags.
fn search_hits<S: Searcher>(
    searcher: &S,
    names: &[String],
    request: &Request,
) -> Result<Vec<String>, String>
where
    S::Error: std::fmt::Display,
{
    let mut found = Vec::new();
    let tags = request.tags.iter().map(String::as_str);
    let specific = tags.filter(|t| !request.generic.contains(t));
    for query in std::iter::once(request.thread).chain(specific) {
        let hits = searcher
            .search(query, HITS_PER_QUERY, 1.5)
            .map_err(|e| format!("pull search {query:?}: {e}"))?;
        let names = hits
            .into_iter()
            .map(|h| h.source_file.trim_end_matches(".md").to_owned())
            .filter(|n| names.contains(n));
        push_unique(&mut found, names.collect());
    }
    Ok(found)
}
/// The lineage, then each candidate with thread-specific evidence, within the budget.
fn fill<D: DiaryReader>(
    cluster: &mut Cluster,
    diary: &D,
    (lineage, candidates): (&[String], &[String]),
    request: &Request,
) -> Result<(), String>
where
    D::Error: std::fmt::Display,
{
    let mut budget = MAX_CHARS;
    let dated = |n: &&String| entry_date(n).is_some_and(|d| d <= request.as_of);
    let ordered = lineage.iter().chain(candidates).filter(dated);
    for name in ordered {
        let Some(text) = diary.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        let entry = Entry::parse(name, text);
        if entry.is_rollup {
            continue; // never summarise the summary
        }
        let joined_by = match lineage.contains(name) {
            true => "lineage".to_owned(),
            false => match joins(&entry, request.thread, request.generic) {
                Some(evidence) => evidence,
                None => continue,
            },
        };
        cluster.joined_by.insert(entry.name.clone(), joined_by);
        let size = entry.text.chars().count();
        if size > budget {
            cluster.dropped.push(entry.name);
            continue;
        }
        budget -= size;
        let member = if cluster.referenced.contains(&entry.name) {
            Member::referenced(entry)
        } else {
            Member::new(entry)
        };
        cluster.members.push(member);
    }
    Ok(())
}
/// Rollups dated on or before as-of whose title or tags name the thread (as before M5b-10: the
/// chosen entries' tags find no prior, M5b-11 item 3).
pub fn prior_rollups<D: DiaryReader>(
    diary: &D,
    names: &[String],
    request: &Request,
) -> Result<Vec<Entry>, String>
where
    D::Error: std::fmt::Display,
{
    let thread = request.thread.to_lowercase();
    let mut priors = Vec::new();
    for name in names
        .iter()
        .filter(|n| entry_date(n).is_some_and(|d| d <= request.as_of))
    {
        let Some(text) = diary.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        let entry = Entry::parse(name, text);
        if entry.is_rollup
            && (entry.tags.contains(&thread) || entry.title.to_lowercase().contains(&thread))
        {
            priors.push(entry);
        }
    }
    Ok(priors)
}
/// The threads of the rollups dated on or before `as_of`, newest first, each once (M5b-10 item
/// 5: the select step is given them).
pub fn rollup_threads<D: DiaryReader>(
    diary: &D,
    names: &[String],
    as_of: &str,
) -> Result<Vec<String>, String>
where
    D::Error: std::fmt::Display,
{
    let mut dated: Vec<&String> = names
        .iter()
        .filter(|n| entry_date(n).is_some_and(|d| d <= as_of))
        .collect();
    dated.sort_by(|a, b| b.cmp(a));
    let mut threads = Vec::new();
    for name in dated {
        let Some(text) = diary.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        if let Some(thread) = rollup_thread(&Entry::parse(name, text)) {
            push_unique(&mut threads, vec![thread]);
        }
    }
    Ok(threads)
}
fn push_unique(list: &mut Vec<String>, names: Vec<String>) {
    for name in names {
        if !list.contains(&name) {
            list.push(name);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{search_hit::SearchHit, test_support::Memory};
    struct Hits(Vec<&'static str>);
    impl Searcher for Hits {
        type Error = String;
        fn search(&self, _: &str, _: usize, _: f64) -> Result<Vec<SearchHit>, String> {
            Ok(self
                .0
                .iter()
                .map(|f| {
                    serde_json::from_value(serde_json::json!({"text":"","source_path":"","source_file":f,"wing":"w","room":"r"}))
                        .unwrap()
                })
                .collect())
        }
    }
    #[test]
    fn prior_rollup_primaries_hits_and_roles() {
        let mut d = Memory::default();
        for (name, text) in [
            ("2026-09-20-aaaaaaaaaaaa", "# A\nTags: t."),
            ("2026-09-20-bbbbbbbbbbbb", "# B\nTags: t, no-compress."),
            (
                "2026-09-20-ffffffffffff",
                "# KG precision audit 2026-09-20\nTags: t.",
            ),
            (
                "2026-09-21-cccccccccccc",
                "# ROLLUP: t — state as of 2026-09-21\nTags: t.\n## Supersedes\n- 2026-09-20-aaaaaaaaaaaa.md",
            ),
            ("2026-09-22-dddddddddddd", "# D\nTags: t."),
            ("2026-09-30-eeeeeeeeeeee", "# future\nTags: t."),
        ] {
            d.0.insert(name.into(), text.into());
        }
        let hits = Hits(vec![
            "2026-09-20-bbbbbbbbbbbb.md",
            "2026-09-20-ffffffffffff.md",
            "2026-09-21-cccccccccccc.md",
            "2026-09-30-eeeeeeeeeeee.md",
            "elsewhere.md",
        ]);
        let request = Request {
            thread: "t",
            tags: &[],
            as_of: "2026-09-22",
            generic: &Default::default(),
        };
        let c = pull(&d, &hits, &request).unwrap();
        assert_eq!(c.prior_rollups, ["2026-09-21-cccccccccccc"]);
        let by_tag = Request {
            thread: "zz-other",
            tags: &["t".to_owned()],
            as_of: "2026-09-22",
            generic: &Default::default(),
        };
        let names = d.names().unwrap();
        assert!(prior_rollups(&d, &names, &by_tag).unwrap().is_empty());
        let names: Vec<_> = c
            .members
            .iter()
            .map(|m| (m.entry.name.as_str(), m.citable, m.supersedable))
            .collect();
        assert_eq!(
            names,
            [
                ("2026-09-20-aaaaaaaaaaaa", true, true),
                ("2026-09-20-bbbbbbbbbbbb", true, false),
                ("2026-09-20-ffffffffffff", false, false),
                ("2026-09-22-dddddddddddd", true, true)
            ]
        );
        assert_eq!(c.joined_by["2026-09-20-aaaaaaaaaaaa"], "lineage");
        assert_eq!(c.joined_by["2026-09-22-dddddddddddd"], "tag t");
    }
    /// SETTLE-1f §2: a hit sharing only generic tags with the thread stays out, as does a hit with
    /// no tag; the thread's own tag and its title bring an entry in, hit or not; a generic tag is
    /// never searched.
    #[test]
    fn only_thread_specific_evidence_joins_the_cluster() {
        let mut d = Memory::default();
        for (name, text) in [
            (
                "2026-09-20-aaaaaaaaaaaa",
                "# Beds\nTags: callum, garden-plan.",
            ),
            (
                "2026-09-20-bbbbbbbbbbbb",
                "# Party\nTags: callum, decision, continuing-thread.",
            ),
            ("2026-09-20-cccccccccccc", "# untagged note"),
            (
                "2026-09-21-dddddddddddd",
                "# Garden plan: the shed\nTags: shed.",
            ),
            (
                "2026-09-21-eeeeeeeeeeee",
                "# Paths\nTags: continuing-thread:garden-plan.",
            ),
        ] {
            d.0.insert(name.into(), text.into());
        }
        struct Logged(Hits, std::cell::RefCell<Vec<String>>);
        impl Searcher for Logged {
            type Error = String;
            fn search(&self, q: &str, n: usize, c: f64) -> Result<Vec<SearchHit>, String> {
                self.1.borrow_mut().push(q.to_owned());
                self.0.search(q, n, c)
            }
        }
        let hits = Logged(
            Hits(vec![
                "2026-09-20-bbbbbbbbbbbb.md",
                "2026-09-20-cccccccccccc.md",
            ]),
            Default::default(),
        );
        let generic = crate::rollup_membership::GenericTags::parse(
            r#"{"people": ["{person}"], "kinds": ["decision"]}"#,
            "callum",
            "fergus",
        )
        .unwrap();
        let tags = [
            "callum".to_owned(),
            "decision".to_owned(),
            "shed".to_owned(),
        ];
        let request = Request {
            thread: "garden-plan",
            tags: &tags,
            as_of: "2026-09-22",
            generic: &generic,
        };
        let c = pull(&d, &hits, &request).unwrap();
        let joined: Vec<(&str, &str)> = c
            .members
            .iter()
            .map(|m| (m.entry.name.as_str(), c.joined_by[&m.entry.name].as_str()))
            .collect();
        assert_eq!(
            joined,
            [
                (
                    "2026-09-21-eeeeeeeeeeee",
                    "tag continuing-thread:garden-plan"
                ),
                ("2026-09-21-dddddddddddd", "title"),
                ("2026-09-20-aaaaaaaaaaaa", "tag garden-plan")
            ]
        );
        assert_eq!(*hits.1.borrow(), ["garden-plan", "shed"]);
        assert_eq!(c.size(), 3);
    }
}
