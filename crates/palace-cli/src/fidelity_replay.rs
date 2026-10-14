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
//!
//! SETTLE-1i: each row is also scored kept anywhere (`anywhere=…`: a value kept when the run's
//! specifics, concepts or entries hold it), with where (`kept_in specific=… concept=… entry=…`);
//! `rule:values-anywhere` rows replay too. `--since <date>` keeps the rows written on or after it.
//! With `--written <dir>` (copies of the entries the runs wrote, by file name), each row also
//! says how many of its run's entry files the copy holds with the run's entry text
//! (` written=<found>/<files>`), the check that the entries scored are the ones filed.
use crate::fidelity_cmd::{range_turns, split_range};
use palace_core::{
    distill::{CopyLine, checklist},
    recall::{self, CommonWords},
};
use palace_store::{Error, claims::ClaimStore};
use serde_json::Value;
use std::path::Path;

/// The rules a stored row may have been scored by, and the unit of each one's old figure.
const RULES: [&str; 3] = ["rule:exact", "rule:values", "rule:values-anywhere"];
const UNITS: [&str; 3] = ["sentences", "values", "values"];

/// One row's (or a pool's) old and new numbers.
#[derive(Default)]
struct Scores {
    old_sampled: u64,
    old_supported: u64,
    sentences: u64,
    carrying: u64,
    values: u64,
    kept: u64,
    /// Values kept anywhere (SETTLE-1i), and by where: specific, concept, entry.
    anywhere: u64,
    kept_in: [u64; 3],
}

/// Where the replay reads from, besides the store.
pub struct Copies<'a> {
    /// `--transcripts`: the rows' transcripts, by file name.
    pub transcripts: Option<&'a Path>,
    /// `--written`: the entries the rows' runs wrote, by file name.
    pub written: Option<&'a Path>,
    /// `--since`: only rows written on or after it.
    pub since: Option<&'a str>,
}

pub fn replay(store: &ClaimStore, common: &CommonWords, copies: &Copies) -> Result<(), Error> {
    let events = store.events("distill_commit")?;
    let mut totals: Vec<(Scores, usize)> = RULES.iter().map(|_| (Scores::default(), 0)).collect();
    for row in store.fidelity_rows(Some(copies.since.unwrap_or("0000")))? {
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
        let lines = copies
            .transcripts
            .map(|dir| range_lines(dir, file))
            .transpose()?;
        let scores = rescore(&row, event, common, lines.as_deref())?;
        let size = lines.as_deref().map(|l| checklist_size(l, event, common));
        let written = copies.written.map(|dir| written(dir, event));
        println!(
            "REPLAY {at} {file}: {} {}{}{}",
            RULES[rule],
            scores.line(rule),
            size.unwrap_or_default(),
            written.unwrap_or_default()
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
        "REPLAY TOTAL all rows={rows} new={}/{} values ({}) {}",
        all.kept,
        all.values,
        percent(all.kept, all.values),
        all.anywhere()
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
        self.anywhere += other.anywhere;
        for (mine, theirs) in self.kept_in.iter_mut().zip(other.kept_in) {
            *mine += theirs;
        }
    }
    /// `old=… new=… values (…) anywhere=… sentences with values …`, old in the row's own unit.
    fn line(&self, rule: usize) -> String {
        format!(
            "old={}/{} {} ({}) new={}/{} values ({}) {} sentences with values {}/{}",
            self.old_supported,
            self.old_sampled,
            UNITS[rule],
            percent(self.old_supported, self.old_sampled),
            self.kept,
            self.values,
            percent(self.kept, self.values),
            self.anywhere(),
            self.carrying,
            self.sentences
        )
    }
    /// `anywhere=<kept>/<values> values (<pct>) kept_in specific=<a> concept=<b> entry=<c>`.
    fn anywhere(&self) -> String {
        let [specific, concept, entry] = self.kept_in;
        format!(
            "anywhere={}/{} values ({}) kept_in specific={specific} concept={concept} entry={entry}",
            self.anywhere,
            self.values,
            percent(self.anywhere, self.values)
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

/// ` written=<found>/<files>`: of the entry files the run wrote, how many `dir` holds with the
/// run's entry (its title and body) as their text.
fn written(dir: &Path, event: &Value) -> String {
    let reply = palace_core::distill::from_json(&event["reply"]);
    let files: Vec<&str> = event["files"]
        .as_array()
        .map_or(vec![], |f| f.iter().filter_map(Value::as_str).collect());
    let found = files
        .iter()
        .zip(&reply.entries)
        .filter(|(name, entry)| {
            std::fs::read_to_string(dir.join(name)).is_ok_and(|text| {
                text.starts_with(&format!("# {}\n\n", entry.title.trim()))
                    && text.contains(entry.body.trim())
            })
        })
        .count();
    format!(" written={found}/{}", files.len())
}

/// The row's stored sentences under the current rule, against its run's specifics, and kept
/// anywhere, against everything the run filed (SETTLE-1i).
fn rescore(
    row: &Value,
    event: &Value,
    common: &CommonWords,
    lines: Option<&[CopyLine]>,
) -> Result<Scores, Error> {
    let run = crate::fidelity_cmd::run_day(event)?;
    let reply = palace_core::distill::from_json(&event["reply"]);
    let filed = crate::fidelity_cmd::run_filed(&reply, run);
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
        let mut kept = recall::kept_in(sentence, &filed, common, run);
        if let Some(lines) = lines {
            let line = item["line"].as_u64().unwrap_or(0) as usize;
            let prose = line_values(lines, line, common);
            kept.retain(|(v, _)| prose.contains(&v.to_lowercase()));
        }
        scores.carrying += u64::from(!kept.is_empty());
        scores.values += kept.len() as u64;
        scores.kept += kept
            .iter()
            .filter(|(_, at)| *at == Some("specific"))
            .count() as u64;
        for (_, at) in &kept {
            if let Some(i) = at.and_then(|at| recall::KEPT_IN.iter().position(|k| *k == at)) {
                scores.anywhere += 1;
                scores.kept_in[i] += 1;
            }
        }
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
