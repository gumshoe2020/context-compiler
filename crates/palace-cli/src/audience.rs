//! Offline-copy audience classification. Reports are review artifacts, never automatic consent.
use clap::Args;
use palace_core::{ids::sha256, traits::Model};
use palace_store::{
    Error,
    privacy::{self, Marker, People},
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::os::unix::ffi::OsStrExt;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Options {
    #[arg(long, required_unless_present_any = ["apply", "undo"])]
    diary: Option<PathBuf>,
    #[arg(long, required_unless_present_any = ["apply", "undo"])]
    store: Option<PathBuf>,
    /// Copy of the identity map; default: people.json beside the store.
    #[arg(long)]
    people: Option<PathBuf>,
    /// Graph copy; default: knowledge_graph.sqlite3 beside the store.
    #[arg(long)]
    kg: Option<PathBuf>,
    /// claude-cli[:model] (Sonnet) | command:<exe> | replay.
    #[arg(long, required_unless_present_any = ["apply", "undo"])]
    llm: Option<String>,
    /// For replay, a JSON object keyed by entry filename, each value a classifier reply.
    #[arg(long)]
    replay_reply: Option<PathBuf>,
    #[arg(long, default_value = "prompts")]
    prompts: PathBuf,
    #[arg(long, default_value_t = 120)]
    model_timeout: u64,
    #[arg(long, required_unless_present_any = ["apply", "undo"])]
    out: Option<PathBuf>,
    /// Reuse unchanged decisions from a previous report with the same diary layout.
    #[arg(long)]
    reuse: Option<PathBuf>,
    /// Apply a reviewed report to its offline copies, with backups first.
    #[arg(long, conflicts_with_all = ["undo", "diary", "store", "llm", "out", "people", "kg", "replay_reply", "reuse"])]
    apply: Option<PathBuf>,
    /// Restore every touched file from the run directory printed by --apply.
    #[arg(long, conflicts_with_all = ["diary", "store", "llm", "out", "people", "kg", "replay_reply", "reuse"])]
    undo: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub category: String,
    pub reason: String,
}
#[derive(Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub hash: String,
    pub title: String,
    pub decision: Decision,
    #[serde(default)]
    pub reused: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Summary {
    pub count: usize,
    pub samples: Vec<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub diary: PathBuf,
    pub store: PathBuf,
    pub store_hash: String,
    pub people: PathBuf,
    pub kg: PathBuf,
    pub entries: Vec<Entry>,
    pub counts: BTreeMap<String, Summary>,
}
pub fn copy_input(path: &Path) -> Result<PathBuf, Error> {
    let path = crate::paths::input(path)?;
    if path.components().any(|p| {
        matches!(
            p.as_os_str().to_str(),
            Some(".mempalace" | "palace-shadow" | "palace-settle" | "plugins")
        )
    }) {
        return Err("classification requires an offline copy under --roots".into());
    }
    Ok(path)
}
/// Immutable SQLite reads cannot create WAL/SHM files. Require a closed copy so no
/// uncheckpointed data can be silently omitted from the report or its store hash.
pub fn immutable_uri(path: &Path) -> Result<String, Error> {
    for suffix in ["-wal", "-shm", "-journal"] {
        if PathBuf::from(format!("{}{suffix}", path.display())).exists() {
            return Err("classification requires a closed, checkpointed offline store copy".into());
        }
    }
    let mut encoded = String::from("file:");
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.".contains(byte) {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(encoded + "?mode=ro&immutable=1")
}
/// The audience-* tags a people map approves, in file order (none without a map).
pub fn approved_tags(mapping: Option<&People>) -> Vec<String> {
    let categories = mapping.map(People::audience_categories).unwrap_or_default();
    categories.into_iter().map(|(tag, _)| tag).collect()
}
pub fn validate(d: &Decision, approved: &[String]) -> Result<(), Error> {
    if (!matches!(d.category.as_str(), "shared" | "keep") && !approved.contains(&d.category))
        || d.reason.trim().is_empty()
        || d.reason.lines().count() != 1
        || d.reason.len() > 512
    {
        return Err("invalid audience category or one-line reason".into());
    }
    Ok(())
}
/// Validate report metadata without opening any of its old copy paths. Copies may
/// move or disappear; only the flat diary-relative filenames are reusable keys.
fn reuse_entries(path: &Path, approved: &[String]) -> Result<BTreeMap<String, Entry>, Error> {
    let report: Report = serde_json::from_str(&fs::read_to_string(copy_input(path)?)?)?;
    if report.version != 1 {
        return Err("unsupported audience report version".into());
    }
    let mut entries = BTreeMap::new();
    for entry in report.entries {
        validate(&entry.decision, approved)?;
        let relative = entry
            .path
            .strip_prefix(report.diary.join("diary"))
            .map_err(|_| "reuse report has a different diary layout")?;
        if relative.components().count() != 1
            || relative.extension().is_none_or(|e| e != "md")
            || relative.as_os_str() != relative.file_name().unwrap_or_default()
        {
            return Err("reuse report has a different diary layout".into());
        }
        let name = relative
            .to_str()
            .ok_or("invalid reuse entry filename")?
            .to_owned();
        if entry.hash.len() != 64 || !entry.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid reuse entry hash".into());
        }
        if entries.insert(name, entry).is_some() {
            return Err("reuse report has duplicate diary filenames".into());
        }
    }
    Ok(entries)
}
struct Replay(BTreeMap<String, serde_json::Value>);
impl Model for Replay {
    type Error = Error;
    fn complete(&self, prompt: &str) -> Result<String, Error> {
        let name = prompt
            .lines()
            .find_map(|l| l.strip_prefix("ENTRY FILE: "))
            .ok_or("missing replay entry")?;
        Ok(self
            .0
            .get(name)
            .ok_or("missing classifier replay reply")?
            .to_string())
    }
}
pub fn run(o: Options) -> Result<(), Error> {
    if let Some(path) = &o.apply {
        return crate::audience_apply::apply(path);
    }
    if let Some(path) = &o.undo {
        return crate::audience_apply::undo(path);
    }
    let llm = o.llm.as_deref().ok_or("classification requires --llm")?;
    let diary = copy_input(&o.diary.ok_or("classification requires --diary")?)?;
    let diary = crate::paths::diary_root(&diary)?;
    let store = copy_input(&o.store.ok_or("classification requires --store")?)?;
    let connection = Connection::open_with_flags(
        immutable_uri(&store)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    if !connection
        .prepare("SELECT 1 FROM sqlite_master WHERE name='privacy_markers'")?
        .exists([])?
    {
        return Err("classification needs a marked store copy".into());
    }
    let store_hash = sha256(&fs::read(&store)?);
    let parent = store.parent().ok_or("store has no parent")?;
    let people = crate::paths::resolved(&o.people.unwrap_or_else(|| parent.join("people.json")))?;
    let kg = crate::paths::resolved(
        &o.kg
            .unwrap_or_else(|| parent.join("knowledge_graph.sqlite3")),
    )?;
    // Validate even optional, nonexistent apply inputs against the output fence now.
    let people = crate::paths::database(&people)?;
    let kg = crate::paths::database(&kg)?;
    let mapping = if people.exists() {
        Some(People::parse(&fs::read_to_string(copy_input(&people)?)?)?)
    } else {
        None
    };
    let approved = approved_tags(mapping.as_ref());
    let previous = o
        .reuse
        .as_deref()
        .map(|path| reuse_entries(path, &approved))
        .transpose()?
        .unwrap_or_default();
    let prompts = crate::paths::input(&o.prompts)?;
    let policy = crate::policy::load(&prompts, "audience.md", mapping.as_ref())?;
    let model: Box<dyn Model<Error = Error>> = if llm == "replay" {
        Box::new(Replay(serde_json::from_str(&fs::read_to_string(
            copy_input(&o.replay_reply.ok_or("replay requires --replay-reply")?)?,
        )?)?))
    } else {
        crate::distill::model_spec(llm, &prompts, o.model_timeout, None)?
    };
    let mut files: Vec<_> = fs::read_dir(diary.join("diary"))?
        .map(|f| f.map(|f| f.path()))
        .collect::<Result<_, _>>()?;
    files.sort();
    let mut entries = Vec::new();
    let mut counts: BTreeMap<String, Summary> = approved
        .iter()
        .cloned()
        .chain(["shared".to_owned(), "keep".to_owned()])
        .map(|c| {
            (
                c,
                Summary {
                    count: 0,
                    samples: Vec::new(),
                },
            )
        })
        .collect();
    let mut reused_count = 0;
    let mut classified_count = 0;
    for file in files {
        if file.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let path = copy_input(&file)?;
        let text = fs::read_to_string(&path)?;
        let title = text
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .unwrap_or("Untitled")
            .to_owned();
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid entry filename")?;
        let hash = sha256(text.as_bytes());
        let previous = previous
            .get(name)
            .filter(|entry| entry.hash == hash && entry.decision.category != "keep");
        let mut reused = false;
        let already_private = privacy::markers(&connection, "entry", name)?
            .iter()
            .any(|marker| marker.private)
            || mapping.as_ref().is_some_and(|mapping| {
                mapping
                    .tagged(&Marker::shared(name), &privacy::entry_tags(&text))
                    .0
                    .private
            });
        let decision = if already_private {
            Decision {
                category: "keep".into(),
                reason: "already private".into(),
            }
        } else if let Some(entry) = previous {
            reused = true;
            reused_count += 1;
            Decision {
                category: entry.decision.category.clone(),
                reason: entry.decision.reason.clone(),
            }
        } else {
            classified_count += 1;
            let prompt = format!(
                "{policy}\nENTRY FILE: {name}\nUNTRUSTED ENTRY (JSON string):\n{}",
                serde_json::to_string(&text)?
            );
            let decision: Decision = serde_json::from_str(&model.complete(&prompt)?)
                .map_err(|_| "invalid classifier reply")?;
            validate(&decision, &approved)?;
            if decision.category == "keep" {
                return Err("keep is reserved for already private entries".into());
            }
            decision
        };
        let summary = counts
            .get_mut(&decision.category)
            .ok_or("unknown category")?;
        summary.count += 1;
        if summary.samples.len() < 8 {
            summary.samples.push(title.clone());
        }
        entries.push(Entry {
            path,
            hash,
            title,
            decision,
            reused,
        });
    }
    let report = Report {
        version: 1,
        diary,
        store,
        store_hash,
        people,
        kg,
        entries,
        counts,
    };
    let out = crate::paths::database(&o.out.ok_or("classification requires --out")?)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)?
        .write_all(serde_json::to_string_pretty(&report)?.as_bytes())?;
    println!("{}", serde_json::to_string(&report.counts)?);
    println!(
        "{}",
        serde_json::json!({
            "reused": reused_count, "classified": classified_count,
            "keep": report.counts["keep"].count
        })
    );
    Ok(())
}
