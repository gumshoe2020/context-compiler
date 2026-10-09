//! Rollup stage 6: write the entry by the diary plugin's recipe, record it in the write log,
//! mine it, gate its relations (claims cited), watch it, and store its claims.
//! Idempotent (M4b.1 S-4): a rollup already committed for the same thread + as-of (write-log
//! `rollup_commit` event, else a diary file whose first line is the target title) is never
//! written twice; the steps not yet logged as `rollup_step` events are resumed against it.
//! Orphans (M4b.2 N-2): a `rollup_commit` whose file exists but has no `store` step is finished at
//! the start of the next non-dry run, whatever its as-of or selected thread.
use crate::watch;
use palace_core::{
    batch::parse_batch,
    canary::Canary,
    claim::{self, Claim},
    diary_file,
    proposal::Proposal,
    rollup::Entry,
    rollup_validate::{cite_claims, relations_block, split_title},
    traits::{DiaryReader, Judge},
    write_gate::propose_relation,
};
use palace_mempalace::{Unavailable, diary::Diary};
use palace_store::{Error, claims::ClaimStore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct Inputs<'a> {
    pub root: &'a Path,
    pub diary: &'a Diary,
    pub palace: &'a Path,
    pub kg: &'a Path,
    pub connection: &'a crate::MemPalaceOptions,
    /// `palace rollup --wing`: where the mine files the rollup.
    pub wing: &'a str,
    pub thread: String,
    pub as_of: &'a str,
    pub judge: Option<&'a dyn Judge<Error = Error>>,
    /// `--llm`, which names the judge on its findings (`judge:<model>`).
    pub llm: &'a str,
    pub canaries: &'a [Canary],
}
/// Steps after the write, in order; each is logged once done.
const STEPS: [&str; 4] = ["mine", "propose", "watch", "store"];

pub fn commit(inputs: Inputs, body: &str) -> Result<i32, Error> {
    let written = write_entry(&inputs, body)?;
    finish(&inputs, &written, BTreeMap::new(), "written")
}
/// The rollup already committed for this thread + as-of, if any (never opens a missing store).
pub fn find_prior(inputs: &Inputs, title: &str) -> Result<Option<String>, Error> {
    let compiler = compiler_path(inputs.kg)?;
    if compiler.is_file() {
        let logged = ClaimStore::open(&compiler)?
            .events("rollup_commit")?
            .into_iter()
            .find(|e| e["thread"] == inputs.thread.as_str() && e["as_of"] == inputs.as_of)
            .and_then(|e| e["file"].as_str().map(str::to_owned));
        if let Some(file) = logged.filter(|f| inputs.diary.read(f).is_ok_and(|t| t.is_some())) {
            return Ok(Some(file));
        }
    }
    let heading = format!("# {title}");
    for name in inputs.diary.names()? {
        let text = inputs.diary.read(&name)?.unwrap_or_default();
        if text.lines().next().map(str::trim) == Some(heading.as_str()) {
            return Ok(Some(format!("{name}.md")));
        }
    }
    Ok(None)
}
/// A committed rollup whose `store` step was never logged.
pub struct Orphan {
    pub file: String,
    pub thread: String,
    pub as_of: String,
}
/// Orphans in write-log order: `rollup_commit` events whose file still exists in the diary and
/// that have no `rollup_step` event with `step: "store"` (never opens a missing store).
pub fn orphans(kg: &Path, diary: &Diary) -> Result<Vec<Orphan>, Error> {
    let compiler = compiler_path(kg)?;
    if !compiler.is_file() {
        return Ok(Vec::new());
    }
    let store = ClaimStore::open(&compiler)?;
    let mut finished: BTreeSet<String> = store
        .events("rollup_step")?
        .into_iter()
        .filter(|e| e["step"] == "store")
        .filter_map(|e| e["file"].as_str().map(str::to_owned))
        .collect();
    let text = |e: &Value, key: &str| e[key].as_str().map(str::to_owned);
    Ok(store
        .events("rollup_commit")?
        .into_iter()
        .filter_map(|e| {
            let (file, thread, as_of) =
                (text(&e, "file")?, text(&e, "thread")?, text(&e, "as_of")?);
            Some(Orphan {
                file,
                thread,
                as_of,
            })
        })
        .filter(|o| finished.insert(o.file.clone())) // unfinished, and each file once
        .filter(|o| diary.read(&o.file).is_ok_and(|t| t.is_some()))
        .collect())
}
/// Resume the steps a previous commit of `file` did not finish.
pub fn resume(inputs: Inputs, file: &str) -> Result<i32, Error> {
    let text = inputs.diary.read(file)?.ok_or("resumed rollup vanished")?;
    let written = Written {
        path: crate::paths::input(&inputs.root.join("diary").join(file))?,
        entry: Entry::parse(file, text),
    };
    let done = done_steps(&inputs, file)?;
    finish(&inputs, &written, done, "resumed")
}
struct Written {
    path: PathBuf,
    entry: Entry,
}
impl Written {
    fn file(&self) -> String {
        format!("{}.md", self.entry.name)
    }
    fn claims(&self) -> Vec<Claim> {
        claim::parse(&self.entry.text).claims
    }
    /// The relations block with each relation's claim number resolved to its claim id.
    fn relations(&self) -> Result<Vec<Proposal>, Error> {
        let Some(block) = relations_block(&self.entry.text) else {
            return Ok(Vec::new());
        };
        let mut relations = parse_batch(block, None)?;
        cite_claims(&mut relations, &self.claims());
        Ok(relations)
    }
}
/// Run every step not in `done`, logging each; print the one-line outcome.
fn finish(
    inputs: &Inputs,
    written: &Written,
    done: BTreeMap<String, Value>,
    verb: &str,
) -> Result<i32, Error> {
    let mut steps = Steps {
        inputs,
        written,
        done,
        ran: Vec::new(),
    };
    if !steps.done.contains_key("mine") {
        mine(inputs)?;
        steps.record("mine", json!({}))?;
    }
    let relations = propose_step(&mut steps)?;
    let status = watch_and_store(&mut steps)?;
    let resumed = match verb {
        "resumed" => format!(" steps={}", steps.summary()),
        _ => String::new(),
    };
    let (path, claims) = (written.path.display(), written.claims().len());
    println!("ROLLUP {verb} {path}{resumed} claims={claims} relations={relations} watch={status}");
    Ok(0)
}
/// The step log of one rollup file: what earlier runs finished, and what this run did.
struct Steps<'s> {
    inputs: &'s Inputs<'s>,
    written: &'s Written,
    done: BTreeMap<String, Value>,
    ran: Vec<&'static str>,
}
impl Steps<'_> {
    /// Logs `step` (with `extra`) and remembers that this run did it.
    fn record(&mut self, step: &'static str, extra: Value) -> Result<(), Error> {
        log_step(self.inputs, self.written, step, extra)?;
        self.ran.push(step);
        Ok(())
    }
    fn summary(&self) -> String {
        match self.ran.is_empty() {
            true => "none".to_owned(),
            false => self.ran.join(","),
        }
    }
}
fn mine(inputs: &Inputs) -> Result<(), Error> {
    let palace = crate::paths::database(inputs.palace)?.canonicalize()?;
    let to = palace_mempalace::mine::Target {
        wing: inputs.wing,
        ..Default::default()
    };
    palace_mempalace::mine::mine_to(&inputs.connection.mempalace, &palace, inputs.root, &to)
        .map_err(Unavailable::error)?;
    Ok(())
}
/// `<created>+<cited>`, gating the relations unless an earlier run finished `propose`.
fn propose_step(steps: &mut Steps) -> Result<String, Error> {
    if let Some(event) = steps.done.get("propose") {
        return Ok(format!("{}+{}", event["created"], event["cited"]));
    }
    let logged = relation_steps(steps.inputs, &steps.written.file())?;
    let r = propose(steps.inputs, steps.written, &logged)?;
    steps.record("propose", json!({"created":r.created,"cited":r.cited}))?;
    Ok(format!("{}+{}", r.created, r.cited))
}
/// Watch the rollup (read-only) and append its claims + findings to the claim store; the watch
/// status (logged, or recomputed when only `store` is missing).
fn watch_and_store(steps: &mut Steps) -> Result<String, Error> {
    if steps.done.contains_key("store") {
        let logged = steps.done.get("watch").and_then(|e| e["status"].as_str());
        return Ok(logged.unwrap_or("ok").to_owned());
    }
    let inputs = steps.inputs;
    let entries = [steps.written.entry.clone()];
    let mut report = watch::evaluate(&entries, inputs.diary, inputs.canaries, inputs.judge)?;
    let status = status(&report);
    if !steps.done.contains_key("watch") {
        steps.record("watch", json!({"status": status}))?;
    }
    let judge = watch::judge_label(inputs.llm);
    watch::persist_claims(&mut report, &entries, &compiler_path(inputs.kg)?, &judge)?;
    steps.record("store", json!({}))?;
    Ok(status)
}
fn write_entry(inputs: &Inputs, body: &str) -> Result<Written, Error> {
    let (title, text) = split_title(body);
    let at = crate::rollup::now_millis()?;
    let file = diary_file::render(at, &title, &text); // render trims, as the plugin does
    let path = crate::paths::database(&inputs.root.join("diary").join(&file.name))?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?
        .write_all(file.content.as_bytes())?;
    let mut store = ClaimStore::open(&compiler_path(inputs.kg)?)?;
    let text_sha256 = palace_core::ids::sha256(file.text.as_bytes()); // the text written (L-2)
    store.log(&json!({"operation":"rollup_commit","file":file.name,"iso":diary_file::iso_millis(at),"title":file.title,"text_sha256":text_sha256,"thread":inputs.thread,"as_of":inputs.as_of}))?;
    Ok(Written {
        path,
        entry: Entry::parse(&file.name, file.content),
    })
}
/// Completed steps of `file` (step name → its logged event), from the write log.
fn done_steps(inputs: &Inputs, file: &str) -> Result<BTreeMap<String, Value>, Error> {
    let compiler = compiler_path(inputs.kg)?;
    if !compiler.is_file() {
        return Ok(BTreeMap::new());
    }
    Ok(ClaimStore::open(&compiler)?
        .events("rollup_step")?
        .into_iter()
        .filter(|e| e["file"] == file)
        .filter_map(|e| Some((e["step"].as_str()?.to_owned(), e)))
        .filter(|(step, _)| STEPS.contains(&step.as_str()))
        .collect())
}
fn log_step(inputs: &Inputs, written: &Written, step: &str, extra: Value) -> Result<(), Error> {
    let mut payload = json!({"operation":"rollup_step","file":written.file(),"step":step});
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        payload.extend(extra.clone());
    }
    ClaimStore::open(&compiler_path(inputs.kg)?)?.log(&payload)
}
/// Relations of a commit: fresh triples created, and existing triples cited (M4b.1 S-5).
#[derive(Default)]
struct Relations {
    created: usize,
    cited: usize,
    citations: Vec<(String, String)>,
}
impl Relations {
    /// Counts one `propose_relation` step (logged now or by an interrupted run).
    fn add(&mut self, step: &Value) {
        match step["outcome"].as_str() {
            Some("created") => self.created += 1,
            Some("cited") => {
                if let (Some(triple), Some(claim)) =
                    (step["triple_id"].as_str(), step["claim_id"].as_str())
                {
                    self.citations.push((triple.to_owned(), claim.to_owned()));
                    self.cited += 1;
                }
            }
            _ => {}
        }
    }
}
/// Gate each relation for real (a fresh triple carries `provenance.claim_id`); a relation that
/// resolves to an existing triple becomes a `citations` row instead. Each relation's outcome is
/// logged as a `propose_relation` step as soon as it is known (M4b.2 L-1), so a resume after a
/// mid-propose `Unavailable` re-gates only the rest and still counts this rollup's own fresh
/// triples as `created`.
fn propose(
    inputs: &Inputs,
    written: &Written,
    logged: &BTreeMap<usize, Value>,
) -> Result<Relations, Error> {
    let evidence = inputs
        .connection
        .evidence(inputs.palace, &inputs.root.join("diary"))
        .map_err(|e| Unavailable::error(format!("after mine: {e}")))?;
    let disputed = disputed_claims(inputs.kg)?;
    let mut graph = crate::graph::open(&crate::graph::GraphPaths {
        palace: Some(inputs.palace.to_owned()),
        kg: inputs.kg.to_owned(),
        wal: None,
    })?;
    let mut relations = Relations::default();
    for (index, relation) in written.relations()?.into_iter().enumerate() {
        let step = match logged.get(&index) {
            Some(step) => step.clone(),
            None => {
                let at = (index, relation);
                gate_unless_disputed(&evidence, &mut graph, at, (written, &disputed))?
            }
        };
        relations.add(&step);
    }
    drop(graph); // the graph holds compiler.sqlite's writer lock
    cite_all(inputs, written, &relations)?;
    Ok(relations)
}
/// The `citations` rows of the relations that resolved to existing triples (idempotent).
fn cite_all(inputs: &Inputs, written: &Written, relations: &Relations) -> Result<(), Error> {
    let mut store = ClaimStore::open(&compiler_path(inputs.kg)?)?;
    for (triple, claim) in &relations.citations {
        store.cite(triple, claim, &written.file())?;
    }
    Ok(())
}
/// Claims marked `disputed` in the claim store (none before the store exists).
fn disputed_claims(kg: &Path) -> Result<Vec<String>, Error> {
    let compiler = compiler_path(kg)?;
    if !compiler.is_file() {
        return Ok(Vec::new());
    }
    ClaimStore::open(&compiler)?.with_status("disputed")
}
/// M4d-2c item 1: a relation whose claim is disputed is never proposed (its step is logged
/// `disputed`); any other is gated.
fn gate_unless_disputed(
    evidence: &palace_mempalace::Evidence,
    graph: &mut palace_store::graph::Graph,
    (index, relation): (usize, Proposal),
    (written, disputed): (&Written, &[String]),
) -> Result<Value, Error> {
    match &relation.claim_id {
        Some(claim) if disputed.contains(claim) => skip_disputed(graph, (index, relation), written),
        _ => gate_one(evidence, graph, (index, relation), written),
    }
}
fn skip_disputed(
    graph: &mut palace_store::graph::Graph,
    (index, relation): (usize, Proposal),
    written: &Written,
) -> Result<Value, Error> {
    let step = json!({"operation":"rollup_step","file":written.file(),"step":"propose_relation",
        "index":index,"outcome":"disputed","claim_id":relation.claim_id});
    eprintln!("rollup: relation {index} not proposed: claim disputed");
    graph.log(&step)?;
    Ok(step)
}
/// Gates one relation and logs its `propose_relation` step; a transport failure is returned
/// before anything is logged, so the relation is gated again on resume.
fn gate_one(
    evidence: &palace_mempalace::Evidence,
    graph: &mut palace_store::graph::Graph,
    (index, relation): (usize, Proposal),
    written: &Written,
) -> Result<Value, Error> {
    let now = graph.now()?;
    let result = propose_relation(evidence, graph, &relation, &now, "palace-rs")
        .map_err(|e| crate::rollup::transport_or(evidence, e))?;
    if let Some(reason) = evidence.searcher.unavailable() {
        return Err(Unavailable::error(format!("during commit: {reason}")));
    }
    eprintln!("rollup: relation {result}");
    let outcome = match (result["ok"] == true, result["value"]["created"] == true) {
        (false, _) => "rejected",
        (true, true) => "created",
        (true, false) => "cited",
    };
    let step = json!({"operation":"rollup_step","file":written.file(),"step":"propose_relation",
        "index":index,"outcome":outcome,"triple_id":result["value"]["triple_id"],"claim_id":relation.claim_id});
    graph.log(&step)?;
    Ok(step)
}
/// Logged `propose_relation` steps of `file`, by relation index.
fn relation_steps(inputs: &Inputs, file: &str) -> Result<BTreeMap<usize, Value>, Error> {
    let compiler = compiler_path(inputs.kg)?;
    if !compiler.is_file() {
        return Ok(BTreeMap::new());
    }
    Ok(ClaimStore::open(&compiler)?
        .events("rollup_step")?
        .into_iter()
        .filter(|e| e["file"] == file && e["step"] == "propose_relation")
        .filter_map(|e| Some((usize::try_from(e["index"].as_u64()?).ok()?, e)))
        .collect())
}
fn status(report: &Value) -> String {
    ["fail", "error", "warn"]
        .into_iter()
        .find(|k| report["summary"][k].as_u64().unwrap_or(0) > 0)
        .unwrap_or("ok")
        .to_owned()
}
pub fn compiler_path(kg: &Path) -> Result<PathBuf, Error> {
    let kg = crate::paths::database(kg)?;
    crate::paths::database(
        &kg.parent()
            .ok_or("KG parent missing")?
            .join("compiler.sqlite"),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logged_relation_steps_count_fresh_triples_as_created() {
        let mut r = Relations::default();
        for step in [
            json!({"outcome":"created","triple_id":"t_new","claim_id":"c_1"}),
            json!({"outcome":"cited","triple_id":"t_old","claim_id":"c_12"}),
            json!({"outcome":"rejected","triple_id":null,"claim_id":"c_3"}),
        ] {
            r.add(&step);
        }
        assert_eq!((r.created, r.cited), (1, 1));
        assert_eq!(r.citations, [("t_old".to_owned(), "c_12".to_owned())]);
    }
    #[test]
    fn orphans_are_unfinished_commits_whose_file_exists() {
        let dir = crate::paths::root().join(format!("target/orphans-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("diary")).unwrap();
        for name in ["2026-09-23-aaaaaaaaaaaa", "2026-09-24-bbbbbbbbbbbb"] {
            fs::write(dir.join(format!("diary/{name}.md")), "# ROLLUP: t\n\nx\n").unwrap();
        }
        let kg = dir.join("knowledge_graph.sqlite3");
        let mut store = ClaimStore::open(&compiler_path(&kg).unwrap()).unwrap();
        let commit = |file: &str, as_of: &str| json!({"operation":"rollup_commit","file":file,"thread":"t","as_of":as_of});
        let step =
            |file: &str, step: &str| json!({"operation":"rollup_step","file":file,"step":step});
        for event in [
            commit("2026-09-23-aaaaaaaaaaaa.md", "2026-09-23"),
            step("2026-09-23-aaaaaaaaaaaa.md", "mine"),
            commit("2026-09-24-bbbbbbbbbbbb.md", "2026-09-24"),
            step("2026-09-24-bbbbbbbbbbbb.md", "store"),
            commit("2026-09-25-cccccccccccc.md", "2026-09-25"), // file gone
        ] {
            store.log(&event).unwrap();
        }
        drop(store);
        let diary = Diary::open(&dir.join("diary")).unwrap();
        let found = orphans(&kg, &diary).unwrap();
        let files: Vec<_> = found
            .iter()
            .map(|o| (o.file.as_str(), o.as_of.as_str()))
            .collect();
        assert_eq!(files, [("2026-09-23-aaaaaaaaaaaa.md", "2026-09-23")]);
        fs::remove_dir_all(dir).unwrap();
    }
}
