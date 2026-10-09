//! Rollup stage 3: pull the thread's cluster — MemPalace hits for the thread and its tags, plus
//! the primaries a prior `ROLLUP: <thread>` superseded. Rollups are never sources.
use crate::{
    evidence::is_bookkeeping,
    rollup::{Entry, rollup_thread, superseded_live, superseded_sources, voided},
    rollup_prep::entry_date,
    traits::{DiaryReader, Searcher},
};

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
}
impl Cluster {
    pub fn member(&self, name: &str) -> Option<&Member> {
        let name = name.strip_suffix(".md").unwrap_or(name);
        self.members.iter().find(|m| m.entry.name == name)
    }
}
pub struct Request<'a> {
    pub thread: &'a str,
    pub tags: &'a [String],
    pub as_of: &'a str,
}
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
    push_unique(&mut ordered, search_hits(searcher, &names, request)?);
    let mut cluster = Cluster {
        thread: request.thread.into(),
        prior_rollups: priors.iter().map(|p| p.name.clone()).collect(),
        voided: followed_voids,
        referenced,
        ..Default::default()
    };
    fill(&mut cluster, diary, &ordered, request.as_of)?;
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
    for query in std::iter::once(request.thread).chain(request.tags.iter().map(String::as_str)) {
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
fn fill<D: DiaryReader>(
    cluster: &mut Cluster,
    diary: &D,
    ordered: &[String],
    as_of: &str,
) -> Result<(), String>
where
    D::Error: std::fmt::Display,
{
    let mut budget = MAX_CHARS;
    for name in ordered
        .iter()
        .filter(|n| entry_date(n).is_some_and(|d| d <= as_of))
    {
        let Some(text) = diary.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        let entry = Entry::parse(name, text);
        if entry.is_rollup {
            continue; // never summarise the summary
        }
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
        };
        let c = pull(&d, &hits, &request).unwrap();
        assert_eq!(c.prior_rollups, ["2026-09-21-cccccccccccc"]);
        let by_tag = Request {
            thread: "zz-other",
            tags: &["t".to_owned()],
            as_of: "2026-09-22",
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
                ("2026-09-20-ffffffffffff", false, false)
            ]
        );
    }
}
