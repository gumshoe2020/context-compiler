//! `palace distill`'s commit, modelled on rollup_commit.rs: the checked reply is logged first
//! (`distill_commit`, with the entry names it will write), then each step runs once and is logged
//! (`distill_step`): entries → mine → propose (the existing write gate and graph write path, one
//! `propose_relation` step per relation) → store (the claim store). A rerun from the same position
//! resumes the steps not logged.
use crate::distill::Options;
use palace_core::{
    claim::claim_id,
    diary_file,
    distill::{self, CopyLine, Reply},
    proposal::{EvidenceReference, Proposal},
    transcript::Source,
    write_gate::propose_relation,
};
use palace_mempalace::Unavailable;
use palace_store::{
    Error,
    claims::{ClaimStore, Distilled, DistilledEntry},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// One distill run: a transcript from a recorded position.
pub struct Run {
    pub key: String,
    pub source: Source,
    pub transcript: PathBuf,
    pub session: String,
    pub wing: String,
    pub from: usize,
    pub to: usize,
    /// compiler.sqlite, beside the KG.
    pub store: PathBuf,
}
impl Run {
    /// The palace copy: `<transcripts>/<wing>/<source>/<session>.jsonl`; an openclaw transcript
    /// is its own copy.
    pub fn copy_path(&self, options: &Options) -> Result<PathBuf, Error> {
        if self.source == Source::OpenClaw {
            return Ok(self.transcript.clone());
        }
        let file = options
            .transcripts
            .join(&self.wing)
            .join(self.source.name())
            .join(format!("{}.jsonl", self.session));
        crate::paths::database(&file)
    }
}

/// The run's commit: logged before any entry is written, then every step.
struct Commit {
    key: String,
    wing: String,
    wing_root: PathBuf,
    copy: String,
    at: i64,
    files: Vec<String>,
    reply: Reply,
    to: u64,
}

/// `checklist`: the values listed in the prompt, logged with the run (SETTLE-1d §5).
pub fn commit(
    options: &Options,
    run: &Run,
    reply: &Reply,
    lines: &[CopyLine],
    checklist: &[(usize, Vec<String>)],
) -> Result<i32, Error> {
    recover(options)?;
    let copy = run.copy_path(options)?.to_string_lossy().into_owned();
    let at = crate::rollup::now_millis()?;
    let range = match (lines.first(), lines.last()) {
        (Some(first), Some(last)) => format!("{copy}:{}-{}", first.line, last.line),
        _ => copy.clone(),
    };
    let files: Vec<String> = reply
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| render(at + i as i64, e, &range).name)
        .collect();
    log(
        run,
        json!({"operation":"distill_commit","run":run.key,"transcript":run.transcript,"source":run.source.name(),"session":run.session,"wing":run.wing,"from":run.from,"to":run.to,"copy":copy,"range":range,"at":at,"files":files,"reply":distill::to_json(reply),"checklist":checklist.iter().map(|(line, values)| json!({"line":line,"values":values})).collect::<Vec<_>>()}),
    )?;
    let commit = Commit {
        key: run.key.clone(),
        wing: run.wing.clone(),
        wing_root: wing_root(options, &run.wing)?,
        copy,
        at,
        files,
        reply: reply.clone(),
        to: run.to as u64,
    };
    finish(options, &commit, &range, "written")
}

/// A logged commit finished: a skipped one only reports again.
pub fn resume(options: &Options, event: Value) -> Result<i32, Error> {
    let to = event["to"].as_u64().unwrap_or(0);
    let wing = event["wing"].as_str().unwrap_or(&options.wing).to_owned();
    if event["skipped"] == true {
        println!("DISTILL skipped (resumed) wing={wing} position={to}");
        return Ok(0);
    }
    recover(options)?;
    let commit = Commit {
        key: event["run"].as_str().unwrap_or_default().to_owned(),
        wing_root: wing_root(options, &wing)?,
        wing,
        copy: event["copy"].as_str().unwrap_or_default().to_owned(),
        at: event["at"]
            .as_i64()
            .ok_or("logged distill_commit without at")?,
        files: event["files"]
            .as_array()
            .map(|f| {
                f.iter()
                    .filter_map(|n| n.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        reply: distill::from_json(&event["reply"]),
        to,
    };
    let range = event["range"].as_str().unwrap_or(&commit.copy).to_owned();
    finish(options, &commit, &range, "resumed")
}

fn finish(options: &Options, commit: &Commit, range: &str, verb: &str) -> Result<i32, Error> {
    let store = crate::rollup_commit::compiler_path(&options.kg)?;
    let done = done_steps(&store, &commit.key)?;
    if !done.contains("entries") {
        write_entries(commit, range)?;
        step(&store, commit, "entries", json!({}))?;
    }
    if !done.contains("mine") && !commit.files.is_empty() {
        mine(options, commit)?;
        step(&store, commit, "mine", json!({}))?;
    }
    let relations = propose_step(options, commit, &store, done.contains("propose"))?;
    let claims = commit.reply.claims.len();
    if !done.contains("store") && !commit.files.is_empty() {
        store_claims(&store, commit)?;
        step(&store, commit, "store", json!({}))?;
    }
    println!(
        "DISTILL {verb} entries={} claims={claims} relations={relations} wing={} copy={} position={}",
        commit.files.len(),
        commit.wing,
        commit.copy,
        commit.to
    );
    Ok(0)
}

fn render(at: i64, entry: &distill::DistilledEntry, range: &str) -> diary_file::DiaryFile {
    diary_file::render(at, &entry.title, &distill::entry_text(entry, range))
}
/// The wing's diary root: the default wing's `--diary`, a private wing's `<wings>/<wing>`.
fn wing_root(options: &Options, wing: &str) -> Result<PathBuf, Error> {
    if wing == options.wing {
        return crate::paths::diary_root(&options.diary);
    }
    let root = options.wings.join(wing);
    crate::paths::database(&root.join("diary").join("entry"))?; // creates <root>/diary
    crate::paths::diary_root(&root)
}
fn write_entries(commit: &Commit, range: &str) -> Result<(), Error> {
    for (i, entry) in commit.reply.entries.iter().enumerate() {
        let file = render(commit.at + i as i64, entry, range);
        let path = crate::paths::database(&commit.wing_root.join("diary").join(&file.name))?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut out) => out.write_all(file.content.as_bytes())?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {} // written before a crash
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        }
    }
    Ok(())
}
fn mine(options: &Options, commit: &Commit) -> Result<(), Error> {
    let palace = crate::paths::database(&options.palace)?.canonicalize()?;
    let to = palace_mempalace::mine::Target {
        wing: &commit.wing,
        ..Default::default()
    };
    palace_mempalace::mine::mine_to(
        &options.connection.mempalace,
        &palace,
        &commit.wing_root,
        &to,
    )
    .map_err(Unavailable::error)?;
    Ok(())
}
/// Unsettled graph intents are reconciled before anything is written (as `palace rollup`).
fn recover(options: &Options) -> Result<(), Error> {
    let mut graph = crate::graph::open(&crate::graph::GraphPaths {
        palace: Some(options.palace.clone()),
        kg: options.kg.clone(),
        wal: None,
    })?;
    crate::recovery::reconcile(&mut graph, "distill")
}

/// `<created>+<cited>`: each relation through the write gate, its outcome logged at once, so a
/// resume gates only the rest (as the rollup's propose step).
fn propose_step(
    options: &Options,
    commit: &Commit,
    store: &Path,
    done: bool,
) -> Result<String, Error> {
    let logged = relation_steps(store, &commit.key)?;
    if done || commit.reply.relations.is_empty() || commit.files.is_empty() {
        return Ok(count(&logged));
    }
    let evidence = options
        .connection
        .evidence(&options.palace, &commit.wing_root.join("diary"))
        .map_err(|e| Unavailable::error(format!("after mine: {e}")))?;
    let mut graph = crate::graph::open(&crate::graph::GraphPaths {
        palace: Some(options.palace.clone()),
        kg: options.kg.clone(),
        wal: None,
    })?;
    let mut outcomes = logged;
    let title = &commit.reply.entries[0].title;
    for (index, r) in commit.reply.relations.iter().enumerate() {
        if outcomes.iter().any(|(i, _)| *i == index) {
            continue;
        }
        let proposal = Proposal {
            from: r.subject.clone(),
            to: r.object.clone(),
            relation: r.predicate.clone(),
            evidence: vec![EvidenceReference {
                query: title.clone(),
            }],
            confidence: r.confidence,
            supplied_by: "palace-distill".into(),
            from_type: None,
            to_type: None,
            claim: None,
            claim_id: None,
            transcript: Some(format!("{}:{}", commit.copy, r.line)),
        };
        let now = graph.now()?;
        let result = propose_relation(&evidence, &mut graph, &proposal, &now, "palace-distill")
            .map_err(|e| crate::rollup::transport_or(&evidence, e))?;
        if let Some(reason) = evidence.searcher.unavailable() {
            return Err(Unavailable::error(format!("during distill: {reason}")));
        }
        let outcome = match (result["ok"] == true, result["value"]["created"] == true) {
            (false, _) => "rejected",
            (true, true) => "created",
            (true, false) => "cited",
        };
        eprintln!("distill: relation {index} {outcome}: {result}");
        graph.log(&json!({"operation":"distill_step","run":commit.key,"step":"propose_relation","index":index,"outcome":outcome,"triple_id":result["value"]["triple_id"]}))?;
        outcomes.push((index, outcome.to_owned()));
    }
    drop(graph);
    let summary = count(&outcomes);
    step(store, commit, "propose", json!({"relations": summary}))?;
    Ok(summary)
}
fn count(outcomes: &[(usize, String)]) -> String {
    let n = |o: &str| outcomes.iter().filter(|(_, x)| x == o).count();
    format!("{}+{}", n("created"), n("cited"))
}
fn relation_steps(store: &Path, key: &str) -> Result<Vec<(usize, String)>, Error> {
    Ok(events(store)?
        .into_iter()
        .filter(|e| e["run"] == key && e["step"] == "propose_relation")
        .filter_map(|e| {
            Some((
                e["index"].as_u64()? as usize,
                e["outcome"].as_str()?.to_owned(),
            ))
        })
        .collect())
}

/// The reply's claims, stated by the run's first entry, each with its transcript locator.
fn store_claims(store: &Path, commit: &Commit) -> Result<(), Error> {
    let file = &commit.files[0];
    let thread = &commit.reply.entries[0].title;
    let date = diary_file::new_york_date(commit.at);
    let rows: Vec<(String, String, String)> = commit
        .reply
        .claims
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let id = claim_id(&c.subject, &c.predicate, &c.object, &c.date, file);
            (
                id,
                format!("D{}", i + 1),
                format!("{}:{}", commit.copy, c.line),
            )
        })
        .collect();
    let claims: Vec<Distilled> = commit
        .reply
        .claims
        .iter()
        .zip(&rows)
        .map(|(c, (id, number, transcript))| Distilled {
            id,
            number,
            kind: &c.kind,
            subject: &c.subject,
            predicate: &c.predicate,
            object: &c.object,
            date: &c.date,
            quote: c.quote.as_deref(),
            transcript,
            demoted: c.demoted.as_deref(),
        })
        .collect();
    let mut claim_store = ClaimStore::open(store)?;
    let at = claim_store.now()?;
    let entry = DistilledEntry {
        file,
        date: &date,
        thread,
    };
    claim_store.record_distilled(&entry, &claims, &at)?;
    Ok(())
}

pub fn log(run: &Run, payload: Value) -> Result<(), Error> {
    ClaimStore::open(&run.store)?.log(&payload)
}
fn step(store: &Path, commit: &Commit, name: &str, extra: Value) -> Result<(), Error> {
    let mut payload = json!({"operation":"distill_step","run":commit.key,"step":name});
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        payload.extend(extra.clone());
    }
    ClaimStore::open(store)?.log(&payload)
}
fn events(store: &Path) -> Result<Vec<Value>, Error> {
    if !store.is_file() {
        return Ok(Vec::new());
    }
    ClaimStore::open(store)?.events("distill_step")
}
fn done_steps(store: &Path, key: &str) -> Result<BTreeSet<String>, Error> {
    Ok(events(store)?
        .into_iter()
        .filter(|e| e["run"] == key)
        .filter_map(|e| e["step"].as_str().map(str::to_owned))
        .collect())
}
