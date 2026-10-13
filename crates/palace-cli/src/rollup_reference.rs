//! SETTLE-1d §6, the IO around `palace_core::rollup_reference`: a rollup supersedes only the
//! sources whose live specifics its claims keep or cite; any other source is referenced. At
//! commit the validated body is marked before it is written (and in a dry run before it is
//! printed). `palace rollup --rescan-supersession <rollup>` applies the rule to a rollup already
//! in the diary: the file is rewritten in place (atomically: a sibling file renamed over it), the
//! change logged (`rollup_rescan`, with the text's hash before and after) and the diary root
//! mined again. Idempotent: a referenced source is no longer superseded, so a second rescan finds
//! nothing and changes nothing. A rollup marked before SETTLE-1e §4 has its referenced entries
//! moved to their own paragraph (`referenced entries listed apart`), once. With `--dry-run`
//! (SETTLE-1e §3) the rescan prints the same line
//! prefixed `DRY RUN` and stops there: no file written, no event logged, no mine.
use palace_core::{
    rollup::{Entry, superseded_sources},
    rollup_reference::{self, LiveSpecific, Uncited},
    traits::DiaryReader,
};
use palace_mempalace::{Unavailable, diary::Diary};
use palace_store::{Error, claims::ClaimStore};
use serde_json::{Value, json};
use std::{fs, path::Path};

/// The body with its uncited sources referenced instead of superseded, and those sources.
pub fn mark(kg: &Path, names: &[String], body: &str) -> Result<(String, Vec<Uncited>), Error> {
    let entry = Entry::parse("rollup-candidate", body.to_owned());
    let sources = superseded_sources(&entry, names).resolved;
    let uncited = rollup_reference::referenced(&entry, names, &live_specifics(kg, &sources)?);
    for u in &uncited {
        eprintln!(
            "rollup: references {} instead of superseding it: its live specifics {} are neither kept nor cited",
            u.entry,
            u.claims.join(", ")
        );
    }
    Ok((rollup_reference::mark(body, names, &uncited), uncited))
}

/// The live specifics the entries state (none before the claim store exists).
fn live_specifics(kg: &Path, entries: &[String]) -> Result<Vec<LiveSpecific>, Error> {
    let store = crate::rollup_commit::compiler_path(kg)?;
    if !store.is_file() || entries.is_empty() {
        return Ok(Vec::new());
    }
    let text = |row: &Value, key: &str| row[key].as_str().unwrap_or_default().to_owned();
    Ok(ClaimStore::open(&store)?
        .live_specifics(entries)?
        .iter()
        .map(|row| LiveSpecific {
            id: text(row, "id"),
            object: text(row, "object"),
            entry: text(row, "entry"),
        })
        .collect())
}

/// `--rescan-supersession`: one rollup of the diary under the rule. Prints one `ROLLUP rescan`
/// line (prefixed `DRY RUN` in a dry run, which changes nothing); exit 0 (changed or not).
pub fn rescan(options: &crate::rollup::Options, file: &Path) -> Result<i32, Error> {
    let root = crate::paths::diary_root(&options.diary)?;
    let diary = Diary::open(&root.join("diary"))?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("--rescan-supersession needs a diary file name")?;
    let name = name.strip_suffix(".md").unwrap_or(name);
    let text = diary
        .read(name)?
        .ok_or_else(|| format!("--rescan-supersession {name}: not in the diary"))?;
    if !Entry::parse(name, text.clone()).is_rollup {
        return Err(format!("--rescan-supersession {name}: not a rollup").into());
    }
    let names = diary.names()?;
    let (marked, uncited) = mark(&options.kg, &names, &text)?;
    let marked = rollup_reference::layout(&marked, &names);
    let dry = if options.dry_run { "DRY RUN " } else { "" };
    if marked == text && uncited.is_empty() {
        println!("{dry}ROLLUP rescan {name}.md: no change");
        return Ok(0);
    }
    if marked == text {
        return Err(
            format!("--rescan-supersession {name}: no `## Supersedes` heading to mark").into(),
        );
    }
    let entries: Vec<&str> = uncited.iter().map(|u| u.entry.as_str()).collect();
    let line = match entries.is_empty() {
        true => format!("ROLLUP rescan {name}.md: referenced entries listed apart"),
        false => format!("ROLLUP rescan {name}.md: referenced {}", entries.join(", ")),
    };
    if options.dry_run {
        println!("{dry}{line}");
        return Ok(0);
    }
    let path = crate::paths::database(&root.join("diary").join(format!("{name}.md")))?;
    let scratch = path.with_extension("md.rescan");
    fs::write(&scratch, &marked)?;
    fs::rename(&scratch, &path)?;
    let sha = |t: &str| palace_core::ids::sha256(t.as_bytes());
    let referenced: Vec<Value> = uncited
        .iter()
        .map(|u| json!({"entry": u.entry, "claims": u.claims}))
        .collect();
    ClaimStore::open(&crate::rollup_commit::compiler_path(&options.kg)?)?.log(&json!({
        "operation": "rollup_rescan", "file": format!("{name}.md"), "referenced": referenced,
        "before_sha256": sha(&text), "after_sha256": sha(&marked)}))?;
    mine(options, &root)?;
    println!("{line}");
    Ok(0)
}
fn mine(options: &crate::rollup::Options, root: &Path) -> Result<(), Error> {
    let palace = crate::paths::database(&options.palace)?.canonicalize()?;
    let to = palace_mempalace::mine::Target {
        wing: &options.wing,
        ..Default::default()
    };
    palace_mempalace::mine::mine_to(&options.connection.mempalace, &palace, root, &to)
        .map_err(Unavailable::error)?;
    Ok(())
}
