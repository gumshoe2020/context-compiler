//! `palace rollup`: prep → select → pull → write → validate → commit (PLAN M4b).
//! Exit 0 written / quiet / model bail / dry run; 4 validation failed on every attempt; 3 MemPalace
//! unavailable at any stage (typed `Unavailable`); 2 any other broken run.
use crate::watch::{self, Overlay};
use clap::Args;
use palace_core::{
    canary::Canary,
    fidelity,
    rollup::Entry,
    rollup_cluster::{self, Cluster, Request},
    rollup_keep,
    rollup_prep::{self, Recent},
    rollup_repair,
    rollup_select::{self, Selection},
    rollup_validate::{self, Specific, Target, Validation},
    traits::{DiaryReader, Judge, Model},
    write_gate::dry_run,
};
use palace_mempalace::{Evidence, Unavailable, diary::Diary};
use palace_store::Error;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Args)]
pub struct Options {
    /// Diary root: the parent of `diary/`.
    #[arg(long)]
    pub diary: PathBuf,
    #[arg(long)]
    pub palace: PathBuf,
    #[arg(long)]
    pub kg: PathBuf,
    #[arg(long, default_value = "3d")]
    pub since: String,
    /// Defaults to today's America/New_York date.
    #[arg(long)]
    pub as_of: Option<String>,
    /// Skip the select step.
    #[arg(long)]
    pub thread: Option<String>,
    /// replay | claude-cli[:model] | command:<exe>
    #[arg(long, default_value = "replay")]
    pub llm: String,
    #[arg(long, default_value = "prompts")]
    pub prompts: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, default_value = "fixtures/rollups")]
    pub replay_rollups: PathBuf,
    #[arg(long, default_value = "fixtures/rollups/valid-replay-rollup.md")]
    pub replay_body: PathBuf,
    #[arg(long, default_value = "fixtures/canaries.json")]
    pub canaries: PathBuf,
    /// Deadline in seconds for the select call and every judge call.
    #[arg(long, default_value_t = 120)]
    pub model_timeout: u64,
    /// Deadline in seconds for the write call (the rollup body).
    #[arg(long, default_value_t = 600)]
    pub write_timeout: u64,
    /// Keep the attempts of a run that fails validation, and the failed attempts of one that
    /// passes after a repair or a rewrite, in this directory (0700; files 0600; the 20 newest
    /// runs). Confined like every output (`--roots`, never under fixtures/), and refused inside
    /// `--diary` or `--palace` (symbolic links resolved).
    #[arg(long)]
    pub keep: Option<PathBuf>,
    /// The wing `mempalace mine` files the rollup in (PKG-1.2 item 3).
    #[arg(long, default_value = palace_mempalace::mine::DEFAULT_WING, value_parser = crate::wing_name)]
    pub wing: String,
    #[command(flatten)]
    pub connection: crate::MemPalaceOptions,
}
pub const EXIT_INVALID: i32 = 4;
pub const EXIT_UNAVAILABLE: i32 = 3;
pub const EXIT_BROKEN: i32 = 2;

/// Exit code for a finished run: `Unavailable` or a locked database → 3, any other error → 2.
pub fn exit_code(result: Result<i32, Error>) -> i32 {
    match result.map_err(palace_store::busy::named) {
        Ok(code) => code,
        Err(error) if Unavailable::is(&error) || palace_store::busy::Busy::is(&error) => {
            eprintln!("rollup: {error}");
            EXIT_UNAVAILABLE
        }
        Err(error) => {
            eprintln!("palace rollup: {error}");
            EXIT_BROKEN
        }
    }
}
/// A failure while `evidence` was in use: `Unavailable` if its transport has failed.
pub fn transport_or(evidence: &Evidence, error: impl std::fmt::Display) -> Error {
    match evidence.searcher.unavailable() {
        Some(reason) => Unavailable::error(reason),
        None => error.to_string().into(),
    }
}

pub(crate) struct Run {
    options: Options,
    keep: Option<PathBuf>,
    root: PathBuf,
    diary: Diary,
    as_of: String,
    pub(crate) policy: String,
    pub(crate) models: Backends,
    canaries: Vec<Canary>,
}
pub fn run(options: Options) -> Result<i32, Error> {
    let root = crate::paths::diary_root(&options.diary)?;
    let diary = Diary::open(&root.join("diary"))?;
    let keep = options
        .keep
        .as_deref()
        .map(|dir| keep_dir(dir, &root, &options.palace))
        .transpose()?;
    if !options.dry_run {
        recover(&options)?;
        resume_orphans(&options, &root, &diary)?;
    }
    let as_of = match &options.as_of {
        Some(date) => date.clone(),
        None => palace_core::diary_file::new_york_date(now_millis()?),
    };
    let recent = prep(&options, &diary, &as_of)?;
    if rollup_prep::should_bail(&recent) {
        println!("{}", rollup_prep::QUIET);
        return Ok(0);
    }
    let prompts = crate::paths::input(&options.prompts)?;
    let (models, canaries) = models_and_canaries(&options, &prompts)?;
    let run = Run {
        // The prompt names the owners; another person and assistant read their own names.
        policy: palace_core::names::in_prose(&fs::read_to_string(prompts.join("rollup.md"))?),
        options,
        keep,
        root,
        diary,
        as_of,
        models,
        canaries,
    };
    run.stages(&recent)
}
/// `--keep`: refused inside the diary root or the palace (M5b-11 item 4: `mempalace mine` would
/// index the kept attempts, and `diary/` would list them), then confined like every output.
fn keep_dir(dir: &Path, root: &Path, palace: &Path) -> Result<PathBuf, Error> {
    let keep = crate::paths::resolved(dir)?;
    for (flag, store) in [("--diary", root), ("--palace", palace)] {
        let store = crate::paths::resolved(store)?;
        if rollup_keep::inside(&keep, &store) {
            return Err(format!(
                "--keep {} is inside {flag} {}: kept attempts must stay outside the diary and the palace",
                dir.display(),
                store.display()
            )
            .into());
        }
    }
    crate::paths::database(dir)
}
/// M4d-2a.1 item 2: unsettled intents are reconciled before the rollup writes anything.
fn recover(options: &Options) -> Result<(), Error> {
    let mut graph = crate::graph::open(&crate::graph::GraphPaths {
        palace: Some(options.palace.clone()),
        kg: options.kg.clone(),
        wal: None,
    })?;
    crate::recovery::reconcile(&mut graph, "rollup")
}
fn prep(options: &Options, diary: &Diary, as_of: &str) -> Result<Recent, Error> {
    let since = rollup_prep::parse_since(&options.since)?;
    let recent = rollup_prep::recent(diary, as_of, since)?;
    let (entries, substantive) = (recent.entries.len(), recent.substantive.len());
    eprintln!("rollup: prep as_of={as_of} recent={entries} substantive={substantive}");
    Ok(recent)
}
fn models_and_canaries(
    options: &Options,
    prompts: &Path,
) -> Result<(Backends, Vec<Canary>), Error> {
    let models = backends(options, prompts)?;
    let canaries = match models.judge {
        Some(_) => serde_json::from_slice(&fs::read(crate::paths::input(&options.canaries)?)?)?,
        None => Vec::new(),
    };
    Ok((models, canaries))
}
/// M4b.2 N-2: before prep and select, finish every committed rollup whose `store` step never ran
/// (an interrupted commit from an earlier day or another thread), printing `ROLLUP resumed …`.
fn resume_orphans(options: &Options, root: &Path, diary: &Diary) -> Result<(), Error> {
    let orphans = crate::rollup_commit::orphans(&options.kg, diary)?;
    if orphans.is_empty() {
        return Ok(());
    }
    let (models, canaries) = models_and_canaries(options, &crate::paths::input(&options.prompts)?)?;
    for orphan in orphans {
        let (file, thread) = (&orphan.file, &orphan.thread);
        eprintln!(
            "rollup: orphan {file} (thread {thread}, as-of {}): resuming",
            orphan.as_of
        );
        let inputs = crate::rollup_commit::Inputs {
            root,
            diary,
            palace: &options.palace,
            kg: &options.kg,
            connection: &options.connection,
            wing: &options.wing,
            thread: orphan.thread.clone(),
            as_of: &orphan.as_of,
            judge: models.judge.as_deref(),
            llm: &options.llm,
            canaries: &canaries,
        };
        crate::rollup_commit::resume(inputs, file)?;
    }
    Ok(())
}
impl Run {
    fn stages(&self, recent: &Recent) -> Result<i32, Error> {
        let Some((thread, tags)) = self.select(recent)? else {
            return Ok(0);
        };
        let target = Target {
            thread: &thread,
            as_of: &self.as_of,
        };
        if let Some(code) = self.resume_prior(&target)? {
            return Ok(code);
        }
        let evidence = self.connect()?;
        let cluster = self.pull(&target, &tags, &evidence)?;
        let job = crate::rollup_write::Job {
            target: &target,
            cluster: &cluster,
            evidence: &evidence,
        };
        let attempts = self.write_validated(&job)?;
        if let Some(dir) = &self.keep {
            crate::rollup_keep::keep_reporting(
                dir,
                &rollup_keep::run_key(&self.as_of, &thread),
                &attempts,
            );
        }
        let last = attempts.last().ok_or("no write attempt")?;
        if !last.validation.passed() {
            println!("{}", serde_json::to_string_pretty(&last.report)?);
            return Ok(EXIT_INVALID);
        }
        let (body, report) = (&last.body, &last.report);
        if self.options.dry_run {
            println!("{body}\n{}", serde_json::to_string_pretty(&report)?);
            return Ok(0);
        }
        drop(evidence); // mempalace-mcp must not hold the palace open while mine writes it
        crate::rollup_commit::commit(self.commit_inputs(&thread), body)
    }
    fn connect(&self) -> Result<Evidence, Error> {
        let diary = self.root.join("diary");
        let connection = &self.options.connection;
        connection
            .evidence(&self.options.palace, &diary)
            .map_err(Unavailable::error)
    }
    /// M4b.1 S-4: a rollup already committed for this thread + as-of is resumed, not rewritten.
    fn resume_prior(&self, target: &Target) -> Result<Option<i32>, Error> {
        if self.options.dry_run {
            return Ok(None);
        }
        let inputs = self.commit_inputs(target.thread);
        let Some(file) = crate::rollup_commit::find_prior(&inputs, &target.title())? else {
            return Ok(None);
        };
        eprintln!(
            "rollup: resume {file} (already committed: {})",
            target.title()
        );
        crate::rollup_commit::resume(inputs, &file).map(Some)
    }
    fn pull(
        &self,
        target: &Target,
        tags: &[String],
        evidence: &Evidence,
    ) -> Result<Cluster, Error> {
        let request = Request {
            thread: target.thread,
            tags,
            as_of: &self.as_of,
        };
        let cluster = rollup_cluster::pull(&self.diary, &evidence.searcher, &request)
            .map_err(|e| transport_or(evidence, e))?;
        report_pull(&cluster);
        Ok(cluster)
    }
    fn select(&self, recent: &Recent) -> Result<Option<(String, Vec<String>)>, Error> {
        if let Some(thread) = &self.options.thread {
            eprintln!("rollup: select thread={thread} (given)");
            return Ok(Some((thread.clone(), Vec::new())));
        }
        let candidates = rollup_prep::candidates(recent);
        if candidates.is_empty() {
            println!("Palace compression: no candidate thread");
            return Ok(None);
        }
        let names = self.diary.names()?;
        let threads = rollup_cluster::rollup_threads(&self.diary, &names, &self.as_of)?;
        let reply = self.models.select.complete(&rollup_select::prompt(
            &self.policy,
            recent,
            &candidates,
            &threads,
        ))?;
        let selection = rollup_select::parse(&reply, &candidates)?;
        let tags = selection.tags(&candidates);
        let entry_tags = selection.entry_tags(&candidates, recent);
        match selection {
            Selection::Bail { reason } => {
                println!("Palace compression: model bailed: {reason}");
                Ok(None)
            }
            Selection::Thread {
                thread,
                candidate_ids,
                ..
            } => {
                let (thread, kept) = self.kept_thread(thread, &entry_tags)?;
                eprintln!(
                    "rollup: select thread={thread}{kept} candidates={candidate_ids:?} tags={tags:?}"
                );
                Ok(Some((thread, tags)))
            }
        }
    }
    pub(crate) fn validate(
        &self,
        body: &str,
        target: &Target,
        cluster: &Cluster,
        evidence: &Evidence,
    ) -> Result<(Validation, Value), Error> {
        let names = self.diary.names()?;
        let mut v = rollup_validate::validate(body, target, cluster, &names);
        rollup_validate::check_specifics(&mut v, body, &self.live_specifics(cluster)?);
        rollup_validate::cite_claims(&mut v.relations, &v.claims);
        let gate: Vec<Value> = v.relations.iter().map(|r| dry_run(evidence, r)).collect();
        for (i, verdict) in gate.iter().enumerate() {
            if verdict["ok"] != true {
                v.problems.push(format!(
                    "relation {i} fails the write gate: {}",
                    verdict["reason"]
                ));
            }
        }
        if let Some(reason) = evidence.searcher.unavailable() {
            return Err(Unavailable::error(format!("during validation: {reason}")));
        }
        let fidelity = self.fidelity(body, &mut v)?;
        let claim_problems: Vec<Value> = rollup_repair::claim_problem_rows(body, &v.problems)
            .iter()
            .map(rollup_repair::ClaimProblem::to_json)
            .collect();
        let report = json!({"passed":v.passed(),"problems":v.problems,"claim_problems":claim_problems,"claims":v.claims.len(),"relations":v.relations.len(),"gate":gate,"superseded":v.superseded,"fidelity":fidelity,"cluster":cluster.members.iter().map(|m| json!({"name":m.entry.name,"role":m.label()})).collect::<Vec<_>>()});
        Ok((v, report))
    }
    /// SETTLE-1b (spec §5): the live specifics the cluster's entries state (none before the
    /// claim store exists, or when no entry of the cluster was distilled).
    pub(crate) fn live_specifics(&self, cluster: &Cluster) -> Result<Vec<Specific>, Error> {
        let store = crate::rollup_commit::compiler_path(&self.options.kg)?;
        if !store.is_file() {
            return Ok(Vec::new());
        }
        let names: Vec<String> = cluster
            .members
            .iter()
            .map(|m| m.entry.name.clone())
            .collect();
        let text = |row: &Value, key: &str| row[key].as_str().unwrap_or_default().to_owned();
        Ok(palace_store::claims::ClaimStore::open(&store)?
            .live_specifics(&names)?
            .iter()
            .map(|row| Specific {
                id: text(row, "id"),
                subject: text(row, "subject"),
                predicate: text(row, "predicate"),
                object: text(row, "object"),
                date: text(row, "date"),
            })
            .collect())
    }
    /// Floor, depth and (with a judge) canaries on the candidate text.
    fn fidelity(&self, body: &str, v: &mut Validation) -> Result<Value, Error> {
        let entry = Entry::parse("rollup-candidate", body.into());
        let overlay = Overlay {
            diary: &self.diary,
            entries: [(entry.name.clone(), body.to_owned())].into(),
        };
        let mut rows = vec![
            fidelity::floor(&entry, &overlay),
            fidelity::depth(&entry, &overlay),
        ];
        if let Some(judge) = &self.models.judge {
            for c in &self.canaries {
                rows.push(palace_core::canary::check(&entry, c, judge.as_ref()));
            }
        }
        for row in &rows {
            if matches!(row["status"].as_str(), Some("fail" | "error")) {
                v.problems
                    .push(format!("{} check {}: {row}", row["check"], row["status"]));
            }
        }
        Ok(json!(rows))
    }
    /// M5b-10 item 5, M5b-11 item 3: when the chosen entries continue a prior rollup (the pull's,
    /// for the model's name) by an equal name, the thread is that rollup's; the stage line's note.
    fn kept_thread(&self, model: String, entry_tags: &[String]) -> Result<(String, String), Error> {
        let request = Request {
            thread: &model,
            tags: &[],
            as_of: &self.as_of,
        };
        let priors = rollup_cluster::prior_rollups(&self.diary, &self.diary.names()?, &request)?;
        Ok(
            match rollup_select::kept_thread(&model, &priors, entry_tags) {
                Some(prior) => (
                    prior,
                    format!(" (the prior rollup's; the model said {model})"),
                ),
                None => (model, String::new()),
            },
        )
    }
    fn commit_inputs(&self, thread: &str) -> crate::rollup_commit::Inputs<'_> {
        crate::rollup_commit::Inputs {
            root: &self.root,
            diary: &self.diary,
            palace: &self.options.palace,
            kg: &self.options.kg,
            connection: &self.options.connection,
            wing: &self.options.wing,
            thread: thread.to_owned(),
            as_of: &self.as_of,
            judge: self.models.judge.as_deref(),
            llm: &self.options.llm,
            canaries: &self.canaries,
        }
    }
}
fn report_pull(cluster: &Cluster) {
    let count = |label| {
        cluster
            .members
            .iter()
            .filter(|m| m.label() == label)
            .count()
    };
    eprintln!(
        "rollup: pull entries={} primaries={} reference_only={} bookkeeping={} prior_rollups={:?} dropped={}",
        cluster.members.len(),
        count("primary"),
        count("reference-only"),
        count("bookkeeping"),
        cluster.prior_rollups,
        cluster.dropped.len()
    );
}
/// Model for select, model for write (separate deadlines, M4b.1 S-6), model for repair, and the
/// judge used for canaries and claim reversal (none in replay).
pub(crate) struct Backends {
    select: Box<dyn Model<Error = Error>>,
    pub(crate) write: Box<dyn Model<Error = Error>>,
    /// The repair of failed claim lines (M5b-10 item 2): `--model-timeout`, not `--write-timeout`.
    pub(crate) repair: Box<dyn Model<Error = Error>>,
    judge: Option<Box<dyn Judge<Error = Error>>>,
}
fn backends(options: &Options, prompts: &Path) -> Result<Backends, Error> {
    use palace_judge::command::{CommandJudge, CommandModel, resolve_executable};
    let llm = options.llm.as_str();
    if llm == "replay" {
        return Ok(Backends {
            select: replay_model(options)?,
            write: replay_model(options)?,
            repair: replay_model(options)?,
            judge: None,
        });
    }
    let model_timeout = Duration::from_secs(options.model_timeout);
    let write_timeout = Duration::from_secs(options.write_timeout);
    let transport = |timeout| -> Result<CommandModel, Error> {
        if let Some(rest) = llm.strip_prefix("claude-cli") {
            let model = rest
                .strip_prefix(':')
                .filter(|m| !m.is_empty())
                .unwrap_or("sonnet");
            let settings = crate::paths::input(&prompts.join("judge-settings.json"))?;
            let exe = resolve_executable("claude")?;
            return Ok(CommandModel::claude(&exe, model, &settings)?.with_timeout(timeout));
        }
        match llm.strip_prefix("command:").filter(|e| !e.is_empty()) {
            Some(exe) => Ok(CommandModel::generic(exe).with_timeout(timeout)),
            None => Err("--llm must be replay, claude-cli[:model] or command:<exe>".into()),
        }
    };
    Ok(Backends {
        select: Box::new(transport(model_timeout)?),
        write: Box::new(transport(write_timeout)?),
        repair: Box::new(transport(model_timeout)?),
        judge: Some(Box::new(CommandJudge(transport(model_timeout)?))),
    })
}
fn replay_model(options: &Options) -> Result<Box<dyn Model<Error = Error>>, Error> {
    let body = fs::read_to_string(crate::paths::input(&options.replay_body)?)?;
    let archived = watch::load_entries(&crate::paths::input(&options.replay_rollups)?)?
        .into_iter()
        .filter(|e| is_diary_name(&e.name))
        .collect();
    Ok(Box::new(palace_judge::replay_model::ReplayModel::new(
        body, archived,
    )?))
}
/// Archived rollups are real diary files (`YYYY-MM-DD-<12 hex>`), not the synthetic fixtures.
fn is_diary_name(name: &str) -> bool {
    name.len() == 23
        && rollup_prep::entry_date(name).is_some()
        && name.as_bytes()[10] == b'-'
        && name[11..].bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn normalise(body: &str) -> String {
    format!("{}\n", body.trim())
}
pub fn now_millis() -> Result<i64, Error> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn select_and_write_deadlines_default_and_flags() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            options: Options,
        }
        let base = ["palace", "--diary", "d", "--palace", "p", "--kg", "k"];
        let o = Cli::parse_from(base).options;
        assert_eq!((o.model_timeout, o.write_timeout), (120, 600));
        let flags = ["--model-timeout", "30", "--write-timeout", "900"];
        let o = Cli::parse_from(base.iter().chain(&flags)).options;
        assert_eq!((o.model_timeout, o.write_timeout), (30, 900));
    }
    #[test]
    fn exit_codes_are_mapped_by_type() {
        assert_eq!(exit_code(Ok(4)), EXIT_INVALID);
        assert_eq!(exit_code(Err(Unavailable::error("EOF"))), EXIT_UNAVAILABLE);
        assert_eq!(
            exit_code(Err("MemPalace unavailable: looks alike".into())),
            EXIT_BROKEN
        );
    }
}
