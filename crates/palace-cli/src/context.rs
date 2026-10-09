//! `palace context --message <text> --kg <kg> --store <compiler.sqlite> --palace <dir>
//! [--token-budget N] [--max-relations 40] [--wing openclaw | --all-wings [--exclude-wing <name>]…]
//! [--json] [--no-excerpts] [--strict] [--timeout-ms 2000]` (M5c item 2): the per-turn context the
//! palace-context-hook injects, one read-only command. Nodes are matched as the hook matches them
//! (`palace_core::entity_match`); their first-order relations come in the hook's order, each with
//! the claim behind it (handle, derived confidence, a DISPUTED or CORRECTED marker); open asks
//! touching a matched node close the graph block; excerpts come from a MemPalace search of one
//! wing, or (PKG-1.2 item 3, `--all-wings`) of every wing but the excluded ones. The graph and the store
//! are opened read-only, no lock is taken, nothing is reconciled or logged, and no pin script is
//! read. The graph part comes first; a MemPalace failure or timeout leaves the graph block without
//! excerpts. Fail-open: an error of the graph part, or no graph within the timeout, prints nothing
//! and exits 0 (`--strict`: the error on stderr, exit 2).
use clap::Args;
use palace_core::context_block::{self, Excerpt, Graph, Line, Palace};
use palace_core::search_hit::SearchHit;
use palace_store::{
    Error,
    context::{Ask, Reader, Relation},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// The sidecar's search: five hits, three shown. The query is the message's first 250
/// characters, not the sidecar's 400: the MCP tool `mempalace_search` takes at most 250
/// (`maxLength` in its schema; MemPalace's query sanitiser keeps the last 250 of a longer one).
const SEARCH_HITS: usize = 5;
pub(crate) const SHOWN_EXCERPTS: usize = 3;
const MAX_QUERY_CHARS: usize = 250;

#[derive(Args, Clone)]
pub struct Options {
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) message: String,
    #[arg(long)]
    pub(crate) kg: PathBuf,
    #[arg(long)]
    pub(crate) store: PathBuf,
    #[arg(long)]
    pub(crate) palace: PathBuf,
    /// Trim the output to about this many tokens (excerpts first, then relations from the end).
    #[arg(long)]
    pub(crate) token_budget: Option<usize>,
    #[arg(long, default_value_t = 40)]
    pub(crate) max_relations: usize,
    /// The only wing excerpts may come from (default openclaw). With `--layered`, naming it
    /// marks a private conversation: that wing only, and no graph.
    #[arg(long)]
    pub(crate) wing: Option<String>,
    /// Excerpts from every wing (the search unfiltered), less each `--exclude-wing`.
    #[arg(long, conflicts_with = "wing")]
    pub(crate) all_wings: bool,
    /// With `--all-wings`: a wing excerpts never come from (repeatable).
    #[arg(long, requires = "all_wings", value_parser = crate::wing_name)]
    pub(crate) exclude_wing: Vec<String>,
    /// The same content as JSON, with each relation's triple, claim id and source entry.
    #[arg(long)]
    pub(crate) json: bool,
    /// Skip the MemPalace search (graph block only).
    #[arg(long)]
    pub(crate) no_excerpts: bool,
    /// Report an error on stderr with exit 2 instead of printing nothing (for tests).
    #[arg(long)]
    pub strict: bool,
    #[arg(long, default_value_t = 2000)]
    pub(crate) timeout_ms: u64,
    #[arg(long, default_value = "mempalace-mcp")]
    pub(crate) mempalace_mcp: PathBuf,
    /// CTX-1: the four layered blocks ([SPECIFICS], [GRAPH], [THREAD], [NEWER]) instead of the
    /// hook's two; `--token-budget` defaults to 900.
    #[arg(long)]
    pub(crate) layered: bool,
    /// With `--layered`: the directory of `specific-triggers.json` (when a message asks for a
    /// specific, CTX-1 §2).
    #[arg(long, default_value = "prompts", requires = "layered")]
    pub(crate) prompts: PathBuf,
}

/// The wing excerpts come from when `--all-wings` is not given.
pub const DEFAULT_WING: &str = "openclaw";

/// How long past `--timeout-ms` the command waits for the MemPalace client, which kills its
/// server's process group at the deadline itself, to reap it.
const REAP_MS: u64 = 300;

pub fn run(o: Options) -> i32 {
    let (strict, limit) = (o.strict, o.timeout_ms);
    let deadline = Instant::now() + Duration::from_millis(limit);
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(produce(&o, deadline).map_err(|e| e.to_string()));
    });
    match receive.recv_timeout(Duration::from_millis(limit + REAP_MS)) {
        Ok(Ok(text)) => {
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        Ok(Err(error)) => failed(strict, &error),
        Err(_) => failed(strict, &format!("no answer within {limit} ms")),
    }
}

/// Fail-open: nothing and 0, or with `--strict` the reason on stderr and 2.
pub fn failed(strict: bool, reason: &str) -> i32 {
    if !strict {
        return 0;
    }
    eprintln!("palace context: {reason}");
    2
}

fn produce(o: &Options, deadline: Instant) -> Result<String, Error> {
    let Some(message) = context_block::message(&o.message) else {
        return Ok(String::new());
    };
    if o.layered {
        return crate::context_layered::produce(o, &message, deadline);
    }
    let pulled = pull(o, &message)?;
    if Instant::now() >= deadline {
        return Err(format!("no graph within {} ms", o.timeout_ms).into());
    }
    let hits = excerpts(o, &message, deadline);
    let graph = Graph {
        lines: pulled.relations.iter().map(line).collect(),
        asks: pulled.asks.iter().map(|a| a.line.clone()).collect(),
        matched: pulled.matched.clone(),
        total: pulled.total,
    };
    let text = match o.token_budget {
        Some(budget) => context_block::fit(graph, palace(&hits), budget),
        None => context_block::render(&graph, &palace(&hits)),
    };
    if !o.json {
        return Ok(text);
    }
    Ok(serde_json::to_string_pretty(&pulled.json(&hits, &text))?)
}

/// The matched nodes, their relations and the open asks touching them, read-only.
pub(crate) fn pull(o: &Options, message: &str) -> Result<Pulled, Error> {
    let reader = Reader::open(
        &crate::paths::input(&o.store)?,
        &crate::paths::input(&o.kg)?,
    )?;
    let matched = palace_core::entity_match::match_node_ids(message, &reader.nodes()?);
    let (relations, total) = reader.relations(&matched, o.max_relations)?;
    let asks = reader.open_asks(&matched, &relations)?;
    Ok(Pulled {
        matched,
        relations,
        total,
        asks,
    })
}

/// A relation as its line shows it: the claim's rounded derived confidence and status when a
/// claim stands behind it, else the triple's own confidence.
pub(crate) fn line(r: &Relation) -> Line {
    let claim = r.claim.as_ref();
    Line {
        subject: r.subject.clone(),
        predicate: r.predicate.clone(),
        object: r.object.clone(),
        confidence: claim
            .map(|c| context_block::rounded(c.confidence))
            .or(r.confidence),
        handle: claim.and_then(|c| c.handle.clone()),
        status: claim.map(|c| c.status.clone()),
    }
}

/// The excerpts, or none when the search fails or runs out of time: the graph block is printed
/// either way (M5c.1 item 4); `--strict` names the reason on stderr.
pub(crate) fn excerpts(o: &Options, message: &str, deadline: Instant) -> Vec<SearchHit> {
    if o.no_excerpts {
        return Vec::new();
    }
    search(o, message, deadline).unwrap_or_else(|error| {
        if o.strict {
            eprintln!("palace context: no excerpts: {error}");
        }
        Vec::new()
    })
}

/// MemPalace search of `--wing` only (any other wing's hit is dropped as well), or of every wing
/// less the excluded ones, over the message's first 250 characters; the server and its
/// descendants are ended by `deadline`.
fn search(o: &Options, message: &str, deadline: Instant) -> Result<Vec<SearchHit>, Error> {
    if Instant::now() >= deadline {
        return Err(format!("no time left for the search within {} ms", o.timeout_ms).into());
    }
    let palace = crate::paths::input(&o.palace)?;
    let mempalace =
        palace_mempalace::MemPalace::open_unpinned(&palace, &o.mempalace_mcp, deadline)?;
    let query: String = message.chars().take(MAX_QUERY_CHARS).collect();
    let scope = Scope::of(o);
    let hits = mempalace.search_in(&query, scope.limit(), 0.0, scope.filter());
    if let Some(reason) = mempalace.unavailable() {
        return Err(reason.into());
    }
    let admitted = hits?.into_iter().filter(|h| scope.admits(&h.wing));
    Ok(admitted.take(SEARCH_HITS).collect())
}

/// The wings excerpts may come from: `--wing`, or every wing but `--exclude-wing`'s.
pub(crate) enum Scope<'a> {
    Wing(&'a str),
    AllBut(&'a [String]),
}
impl<'a> Scope<'a> {
    pub(crate) fn of(o: &'a Options) -> Self {
        match o.all_wings {
            true => Self::AllBut(&o.exclude_wing),
            false => Self::Wing(o.wing.as_deref().unwrap_or(DEFAULT_WING)),
        }
    }
    /// The search's own wing filter (none across all wings).
    fn filter(&self) -> Option<&str> {
        match self {
            Self::Wing(wing) => Some(wing),
            Self::AllBut(_) => None,
        }
    }
    /// More hits are asked for when some will be dropped, so that excluded wings do not crowd
    /// out the rest.
    fn limit(&self) -> usize {
        match self {
            Self::AllBut(excluded) if !excluded.is_empty() => SEARCH_HITS * 4,
            _ => SEARCH_HITS,
        }
    }
    pub(crate) fn admits(&self, wing: &str) -> bool {
        match self {
            Self::Wing(only) => wing == *only,
            Self::AllBut(excluded) => !excluded.iter().any(|e| e == wing),
        }
    }
}

pub(crate) fn room(hit: &SearchHit) -> String {
    format!("{} / {}", hit.wing, hit.room)
}

fn palace(hits: &[SearchHit]) -> Palace {
    let rooms: BTreeSet<String> = hits.iter().map(room).collect();
    Palace {
        hits: hits.len(),
        rooms: rooms.len(),
        excerpts: hits
            .iter()
            .take(SHOWN_EXCERPTS)
            .map(|h| Excerpt {
                room: room(h),
                source: h.source_file.clone(),
                text: h.text.clone(),
            })
            .collect(),
    }
}

pub(crate) struct Pulled {
    pub(crate) matched: Vec<String>,
    pub(crate) relations: Vec<Relation>,
    pub(crate) total: usize,
    pub(crate) asks: Vec<Ask>,
}

impl Pulled {
    fn json(&self, hits: &[SearchHit], text: &str) -> Value {
        json!({
            "matched": self.matched,
            "relations": self.relations.iter().map(relation_json).collect::<Vec<_>>(),
            "total": self.total,
            "asks": self.asks.iter().map(|a| json!({"finding":a.finding,"claim":a.claim,"token":a.token,"line":a.line})).collect::<Vec<_>>(),
            "excerpts": hits.iter().take(SHOWN_EXCERPTS).map(|h| json!({"source":h.source_file,"room":room(h),"wing":h.wing,"text":context_block::clip(&h.text)})).collect::<Vec<_>>(),
            "hits": hits.len(),
            "text": text,
        })
    }
}

pub(crate) fn relation_json(r: &Relation) -> Value {
    let claim = r.claim.as_ref().map(|c| {
        json!({"id":c.id,"handle":c.handle,"status":c.status,"confidence":context_block::rounded(c.confidence),"source":c.source})
    });
    json!({"subject":r.subject,"predicate":r.predicate,"object":r.object,"triple":r.triple,
        "triple_confidence":r.confidence,"claim":claim,"line":context_block::relation_line(&line(r))})
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        options: Options,
    }
    fn parse(extra: &[&str]) -> Result<Options, clap::Error> {
        let base = [
            "palace",
            "--message",
            "m",
            "--kg",
            "k",
            "--store",
            "s",
            "--palace",
            "p",
        ];
        Cli::try_parse_from(base.iter().chain(extra)).map(|c| c.options)
    }
    #[test]
    fn one_wing_by_default() {
        let o = parse(&[]).unwrap();
        let scope = Scope::of(&o);
        assert_eq!(
            (scope.filter(), scope.limit()),
            (Some("openclaw"), SEARCH_HITS)
        );
        assert!(scope.admits("openclaw") && !scope.admits("work"));
        let o = parse(&["--wing", "work"]).unwrap();
        assert!(Scope::of(&o).admits("work") && !Scope::of(&o).admits("openclaw"));
    }
    #[test]
    fn all_wings_less_the_excluded() {
        let o = parse(&["--all-wings"]).unwrap();
        let scope = Scope::of(&o);
        assert_eq!((scope.filter(), scope.limit()), (None, SEARCH_HITS));
        assert!(scope.admits("openclaw") && scope.admits("work"));
        let o = parse(&[
            "--all-wings",
            "--exclude-wing",
            "private",
            "--exclude-wing",
            "health",
        ])
        .unwrap();
        let scope = Scope::of(&o);
        assert_eq!((scope.filter(), scope.limit()), (None, SEARCH_HITS * 4));
        assert!(scope.admits("openclaw") && !scope.admits("private") && !scope.admits("health"));
    }
    #[test]
    fn exclusions_need_all_wings() {
        assert!(parse(&["--exclude-wing", "private"]).is_err());
        assert!(parse(&["--all-wings", "--wing", "work"]).is_err());
        assert!(parse(&["--all-wings", "--exclude-wing", "../x"]).is_err());
    }
}
