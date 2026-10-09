//! `palace context --layered` (CTX-1): the per-turn context by layer, not by search score.
//! Block 1 lists the live specifics the message names, from the wings in scope only (a
//! specific's wing is the one its entry was distilled into), and only when the message asks for
//! one (`<--prompts>/specific-triggers.json`; unreadable: no block, the reason on stderr with
//! `--strict`). Block 2 is the graph of
//! `palace context`, left out for a private conversation (an explicit `--wing`). Blocks 3 and 4
//! come from the MemPalace search of the wings in scope: the rollup of each matching thread, then
//! the entries newer than it, else the top entry hits. A hit's thread is found in the hit's own
//! diary directory (`palace_core::context_thread`: supersession, tags, claim subjects), so a
//! thread never reaches into another wing. Read-only and fail-open as the two-block
//! output is; `--token-budget` defaults to 900.
use crate::context::{self, Options, SHOWN_EXCERPTS, Scope};
use palace_core::context_block::{self, Graph};
use palace_core::context_layered::{self, Layered, Newer, Specific, Thread};
use palace_core::context_thread::ThreadDiary;
use palace_core::rollup::{self, Entry};
use palace_core::rollup_prep::entry_date;
use palace_core::search_hit::SearchHit;
use palace_core::specific_trigger;
use palace_core::traits::DiaryReader;
use palace_mempalace::diary::Diary;
use palace_store::{
    Error,
    context::{Reader, SpecificRow},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

/// At most this many specifics, threads, newer entries of a thread and block-4 items.
const MAX_SPECIFICS: usize = 5;
const MAX_THREADS: usize = 2;
const NEWER_PER_THREAD: usize = 3;
const MAX_NEWER: usize = 5;

pub fn produce(o: &Options, message: &str, deadline: Instant) -> Result<String, Error> {
    let private = o.wing.is_some();
    let pulled = match private {
        true => empty_pull(),
        false => context::pull(o, message)?,
    };
    let reader = Reader::open(
        &crate::paths::input(&o.store)?,
        &crate::paths::input(&o.kg)?,
    )?;
    let specifics = specifics(o, &reader, message)?;
    if Instant::now() >= deadline {
        return Err(format!("no graph within {} ms", o.timeout_ms).into());
    }
    let hits = context::excerpts(o, message, deadline);
    let (threads, newer) = threads(&hits, &reader.entry_subjects()?);
    let layered = Layered {
        specifics: specifics.iter().map(shown).collect(),
        graph: Graph {
            lines: pulled.relations.iter().map(context::line).collect(),
            asks: pulled.asks.iter().map(|a| a.line.clone()).collect(),
            matched: pulled.matched.clone(),
            total: pulled.total,
        },
        threads,
        newer,
    };
    let budget = o.token_budget.unwrap_or(context_layered::DEFAULT_BUDGET);
    let parts = Parts {
        specifics: &specifics,
        threads: layered.threads.clone(),
        newer: layered.newer.clone(),
    };
    let text = context_layered::fit(layered, budget);
    if !o.json {
        return Ok(text);
    }
    Ok(serde_json::to_string_pretty(&json_of(
        &pulled, &parts, &hits, &text,
    ))?)
}

fn empty_pull() -> context::Pulled {
    context::Pulled {
        matched: Vec::new(),
        relations: Vec::new(),
        total: 0,
        asks: Vec::new(),
    }
}

/// The live specifics the message names, from wings in scope, newest first, at most five; none
/// unless the message asks for one.
fn specifics(o: &Options, reader: &Reader, message: &str) -> Result<Vec<SpecificRow>, Error> {
    if !asks(o, message) {
        return Ok(Vec::new());
    }
    let scope = Scope::of(o);
    let tokens = specific_trigger::entity_tokens(message);
    Ok(reader
        .live_specifics()?
        .into_iter()
        .filter(|s| s.wing.as_deref().is_some_and(|w| scope.admits(w)))
        .filter(|s| specific_trigger::names_specific(&tokens, &s.subject, &s.object))
        .take(MAX_SPECIFICS)
        .collect())
}

fn asks(o: &Options, message: &str) -> bool {
    let triggers = crate::paths::input(&o.prompts.join("specific-triggers.json"))
        .and_then(|path| Ok(fs::read_to_string(path)?))
        .and_then(|json| Ok(specific_trigger::Triggers::parse(&json)?));
    match triggers {
        Ok(triggers) => specific_trigger::asks(message, &triggers),
        Err(error) => {
            if o.strict {
                eprintln!("palace context: no specifics: {error}");
            }
            false
        }
    }
}

fn shown(s: &SpecificRow) -> Specific {
    Specific {
        date: s.date.clone(),
        subject: s.subject.clone(),
        predicate: s.predicate.clone(),
        object: s.object.clone(),
        quote: s.quote.clone(),
        source: s.entry.clone(),
    }
}

/// Blocks 3 and 4 from the top three hits (today's excerpt window), in rank order: a hit with a
/// thread adds the thread's rollup to block 3 (at most two threads) and the thread's newer
/// entries (at most three) to block 4; a hit without one is itself a block-4 item. Block 4 keeps
/// the hits' rank order, each source once, at most five items.
fn threads(hits: &[SearchHit], subjects: &Subjects) -> (Vec<Thread>, Vec<Newer>) {
    let mut diaries = Diaries::default();
    let (mut threads, mut newer) = (Vec::<Thread>::new(), Vec::<Newer>::new());
    for hit in hits.iter().take(SHOWN_EXCERPTS) {
        let items = match diaries.thread(hit, subjects) {
            Some((thread, _)) if threads.iter().any(|t| t.source == thread.source) => continue,
            Some((_, _)) if threads.len() == MAX_THREADS => continue,
            Some((thread, items)) => {
                threads.push(thread);
                items
            }
            None => vec![top_hit(hit)],
        };
        for item in items {
            if !newer.iter().any(|n| n.source == item.source) {
                newer.push(item);
            }
        }
    }
    newer.truncate(MAX_NEWER);
    (threads, newer)
}

type Subjects = HashMap<String, BTreeSet<String>>;

/// Each hit's diary directory, read once per turn (None when it is not readable under --roots).
#[derive(Default)]
struct Diaries(HashMap<PathBuf, Option<ThreadDiary>>);

impl Diaries {
    /// The hit's thread and its newer entries, when the hit has a thread.
    fn thread(&mut self, hit: &SearchHit, subjects: &Subjects) -> Option<(Thread, Vec<Newer>)> {
        let dir = Path::new(&hit.source_path).parent()?.to_path_buf();
        let diary = self
            .0
            .entry(dir.clone())
            .or_insert_with(|| load(&dir))
            .as_ref()?;
        let (rollup, name) = diary.thread_of(&hit.source_file)?;
        let of = |entry: &str| subjects.get(entry).cloned().unwrap_or_default();
        let newer = diary
            .newer(rollup, &name, of)
            .into_iter()
            .take(NEWER_PER_THREAD)
            .map(|e| Newer {
                room: hit.wing.clone(),
                source: format!("{}.md", e.name),
                date: entry_date(&e.name).unwrap_or_default().to_owned(),
                text: body(&e.text),
                thread: Some(name.clone()),
            })
            .collect();
        Some((thread_of(rollup, name), newer))
    }
}

/// Every entry of a diary directory with its voided supersessions.
/// A directory outside `--roots` (checked on the path as MemPalace recorded it, before any file
/// system call) is not read at all.
fn load(dir: &Path) -> Option<ThreadDiary> {
    if !crate::paths::roots()
        .iter()
        .any(|root| dir.starts_with(root))
    {
        return None;
    }
    let diary = Diary::open(&crate::paths::input(dir).ok()?).ok()?;
    let names = diary.names().ok()?;
    let voids = rollup::voided(&diary, &names).ok()?;
    let entries = names
        .iter()
        .filter_map(|n| Some(Entry::parse(n, diary.read(n).ok()??)))
        .collect();
    Some(ThreadDiary::new(entries, &voids))
}

/// An entry's text without its title, `Transcript:` and `Tags:` lines.
fn body(text: &str) -> String {
    text.lines()
        .skip(1)
        .filter(|l| !l.starts_with("Transcript: ") && !l.to_lowercase().starts_with("tags:"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

fn thread_of(entry: &Entry, thread: String) -> Thread {
    Thread {
        thread,
        as_of: rollup::title_parts(entry).1,
        source: format!("{}.md", entry.name),
        text: consolidated(&entry.text),
    }
}

/// A rollup's "Current consolidated truth" section, else its text after the title.
fn consolidated(text: &str) -> String {
    let heading = |l: &&str| {
        l.trim_start_matches('#')
            .trim()
            .eq_ignore_ascii_case("current consolidated truth")
    };
    let section: Vec<&str> = text
        .lines()
        .skip_while(|l| !heading(l))
        .skip(1)
        .take_while(|l| !l.starts_with('#'))
        .collect();
    let lines = match section.iter().any(|l| !l.trim().is_empty()) {
        true => section,
        false => text.lines().skip(1).collect(),
    };
    lines.join(" ").trim().to_owned()
}

fn top_hit(hit: &SearchHit) -> Newer {
    Newer {
        room: context::room(hit),
        source: hit.source_file.clone(),
        date: entry_date(&hit.source_file).unwrap_or_default().to_owned(),
        text: hit.text.clone(),
        thread: None,
    }
}

struct Parts<'a> {
    specifics: &'a [SpecificRow],
    threads: Vec<Thread>,
    newer: Vec<Newer>,
}

fn json_of(pulled: &context::Pulled, parts: &Parts, hits: &[SearchHit], text: &str) -> Value {
    json!({
        "layered": true,
        "specifics": parts.specifics.iter().map(|s| json!({"id":s.id,"subject":s.subject,"predicate":s.predicate,"object":s.object,"date":s.date,"quote":s.quote,"entry":s.entry,"wing":s.wing})).collect::<Vec<_>>(),
        "matched": pulled.matched,
        "relations": pulled.relations.iter().map(context::relation_json).collect::<Vec<_>>(),
        "total": pulled.total,
        "asks": pulled.asks.iter().map(|a| json!({"finding":a.finding,"claim":a.claim,"token":a.token,"line":a.line})).collect::<Vec<_>>(),
        "threads": parts.threads.iter().map(|t| json!({"thread":t.thread,"as_of":t.as_of,"source":t.source,"line":context_layered::thread_line(t)})).collect::<Vec<_>>(),
        "newer": parts.newer.iter().map(|n| json!({"source":n.source,"room":n.room,"date":n.date,"thread":n.thread,"text":context_block::clip(&n.text)})).collect::<Vec<_>>(),
        "hits": hits.len(),
        "text": text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_dates_and_rollup_text() {
        assert_eq!(
            body("# t\n\nIt moved.\n\nTranscript: /x:1-4\n\nTags: a, b."),
            "It moved."
        );
        let rollup = "# ROLLUP: x — state as of 2031-04-06\n\n## Claims\n- C1\n\n## Current consolidated truth\nIt is so.\nStill.\n\n## Supersedes\n- a.md";
        assert_eq!(consolidated(rollup), "It is so. Still.");
        assert_eq!(consolidated("# t\nbody\nmore"), "body more");
    }
}
