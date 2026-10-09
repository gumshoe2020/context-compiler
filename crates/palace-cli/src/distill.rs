//! `palace distill` (SETTLE-1b, spec §3): one transcript's new text becomes diary entries in its
//! conversation's wing, typed claims and graph relations, after the text is copied into the
//! palace. Read → copy → (skip) → model → commit (distill_commit.rs).
//! Exit 0 written / skipped / nothing new / dry run; 4 the model's reply failed its check twice;
//! 3 MemPalace unavailable; 2 any other broken run (a transcript that cannot be read included).
//! A run is keyed by (transcript, recorded position): a rerun from the same position resumes the
//! logged run (the same text, reply and entries), so a settle run that failed after distilling
//! never writes an entry twice.
use crate::distill_commit::{self, Run};
use clap::Args;
use palace_core::{
    distill::{self, Checked, CopyLine},
    ids::{length_prefixed, sha256},
    traits::Model,
    transcript::{self, Source, Turn},
};
use palace_store::{Error, claims::ClaimStore};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Args)]
pub struct Options {
    /// The session's transcript (Claude Code or Codex JSONL; for openclaw the normalized copy).
    #[arg(long)]
    pub transcript: PathBuf,
    /// claude-code | codex | openclaw
    #[arg(long)]
    pub source: String,
    /// The default wing's diary root (the parent of `diary/`).
    #[arg(long)]
    pub diary: PathBuf,
    /// Private wings: `<wings>/<wing>` is a wing's diary root.
    #[arg(long)]
    pub wings: PathBuf,
    /// The palace copies: `<transcripts>/<wing>/<source>/<session id>.jsonl`.
    #[arg(long)]
    pub transcripts: PathBuf,
    #[arg(long)]
    pub palace: PathBuf,
    #[arg(long)]
    pub kg: PathBuf,
    /// The default wing (a session the wing map does not name).
    #[arg(long, default_value = "memory", value_parser = crate::wing_name)]
    pub wing: String,
    /// `{"<session key or prefix>": "<wing>"}`; none: every session in the default wing.
    #[arg(long)]
    pub wing_map: Option<PathBuf>,
    /// Recorded positions, one `<source>\t<transcript>\t<lines read>` per transcript (missing
    /// file or line: from the start).
    #[arg(long)]
    pub positions: Option<PathBuf>,
    /// Write the new position into --positions (settle.sh records it itself, after a clean run).
    #[arg(long, requires = "positions")]
    pub record: bool,
    /// replay | claude-cli[:model] | command:<exe>
    #[arg(long, default_value = "replay")]
    pub llm: String,
    /// `--llm replay`: the answer file.
    #[arg(long)]
    pub replay_reply: Option<PathBuf>,
    #[arg(long, default_value = "prompts")]
    pub prompts: PathBuf,
    /// Fewer exchanges (user turns) than this: the position is recorded, nothing written.
    #[arg(long, default_value_t = 3)]
    pub min_turns: usize,
    /// Deadline in seconds for each model call.
    #[arg(long, default_value_t = 600)]
    pub model_timeout: u64,
    /// Print what would be filed (entries, claims, relations); copy, write and record nothing.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub connection: crate::MemPalaceOptions,
}

pub fn run(options: Options) -> Result<i32, Error> {
    let source = Source::parse(&options.source)?;
    let transcript = crate::paths::input(&options.transcript)?;
    let text = fs::read_to_string(&transcript)
        .map_err(|e| format!("transcript {} cannot be read: {e}", transcript.display()))?;
    let from = recorded(&options)?;
    let session = transcript::session_id(&transcript);
    let wing = wing_of(&options, &session)?;
    let key = run_key(&transcript, from);
    let store = crate::rollup_commit::compiler_path(&options.kg)?;
    if let Some(commit) = logged(&store, "distill_commit", &key)? {
        eprintln!("distill: resume {key} (already committed from line {from})");
        let to = commit["to"].as_u64().unwrap_or(0) as usize;
        let code = distill_commit::resume(&options, commit)?;
        return record(&options, to).map(|()| code);
    }
    let read = transcript::read(source, &text, from)
        .map_err(|e| format!("transcript {} cannot be read: {e}", transcript.display()))?;
    let mut read = read;
    if let Some(copy) = logged(&store, "distill_copy", &key)? {
        cut_at(&mut read, copy["to"].as_u64().unwrap_or(0) as usize);
    }
    if read.turns.is_empty() {
        println!("DISTILL nothing new wing={wing} position={}", read.position);
        return record(&options, read.position).map(|()| 0);
    }
    let run = Run {
        key,
        source,
        transcript,
        session,
        wing,
        from,
        to: read.position,
        store,
    };
    distill(&options, run, &read.turns)
}

fn distill(options: &Options, run: Run, turns: &[(usize, Turn)]) -> Result<i32, Error> {
    let lines = match options.dry_run {
        true => dry_lines(&run, turns),
        false => copy(options, &run, turns)?,
    };
    if transcript::exchanges(turns) < options.min_turns
        || turns.iter().all(|(_, t)| t.text.trim().is_empty())
    {
        let exchanges = transcript::exchanges(turns);
        if !options.dry_run {
            distill_commit::log(
                &run,
                json!({"operation":"distill_commit","run":run.key,"skipped":true,"to":run.to,"wing":run.wing,"copy":run.copy_path(options)?}),
            )?;
        }
        println!(
            "DISTILL skipped exchanges={exchanges} wing={} position={}",
            run.wing, run.to
        );
        return record(options, run.to).map(|()| 0);
    }
    let Some(checked) = ask(options, &run, &lines)? else {
        return Ok(crate::rollup::EXIT_INVALID);
    };
    if options.dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&distill::to_json(&checked.reply))?
        );
        println!("DISTILL dry-run wing={} position={}", run.wing, run.to);
        return Ok(0);
    }
    let code = distill_commit::commit(options, &run, &checked.reply, &lines)?;
    record(options, run.to)?;
    Ok(code)
}

/// The model's checked reply: asked once, and once more with the problems named when the reply
/// fails its check (spec §4.4); `None` (printed DISTILL INVALID) when the second also fails its
/// shape. A `specific` still without a verbatim quote after the retry becomes a `concept`.
fn ask(options: &Options, run: &Run, lines: &[CopyLine]) -> Result<Option<Checked>, Error> {
    let model = model(options)?;
    let policy = palace_core::names::in_prose(&fs::read_to_string(crate::paths::input(
        &options.prompts.join("distill.md"),
    )?)?);
    let prompt = distill::prompt(&policy, &run.session, lines);
    let first = distill::check(&model.complete(&prompt)?, lines);
    if first.passed() {
        return Ok(Some(first));
    }
    eprintln!("distill: retry: {}", problems(&first).join("; "));
    let retry = format!(
        "{prompt}\nYour previous answer failed its check: {}. Answer again, with one JSON object.\n",
        problems(&first).join("; ")
    );
    let mut second = distill::check(&model.complete(&retry)?, lines);
    if !second.problems.is_empty() {
        println!("DISTILL INVALID {}", json!(second.problems));
        return Ok(None);
    }
    distill::demote_unquoted(&mut second);
    Ok(Some(second))
}
fn problems(checked: &Checked) -> Vec<String> {
    let mut all = checked.problems.clone();
    for index in &checked.unquoted_specifics {
        all.push(format!(
            "claim {index} is marked specific but its quote is missing or not verbatim in the conversation"
        ));
    }
    all
}

fn model(options: &Options) -> Result<Box<dyn Model<Error = Error>>, Error> {
    use palace_judge::command::{CommandModel, resolve_executable};
    let llm = options.llm.as_str();
    let timeout = Duration::from_secs(options.model_timeout);
    if llm == "replay" {
        let file = options
            .replay_reply
            .as_deref()
            .ok_or("--llm replay needs --replay-reply <file>")?;
        return Ok(Box::new(Replay(fs::read_to_string(crate::paths::input(
            file,
        )?)?)));
    }
    if let Some(rest) = llm.strip_prefix("claude-cli") {
        let model = rest
            .strip_prefix(':')
            .filter(|m| !m.is_empty())
            .unwrap_or("sonnet");
        let settings = crate::paths::input(&options.prompts.join("judge-settings.json"))?;
        let exe = resolve_executable("claude")?;
        return Ok(Box::new(
            CommandModel::claude(&exe, model, &settings)?.with_timeout(timeout),
        ));
    }
    match llm.strip_prefix("command:").filter(|e| !e.is_empty()) {
        Some(exe) => Ok(Box::new(CommandModel::generic(exe).with_timeout(timeout))),
        None => Err("--llm must be replay, claude-cli[:model] or command:<exe>".into()),
    }
}
/// `--llm replay`: the same answer every time.
struct Replay(String);
impl Model for Replay {
    type Error = Error;
    fn complete(&self, _prompt: &str) -> Result<String, Error> {
        Ok(self.0.clone())
    }
}

/// Spec §3.2: the new turns appended to the palace copy (an openclaw transcript is the copy, read
/// in place). Resumable: the copy's start line is logged before anything is appended, and only
/// the lines not yet there are appended.
fn copy(options: &Options, run: &Run, turns: &[(usize, Turn)]) -> Result<Vec<CopyLine>, Error> {
    if run.source == Source::OpenClaw {
        return Ok(lines_at(turns.iter().map(|(line, t)| (*line, t))));
    }
    let path = run.copy_path(options)?;
    let logged = logged(&run.store, "distill_copy", &run.key)?;
    let start = match &logged {
        Some(event) => event["copy_from"].as_u64().unwrap_or(0) as usize,
        None => {
            let start = line_count(&path)?;
            distill_commit::log(
                run,
                json!({"operation":"distill_copy","run":run.key,"to":run.to,"copy":path,"copy_from":start,"lines":turns.len()}),
            )?;
            start
        }
    };
    let present = line_count(&path)?.saturating_sub(start);
    if present < turns.len() {
        fs::create_dir_all(path.parent().ok_or("copy path has no parent")?)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        for (_, turn) in &turns[present..] {
            writeln!(file, "{}", turn.to_line())?;
        }
        file.sync_all()?;
    }
    Ok(lines_at(
        turns
            .iter()
            .enumerate()
            .map(|(i, (_, t))| (start + i + 1, t)),
    ))
}
fn dry_lines(run: &Run, turns: &[(usize, Turn)]) -> Vec<CopyLine> {
    match run.source {
        Source::OpenClaw => lines_at(turns.iter().map(|(line, t)| (*line, t))),
        _ => lines_at(turns.iter().enumerate().map(|(i, (_, t))| (i + 1, t))),
    }
}
fn lines_at<'a>(turns: impl Iterator<Item = (usize, &'a Turn)>) -> Vec<CopyLine> {
    turns
        .map(|(line, t)| CopyLine {
            line,
            role: t.role.clone(),
            text: t.text.clone(),
        })
        .collect()
}
/// Complete lines in a file (0 when it does not exist).
fn line_count(path: &Path) -> Result<usize, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes.iter().filter(|b| **b == b'\n').count()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("{}: {e}", path.display()).into()),
    }
}
/// A resumed copy covers the source lines up to the logged `to`, whatever was written since.
fn cut_at(read: &mut transcript::Read, to: usize) {
    read.turns.retain(|(line, _)| *line <= to);
    read.position = to;
}

fn run_key(transcript: &Path, from: usize) -> String {
    let path = transcript.to_string_lossy();
    sha256(length_prefixed(&[&path, &from.to_string()]).as_bytes())[..12].to_owned()
}
/// The last write-log event of `operation` for this run, if any (never opens a missing store).
pub(crate) fn logged(store: &Path, operation: &str, key: &str) -> Result<Option<Value>, Error> {
    if !store.is_file() {
        return Ok(None);
    }
    Ok(ClaimStore::open(store)?
        .events(operation)?
        .into_iter()
        .rfind(|e| e["run"] == key))
}

/// The wing a session files in: the wing map's, else the default.
fn wing_of(options: &Options, session: &str) -> Result<String, Error> {
    let Some(file) = &options.wing_map else {
        return Ok(options.wing.clone());
    };
    let map: Value = serde_json::from_slice(&fs::read(crate::paths::input(file)?)?)
        .map_err(|e| format!("--wing-map {}: {e}", file.display()))?;
    let map = map
        .as_object()
        .ok_or("--wing-map must be one JSON object")?;
    match transcript::wing_for(map, session) {
        Some(wing) => crate::wing_name(wing).map_err(Into::into),
        None => Ok(options.wing.clone()),
    }
}

/// The recorded position of this transcript (0 when none).
fn recorded(options: &Options) -> Result<usize, Error> {
    let Some(file) = &options.positions else {
        return Ok(0);
    };
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("--positions {}: {e}", file.display()).into()),
    };
    let wanted = options.transcript.to_string_lossy();
    Ok(text
        .lines()
        .filter_map(|l| {
            let mut fields = l.split('\t');
            let (_, path, position) = (fields.next()?, fields.next()?, fields.next()?);
            (path == wanted)
                .then(|| position.trim().parse().ok())
                .flatten()
        })
        .next_back()
        .unwrap_or(0))
}
/// `--record`: this transcript's line in --positions replaced (or added), written atomically.
fn record(options: &Options, position: usize) -> Result<(), Error> {
    let (Some(file), true, false) = (&options.positions, options.record, options.dry_run) else {
        return Ok(());
    };
    let file = crate::paths::database(file)?;
    let wanted = options.transcript.to_string_lossy().into_owned();
    let mut lines: Vec<String> = fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split('\t').nth(1) != Some(wanted.as_str()))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{}\t{wanted}\t{position}", options.source));
    let scratch = file.with_extension("new");
    fs::write(&scratch, lines.join("\n") + "\n")?;
    fs::rename(scratch, file)?;
    Ok(())
}
