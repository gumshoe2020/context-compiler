//! Marker-based filtering of closed staged backup databases; never opens a live install.
use clap::Args;
use palace_store::{Error, privacy_backup};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Options {
    #[arg(long)]
    store: PathBuf,
    #[arg(long)]
    kg: PathBuf,
    /// Diary root; files are only listed, never changed.
    #[arg(long)]
    diary: PathBuf,
    #[arg(long)]
    exclude_person: String,
}

fn staged(path: &Path) -> Result<PathBuf, Error> {
    // Reject lexical live paths as well as aliases resolving into them.
    for candidate in [path.to_owned(), crate::paths::input(path)?] {
        let parts: Vec<_> = candidate
            .components()
            .filter_map(|p| p.as_os_str().to_str())
            .collect();
        if parts.iter().any(|s| {
            matches!(
                *s,
                ".mempalace" | "plugins" | "apply2" | "palace-settle" | "palace-shadow"
            )
        }) || parts
            .windows(2)
            .any(|p| p[0] == ".openclaw" && p[1].starts_with("palace-"))
        {
            return Err("backup-filter refuses live store paths; use staged copies".into());
        }
    }
    let resolved = crate::paths::input(path)?;
    if fs::symlink_metadata(path)?.file_type().is_symlink() || fs::metadata(&resolved)?.nlink() != 1
    {
        return Err("backup-filter requires independent regular staged files (no links)".into());
    }
    if !resolved.is_file() {
        return Err("backup-filter requires regular staged files".into());
    }
    crate::audience::immutable_uri(&resolved)?;
    crate::paths::database(&resolved)
}

pub fn run(o: Options) -> Result<(), Error> {
    // Slug validation also prevents ambiguous/empty exclusion identities.
    let audience: palace_store::privacy::Audience = o.exclude_person.parse()?;
    if !audience.private() || o.exclude_person.contains(',') {
        return Err("--exclude-person requires one person ID".into());
    }
    let store = staged(&o.store)?;
    let kg = staged(&o.kg)?;
    if store == kg {
        return Err("store and kg must be distinct staged copies".into());
    }
    let root = crate::paths::diary_root(&o.diary)?;
    let read = Connection::open_with_flags(
        crate::audience::immutable_uri(&store)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let plan = privacy_backup::plan(&read, &o.exclude_person)?;
    drop(read);
    let entries = plan.entries();
    let mut paths = Vec::new();
    for entry in fs::read_dir(root.join("diary"))? {
        let path = entry?.path();
        if path.extension().is_none_or(|s| s != "md") {
            continue;
        }
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err("backup-filter refuses linked diary entries".into());
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("invalid diary entry name")?;
        if entries.contains(name) {
            if path
                .as_os_str()
                .as_encoded_bytes()
                .iter()
                .any(|b| matches!(b, b'\n' | b'\r'))
            {
                return Err("backup-filter refuses multiline exclusion paths".into());
            }
            paths.push(path);
        }
    }
    paths.sort();
    let mut c = Connection::open_with_flags(&store, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    c.execute(
        "ATTACH DATABASE ?1 AS kg",
        [kg.to_str().ok_or("non-UTF8 graph path")?],
    )?;
    privacy_backup::apply(&mut c, &plan, &o.exclude_person)?;
    for path in paths {
        println!("{}", path.display());
    }
    Ok(())
}
