use clap::{Args, Subcommand};
use palace_store::{
    Error,
    privacy::{self, People},
};
use std::{fs, path::PathBuf};

static SENSITIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn sensitive(value: bool) {
    if value {
        SENSITIVE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
pub fn diagnostic(message: std::fmt::Arguments<'_>, stdout: bool) {
    let text = message.to_string();
    let text = if SENSITIVE.load(std::sync::atomic::Ordering::Relaxed) {
        format!(
            "PRIVACY operation id={}",
            palace_core::ids::sha256(text.as_bytes())
        )
    } else {
        text
    };
    if stdout {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
}
pub fn has_private(store: &std::path::Path) -> bool {
    let check = || -> Result<bool, Error> {
        if !store.is_file() {
            return Ok(false);
        }
        let c = rusqlite::Connection::open_with_flags(
            store,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        if !c
            .prepare("SELECT 1 FROM sqlite_master WHERE name='privacy_markers'")?
            .exists([])?
        {
            return Ok(false);
        }
        Ok(c.prepare("SELECT 1 FROM privacy_markers WHERE private=1")?
            .exists([])?)
    };
    check().unwrap_or(true)
}

pub fn filter_diary(
    diary: &mut palace_mempalace::diary::Diary,
    store: &std::path::Path,
    readers: &[String],
) -> Result<(), Error> {
    use palace_core::traits::DiaryReader;
    if !store.is_file() {
        diary.admit_only(Default::default());
        return Ok(());
    }
    let c =
        rusqlite::Connection::open_with_flags(store, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if !c
        .prepare("SELECT 1 FROM sqlite_master WHERE name='privacy_markers'")?
        .exists([])?
    {
        diary.admit_only(Default::default());
        return Ok(());
    }
    let sql = format!("SELECT {}", privacy::visible("entry", "?1", readers));
    let mut allowed = std::collections::BTreeSet::new();
    for name in diary.names()? {
        if c.query_row(&sql, [&name], |r| r.get::<_, bool>(0))? {
            allowed.insert(name);
        }
    }
    diary.admit_only(allowed);
    Ok(())
}
/// Rollups consume exactly one audience, independently of the invoking conversation.
pub fn filter_audience_diary(
    diary: &mut palace_mempalace::diary::Diary,
    store: &std::path::Path,
    audience: &privacy::Audience,
) -> Result<(), Error> {
    use palace_core::traits::DiaryReader;
    let mut allowed = std::collections::BTreeSet::new();
    if store.is_file() {
        let c = rusqlite::Connection::open_with_flags(
            store,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        for name in diary.names()? {
            if audience.admits_entry(&c, &name)? {
                allowed.insert(name);
            }
        }
    }
    diary.admit_only(allowed);
    Ok(())
}

pub fn current_readers() -> Vec<String> {
    let identity = privacy::identity();
    match (&identity.people, &identity.conversation) {
        (Some(p), Some(c)) => p.resolve(c, None).people.into_iter().collect(),
        _ => Vec::new(),
    }
}

pub fn filter_gate_diary(
    diary: &mut palace_mempalace::diary::Diary,
    store: &std::path::Path,
    marker: &privacy::Marker,
) -> Result<(), Error> {
    use palace_core::traits::DiaryReader;
    let c =
        rusqlite::Connection::open_with_flags(store, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let visible = privacy::visible(
        "entry",
        "?1",
        &marker.audience.iter().cloned().collect::<Vec<_>>(),
    );
    let sql = format!(
        "SELECT {visible} OR EXISTS (SELECT 1 FROM privacy_markers WHERE kind='entry' AND id=?1 AND source=?2)"
    );
    let mut allowed = std::collections::BTreeSet::new();
    for name in diary.names()? {
        if c.query_row(&sql, rusqlite::params![name, marker.source], |r| {
            r.get::<_, bool>(0)
        })? {
            allowed.insert(name);
        }
    }
    diary.admit_only(allowed);
    Ok(())
}

#[derive(Subcommand)]
pub enum Operation {
    Resolve {
        #[arg(long)]
        people: PathBuf,
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        sender: Option<String>,
    },
    Backfill(Backfill),
    /// Remove one person's audience data from staged backup copies.
    BackupFilter(crate::backup_filter::Options),
    /// Classify an offline diary/store copy for review.
    Classify(crate::audience::Options),
}
#[derive(Args)]
pub struct Backfill {
    #[arg(long)]
    store: PathBuf,
    #[arg(long)]
    kg: PathBuf,
    #[arg(long)]
    people: PathBuf,
    /// Read-only plan first. Omit only after reviewing the report.
    #[arg(long)]
    dry_run: bool,
    /// Existing session-to-wing map, used to resolve private wing audiences.
    #[arg(long)]
    wing_map: Option<PathBuf>,
    /// Include legacy hand-filed entries, including every old wing's diary root (repeatable).
    #[arg(long)]
    diary_root: Vec<PathBuf>,
}
pub fn run(operation: Operation) -> Result<(), Error> {
    match operation {
        Operation::Classify(o) => return crate::audience::run(o),
        Operation::BackupFilter(o) => return crate::backup_filter::run(o),
        Operation::Resolve {
            people,
            conversation,
            sender,
        } => {
            let p = People::parse(&fs::read_to_string(crate::paths::input(&people)?)?)?;
            println!("{}", p.resolve(&conversation, sender.as_deref()).json());
        }
        Operation::Backfill(o) => {
            let people = People::parse(&fs::read_to_string(crate::paths::input(&o.people)?)?)?;
            let path = crate::paths::input(&o.store)?;
            // Dry-run never opens Store (which migrates); both databases are read-only.
            let c = rusqlite::Connection::open_with_flags(
                &path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let kg = crate::paths::input(&o.kg)?;
            c.execute(
                "ATTACH DATABASE ?1 AS kg",
                [format!("file:{}?mode=ro", kg.display())],
            )?;
            let mut evidence = privacy::LegacyEvidence::default();
            if let Some(map) = &o.wing_map {
                let value: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(crate::paths::input(map)?)?)?;
                evidence.wing_map = value
                    .as_object()
                    .ok_or("--wing-map must be one JSON object")?
                    .clone();
                for wing in evidence.wing_map.values() {
                    crate::wing_name(wing.as_str().ok_or("wing-map values must be wing names")?)?;
                }
            }
            for root in &o.diary_root {
                let root = crate::paths::diary_root(root)?;
                let wing = privacy::LegacyEvidence::diary_wing(&root);
                for file in fs::read_dir(root.join("diary"))? {
                    let file = file?.path();
                    if file.extension().is_none_or(|e| e != "md") {
                        continue;
                    }
                    let file = crate::paths::input(&file)?;
                    let name = file
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .ok_or("invalid diary name")?
                        .to_owned();
                    let text = fs::read_to_string(&file)?;
                    evidence
                        .tags
                        .insert(name.clone(), privacy::entry_tags(&text));
                    let locator = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Transcript: "))
                        .unwrap_or_default()
                        .to_owned();
                    let value = (wing.clone(), locator);
                    if evidence.entries.get(&name).is_some_and(|old| old != &value) {
                        return Err("conflicting diary entry wing evidence".into());
                    }
                    evidence.entries.insert(name, value);
                }
            }
            let plan = privacy::legacy_backfill(&c, &people, &evidence)?;
            drop(c);
            if !o.dry_run {
                let mut store = palace_store::Store::open(&path)?;
                store.apply_legacy_privacy_backfill(&plan)?;
            }
            let mut counts: std::collections::BTreeMap<
                String,
                std::collections::BTreeMap<String, usize>,
            > = Default::default();
            for row in &plan {
                *counts
                    .entry(row.kind.clone())
                    .or_default()
                    .entry(row.reason.clone())
                    .or_default() += 1;
                println!(
                    "{}",
                    serde_json::json!({"kind":row.kind,"id":row.id,"marker":row.marker.json(),"reason":row.reason,"dry_run":o.dry_run})
                );
            }
            eprintln!(
                "{}",
                serde_json::json!({"privacy_backfill_counts":counts,"dry_run":o.dry_run})
            );
        }
    }
    Ok(())
}
