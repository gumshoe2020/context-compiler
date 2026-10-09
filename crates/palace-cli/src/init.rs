//! `palace init <diary-root> <palace-dir>`: pinned MemPalace, a mined palace, a KG created by
//! MemPalace, `compiler.sqlite` (claim store + write log) and `<palace-dir>/wal/`.
use clap::Args;
use palace_store::{Error, claims::ClaimStore};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Args)]
pub struct Options {
    /// Diary root: the parent of `diary/`.
    pub diary_root: PathBuf,
    pub palace_dir: PathBuf,
    /// Defaults to `<palace-dir>/knowledge_graph.sqlite3`, created by MemPalace when absent.
    #[arg(long)]
    pub kg: Option<PathBuf>,
    /// The wing `mempalace mine` files the diary in (PKG-1.2 item 3).
    #[arg(long, default_value = palace_mempalace::mine::DEFAULT_WING, value_parser = crate::wing_name)]
    pub wing: String,
    #[command(flatten)]
    pub connection: crate::MemPalaceOptions,
}
pub fn run(o: Options) -> Result<(), Error> {
    let root = crate::paths::diary_root(&o.diary_root)?;
    let entries = count_entries(&root.join("diary"))?;
    let palace = crate::paths::database(&o.palace_dir)?;
    fs::create_dir_all(&palace)?;
    let palace = palace.canonicalize()?;
    let check = palace_mempalace::version::script();
    let version = ensure_pin(&o, &check, &palace, &root)?;
    let to = palace_mempalace::mine::Target {
        wing: &o.wing,
        ..Default::default()
    };
    palace_mempalace::mine::mine_to(&o.connection.mempalace, &palace, &root, &to)?;
    let kg = knowledge_graph(&o, &palace)?;
    let compiler = crate::rollup_commit::compiler_path(&kg)?;
    let mut store = ClaimStore::open(&compiler)?;
    store.log(&json!({"operation":"init","palace":palace,"kg":kg,"entries":entries}))?;
    create_private_dir(&palace.join("wal"))?;
    run_check(&check, &palace, &root, true)?;
    println!(
        "INIT OK mempalace={} entries={entries} drawers={} kg={} compiler={}",
        version.strip_prefix("MemPalace ").unwrap_or(&version),
        palace_mempalace::drawers::count(&palace)?,
        kg.display(),
        compiler.display()
    );
    Ok(())
}
/// The pin must hold before mining; if it does not, the check script's `--install` repairs it.
fn ensure_pin(o: &Options, check: &Path, palace: &Path, root: &Path) -> Result<String, Error> {
    let pinned = |allow| palace_mempalace::version::check(&o.connection.mempalace, allow);
    if let Ok(version) = pinned(false) {
        return Ok(version);
    }
    let _ = run_check(check, palace, root, false); // layout checks fail before the first mine
    pinned(o.connection.allow_mempalace_version)
}
/// `<check> --install --palace <palace> --diary <root>/diary`, with MEMPALACE_PALACE_PATH set.
fn run_check(check: &Path, palace: &Path, root: &Path, required: bool) -> Result<(), Error> {
    let mut command = Command::new(check);
    command
        .arg("--install")
        .arg("--palace")
        .arg(palace)
        .arg("--diary")
        .arg(root.join("diary"));
    palace_mempalace::mine::palace_env(&mut command, palace);
    let output = command.output()?;
    if required && !output.status.success() {
        return Err(format!(
            "mempalace-check failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout)
        )
        .into());
    }
    Ok(())
}
fn knowledge_graph(o: &Options, palace: &Path) -> Result<PathBuf, Error> {
    let default = palace.join("knowledge_graph.sqlite3");
    let kg = o.kg.clone().unwrap_or_else(|| default.clone());
    if kg.is_file() {
        return crate::paths::database(&kg);
    }
    if crate::paths::database(&kg)? != default {
        return Err("--kg must name an existing KG, or be omitted so MemPalace creates one".into());
    }
    palace_mempalace::create_kg(&o.connection.mempalace_mcp, palace)
}
fn count_entries(diary: &Path) -> Result<usize, Error> {
    let mut count = 0;
    for entry in fs::read_dir(diary)? {
        if entry?.path().extension().is_some_and(|e| e == "md") {
            count += 1;
        }
    }
    Ok(count)
}
fn create_private_dir(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::DirBuilderExt;
    if !path.is_dir() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    Ok(())
}
