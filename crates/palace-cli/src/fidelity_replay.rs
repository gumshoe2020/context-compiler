//! `palace fidelity --replay-values` (SETTLE-1d acceptance 3, SETTLE-1e acceptance 2): the stored
//! distill-point specifics rows rescored under the current values rule, offline, on a copy of a
//! claim store. Both rules' rows: `rule:exact` (old = sentences supported / sampled) and
//! `rule:values` (old = values kept / sampled, as SETTLE-1d scored them). Each stored sentence's
//! values are read again (a sentence that now carries none drops out as noise) and each value is
//! kept or not by the specifics of the run that wrote the row (its `distill_commit` event, matched
//! by range; the run's date from the event). The sample itself is the stored one.
//!
//! With `--transcripts <dir>` (copies of the rows' transcripts, by file name), a stored sentence
//! keeps only the values its line still carries once code is dropped (a sentence that was code
//! carries none), and each row also gets its run's checklist size under the current rule, against
//! the one the run listed (`-`: a run from before the checklist). Prints one `REPLAY` line per row
//! and a pooled `REPLAY TOTAL` per rule and for all rows; writes nothing.
use crate::fidelity_cmd::{range_turns, split_range};
use palace_core::{
    distill::{CopyLine, checklist},
    recall::{self, CommonWords},
};
use palace_store::{Error, claims::ClaimStore};
use serde_json::Value;
use std::path::Path;

/// The rules a stored row may have been scored by.
const RULES: [&str; 2] = ["rule:exact", "rule:values"];

/// One row's (or a pool's) old and new numbers.
#[derive(Default)]
struct Scores {
    old_sampled: u64,
    old_supported: u64,
    sentences: u64,
    carrying: u64,
    values: u64,
    kept: u64,
}

pub fn replay(
    store: &ClaimStore,
    common: &CommonWords,
    transcripts: Option<&Path>,
) -> Result<(), Error> {
    let events = store.events("distill_commit")?;
    let mut totals: Vec<(Scores, usize)> = RULES.iter().map(|_| (Scores::default(), 0)).collect();
    for row in store.fidelity_rows(Some("0000"))? {
        let specifics = row["point"] == "distill" && row["type"] == "specific";
        let Some(rule) = RULES.iter().position(|r| row["judge"] == *r) else {
            continue;
        };
        if !specifics {
            continue;
        }
        let file = row["file"].as_str().unwrap_or_default();
        let at = row["at"].as_str().unwrap_or_default();
        let Some(event) = events.iter().find(|e| e["range"] == file) else {
            println!("REPLAY {at} {file}: no distill_commit event: skipped");
            continue;
        };
        let lines = transcripts.map(|dir| range_lines(dir, file)).transpose()?;
        let scores = rescore(&row, event, common, lines.as_deref())?;
        let size = lines.as_deref().map(|l| checklist_size(l, event, common));
        println!(
            "REPLAY {at} {file}: {} {}{}",
            RULES[rule],
            scores.line(rule),
            size.unwrap_or_default()
        );
        totals[rule].0.add(&scores);
        totals[rule].1 += 1;
    }
    let mut all = Scores::default();
    for (rule, (scores, rows)) in totals.iter().enumerate() {
        println!(
            "REPLAY TOTAL {} rows={rows} {}",
            RULES[rule],
            scores.line(rule)
        );
        all.add(scores);
    }
    let rows: usize = totals.iter().map(|(_, n)| n).sum();
    println!(
        "REPLAY TOTAL all rows={rows} new={}/{} values ({})",
        all.kept,
        all.values,
        percent(all.kept, all.values)
    );
    Ok(())
}

impl Scores {
    fn add(&mut self, other: &Scores) {
        self.old_sampled += other.old_sampled;
        self.old_supported += other.old_supported;
        self.sentences += other.sentences;
        self.carrying += other.carrying;
        self.values += other.values;
        self.kept += other.kept;
    }
    /// `old=… new=… values (…) sentences with values …`, old in the row's own unit.
    fn line(&self, rule: usize) -> String {
        let unit = ["sentences", "values"][rule];
        format!(
            "old={}/{} {unit} ({}) new={}/{} values ({}) sentences with values {}/{}",
            self.old_supported,
            self.old_sampled,
            percent(self.old_supported, self.old_sampled),
            self.kept,
            self.values,
            percent(self.kept, self.values),
            self.carrying,
            self.sentences
        )
    }
}
fn percent(part: u64, whole: u64) -> String {
    match whole {
        0 => "-".to_owned(),
        n => format!("{}%", part * 100 / n),
    }
}

/// The lines of the row's range, read from the copy of its transcript in `dir`.
fn range_lines(dir: &Path, range: &str) -> Result<Vec<CopyLine>, Error> {
    let (copy, first, last) = split_range(range)?;
    let name = Path::new(&copy)
        .file_name()
        .ok_or_else(|| format!("{range}: no transcript file name"))?;
    Ok(range_turns(&dir.join(name), first, last)?
        .into_iter()
        .map(|t| CopyLine {
            line: t.line,
            role: t.role,
            text: t.text,
        })
        .collect())
}
/// ` checklist old=<listed|-> new=<values> values (<lines> lines)`: the run's checklist, as
/// listed and under the current rule.
fn checklist_size(lines: &[CopyLine], event: &Value, common: &CommonWords) -> String {
    let old = event["checklist"]
        .as_array()
        .map_or("-".to_owned(), |list| {
            let values: usize = list
                .iter()
                .map(|l| l["values"].as_array().map_or(0, Vec::len))
                .sum();
            values.to_string()
        });
    let list = checklist(lines, common);
    let values: usize = list.iter().map(|(_, v)| v.len()).sum();
    format!(
        " checklist old={old} new={values} values ({} lines)",
        list.len()
    )
}

/// The row's stored sentences under the current rule, against its run's specifics.
fn rescore(
    row: &Value,
    event: &Value,
    common: &CommonWords,
    lines: Option<&[CopyLine]>,
) -> Result<Scores, Error> {
    let run = crate::fidelity_cmd::run_day(event)?;
    let reply = palace_core::distill::from_json(&event["reply"]);
    let specifics = crate::fidelity_cmd::run_specifics(&reply, run);
    let items = match &row["items"] {
        Value::Array(items) => items.as_slice(),
        nested => nested["items"].as_array().map_or(&[][..], Vec::as_slice),
    };
    let mut scores = Scores {
        old_sampled: row["sampled"].as_u64().unwrap_or(0),
        old_supported: row["supported"].as_u64().unwrap_or(0),
        sentences: items.len() as u64,
        ..Scores::default()
    };
    for item in items {
        let sentence = item["sentence"].as_str().unwrap_or_default();
        let mut kept = recall::kept_values(sentence, &specifics, common, run);
        if let Some(lines) = lines {
            let line = item["line"].as_u64().unwrap_or(0) as usize;
            let prose = line_values(lines, line, common);
            kept.retain(|(v, _)| prose.contains(&v.to_lowercase()));
        }
        scores.carrying += u64::from(!kept.is_empty());
        scores.values += kept.len() as u64;
        scores.kept += kept.iter().filter(|(_, k)| *k).count() as u64;
    }
    Ok(scores)
}
/// The values (lowercase) of a line's prose, code dropped.
fn line_values(lines: &[CopyLine], line: usize, common: &CommonWords) -> Vec<String> {
    let text = lines
        .iter()
        .find(|l| l.line == line)
        .map_or("", |l| l.text.as_str());
    recall::sentences(&recall::prose(text))
        .iter()
        .flat_map(|s| recall::values(s, common))
        .map(|v| v.to_lowercase())
        .collect()
}
