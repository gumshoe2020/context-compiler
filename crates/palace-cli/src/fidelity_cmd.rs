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
//! state; a `not_applicable` item stays in the row's items but out of `sampled`. Its specifics
//! sample (SETTLE-1d §2) draws up to `--specific-sample` sentences that carry a value and scores
//! each value (`judge` `rule:values`: its `sampled`/`supported` count values).
//! Exit 0 (whatever it measured), 2 broken.
use clap::Args;
use palace_core::{
    recall::{self, Counts},
    rollup::{Entry, superseded_sources},
    rollup_reference,
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
    /// SETTLE-1d: rescore the store's distill-point specifics rows (`rule:exact`, and since
    /// SETTLE-1e `rule:values`) under the current values rule and print old against new; writes
    /// nothing (meant for a copy).
    #[arg(long)]
    replay_values: bool,
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
    /// `DISTILL_SPECIFIC_SAMPLE`: sentences carrying a value sampled for a distill-point
    /// specifics check (SETTLE-1d §2), each value scored.
    #[arg(long, default_value_t = recall::DISTILL_SPECIFIC_SAMPLE)]
    pub specific_sample: usize,
    /// Run with one `FIDELITY PAGE …` line on its standard input when a check pages.
    #[arg(long)]
    pub page: Option<PathBuf>,
    #[arg(long, default_value = "prompts")]
    pub prompts: PathBuf,
    /// Deadline in seconds for each judge call.
    #[arg(long, default_value_t = 120)]
    pub model_timeout: u64,
    /// With `--replay-values` (SETTLE-1e): a directory holding copies of the rows' transcripts
    /// (by file name), to drop code from the stored sentences and size each run's checklist.
    #[arg(long, requires = "replay_values")]
    pub transcripts: Option<PathBuf>,
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
    if options.target.replay_values {
        let store = ClaimStore::open(&crate::rollup_commit::compiler_path(&options.kg)?)?;
        let common = common_words(&options.prompts)?;
        return crate::fidelity_replay::replay(&store, &common, options.transcripts.as_deref());
    }
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
        let common = common_words(&options.prompts)?;
        checks.extend(distilled(&store, &transcript, options, &judge, &common)?);
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
/// The common word list of the specifics measure (SETTLE-1d §1): `<prompts>/
/// recall-common-words.json`, read on every run; when there is no such file, the copy built in.
pub(crate) fn common_words(prompts: &Path) -> Result<recall::CommonWords, Error> {
    let file = prompts.join("recall-common-words.json");
    if !file.exists() {
        eprintln!(
            "palace: {} not found: the built-in common word list is used",
            file.display()
        );
        return Ok(recall::CommonWords::parse(BUILT_IN_WORDS)?);
    }
    let text = fs::read_to_string(crate::paths::input(&file)?)?;
    recall::CommonWords::parse(&text).map_err(|e| format!("{}: {e}", file.display()).into())
}
const BUILT_IN_WORDS: &str = include_str!("../../../prompts/recall-common-words.json");
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
        if !matches!(verdict, Some(recall::NOT_APPLICABLE | recall::DEFERRED)) {
            self.counts.add(verdict);
        }
        let mut item = item;
        item["verdict"] = json!(verdict.unwrap_or("unjudged"));
        if let Some(items) = self.items.as_array_mut() {
            items.push(item);
        }
    }
    /// One sampled sentence scored value by value (SETTLE-1d §2): each value counts once in
    /// the row (kept: `supported`, else `missing`); the sentence keeps its own verdict.
    fn add_values(&mut self, mut item: Value, kept: &[(String, bool)]) {
        for (_, k) in kept {
            self.counts
                .add(Some(if *k { "supported" } else { "missing" }));
        }
        let named = |want: bool| -> Vec<&str> {
            kept.iter()
                .filter(|(_, k)| *k == want)
                .map(|(v, _)| v.as_str())
                .collect()
        };
        item["kept"] = json!(named(true));
        item["missing"] = json!(named(false));
        let all = kept.iter().all(|(_, k)| *k);
        item["verdict"] = json!(if all { "supported" } else { "missing" });
        if let Some(items) = self.items.as_array_mut() {
            items.push(item);
        }
    }
    fn judge_label<'a>(&self, judge: &'a Judge) -> &'a str {
        match (self.kind, self.point) {
            ("specific", "distill") => recall::RULE_VALUES,
            ("specific", _) => "rule:exact",
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
        let mut line = format!(
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
        );
        if let Some(unexplained) = self.items["values_unexplained"].as_u64() {
            let listed = self.items["checklist_values"]
                .as_u64()
                .map_or("-".to_owned(), |n| n.to_string());
            let skipped = &self.items["values_skipped"];
            line.push_str(&format!(
                " unexplained={unexplained} skipped={skipped}/{listed}"
            ));
        }
        line
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
    let names = diary.names()?;
    let sources = superseded_sources(&entry, &names).resolved;
    let file = format!("{name}.md");
    let depth = palace_core::fidelity::depth(&entry, diary);
    let mut checks = Vec::new();
    let specifics = store.live_specifics(&sources)?;
    let referenced = store.live_specifics(&rollup_reference::referenced_entries(&entry, &names))?;
    if !specifics.is_empty() || !referenced.is_empty() {
        let mut check = Check::new("consolidation", &file, "specific");
        for s in &specifics {
            let (id, value) = (text_of(s, "id"), text_of(s, "object"));
            let kept = entry.text.contains(value.trim()) || entry.text.contains(&id);
            let verdict = if kept { "supported" } else { "missing" };
            check.add(json!({"id": id, "value": value}), Some(verdict));
        }
        // SETTLE-1d §6: a referenced entry's specifics the claims leave uncited are deferred.
        for s in &referenced {
            let (id, value) = (text_of(s, "id"), text_of(s, "object"));
            let kept = rollup_reference::claims_keep(&entry.text, &value, &id);
            let verdict = if kept { "supported" } else { recall::DEFERRED };
            check.add(
                json!({"id": id, "value": value, "referenced": true}),
                Some(verdict),
            );
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
    common: &recall::CommonWords,
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
        let rule = ValueRule {
            common,
            run: run_day(&event)?,
            sample: options.specific_sample,
            listed: listed_values(&event),
        };
        checks.push(distilled_specifics(&range, &turns, &reply, &rule));
        checks.push(distilled_concepts(&range, &turns, &reply, options, judge)?);
    }
    Ok(checks)
}
/// How a distill-point specifics check reads values: the common word list, the run's date and
/// the sample size.
struct ValueRule<'a> {
    common: &'a recall::CommonWords,
    run: recall::Day,
    sample: usize,
    /// The values the run's checklist listed (SETTLE-1d §5; `None`: a run from before it).
    listed: Option<usize>,
}
fn listed_values(event: &Value) -> Option<usize> {
    let checklist = event["checklist"].as_array()?;
    Some(
        checklist
            .iter()
            .map(|l| l["values"].as_array().map_or(0, Vec::len))
            .sum(),
    )
}
/// Whether the reply skipped `value` on `line` (SETTLE-1d §5): the same line, and either value
/// holds the other, case-insensitively.
fn skipped(reply: &palace_core::distill::Reply, line: usize, value: &str) -> bool {
    let value = value.to_lowercase();
    reply.skipped.iter().any(|k| {
        let named = k.value.to_lowercase();
        k.line == line && (named.contains(&value) || value.contains(&named))
    })
}
/// The distill run's date (America/New_York), the year of the dates it reads without one.
pub(crate) fn run_day(event: &Value) -> Result<recall::Day, Error> {
    let at = event["at"].as_i64().ok_or("distill_commit without at")?;
    let date = palace_core::diary_file::new_york_date(at);
    Ok(recall::Day::parse(&date).ok_or("distill_commit at is not a date")?)
}
/// What the run's specifics state: subject, object and quote, with their dates (SETTLE-1d §1).
pub(crate) fn run_specifics(
    reply: &palace_core::distill::Reply,
    run: recall::Day,
) -> recall::Specifics {
    let texts: Vec<(String, &str)> = reply
        .claims
        .iter()
        .filter(|c| c.kind == "specific")
        .map(|c| {
            let quote = c.quote.as_deref().unwrap_or("");
            (
                format!("{} {} {quote}", c.subject, c.object),
                c.date.as_str(),
            )
        })
        .collect();
    recall::Specifics::new(texts.iter().map(|(t, d)| (t.as_str(), *d)), run)
}
/// The distill-point specifics (SETTLE-1d §2): up to `--specific-sample` sentences of the range
/// (both roles, code dropped first: SETTLE-1e §1) that carry a value, each value kept by the run's specifics or `missing`; the row
/// counts values, not sentences (`rule:values`), and its items record the totals.
fn distilled_specifics(
    range: &str,
    turns: &[Turn],
    reply: &palace_core::distill::Reply,
    rule: &ValueRule,
) -> Check {
    let (common, run) = (rule.common, rule.run);
    let specifics = run_specifics(reply, run);
    let carrying: Vec<(usize, String)> = turns
        .iter()
        .flat_map(|t| {
            let line = t.line;
            recall::sentences(&recall::prose(&t.text))
                .into_iter()
                .map(move |s| (line, s))
        })
        .filter(|(_, s)| !recall::values(s, common).is_empty())
        .collect();
    let mut check = Check::new("distill", range, "specific");
    let key = |(line, s): &(usize, String)| format!("{line}:{s}");
    let mut unexplained = 0;
    for (line, sentence) in recall::sample(range, &carrying, key, rule.sample) {
        let kept = recall::kept_values(&sentence, &specifics, common, run);
        let values: Vec<&str> = kept.iter().map(|(v, _)| v.as_str()).collect();
        let explained: Vec<&str> = kept
            .iter()
            .filter(|(v, k)| !k && skipped(reply, line, v))
            .map(|(v, _)| v.as_str())
            .collect();
        unexplained += kept.iter().filter(|(_, k)| !k).count() - explained.len();
        check.add_values(
            json!({"line": line, "sentence": sentence, "values": values, "skipped": explained}),
            &kept,
        );
    }
    let c = &check.counts;
    check.items = json!({"values_sampled": c.sampled, "values_kept": c.supported,
        "values_missing": c.missing, "values_unexplained": unexplained,
        "values_skipped": reply.skipped.len(), "checklist_values": rule.listed,
        "sentences": carrying.len(), "items": check.items});
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
pub(crate) struct Turn {
    pub(crate) line: usize,
    pub(crate) role: String,
    pub(crate) text: String,
}
impl Turn {
    /// The message as the judge sees it: `[role] text`.
    fn shown(&self) -> String {
        format!("[{}] {}", self.role, self.text)
    }
}

/// The copy lines a distill run read, from its logged range `<copy>:<a>-<b>`.
fn copy_turns(event: &Value) -> Result<Vec<Turn>, Error> {
    let (copy, first, last) = split_range(&text_of(event, "range"))?;
    range_turns(Path::new(&copy), first, last)
}
/// A distill range, `<copy>:<first>-<last>`, as its parts.
pub(crate) fn split_range(range: &str) -> Result<(String, usize, usize), Error> {
    let (copy, lines) = range
        .rsplit_once(':')
        .ok_or("distill_commit without a range")?;
    let (first, last) = lines
        .split_once('-')
        .ok_or("distill_commit range without lines")?;
    Ok((copy.to_owned(), first.parse()?, last.parse()?))
}
/// The turns of a normalized transcript copy from line `first` to `last`.
pub(crate) fn range_turns(copy: &Path, first: usize, last: usize) -> Result<Vec<Turn>, Error> {
    let text = fs::read_to_string(crate::paths::input(copy)?)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use palace_core::distill::{DistilledClaim, Reply};
    fn turn(line: usize, role: &str, text: &str) -> Turn {
        Turn {
            line,
            role: role.into(),
            text: text.into(),
        }
    }
    fn specific(object: &str, quote: &str, date: &str) -> DistilledClaim {
        DistilledClaim {
            kind: "specific".into(),
            subject: "Ravi".into(),
            predicate: "takes".into(),
            object: object.into(),
            date: date.into(),
            quote: Some(quote.into()),
            line: 2,
            demoted: None,
        }
    }
    #[test]
    fn the_specifics_sample_draws_value_sentences_and_counts_values() {
        let common = common_words(&crate::paths::root().join("prompts")).unwrap();
        let rule = ValueRule {
            common: &common,
            run: recall::Day::parse("2026-10-10").unwrap(),
            sample: 20,
            listed: Some(5),
        };
        let turns = [
            turn(
                1,
                "user",
                "If you're up with her, I'm here. - **Ida goes in first** then.",
            ),
            turn(2, "user", "Ravi and Theo swap: Ravi takes the 24th."),
            turn(3, "assistant", "I'll note it. We meet Saturday, 10/17."),
        ];
        let empty = Reply::default();
        let check = distilled_specifics("t.jsonl:1-3", &turns, &empty, &rule);
        let lines: Vec<u64> = check.items["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["line"].as_u64().unwrap())
            .collect();
        assert_eq!(lines, [2, 3], "the noise of line 1 is never sampled");
        assert_eq!(
            (
                check.counts.sampled,
                check.counts.supported,
                check.counts.missing
            ),
            (5, 0, 5)
        );
        assert_eq!(check.items["values_missing"], 5);
        assert_eq!(check.items["values_unexplained"], 5);
        let trivia = Reply {
            skipped: vec![palace_core::distill::Skip {
                line: 3,
                value: "10/17".into(),
                reason: "trivia".into(),
            }],
            ..Reply::default()
        };
        let check = distilled_specifics("t.jsonl:1-3", &turns, &trivia, &rule);
        assert_eq!(
            check.counts.missing, 5,
            "a skip leaves raw recall unchanged"
        );
        assert_eq!(check.items["values_unexplained"], 4);
        assert_eq!(check.items["items"][1]["skipped"], json!(["10/17"]));
        assert!(
            check
                .line(&Judge::none())
                .ends_with(" unexplained=4 skipped=1/5")
        );
        let filed = Reply {
            claims: vec![
                specific(
                    "the 24th, swapping with Theo",
                    "Ravi takes the 24th",
                    "2026-10-10",
                ),
                specific("the meeting", "We meet", "2026-10-17"),
            ],
            ..Reply::default()
        };
        let check = distilled_specifics("t.jsonl:1-3", &turns, &filed, &rule);
        assert_eq!((check.counts.sampled, check.counts.supported), (5, 5));
        assert_eq!(
            check.items["items"][1]["kept"],
            json!(["Saturday", "10/17"])
        );
        assert_eq!(check.judge_label(&Judge::none()), "rule:values");
        let mut consolidation = Check::new("consolidation", "r.md", "specific");
        consolidation.add(json!({}), Some("supported"));
        assert_eq!(consolidation.judge_label(&Judge::none()), "rule:exact");
    }
}
