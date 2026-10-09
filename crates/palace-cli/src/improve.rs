//! `palace improve <verb> <target> ["<text>"] --by <who>` and `palace improve undo <event-id>`
//! (M4d item 3). One call = one ImprovementEvent (row + hash-chained log line), its graph effect
//! (new triples through the write gate; `valid_to` / confidence on the claim's direct edges) and
//! one append-only IMPROVE diary entry written with the M4b recipe. Exit 0 applied, 1 refused,
//! 2 broken, 3 MemPalace unavailable.
use clap::Args;
use palace_core::{
    diary_file,
    improve::{Verb, valid_by},
    write_gate::{check_proposal, propose_relation},
};
use palace_mempalace::Unavailable;
use palace_store::{
    Error, Refused, Skipped,
    claims::ClaimStore,
    graph::{Applied, Graph, Prepared, Request},
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, path::PathBuf};

#[derive(Args)]
#[command(mut_arg("kg", |a| a.required(false).required_unless_present_any(STORE_ONLY)))]
pub struct Options {
    /// confirm | contradict | add | weaken | retract | split | expire | coin | undo | escalate
    #[arg(required_unless_present_any = ["auto", "reply", "digest", "unsent", "mark_sent", "classify"])]
    pub verb: Option<String>,
    /// `c_…`, `f_…`, `#f_…` or "<rollup date> C7"; for `undo`, the event id.
    #[arg(required_unless_present_any = ["auto", "reply", "digest", "unsent", "mark_sent", "classify"])]
    pub target: Option<String>,
    /// The human sentence; for contradict/add the new claim (`<subject> | <predicate> |
    /// <object> [— <date> — <source> [— quote: "…"]]`), for split two claims joined by ` || `,
    /// for coin `<word>: <definition> || <what it is NOT>`.
    pub text: Option<String>,
    /// The person | the assistant (`palace --person/--assistant`) | rule:<name> |
    /// judge:<model> (`--auto` acts as rule:auto).
    #[arg(long, required_unless_present_any = ["auto", "digest", "unsent", "mark_sent", "classify"])]
    pub by: Option<String>,
    /// M4d item 4: the automatic loop over every open finding (SPEC §5.1).
    #[arg(long, conflicts_with_all = ["verb", "reply"])]
    pub auto: bool,
    /// `--auto` only: print the plan; write nothing.
    #[arg(long, requires = "auto")]
    pub dry_run: bool,
    /// `--auto` only: `table:<file>` (JSON `{"C5": "reversed", …}`), `replay`, `claude-cli[:model]`
    /// or `command:<exe>`; asked only to locate a passage when grounding. None: no judge.
    #[arg(long, requires = "auto")]
    pub judge: Option<String>,
    /// `--auto` only: automatic non-weaken verbs per run (at most one of them a `coin`).
    #[arg(long, default_value_t = palace_core::auto::MAX_VERBS)]
    pub max_verbs: usize,
    /// The claim store; must be the compiler.sqlite beside `--kg` (the default).
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// M4d item 5: a Signal reply (`keep`, `accept`, `fix: <text>`, `source: <file>`, `drop`,
    /// `veto <word>`; `ok` and `no` are refused).
    #[arg(long, conflicts_with = "verb")]
    pub reply: Option<String>,
    /// M4d-2d item 3: the corrections digest (people's corrections with `undo e_…`, one rule
    /// line, open counts; nothing when there is nothing). Needs only `--store`.
    #[arg(long, conflicts_with_all = ["verb", "auto", "reply"])]
    pub digest: bool,
    /// M4d-2e item 2: every open ask not yet marked sent, one Signal line each, oldest first (the
    /// wrapper's source of truth for delivery). Needs only `--store`.
    #[arg(long, conflicts_with_all = ["verb", "auto", "reply", "digest", "mark_sent"])]
    pub unsent: bool,
    /// M4d-2e item 2: mark the asks with these reply tokens (`#f_…`) sent; idempotent; an unknown
    /// token is refused and nothing is marked. Needs only `--store`.
    #[arg(long, num_args = 1.., value_name = "TOKEN", conflicts_with_all = ["verb", "auto", "reply", "digest"])]
    pub mark_sent: Vec<String>,
    /// `--digest` only: events at or after this date or time (default: the last 24 hours).
    #[arg(long, requires = "digest")]
    pub since: Option<String>,
    /// `--digest` only: at most this many lines.
    #[arg(long, default_value_t = palace_core::digest::DIGEST_MAX)]
    pub max: usize,
    /// `--reply` only: the reply token `#f_…`; without it the reply needs exactly one open ask.
    #[arg(long, requires = "reply")]
    pub token: Option<String>,
    /// M4d item 7: where the person's and the assistant's contradicts append canaries (default
    /// `<palace>/canaries.json`; never under fixtures/).
    #[arg(long)]
    pub canaries_out: Option<PathBuf>,
    /// `add` only: supersede the anchor claim by the new one even when the anchor is grounded
    /// (an ungrounded anchor is always superseded).
    #[arg(long)]
    pub supersede: bool,
    /// `weaken` only: the event is a failed grounding attempt (SPEC §5.1 row 2): it decides no
    /// finding and always lowers confidence by 0.15. Only on an ungrounded claim, by a rule, the
    /// assistant or the person; once per claim per UTC day (a second prints IMPROVE SKIPPED, exit 0).
    #[arg(long)]
    pub grounding_failed: bool,
    /// M5c item 3: the verb a correction said in conversation calls for (one of the eight, or
    /// `ambiguous: a | b`, or `none`), from cue words and the claim's open findings; a
    /// suggestion only, nothing is applied. Needs `--claim` and `--store`. A text beginning
    /// with `-` is taken as the text (M5c.3 item 5).
    #[arg(long, allow_hyphen_values = true, requires = "claim", conflicts_with_all = ["verb", "auto", "reply", "digest", "unsent", "mark_sent"])]
    pub classify: Option<String>,
    /// `--classify` only: the claim the correction is about (`c_…` or "<rollup date> C7").
    #[arg(long, requires = "classify")]
    pub claim: Option<String>,
    /// `--kg` (required but for `--digest`), `--palace`, `--wal`.
    #[command(flatten)]
    pub paths: Option<crate::graph::GraphPaths>,
    /// Diary root (the parent of `diary/`): the IMPROVE entry is written there.
    #[arg(long, required_unless_present_any = STORE_ONLY)]
    pub diary: Option<PathBuf>,
    #[command(flatten)]
    pub connection: crate::MemPalaceOptions,
}

/// Modes that need only the claim store (no `--kg`, no `--diary`).
const STORE_ONLY: [&str; 4] = ["digest", "unsent", "mark_sent", "classify"];

pub fn run(o: Options) -> i32 {
    if let Some(code) = other_mode(&o) {
        return code;
    }
    let (verb, target) = (
        o.verb.as_deref().unwrap_or(""),
        o.target.as_deref().unwrap_or(""),
    );
    match apply(&o) {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(e) if Skipped::is(&e) => {
            println!("IMPROVE SKIPPED {verb} {target}: {e}");
            0
        }
        Err(e) => fail("palace improve", e),
    }
}

/// The modes other than one verb on one claim, when one is asked for.
fn other_mode(o: &Options) -> Option<i32> {
    Some(match () {
        _ if o.classify.is_some() => classify(o, o.classify.as_deref().unwrap_or_default()),
        _ if o.unsent || !o.mark_sent.is_empty() => crate::delivery::run(o),
        _ if o.auto => crate::auto::run(o),
        _ if o.digest => crate::digest::run(o),
        _ if o.reply.is_some() => crate::reply::run(o),
        _ if o.verb.as_deref() == Some("escalate") => crate::escalate::run(o),
        _ => return None,
    })
}

/// `--classify`: prints the suggestion; reads the store read-only and writes nothing.
fn classify(o: &Options, text: &str) -> i32 {
    let kinds = || -> Result<Vec<String>, Error> {
        let store = crate::paths::input(o.store.as_deref().ok_or("--classify needs --store")?)?;
        palace_store::context::open_finding_kinds(&store, o.claim.as_deref().unwrap_or_default())
    };
    match kinds() {
        Ok(kinds) => {
            println!("{}", palace_core::classify::classify(text, &kinds));
            0
        }
        Err(e) => fail("palace improve --classify", e),
    }
}

/// Prints `<context>: <error>` (lock contention reworded to name the database) and returns its
/// exit code.
pub fn fail(context: &str, e: Error) -> i32 {
    let e = palace_store::busy::named(e);
    eprintln!("{context}: {e}");
    exit_code(&e)
}

/// 1 refused, 3 MemPalace unavailable or a database locked (retry later), else 2.
pub fn exit_code(e: &Error) -> i32 {
    if palace_store::busy::Busy::is(e) || palace_store::busy::locked(e) {
        3
    } else if Refused::is(e) {
        1
    } else if Unavailable::is(e) {
        3
    } else {
        2
    }
}

/// Where verbs act: the graph paths, MemPalace for the write gate, the diary root for IMPROVE
/// entries, and the canaries-out file for the person's and the assistant's contradicts.
pub struct Ctx<'a> {
    pub paths: &'a crate::graph::GraphPaths,
    pub connection: &'a crate::MemPalaceOptions,
    pub root: PathBuf,
    pub canaries_out: PathBuf,
}

/// One verb (or `undo`) on one target.
pub struct Call<'a> {
    pub verb: &'a str,
    pub target: &'a str,
    pub text: Option<&'a str>,
    pub by: &'a str,
    pub supersede: bool,
    pub grounding_failed: bool,
}

/// A written event and its IMPROVE entry's file name.
pub struct Done {
    pub applied: Applied,
    pub entry: String,
}

impl Options {
    /// The graph paths; `--kg` is required for everything but `--digest`.
    pub fn graph_paths(&self) -> Result<&crate::graph::GraphPaths, Error> {
        self.paths
            .as_ref()
            .ok_or_else(|| Refused::error("--kg is required"))
    }
}

impl Ctx<'_> {
    pub fn of(o: &Options) -> Result<Ctx<'_>, Error> {
        let paths = o.graph_paths()?;
        let root = crate::paths::diary_root(o.diary.as_deref().ok_or("--diary is required")?)?;
        let compiler = crate::rollup_commit::compiler_path(&paths.kg)?;
        if let Some(store) = &o.store
            && crate::paths::database(store)? != compiler
        {
            return Err(Refused::error(format!(
                "--store {} is not the claim store beside --kg ({})",
                store.display(),
                compiler.display()
            )));
        }
        Ok(Ctx {
            paths,
            connection: &o.connection,
            root,
            canaries_out: crate::canary_out::path(o)?,
        })
    }
    pub fn compiler(&self) -> Result<PathBuf, Error> {
        crate::rollup_commit::compiler_path(&self.paths.kg)
    }
}

fn apply(o: &Options) -> Result<String, Error> {
    let by = o.by.as_deref().unwrap_or_default();
    let ctx = Ctx::of(o)?;
    recover(&ctx)?;
    let call = Call {
        verb: o.verb.as_deref().unwrap_or_default(),
        target: o.target.as_deref().unwrap_or_default(),
        text: o.text.as_deref(),
        by,
        supersede: o.supersede,
        grounding_failed: o.grounding_failed,
    };
    let done = apply_call(&ctx, &call)?;
    Ok(ok_line(&done))
}

/// `IMPROVE OK <verb> <date> Cn event=… reversible_by=… to_claim=… entry=…`.
pub fn ok_line(done: &Done) -> String {
    let applied = &done.applied;
    format!(
        "IMPROVE OK {} {} event={} reversible_by={} to_claim={} entry={}",
        applied.verb,
        label(&applied.listed),
        applied.event,
        applied.reversible_by,
        applied.to_claim.as_deref().unwrap_or("-"),
        done.entry
    )
}

/// One verb through the whole path: the event (graph and claim store), its IMPROVE entry, and
/// for a person's contradict (or its undo) the canary (M4d item 7, M4d-2c item 2). Recovery is the caller's.
pub fn apply_call(ctx: &Ctx, call: &Call) -> Result<Done, Error> {
    if !valid_by(call.by) {
        return Err(Refused::error(format!(
            "--by {:?}: use {}, {}, rule:<name> or judge:<model>",
            call.by,
            palace_core::names::person(),
            palace_core::names::assistant()
        )));
    }
    let applied = event(ctx, call)?;
    if let Some(warning) = &applied.wal_warning {
        eprintln!(
            "palace improve: warning: palace WAL not written yet ({warning}); the outbox keeps the lines"
        );
    }
    let file = record_entry(ctx, &applied)?;
    crate::canary_out::follow(ctx, &applied)?;
    Ok(Done {
        applied,
        entry: file.name,
    })
}

/// The IMPROVE entry for `applied`, and its `improve_entry` line in the compiler's write log.
fn record_entry(ctx: &Ctx, applied: &Applied) -> Result<Entry, Error> {
    entry_in(&ctx.root, &ctx.compiler()?, applied)
}

/// The IMPROVE entry of an event made outside `apply_call` (a term's adopt or veto); its file name.
pub fn record_entry_at(root: &Path, compiler: &Path, applied: &Applied) -> Result<String, Error> {
    Ok(entry_in(root, compiler, applied)?.name)
}

fn entry_in(root: &Path, compiler: &Path, applied: &Applied) -> Result<Entry, Error> {
    let file = write_entry(root, applied)?;
    ClaimStore::open(compiler)?.log(&json!({"operation":"improve_entry","event":applied.event,"file":file.name,"iso":diary_file::iso_millis(file.at),"title":file.title,"text_sha256":palace_core::ids::sha256(file.text.as_bytes())}))?;
    Ok(file)
}

/// M4d-2a item 5 (L-3): an earlier call interrupted between stores is finished first — every
/// unsettled intent reconciled (M4d-2a.1 item 2), a pending step that never committed is rolled back, a committed event without its IMPROVE
/// entry gets it now.
pub fn recover(ctx: &Ctx) -> Result<(), Error> {
    let mut graph = crate::graph::open(ctx.paths)?;
    crate::recovery::reconcile(&mut graph, "improve")?;
    for rolled in graph.recover()? {
        eprintln!(
            "palace improve: rolled back unfinished {} {} ({} change(s))",
            rolled["verb"].as_str().unwrap_or("?"),
            rolled["key"].as_str().unwrap_or("?"),
            rolled["changes"].as_array().map_or(0, Vec::len)
        );
    }
    let missing = graph.unrecorded()?;
    drop(graph);
    for applied in &missing {
        let file = record_entry(ctx, applied)?;
        eprintln!(
            "palace improve: resumed entry {} for {}",
            file.name, applied.event
        );
    }
    Ok(())
}

/// The event itself (graph and claim store), with the graph closed again before the diary write.
fn event(ctx: &Ctx, call: &Call) -> Result<Applied, Error> {
    let mut graph = crate::graph::open(ctx.paths)?;
    if call.verb == "undo" {
        return graph.undo(call.target, call.by);
    }
    let verb = Verb::parse(call.verb)
        .ok_or_else(|| Refused::error(format!("unknown verb {:?}", call.verb)))?;
    let request = Request {
        verb,
        target: call.target,
        text: call.text,
        by: call.by,
        supersede: call.supersede,
        grounding_failed: call.grounding_failed,
    };
    let mut prepared = graph.prepare(&request)?;
    if !prepared.proposals.is_empty() || prepared.coinage.is_some() {
        graph.begin(&mut prepared)?;
    }
    let result = finish(ctx, &mut graph, &prepared);
    if result.is_err()
        && prepared.pending.is_some()
        && let Err(e) = graph.recover()
    {
        eprintln!("palace improve: rollback deferred to the next call: {e}");
    }
    result
}

/// The cross-store part: gated triples, the coined entity, then the event.
fn finish(ctx: &Ctx, graph: &mut Graph, p: &Prepared) -> Result<Applied, Error> {
    let created = gate(ctx, graph, p)?;
    coin_entity(graph, p)?;
    graph.commit(p, &created)
}

/// Every new triple passes the write gate (MemPalace evidence) before any is written; a
/// rejection refuses the whole call. Returns the triples actually created.
fn gate(ctx: &Ctx, graph: &mut Graph, p: &Prepared) -> Result<Vec<String>, Error> {
    if p.proposals.is_empty() {
        return Ok(Vec::new());
    }
    let palace = ctx
        .paths
        .palace
        .as_deref()
        .ok_or("improve needs --palace for the write gate's evidence")?;
    let evidence = ctx
        .connection
        .evidence(palace, &ctx.root.join("diary"))
        .map_err(Unavailable::error)?;
    check_all(&evidence, &p.proposals)?;
    let mut created = Vec::new();
    for proposal in &p.proposals {
        let now = graph.now()?;
        let value =
            propose_relation(&evidence, graph, proposal, &now, "palace-improve")?["value"].take();
        if value["created"] == true {
            created.push(
                value["triple_id"]
                    .as_str()
                    .ok_or("gate result without triple_id")?
                    .to_owned(),
            );
        }
    }
    Ok(created)
}

fn check_all(
    evidence: &palace_mempalace::Evidence,
    proposals: &[palace_core::proposal::Proposal],
) -> Result<(), Error> {
    for proposal in proposals {
        let rejected = check_proposal(evidence, proposal);
        if let Some(reason) = evidence.searcher.unavailable() {
            return Err(Unavailable::error(reason));
        }
        if let Some(rejected) = rejected {
            return Err(Refused::error(format!(
                "write gate: {}",
                rejected["reason"]
            )));
        }
    }
    Ok(())
}

/// `coin`: the term entity (`type=term`, status proposed) through the audited graph mutation.
fn coin_entity(graph: &mut Graph, p: &Prepared) -> Result<(), Error> {
    let Some(coined) = &p.coinage else {
        return Ok(());
    };
    let properties = json!({"kind":"predicate","status":"proposed","definition":coined.definition,"negative_space":coined.negative_space,"coined_by":p.by,"source":p.claim["source"]});
    graph.call(
        "set_entity",
        &json!({"name":coined.word,"type":"term","properties":properties}),
    )?;
    Ok(())
}

/// `<rollup date> C7` of the improved claim.
fn label(listed: &Value) -> String {
    format!(
        "{} {}",
        listed["date"].as_str().unwrap_or("?"),
        listed["number"].as_str().unwrap_or("?")
    )
}

struct Entry {
    name: String,
    title: String,
    text: String,
    at: i64,
}

/// The IMPROVE entry (M4b recipe): title `IMPROVE: <verb> <date> C7 — <thread>`, tags
/// `claim-improvement`, the thread, and `no-compress` iff `--by <person>`. Never overwrites.
fn write_entry(root: &Path, a: &Applied) -> Result<Entry, Error> {
    let thread = a.listed["thread"].as_str().unwrap_or("unknown");
    let title = format!("IMPROVE: {} {} — {thread}", a.verb, label(&a.listed));
    let text = entry_text(a, &thread_tag(thread));
    let at = crate::rollup::now_millis()?;
    let file = diary_file::render(at, &title, &text);
    let path = crate::paths::database(&root.join("diary").join(&file.name))?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?
        .write_all(file.content.as_bytes())?;
    Ok(Entry {
        name: file.name,
        title: file.title,
        text: file.text,
        at,
    })
}

fn entry_text(a: &Applied, tag: &str) -> String {
    let no_compress = if a.by == palace_core::names::person() {
        ", no-compress"
    } else {
        ""
    };
    let none = |v: &Option<String>| v.clone().unwrap_or_else(|| "none".into());
    format!(
        "Tags: claim-improvement, {tag}{no_compress}.\n\n{} applied `{}` to {} ({}) in {}: {}\n\nEvent {}; reversible by {}; axis {}; finding {}; successor {}.",
        a.by,
        a.verb,
        label(&a.listed),
        none(&a.from_claim),
        a.listed["rollup"].as_str().unwrap_or("?"),
        if a.text.is_empty() {
            "(no text)"
        } else {
            &a.text
        },
        a.event,
        a.reversible_by,
        a.axis.join(", "),
        none(&a.finding),
        none(&a.to_claim),
    )
}

/// A thread name as a tag: lowercase, runs of other characters → `-`.
pub fn thread_tag(thread: &str) -> String {
    thread
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    #[test]
    fn thread_tags() {
        assert_eq!(
            super::thread_tag("synthetic claims test"),
            "synthetic-claims-test"
        );
        assert_eq!(
            super::thread_tag("harbor-walk-etiquette / Halloran"),
            "harbor-walk-etiquette-halloran"
        );
    }
}
