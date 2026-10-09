use clap::Args;
use palace_core::{
    canary::{self, Canary},
    claim, fidelity,
    rollup::Entry,
    traits::{DiaryReader, Judge},
};
use palace_mempalace::diary::Diary;
use palace_store::Error;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Args)]
pub struct Options {
    #[arg(long, default_value = "fixtures/diary")]
    pub diary: PathBuf,
    #[arg(long, default_value = "fixtures/canaries.json")]
    pub canaries: PathBuf,
    #[arg(long, alias = "report-dir", default_value = "target/fidelity-reports")]
    pub reports: PathBuf,
    #[arg(long)]
    pub no_judge: bool,
    #[arg(long,conflicts_with_all=["all","rollups"])]
    pub since_minutes: Option<u64>,
    #[arg(long, conflicts_with = "rollups")]
    pub all: bool,
    #[arg(long)]
    pub rollups: Option<PathBuf>,
    #[arg(long, default_value = "replay")]
    pub judge: String,
    #[arg(long, default_value = "sonnet")]
    pub model: String,
    #[arg(long, default_value = "fixtures/fidelity-reports")]
    pub replay_reports: PathBuf,
    /// Minimal settings for the claude-cli judge (P5: hooks disabled).
    #[arg(long, default_value = "prompts/judge-settings.json")]
    pub judge_settings: PathBuf,
    /// compiler.sqlite claim store: record claims + findings and derive confidence across runs.
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// M4d item 7: canaries from the person's corrections, read with `--canaries` when the file exists
    /// (default `<the store's directory>/canaries.json` with `--store`).
    #[arg(long)]
    pub canaries_out: Option<PathBuf>,
    /// M4d item 8: the terms table (prompts/terms.json): add `vocabulary_gap` rows (and, with
    /// `--store`, findings). Without it the watch is unchanged.
    #[arg(long)]
    pub terms: Option<PathBuf>,
}
pub struct Overlay<'a> {
    pub diary: &'a Diary,
    pub entries: BTreeMap<String, String>,
}
impl DiaryReader for Overlay<'_> {
    type Error = Error;
    fn header(&self, path: &str) -> Result<String, Error> {
        self.diary.header(path)
    }
    fn names(&self) -> Result<Vec<String>, Error> {
        let mut names = self.diary.names()?;
        names.extend(self.entries.keys().cloned());
        names.sort();
        names.dedup();
        Ok(names)
    }
    fn read(&self, name: &str) -> Result<Option<String>, Error> {
        let name = name.strip_suffix(".md").unwrap_or(name);
        if let Some(text) = self.entries.get(name) {
            Ok(Some(text.clone()))
        } else {
            self.diary.read(name)
        }
    }
}
pub fn load_entries(root: &Path) -> Result<Vec<Entry>, Error> {
    let diary = Diary::open(root)?;
    let mut entries = Vec::new();
    for name in diary.names()? {
        if let Ok(Some(text)) = diary.read(&name) {
            let entry = Entry::parse(&name, text);
            if entry.is_rollup {
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}
pub fn evaluate<D: DiaryReader>(
    entries: &[Entry],
    diary: &D,
    canaries: &[Canary],
    judge: Option<&dyn Judge<Error = Error>>,
) -> Result<Value, Error>
where
    D::Error: std::fmt::Display,
{
    let mut context = Vec::new();
    for name in diary.names().map_err(|e| e.to_string())? {
        if let Ok(Some(text)) = diary.read(&name) {
            let source = Entry::parse(&name, text);
            if source.is_rollup {
                context.extend(claim::parse(&source.text).claims);
            }
        }
    }
    let mut results = Vec::new();
    for entry in entries {
        results.push(fidelity::floor(entry, diary));
        results.push(fidelity::depth(entry, diary));
        if let Some(judge) = judge {
            for c in canaries {
                results.push(canary::check(entry, c, judge));
            }
        }
        let parsed = claim::parse(&entry.text);
        results.extend(claim::check_against(
            entry,
            &parsed.claims,
            &context,
            diary,
            judge,
        ));
        results.extend(claim::malformed_rows(entry, &parsed.malformed));
    }
    Ok(fidelity::report(entries.len(), results))
}
pub fn run(options: Options) -> Result<i32, Error> {
    let diary_root = crate::paths::input(&options.diary)?;
    let diary = Diary::open(&diary_root)?;
    let entries = selected_entries(&options, &diary_root)?;
    let canaries = canaries(&options)?;
    let judge = judge(&options)?;
    let overlay = Overlay {
        diary: &diary,
        entries: entries
            .iter()
            .map(|e| (e.name.clone(), e.text.clone()))
            .collect(),
    };
    let mut report = evaluate(&entries, &overlay, &canaries, judge.as_deref())?;
    add_vocabulary(&options, &entries, &mut report)?;
    report_undated_voids(&overlay, &mut report)?;
    report_unclassified(&overlay, &entries, &mut report)?;
    if let Some(store) = &options.store {
        store_claims(&options, store, &mut report, &entries)?;
    }
    write_report(&options, &report)?;
    Ok(fidelity::exit_code(&report))
}

/// The rollups to check: every file under `--rollups`, else the diary's rollups modified in the
/// last `--since-minutes` (90) unless `--all`.
fn selected_entries(options: &Options, diary_root: &Path) -> Result<Vec<Entry>, Error> {
    let rollup_root = options
        .rollups
        .as_ref()
        .map(|p| crate::paths::input(p))
        .transpose()?
        .unwrap_or(diary_root.to_path_buf());
    let mut entries = load_entries(&rollup_root)?;
    if options.rollups.is_none() && !options.all {
        let seconds = options.since_minutes.unwrap_or(90).saturating_mul(60);
        let cutoff = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(seconds))
            .unwrap_or(UNIX_EPOCH);
        entries.retain(|e| {
            fs::metadata(rollup_root.join(format!("{}.md", e.name)))
                .and_then(|m| m.modified())
                .is_ok_and(|t| t >= cutoff)
        });
    }
    Ok(entries)
}

fn canaries(options: &Options) -> Result<Vec<palace_core::canary::Canary>, Error> {
    if options.no_judge {
        return Ok(Vec::new());
    }
    let bytes = fs::read(crate::paths::input(&options.canaries)?)
        .map_err(|e| format!("cannot read canaries {}: {e}", options.canaries.display()))?;
    let mut all: Vec<Canary> = serde_json::from_slice(&bytes)?;
    if let Some(out) = canaries_out(options) {
        all.extend(crate::canary_out::read(&out)?);
    }
    Ok(all)
}

/// The canaries-out file `watch` also reads: `--canaries-out`, else `canaries.json` beside `--store`.
fn canaries_out(options: &Options) -> Option<PathBuf> {
    options.canaries_out.clone().or_else(|| {
        let store = options.store.as_ref()?;
        Some(store.parent()?.join("canaries.json"))
    })
}

fn judge(options: &Options) -> Result<Option<Box<dyn Judge<Error = Error>>>, Error> {
    if options.no_judge {
        return Ok(None);
    }
    Ok(Some(match options.judge.as_str() {
        "replay" => Box::new(palace_judge::replay::Replay::open(&crate::paths::input(
            &options.replay_reports,
        )?)?),
        "claude-cli" => Box::new(palace_judge::command::CommandJudge::claude(
            &options.model,
            &crate::paths::input(&options.judge_settings)?,
        )?),
        value if value.starts_with("command:") && !value[8..].is_empty() => {
            Box::new(palace_judge::command::CommandJudge::generic(&value[8..]))
        }
        value if value.starts_with("table:") => crate::auto_ground::judge(value)?,
        _ => return Err("judge must be replay, claude-cli, command:<exe> or table:<file>".into()),
    }))
}

/// A judge from one spec (M4d-2b `improve --auto --judge`): `replay` (fixtures/fidelity-reports),
/// `claude-cli[:model]` (prompts/judge-settings.json) or `command:<exe>`.
pub fn judge_spec(spec: &str) -> Result<Box<dyn Judge<Error = Error>>, Error> {
    if let Some(rest) = spec.strip_prefix("claude-cli") {
        let model = rest
            .strip_prefix(':')
            .filter(|m| !m.is_empty())
            .unwrap_or("sonnet");
        let settings = crate::paths::input(Path::new("prompts/judge-settings.json"))?;
        return Ok(Box::new(palace_judge::command::CommandJudge::claude(
            model, &settings,
        )?));
    }
    match spec {
        "replay" => Ok(Box::new(palace_judge::replay::Replay::open(
            &crate::paths::input(Path::new("fixtures/fidelity-reports"))?,
        )?)),
        value if value.starts_with("command:") && !value[8..].is_empty() => Ok(Box::new(
            palace_judge::command::CommandJudge::generic(&value[8..]),
        )),
        _ => Err("judge must be table:<file>, replay, claude-cli[:model] or command:<exe>".into()),
    }
}

/// `--store`: unsettled intents reconciled first (M4d-2a.1 item 2), then the claims recorded.
fn store_claims(
    options: &Options,
    store: &Path,
    report: &mut Value,
    entries: &[Entry],
) -> Result<(), Error> {
    let store = crate::paths::database(store)?;
    crate::recovery::beside_store(&store)?;
    let judge = judge_label(&match options.judge.as_str() {
        "claude-cli" => format!("claude-cli:{}", options.model),
        other => other.to_owned(),
    });
    persist_claims(report, entries, &store, &judge)?;
    let mut claims = palace_store::claims::ClaimStore::open(&store)?;
    let at = claims.now()?;
    let rows = report["results"].as_array().cloned().unwrap_or_default();
    for entry in entries {
        claims.record_vocabulary(&entry.name, &rows, &at)?;
    }
    Ok(())
}

/// `--terms`: one `vocabulary` row per unmarked claim whose quote the table says its predicate
/// cannot carry (SPEC §4).
fn add_vocabulary(options: &Options, entries: &[Entry], report: &mut Value) -> Result<(), Error> {
    let Some(file) = &options.terms else {
        return Ok(());
    };
    let table: Value = serde_json::from_slice(&fs::read(crate::paths::input(file)?)?)?;
    let table = palace_core::terms::Table::from_json(&table)?;
    let results = report["results"]
        .as_array_mut()
        .ok_or("report missing results")?;
    for entry in entries {
        for c in claim::parse(&entry.text).claims {
            if c.superseded_by.is_none() && palace_core::terms::vocabulary_gap(&c, &table) {
                results.push(palace_core::terms::gap_row(&entry.name, &c));
            }
        }
    }
    Ok(())
}

/// M4d-2a.1 item 4 (review L-3): a `supersession-void` note without a date in its file name
/// voids nothing; it is named on stderr and in the report (`undated_void_notes`).
fn report_undated_voids(diary: &Overlay, report: &mut Value) -> Result<(), Error> {
    let undated = palace_core::rollup::undated_void_notes(diary, &diary.names()?)?;
    for note in &undated {
        eprintln!(
            "palace watch: supersession-void note {note} has no date in its file name; it voids nothing"
        );
    }
    if !undated.is_empty() {
        report["undated_void_notes"] = serde_json::json!(undated);
    }
    Ok(())
}
/// M5b-2 item 8 (review S-e): diary files a rollup's Supersedes section names outside both the
/// superseded list and every referenced-only region are named on stderr and in the report
/// (`unclassified`: `[{"rollup", "names"}]`), never dropped silently.
fn report_unclassified(
    diary: &Overlay,
    entries: &[Entry],
    report: &mut Value,
) -> Result<(), Error> {
    let names = diary.names()?;
    let mut found = Vec::new();
    for entry in entries {
        let unclassified = palace_core::rollup::unclassified(entry, &names);
        if unclassified.is_empty() {
            continue;
        }
        eprintln!(
            "palace watch: warning: {} names {} in its Supersedes section outside both the superseded list and every referenced-only region (unclassified)",
            entry.name,
            unclassified.join(", ")
        );
        found.push(serde_json::json!({"rollup": entry.name, "names": unclassified}));
    }
    if !found.is_empty() {
        report["unclassified"] = Value::Array(found);
    }
    Ok(())
}
fn write_report(options: &Options, report: &Value) -> Result<(), Error> {
    let directory = crate::paths::database(&options.reports)?;
    fs::create_dir_all(&directory)?;
    let path = crate::paths::database(&directory.join(format!(
        "{}.json",
        utc_name(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
    )))?;
    fs::write(&path, serde_json::to_string_pretty(report)?)?;
    let s = &report["summary"];
    println!(
        "palace-fidelity-watch: {} rollup(s), fail={} warn={} error={} — {}",
        s["rollups"],
        s["fail"],
        s["warn"],
        s["error"],
        path.display()
    );
    Ok(())
}

/// `judge:<model>` for a judge backend spec: `replay`, `claude-cli[:model]` (default sonnet) or
/// `command:<exe>`.
/// `judge:<model>` for a backend; a `table:` or `command:` backend is named without its path
/// (review L-4), so a survived check keys on the judge, not on where its file lives.
pub fn judge_label(backend: &str) -> String {
    let model = match backend.strip_prefix("claude-cli") {
        Some(rest) => rest
            .strip_prefix(':')
            .filter(|m| !m.is_empty())
            .unwrap_or("sonnet"),
        None => backend.split(':').next().unwrap_or(backend),
    };
    format!("judge:{model}")
}
/// Records every rollup's claim rows in the claim store; rows get history-derived confidence and
/// judge findings name `judge` as their checker.
pub fn persist_claims(
    report: &mut Value,
    entries: &[Entry],
    store: &Path,
    judge: &str,
) -> Result<(), Error> {
    let mut store = palace_store::claims::ClaimStore::open(store)?;
    let at = store.now()?;
    let results = report["results"]
        .as_array_mut()
        .ok_or("report missing results")?;
    for entry in entries {
        let claims = claim::parse(&entry.text).claims;
        let mut rows: Vec<Value> = results
            .iter()
            .filter(|r| r["check"] == "claim" && r["rollup"] == entry.name.as_str())
            .cloned()
            .collect();
        let (thread, date) = palace_core::rollup::title_parts(entry);
        let checked = palace_store::claims::Checked {
            rollup: &entry.name,
            date: &date,
            thread: &thread,
            judge,
            claims: &claims,
            rows: &mut rows,
        };
        store.record(checked, &at)?;
        let mut updated = rows.into_iter();
        for row in results
            .iter_mut()
            .filter(|r| r["check"] == "claim" && r["rollup"] == entry.name.as_str())
        {
            *row = updated
                .next()
                .ok_or("claim rows changed during persistence")?;
        }
    }
    Ok(())
}
pub fn utc_name(seconds: u64) -> String {
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn malformed_claim_is_a_finding_and_the_watch_continues() {
        use palace_core::diary_memory::Memory;
        use palace_core::rollup::Entry;
        let text = "# ROLLUP: t — state as of 2026-09-23\n\n## Claims\n- C1. prose without shape\n- C2. Callum | chose | Rust — 2026-09-21 — source.md\n";
        let mut diary = Memory::default();
        diary.0.insert("source".into(), "Callum chose Rust".into());
        diary.0.insert("r".into(), text.into());
        let entry = Entry::parse("r", text.into());
        let report = super::evaluate(&[entry], &diary, &[], None).unwrap();
        let rows = report["results"].as_array().unwrap();
        let checks: Vec<_> = rows.iter().map(|r| r["check"].as_str().unwrap()).collect();
        assert_eq!(checks, ["floor", "depth", "claim", "claim"]);
        assert_eq!(rows[3]["status"], "malformed");
        assert_eq!(rows[3]["finding"]["kind"], "malformed_claim");
        assert_eq!(rows[3]["finding"]["raw"], "- C1. prose without shape");
        assert_eq!(palace_core::fidelity::exit_code(&report), 1);
    }
    #[test]
    fn utc_dates() {
        assert_eq!(super::utc_name(0), "19700101T000000Z");
        assert_eq!(super::utc_name(951782400), "20000229T000000Z");
        assert_eq!(super::utc_name(1709164799), "20240228T235959Z");
    }
}
