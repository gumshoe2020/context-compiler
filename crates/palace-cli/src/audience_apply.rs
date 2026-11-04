//! Review report application on offline copies, with a durable byte-for-byte undo journal.
use crate::audience::{Report, approved_tags, copy_input, immutable_uri, validate};
use palace_core::ids::sha256;
use palace_store::{
    Error,
    privacy::{self, LegacyEvidence, People},
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Backup {
    path: PathBuf,
    original_hash: String,
    applied_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    files: Vec<Backup>,
}
fn file_result<T>(path: &Path, result: Result<T, impl std::fmt::Display>) -> Result<T, Error> {
    result.map_err(|e| format!("{}: {e}", path.display()).into())
}
fn input(path: &Path) -> Result<PathBuf, Error> {
    file_result(path, copy_input(path))
}
fn read(path: &Path) -> Result<Vec<u8>, Error> {
    file_result(path, fs::read(path))
}
fn read_text(path: &Path) -> Result<String, Error> {
    file_result(path, fs::read_to_string(path))
}
fn quiescent(path: &Path) -> Result<(), Error> {
    for suffix in ["-wal", "-shm", "-journal"] {
        if PathBuf::from(format!("{}{suffix}", path.display())).exists() {
            return Err(format!(
                "{}: apply/undo requires a closed, checkpointed offline store copy",
                path.display()
            )
            .into());
        }
    }
    Ok(())
}
/// Changes only one plain tag field, or appends a final field when none exists.
fn append_tag(text: &str, tag: &str) -> Result<String, Error> {
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let mut front_matter = text.lines().next().is_some_and(|l| l.trim() == "---");
    let mut positions = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if i > 0 && matches!(trimmed, "---" | "...") {
            front_matter = false;
        }
        if trimmed.to_lowercase().starts_with("tags:") {
            let values = trimmed[5..].trim();
            let block_list = values.is_empty()
                && lines[i + 1..]
                    .iter()
                    .find(|l| !l.trim().is_empty())
                    .is_some_and(|l| l.trim().starts_with("- "));
            if front_matter || values.starts_with('[') || block_list {
                return Err("apply refuses YAML tags; review a plain Tags: field".into());
            }
            positions.push(i);
        }
    }
    if positions.len() > 1 {
        return Err("apply refuses multiple tag fields; no other text may change".into());
    }
    let existing = privacy::entry_tags(text);
    if existing.contains(tag) {
        return Ok(text.to_owned());
    }
    if existing.iter().any(|t| t.starts_with("audience-")) {
        return Err("entry already has a different audience tag; review a fresh report".into());
    }
    if positions.is_empty() {
        let ending = if text.ends_with("\r\n") { "\r\n" } else { "\n" };
        let mut result = text.to_owned();
        if !text.ends_with('\n') {
            result.push_str(ending);
        }
        if !text.ends_with('\n') || text.lines().last().is_some_and(|l| !l.trim().is_empty()) {
            result.push_str(ending);
        }
        result.push_str(&format!("Tags: {tag}.{ending}"));
        return Ok(result);
    }
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i != positions[0] {
            result.push_str(line);
            continue;
        }
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let trim = body.trim_end();
        let (prefix, period) = trim.strip_suffix('.').map_or((trim, ""), |s| (s, "."));
        let separator = if prefix.trim().eq_ignore_ascii_case("tags:") {
            " "
        } else {
            ", "
        };
        result.push_str(&format!(
            "{prefix}{separator}{tag}{period}{}{ending}",
            &body[trim.len()..]
        ));
    }
    Ok(result)
}
fn save_journal(run: &Path, journal: &Journal) -> Result<(), Error> {
    let pending = run.join("manifest.pending");
    let mut out = fs::File::create(&pending)?;
    out.write_all(serde_json::to_string_pretty(journal)?.as_bytes())?;
    out.sync_all()?;
    fs::rename(pending, run.join("manifest.json"))?;
    fs::File::open(run)?.sync_all()?;
    Ok(())
}
pub fn apply(path: &Path) -> Result<(), Error> {
    apply_report(path).map_err(|e| format!("{}: {e}", path.display()).into())
}
fn apply_report(path: &Path) -> Result<(), Error> {
    let report_path = input(path)?;
    let report: Report = serde_json::from_str(&read_text(&report_path)?)?;
    if report.version != 1 {
        return Err("unsupported audience report version".into());
    }
    let diary = crate::paths::diary_root(&input(&report.diary)?)?;
    let store = input(&report.store)?;
    quiescent(&store)?;
    if sha256(&read(&store)?) != report.store_hash {
        return Err(format!(
            "{}: store changed since classification; reclassify the copy",
            store.display()
        )
        .into());
    }
    let people_path = input(&report.people)?;
    let people_text = read_text(&people_path)?;
    let people =
        People::parse(&people_text).map_err(|e| format!("{}: {e}", people_path.display()))?;
    let approved = approved_tags(Some(&people));
    let kg = input(&report.kg)?;
    quiescent(&kg)?;
    let mut evidence = LegacyEvidence::default();
    let mut changes = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in &report.entries {
        file_result(&entry.path, validate(&entry.decision, &approved))?;
        let file = input(&entry.path)?;
        if file.parent() != Some(diary.join("diary").as_path())
            || file.extension().is_none_or(|e| e != "md")
            || !seen.insert(file.clone())
        {
            return Err(format!(
                "{}: report entry must be a unique diary file in the copy",
                file.display()
            )
            .into());
        }
        let text = read_text(&file)?;
        if sha256(text.as_bytes()) != entry.hash {
            return Err(format!(
                "{}: diary changed since classification; reclassify the copy",
                file.display()
            )
            .into());
        }
        let updated = if matches!(entry.decision.category.as_str(), "shared" | "keep") {
            text.clone()
        } else {
            append_tag(&text, &entry.decision.category)
                .map_err(|e| format!("{}: {e}", file.display()))?
        };
        let name = file
            .file_stem()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{}: invalid entry filename", file.display()))?
            .to_owned();
        let locator = text
            .lines()
            .find_map(|l| l.strip_prefix("Transcript: "))
            .unwrap_or_default()
            .to_owned();
        evidence
            .entries
            .insert(name.clone(), (LegacyEvidence::diary_wing(&diary), locator));
        evidence.tags.insert(name, privacy::entry_tags(&updated));
        if updated != text {
            changes.push((file, updated.into_bytes()));
        }
    }
    // Reject incomplete reports; omitted inputs must not escape classification/inheritance.
    for file in fs::read_dir(diary.join("diary"))? {
        let file = file?.path();
        if file.extension().is_some_and(|e| e == "md") && !seen.contains(&input(&file)?) {
            return Err(format!(
                "{}: diary entry added or omitted since classification",
                file.display()
            )
            .into());
        }
    }
    let connection = Connection::open_with_flags(
        immutable_uri(&store)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    connection.execute("ATTACH DATABASE ?1 AS kg", [immutable_uri(&kg)?])?;
    let plan = privacy::legacy_backfill(&connection, &people, &evidence)?;
    // Only restrictions traceable to a private tag may affect existing rows. Shared
    // decisions never authorize declassification or unrelated legacy backfill.
    let plan: Vec<_> = plan
        .into_iter()
        .filter(|r| r.reason.starts_with("tag:"))
        .collect();
    drop(connection);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run = crate::paths::database(&report_path.with_extension(format!("run-{stamp}")))?;
    fs::create_dir(&run)?;
    fs::set_permissions(&run, fs::Permissions::from_mode(0o700))?;
    let mut journal = Journal {
        version: 1,
        files: Vec::new(),
    };
    let mut paths: Vec<_> = changes.iter().map(|(p, _)| p.clone()).collect();
    paths.push(store.clone());
    let mut distinct = BTreeSet::new();
    for (i, path) in paths.iter().enumerate() {
        if !distinct.insert(path.clone()) {
            return Err(format!("{}: apply input paths overlap", path.display()).into());
        }
        let bytes = read(path)?;
        let backup = run.join(format!("{i}.bak"));
        fs::copy(path, &backup)?;
        fs::File::open(&backup)?.sync_all()?;
        journal.files.push(Backup {
            path: path.clone(),
            original_hash: sha256(&bytes),
            applied_hash: None,
        });
    }
    save_journal(&run, &journal)?; // Durable backups of ALL touched files precede the first edit.
    eprintln!("audience undo run: {}", run.display());
    let result = (|| -> Result<(), Error> {
        // Markers go in first: a partial failure cannot leave a tag on an unprotected shared row.
        let mut database = Connection::open_with_flags(&store, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        privacy::apply_legacy_privacy_backfill(&mut database, &plan)?;
        drop(database);
        for (path, bytes) in &changes {
            fs::write(path, bytes)?;
            fs::File::open(path)?.sync_all()?;
        }
        for file in &mut journal.files {
            file.applied_hash = Some(sha256(&read(&file.path)?));
        }
        save_journal(&run, &journal)?;
        Ok(())
    })();
    if result.is_err() {
        // The durable manifest also permits manual undo if rollback is interrupted.
        restore(&run, &journal, false)?;
        return Err("audience apply failed; original files restored from the undo run".into());
    }
    println!(
        "{}",
        serde_json::json!({"run":run,"marked_rows":plan.len(),"changed_entries":changes.len()})
    );
    Ok(())
}
fn restore(run: &Path, journal: &Journal, check_after: bool) -> Result<(), Error> {
    if journal.version != 1 {
        return Err(format!(
            "{}: unsupported undo version",
            run.join("manifest.json").display()
        )
        .into());
    }
    // Preflight every input and backup before restoring anything.
    for (i, file) in journal.files.iter().enumerate() {
        let path = input(&file.path)?;
        if path != file.path {
            return Err(format!("{}: undo target moved", file.path.display()).into());
        }
        quiescent(&path)?;
        let backup = input(&run.join(format!("{i}.bak")))?;
        if !backup.starts_with(run) || sha256(&read(&backup)?) != file.original_hash {
            return Err(format!(
                "{}: undo backup changed ({})",
                file.path.display(),
                run.join(format!("{i}.bak")).display()
            )
            .into());
        }
        let current = sha256(&read(&path)?);
        if check_after
            && file.applied_hash.as_ref().is_some_and(|h| *h != current)
            && current != file.original_hash
        {
            return Err(format!(
                "{}: undo target changed after apply; refusing to overwrite later work",
                path.display()
            )
            .into());
        }
    }
    for (i, file) in journal.files.iter().enumerate() {
        fs::copy(run.join(format!("{i}.bak")), &file.path)?;
        fs::File::open(&file.path)?.sync_all()?;
    }
    Ok(())
}
pub fn undo(path: &Path) -> Result<(), Error> {
    undo_run(path).map_err(|e| format!("{}: {e}", path.display()).into())
}
fn undo_run(path: &Path) -> Result<(), Error> {
    let run = input(path)?;
    let manifest = input(&run.join("manifest.json"))?;
    let journal: Journal = file_result(&manifest, serde_json::from_str(&read_text(&manifest)?))?;
    restore(&run, &journal, true)?;
    println!("{}", serde_json::json!({"undone":run}));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appends_only_inside_tags_preserving_all_other_bytes() {
        for (text, expected) in [
            (
                "# Invented\nTags: dome, brass.\n\nProse.\n",
                "# Invented\nTags: dome, brass, audience-household.\n\nProse.\n",
            ),
            (
                "# Invented\r\nTags: dome.  \r\nProse\r\n",
                "# Invented\r\nTags: dome, audience-household.  \r\nProse\r\n",
            ),
            ("Tags:", "Tags: audience-household"),
            ("  tAgS: dome.\n", "  tAgS: dome, audience-household.\n"),
            ("  tags:  ", "  tags: audience-household  "),
        ] {
            assert_eq!(append_tag(text, "audience-household").unwrap(), expected);
        }
        for text in [
            "No tag line",
            "Prose. Tags: telescope.",
            "Prose.\n",
            "Prose.\n\n",
            "Prose.\r\n",
            "Prose.\r\n\r\n",
        ] {
            let updated = append_tag(text, "audience-household").unwrap();
            assert!(updated.starts_with(text));
            assert!(privacy::entry_tags(&updated).contains("audience-household"));
            let ending = if text.ends_with("\r\n") { "\r\n" } else { "\n" };
            assert!(updated.ends_with(&format!(
                "{ending}{ending}Tags: audience-household.{ending}"
            )));
        }
        for text in [
            "Tags: one.\n  tAgS: two.",
            "---\ntags: one\n---\nProse",
            "tags: [one]",
            "tags:\n  - one",
        ] {
            assert!(append_tag(text, "audience-household").is_err());
        }
        assert!(append_tag("Tags: audience-orin.", "audience-household").is_err());
    }
}
