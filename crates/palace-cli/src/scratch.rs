//! A scratch directory this process creates, copies a palace into, and removes when dropped
//! (unless kept). `palace regress` deletes rows only inside such a copy (M4c review S-1).
use palace_store::Error;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Scratch {
    dir: PathBuf,
    keep: bool,
}

impl Scratch {
    /// Creates `requested` (which must not exist yet), or `target/regress-<pid>-<ns>/`.
    pub fn create(requested: Option<&Path>, keep: bool) -> Result<Self, Error> {
        let dir = match requested {
            Some(path) => crate::paths::database(path)?,
            None => default_dir()?,
        };
        fs::create_dir_all(dir.parent().ok_or("scratch dir has no parent")?)?;
        fs::create_dir(&dir).map_err(|e| format!("scratch {}: {e}", dir.display()))?;
        Ok(Self { dir, keep })
    }

    /// Copies the palace at `source` to `<scratch>/palace` (owner-writable) and returns it.
    pub fn copy_palace(&self, source: &Path) -> Result<PathBuf, Error> {
        if !source.join("chroma.sqlite3").is_file() {
            return Err(format!("not a palace (no chroma.sqlite3): {}", source.display()).into());
        }
        let copy = self.dir.join("palace");
        copy_tree(source, &copy)?;
        Ok(copy.canonicalize()?)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.keep {
            eprintln!("kept scratch: {}", self.dir.display());
        } else if let Err(error) = fs::remove_dir_all(&self.dir) {
            eprintln!("scratch {} not removed: {error}", self.dir.display());
        }
    }
}

fn default_dir() -> Result<PathBuf, Error> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(crate::paths::root().join(format!("target/regress-{}-{nanos}", std::process::id())))
}

/// `cp -R` then `chmod -R u+w`; symlinks are refused rather than followed.
pub fn copy_tree(source: &Path, destination: &Path) -> Result<(), Error> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_symlink() {
            return Err(format!("palace copy: symlink {}", entry.path().display()).into());
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
            add_owner_write(&target)?;
        }
    }
    add_owner_write(destination)
}

fn add_owner_write(path: &Path) -> Result<(), Error> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o200);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

/// sha256 over every file's relative path, mode and content bytes, in path order.
#[cfg(doctors)]
pub fn tree_digest(root: &Path) -> Result<String, Error> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries)?;
    entries.sort();
    Ok(palace_core::ids::sha256(entries.join("\n").as_bytes()))
}

/// `tree_digest`'s entries (relative path, mode, content digest), sorted, without the file at
/// `skip` (a path relative to `root`).
#[cfg(doctors)]
pub fn tree_entries_without(root: &Path, skip: &str) -> Result<Vec<String>, Error> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries)?;
    entries.retain(|e| !e.starts_with(&format!("{skip} ")));
    entries.sort();
    Ok(entries)
}

/// `tree_digest` plus the modification time of the root and of every file and directory below
/// it (M5b-1): equal only when nothing was written, created, removed or touched.
#[cfg(doctors)]
pub fn tree_stamp(root: &Path) -> Result<String, Error> {
    let mut entries = vec![format!(". {:?}", fs::metadata(root)?.modified()?)];
    collect_with(root, root, &mut entries, true)?;
    entries.sort();
    Ok(palace_core::ids::sha256(entries.join("\n").as_bytes()))
}

#[cfg(doctors)]
fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    collect_with(root, dir, out, false)
}

#[cfg(doctors)]
fn collect_with(root: &Path, dir: &Path, out: &mut Vec<String>, times: bool) -> Result<(), Error> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        let relative = path.strip_prefix(root)?.display().to_string();
        let mode = meta.permissions().mode();
        let time = if times {
            format!(" {:?}", meta.modified()?)
        } else {
            String::new()
        };
        if meta.is_dir() {
            out.push(format!("{relative}/ {mode:o}{time}"));
            collect_with(root, &path, out, times)?;
        } else {
            let digest = palace_core::ids::sha256(&fs::read(&path)?);
            out.push(format!("{relative} {mode:o} {digest}{time}"));
        }
    }
    Ok(())
}
