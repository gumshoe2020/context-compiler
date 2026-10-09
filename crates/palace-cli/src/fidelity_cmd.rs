//! `palace fidelity` (SETTLE-1b, spec §8): recall at both compression points, observe only.
//! `--rollup <file>` (consolidation), `--distill <transcript>` (each distill run of it not scored
//! yet), `--all` (every rollup not scored yet: the baseline). Specifics by exact match, no judge;
//! concepts by `--judge` (none: `unjudged`). Sampling is seeded by the file name. One `fidelity`
//! row per check, written once. Reads the diary, the copies and the claim store; writes only
//! its rows (and the write log). Prints one `FIDELITY …` line per check; with `--page`, a check
//! on the spec's conditions (a missing specific at consolidation, any contradiction, concept
//! recall under the floor) runs it once with one `FIDELITY PAGE …` line on its standard input.
//! At the distillation point (SETTLE-1c) the concept sample draws only from messages of
//! `--min-words` or more, and the judge sees each with the messages after it, judging the final
//! state; a `not_applicable` item stays in the row's items but out of `sampled`.
//! Exit 0 (whatever it measured), 2 broken.
use clap::Args;
use palace_core::{
    recall::{self, Counts},
    rollup::{Entry, superseded_sources},
    traits::{DiaryReader, Model},
};
use palace_mempalace::diary::Diary;
use palace_store::{Error, claims::ClaimStore, fidelity::Row};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Target {
    /// One rollup entry (a diary file).
    #[arg(long)]
    rollup: Option<PathBuf>,
    /// One transcript, as `palace distill --transcript` was given it.
    #[arg(long)]
    distill: Option<PathBuf>,
    /// Every rollup of the diary not scored yet.
    #[arg(long)]
    all: bool,
}
#[derive(Args)]
pub struct Options {
    #[command(flatten)]
    target: Target,
    /// The diary root (the parent of `diary/`).
    #[arg(long)]
    pub diary: PathBuf,
    /// The graph; the claim store is the compiler.sqlite beside it.
    #[arg(long)]
    pub kg: PathBuf,
    /// The concepts' judge: table:<file> | command:<exe> | claude-cli[:model] | replay (none:
    /// the concepts are recorded `unjudged`).
    #[arg(long)]
    pub judge: Option<String>,
    /// `FIDELITY_SAMPLE`: items sampled per check.
    #[arg(long, default_value_t = recall::SAMPLE)]
    pub sample: usize,
    /// `FIDELITY_FLOOR`: concept recall below it pages.
    #[arg(long, default_value_t = recall::FLOOR)]
    pub floor: f64,
    /// `DISTILL_RECALL_MIN_WORDS`: a distill-point concept sample draws only from messages with
    /// at least this many words (markdown and emoji stripped).
    #[arg(long, default_value_t = recall::DISTILL_MIN_WORDS)]
    pub min_words: usize,
    /// `DISTILL_RECALL_FOLLOWING`: the messages after a sampled one (in the same distilled
    /// range, at most 4000 characters) that the distill-point question carries.
    #[arg(long, default_value_t = recall::DISTILL_FOLLOWING)]
    pub following: usize,
    /// Run with one `FIDELITY PAGE …` line on its standard input when a check pages.
    #[arg(long)]
    pub page: Option<PathBuf>,
    #[arg(long, default_value = "prompts")]
    pub prompts: PathBuf,
    /// Deadline in seconds for each judge call.
    #[arg(long, default_value_t = 120)]
    pub model_timeout: u64,
}

pub fn run(options: Options) -> i32 {
    match measure(&options) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("palace fidelity: {error}");
            2
        }
    }
}

fn measure(options: &Options) -> Result<(), Error> {
    let root = crate::paths::diary_root(&options.diary)?;
    let diary = Diary::open(&root.join("diary"))?;
    let judge = Judge::from(options)?;
    let store_path = crate::rollup_commit::compiler_path(&options.kg)?;
    let mut store = ClaimStore::open(&store_path)?;
    let mut checks = Vec::new();
    if let Some(file) = &options.target.rollup {
        let name = crate::paths::input(file)?;
        checks.extend(consolidation(
            &store,
            &diary,
            &name_of(&name),
            options,
            &judge,
        )?);
    }
    if options.target.all {
        for name in diary.names()? {
            let is_rollup = diary
                .read(&name)?
                .is_some_and(|t| Entry::parse(&name, t).is_rollup);
            if is_rollup && !store.fidelity_scored("consolidation", &format!("{name}.md"))? {
                checks.extend(consolidation(&store, &diary, &name, options, &judge)?);
            }
        }
    }
    if let Some(transcript) = &options.target.distill {
        let transcript = crate::paths::input(transcript)?;
        checks.extend(distilled(&store, &transcript, options, &judge)?);
    }
    let at = store.now()?;
    let mut recorded = 0;
    for check in &checks {
        if !store.record_fidelity(&check.row(&judge), &at)? {
            continue;
        }
        recorded += 1;
        println!("{}", check.line(&judge));
        if let Some(reason) = check.page(options.floor) {
            page(
                options,
                &format!("FIDELITY PAGE {} {}: {reason}", check.point, check.file),
            )?;
        }
    }
    println!("FIDELITY OK rows={recorded}");
    Ok(())
}
fn name_of(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name.strip_suffix(".md").unwrap_or(name).to_owned()
}

/// One check's result, before it is recorded.
struct Check {
    point: &'static str,
    file: String,
    kind: &'static str,
    counts: Counts,
    depth: Option<i64>,
    primary_share: Option<f64>,
    /// The sampled items, each with its verdict (a JSON array).
    items: Value,
}
impl Check {
    fn new(point: &'static str, file: &str, kind: &'static str) -> Self {
        Self {
            point,
            file: file.to_owned(),
            kind,
            counts: Counts::default(),
            depth: None,
            primary_share: None,
            items: json!([]),
        }
    }
    /// One sampled item and its verdict; a `not_applicable` one is kept in the items but left
    /// out of the counts (SETTLE-1c §2).
    fn add(&mut self, item: Value, verdict: Option<&str>) {
        if verdict != Some(recall::NOT_APPLICABLE) {
            self.counts.add(verdict);
        }
        let mut item = item;
        item["verdict"] = json!(verdict.unwrap_or("unjudged"));
        if let Some(items) = self.items.as_array_mut() {
            items.push(item);
        }
    }
    fn judge_label<'a>(&self, judge: &'a Judge) -> &'a str {
        match self.kind {
            "specific" => "rule:exact",
            _ => &judge.label,
        }
    }
    fn row<'a>(&'a self, judge: &'a Judge) -> Row<'a> {
        Row {
            point: self.point,
            file: &self.file,
            kind: self.kind,
            counts: &self.counts,
            rollup_depth: self.depth,
            primary_share: self.primary_share,
            judge: self.judge_label(judge),
            items: &self.items,
        }
    }
    fn page(&self, floor: f64) -> Option<String> {
        recall::page_reason(self.point, self.kind, &self.counts, floor)
    }
    fn line(&self, judge: &Judge) -> String {
        let c = &self.counts;
        format!(
            "FIDELITY {} {} {}: sampled={} supported={} missing={} contradicted={} unjudged={} judge={}",
            self.point,
            self.file,
            self.kind,
            c.sampled,
            c.supported,
            c.missing,
            c.contradicted,
            c.unjudged,
            self.judge_label(judge)
        )
    }
}

/// Consolidation (spec §8): every live specific its superseded sources state, present verbatim
/// or cited by id (no judge); up to `--sample` of their live concept claims, judged against the
/// rollup's text. The specific row only when the sources state a specific.
fn consolidation(
    store: &ClaimStore,
    diary: &Diary,
    name: &str,
    options: &Options,
    judge: &Judge,
) -> Result<Vec<Check>, Error> {
    let text = diary
        .read(name)?
        .ok_or_else(|| format!("rollup {name}: not in the diary"))?;
    let entry = Entry::parse(name, text);
    let sources = superseded_sources(&entry, &diary.names()?).resolved;
    let file = format!("{name}.md");
    let depth = palace_core::fidelity::depth(&entry, diary);
    let mut checks = Vec::new();
    let specifics = store.live_specifics(&sources)?;
    if !specifics.is_empty() {
        let mut check = Check::new("consolidation", &file, "specific");
        for s in &specifics {
            let (id, value) = (text_of(s, "id"), text_of(s, "object"));
            let kept = entry.text.contains(value.trim()) || entry.text.contains(&id);
            let verdict = if kept { "supported" } else { "missing" };
            check.add(json!({"id": id, "value": value}), Some(verdict));
        }
        checks.push(check);
    }
    let concepts: Vec<Value> = store
        .claims_of(&sources, "concept")?
        .into_iter()
        .filter(|c| c["status"] == "live")
        .collect();
    let mut check = Check::new("consolidation", &file, "concept");
    for c in recall::sample(&file, &concepts, |c| text_of(c, "id"), options.sample) {
        let statement = statement(&c);
        let verdict = judge.ask(&text_of(&c, "id"), &statement, &entry.text)?;
        check.add(json!({"id": c["id"], "statement": statement}), verdict);
    }
    checks.push(check);
    for check in &mut checks {
        check.depth = depth["depth"].as_i64();
        check.primary_share = depth["primary_share"].as_f64();
    }
    Ok(checks)
}
fn statement(claim: &Value) -> String {
    let mut text = format!(
        "{} {} {}",
        text_of(claim, "subject"),
        text_of(claim, "predicate"),
        text_of(claim, "object")
    );
    if let Some(quote) = claim["quote"].as_str() {
        text.push_str(&format!(" (\"{quote}\")"));
    }
    text
}
fn text_of(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

/// Distillation (spec §8), for each distill run of the transcript: up to `--sample` sentences of
/// the distilled text that carry values, each value verbatim in the run's specifics or `missing`
/// (no judge); up to `--sample` of its messages with `--min-words` or more words, judged against
/// the run's entries (SETTLE-1c §1: shorter messages are never sampled).
fn distilled(
    store: &ClaimStore,
    transcript: &Path,
    options: &Options,
    judge: &Judge,
) -> Result<Vec<Check>, Error> {
    let wanted = transcript.to_string_lossy();
    let mut checks = Vec::new();
    for event in store.events("distill_commit")? {
        if event["transcript"] != *wanted || event["skipped"] == true {
            continue;
        }
        let range = text_of(&event, "range");
        if store.fidelity_scored("distill", &range)? {
            continue;
        }
        let turns = copy_turns(&event)?;
        let reply = palace_core::distill::from_json(&event["reply"]);
        checks.push(distilled_specifics(&range, &turns, &reply, options));
        checks.push(distilled_concepts(&range, &turns, &reply, options, judge)?);
    }
    Ok(checks)
}
fn distilled_specifics(
    range: &str,
    turns: &[Turn],
    reply: &palace_core::distill::Reply,
    options: &Options,
) -> Check {
    let specifics: String = reply
        .claims
        .iter()
        .filter(|c| c.kind == "specific")
        .map(|c| {
            format!(
                "{} {} {}\n",
                c.subject,
                c.object,
                c.quote.as_deref().unwrap_or("")
            )
        })
        .collect();
    let carrying: Vec<(usize, String)> = turns
        .iter()
        .flat_map(|t| {
            let line = t.line;
            recall::sentences(&t.text)
                .into_iter()
                .map(move |s| (line, s))
        })
        .filter(|(_, s)| !recall::values(s).is_empty())
        .collect();
    let mut check = Check::new("distill", range, "specific");
    let key = |(line, s): &(usize, String)| format!("{line}:{s}");
    for (line, sentence) in recall::sample(range, &carrying, key, options.sample) {
        let verdict = recall::specific_verdict(&sentence, &specifics);
        check.add(
            json!({"line": line, "sentence": sentence, "values": recall::values(&sentence)}),
            Some(verdict),
        );
    }
    check
}
/// The concepts: each sampled message judged with the messages after it in the range (up to
/// `--following`), as the conversation finally left it (SETTLE-1c §2).
fn distilled_concepts(
    range: &str,
    turns: &[Turn],
    reply: &palace_core::distill::Reply,
    options: &Options,
    judge: &Judge,
) -> Result<Check, Error> {
    let entries: String = reply
        .entries
        .iter()
        .map(|e| format!("# {}\n{}\n\n", e.title, e.body))
        .collect();
    let eligible: Vec<usize> = (0..turns.len())
        .filter(|&i| recall::words(&turns[i].text) >= options.min_words)
        .collect();
    let mut check = Check::new("distill", range, "concept");
    let key = |&i: &usize| turns[i].line.to_string();
    for i in recall::sample(range, &eligible, key, options.sample) {
        let turn = &turns[i];
        let later: Vec<String> = turns[i + 1..].iter().map(Turn::shown).collect();
        let following = recall::following(&later, options.following);
        let verdict = judge.ask_distill(turn.line, &turn.shown(), &following, &entries)?;
        check.add(json!({"line": turn.line, "passage": turn.text}), verdict);
    }
    check.items = substance_items(check.items, eligible.len(), turns.len());
    Ok(check)
}
/// A distill-point concept row's `items` (SETTLE-1c §1): the sampled items with how many of the
/// range's messages were eligible (`DISTILL_RECALL_MIN_WORDS` or more words) and in all.
fn substance_items(items: Value, eligible: usize, messages: usize) -> Value {
    json!({"eligible": eligible, "messages": messages, "items": items})
}
/// One message of a distilled range.
struct Turn {
    line: usize,
    role: String,
    text: String,
}
impl Turn {
    /// The message as the judge sees it: `[role] text`.
    fn shown(&self) -> String {
        format!("[{}] {}", self.role, self.text)
    }
}

/// The copy lines a distill run read, from its logged range `<copy>:<a>-<b>`.
fn copy_turns(event: &Value) -> Result<Vec<Turn>, Error> {
    let range = text_of(event, "range");
    let (copy, lines) = range
        .rsplit_once(':')
        .ok_or("distill_commit without a range")?;
    let (first, last) = lines
        .split_once('-')
        .ok_or("distill_commit range without lines")?;
    let (first, last): (usize, usize) = (first.parse()?, last.parse()?);
    let text = fs::read_to_string(crate::paths::input(Path::new(copy))?)?;
    Ok(text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .filter(|(n, _)| (first..=last).contains(n))
        .filter_map(|(n, l)| {
            let value: Value = serde_json::from_str(l).ok()?;
            Some(Turn {
                line: n,
                role: value["role"].as_str().unwrap_or("unknown").to_owned(),
                text: value["text"].as_str()?.to_owned(),
            })
        })
        .collect())
}

/// The concepts' judge.
struct Judge {
    label: String,
    kind: JudgeKind,
}
enum JudgeKind {
    None,
    Table(BTreeMap<String, String>),
    Model(Box<dyn Model<Error = Error>>),
}
impl Judge {
    fn from(options: &Options) -> Result<Self, Error> {
        use palace_judge::command::{CommandModel, resolve_executable};
        let timeout = Duration::from_secs(options.model_timeout);
        let Some(spec) = options.judge.as_deref() else {
            return Ok(Self::none());
        };
        if spec == "replay" {
            return Ok(Self::none()); // no recorded recall verdicts: unjudged
        }
        let label = crate::watch::judge_label(spec);
        let kind = if let Some(file) = spec.strip_prefix("table:") {
            let text = fs::read(crate::paths::input(Path::new(file))?)?;
            JudgeKind::Table(serde_json::from_slice(&text)?)
        } else if let Some(rest) = spec.strip_prefix("claude-cli") {
            let model = rest
                .strip_prefix(':')
                .filter(|m| !m.is_empty())
                .unwrap_or("sonnet");
            let settings = crate::paths::input(&options.prompts.join("judge-settings.json"))?;
            let exe = resolve_executable("claude")?;
            JudgeKind::Model(Box::new(
                CommandModel::claude(&exe, model, &settings)?.with_timeout(timeout),
            ))
        } else if let Some(exe) = spec.strip_prefix("command:").filter(|e| !e.is_empty()) {
            JudgeKind::Model(Box::new(CommandModel::generic(exe).with_timeout(timeout)))
        } else {
            return Err(
                "--judge must be table:<file>, command:<exe>, claude-cli[:model] or replay".into(),
            );
        };
        Ok(Self { label, kind })
    }
    fn none() -> Self {
        Self {
            label: "none".into(),
            kind: JudgeKind::None,
        }
    }
    /// The verdict on one item (`None`: unjudged). A table answers by item id (an unlisted item is
    /// supported); a model by the recall question.
    fn ask(&self, id: &str, statement: &str, text: &str) -> Result<Option<&'static str>, Error> {
        Ok(match &self.kind {
            JudgeKind::None => None,
            JudgeKind::Table(table) => {
                Some(table.get(id).map_or("supported", |v| recall::verdict(v)))
            }
            JudgeKind::Model(model) => {
                recall::parse_answer(&model.complete(&recall::question(statement, text))?)
            }
        })
    }
}

impl Judge {
    /// The distill-point verdict on the message at `line` (`None`: unjudged): a table answers by
    /// `L<line>` (unlisted: supported; `not_applicable` allowed); a model by the distill question.
    fn ask_distill(
        &self,
        line: usize,
        statement: &str,
        following: &str,
        text: &str,
    ) -> Result<Option<&'static str>, Error> {
        Ok(match &self.kind {
            JudgeKind::None => None,
            JudgeKind::Table(table) => Some(
                table
                    .get(&format!("L{line}"))
                    .map_or("supported", |v| recall::distill_verdict(v)),
            ),
            JudgeKind::Model(model) => recall::parse_distill_answer(
                &model.complete(&recall::distill_question(statement, following, text))?,
            ),
        })
    }
}

/// `--page`: the exe with the line on its standard input (a failed page is named, never fatal).
fn page(options: &Options, line: &str) -> Result<(), Error> {
    println!("{line}");
    let Some(exe) = &options.page else {
        return Ok(());
    };
    let child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn();
    let result = child.and_then(|mut child| {
        if let Some(mut stdin) = child.stdin.take() {
            writeln!(stdin, "{line}")?;
        }
        child.wait()
    });
    match result {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("palace fidelity: --page {} exited {status}", exe.display()),
        Err(e) => eprintln!("palace fidelity: --page {}: {e}", exe.display()),
    }
    Ok(())
}
