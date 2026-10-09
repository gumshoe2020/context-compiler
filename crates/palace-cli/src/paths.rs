use palace_store::Error;
use std::{
    fs,
    path::{Path, PathBuf},
};
/// The project root, the default `--roots`: the checkout in a development build (cargo feature
/// `doctors`) and in unit tests; in a release build the install root, the parent of the binary's own directory
/// (`$CC_HOME` for `$CC_HOME/bin/palace`), so that no build path is compiled in (PKG-1.2 item 5).
pub fn root() -> PathBuf {
    let root = project_root();
    root.canonicalize().unwrap_or(root)
}
#[cfg(any(test, feature = "doctors"))]
fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[cfg(not(any(test, feature = "doctors")))]
fn project_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}
static ROOTS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
pub fn configure(roots: Vec<PathBuf>) -> Result<(), Error> {
    let roots = if roots.is_empty() {
        vec![root()]
    } else {
        roots
    };
    let roots = roots
        .iter()
        .map(|p| canonical(p, "--roots"))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(file) = roots.iter().find(|p| !p.is_dir()) {
        return Err(format!("--roots {}: not a directory", file.display()).into());
    }
    ROOTS
        .set(roots)
        .map_err(|_| "roots already configured".into())
}
/// The effective `--roots` (default: the project root), for child runs.
pub fn roots() -> Vec<PathBuf> {
    ROOTS.get().cloned().unwrap_or_else(|| vec![root()])
}
pub fn database(path: &Path) -> Result<PathBuf, Error> {
    for allowed in ROOTS.get().cloned().unwrap_or_else(|| vec![root()]) {
        if let Ok(path) = safe_output(path, &allowed, &root().join("fixtures")) {
            return Ok(path);
        }
    }
    Err(format!(
        "{}: output must stay under --roots and outside fixtures",
        path.display()
    )
    .into())
}
/// An existing input under `--roots`; every error names the path (M4d-2a.1 item 7, review L-7).
pub fn input(path: &Path) -> Result<PathBuf, Error> {
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(format!("{}: input parent traversal forbidden", path.display()).into());
    }
    let canonical = canonical(path, "input")?;
    if roots().iter().any(|r| canonical.starts_with(r)) {
        return Ok(canonical);
    }
    Err(format!("{}: input must stay under --roots", path.display()).into())
}
/// A diary ROOT (the parent of `diary/`): passing the `diary/` directory itself, or any
/// directory without one, names the path and the fix.
pub fn diary_root(path: &Path) -> Result<PathBuf, Error> {
    let root = input(path)?;
    if root.join("diary").is_dir() {
        return Ok(root);
    }
    Err(format!(
        "--diary {}: no diary/ directory inside; pass the diary root (the parent of diary/)",
        path.display()
    )
    .into())
}
/// `path` made absolute with its nearest existing ancestor canonical (symbolic links resolved)
/// and the rest appended; nothing is created.
pub fn resolved(path: &Path) -> Result<PathBuf, Error> {
    let absolute = std::env::current_dir()?.join(path);
    let mut ancestor = absolute.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or("path has no existing ancestor")?;
    }
    let (rest, canonical) = (absolute.strip_prefix(ancestor)?, ancestor.canonicalize()?);
    if rest.as_os_str().is_empty() {
        return Ok(canonical);
    }
    Ok(canonical.join(rest))
}
fn canonical(path: &Path, what: &str) -> Result<PathBuf, Error> {
    path.canonicalize()
        .map_err(|e| format!("{what} {}: {e}", path.display()).into())
}
pub(crate) fn safe_output(path: &Path, root: &Path, fixtures: &Path) -> Result<PathBuf, Error> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    if absolute
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err("output must not contain parent traversal".into());
    }
    let mut ancestor = absolute.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or("output has no existing ancestor")?;
    }
    let canonical = ancestor.canonicalize()?;
    if !canonical.starts_with(root)
        || canonical.starts_with(fixtures)
        || canonical.starts_with(root.join("models"))
    {
        return Err("output must stay in project and outside fixtures".into());
    }
    for candidate in [
        absolute.clone(),
        absolute.with_extension("sqlite.lock"),
        PathBuf::from(format!("{}-wal", absolute.display())),
        PathBuf::from(format!("{}-shm", absolute.display())),
    ] {
        if fs::symlink_metadata(candidate).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("output symlinks are forbidden".into());
        }
    }
    fs::create_dir_all(absolute.parent().ok_or("missing output parent")?)?;
    Ok(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A checkout-like scratch root under target/ with its own fixtures/diary (PUB-1a item 3:
    /// the tests need no real fixtures/).
    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let scratch = root.join(format!("target/{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(scratch.join("fixtures/diary")).unwrap();
        let scratch = scratch.canonicalize().unwrap();
        (scratch.clone(), scratch.join("fixtures"))
    }
    #[test]
    fn rejects_fixture_writes_and_parent_traversal() {
        let (root, fixtures) = scratch("paths-fixture-writes");
        assert!(safe_output(&fixtures.join("forbidden.sqlite"), &root, &fixtures).is_err());
        assert!(safe_output(&root.join("target/../forbidden.sqlite"), &root, &fixtures).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    /// M4d-2a.1 item 7 (review L-7): a missing input and a `diary/` passed as the diary root
    /// both name the path.
    #[test]
    fn input_errors_name_the_path() {
        let missing = root().join("target/no-such-input-m4d2a1");
        let error = input(&missing).unwrap_err().to_string();
        assert!(error.contains("no-such-input-m4d2a1"), "{error}");
        let (scratch, fixtures) = scratch("paths-input-errors");
        let error = diary_root(&fixtures.join("diary")).unwrap_err().to_string();
        assert!(
            error.contains("fixtures/diary") && error.contains("parent of diary/"),
            "{error}"
        );
        assert!(diary_root(&fixtures).is_ok());
        fs::remove_dir_all(scratch).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn rejects_output_symlink_into_fixtures() {
        let (root, fixtures) = scratch("m0-symlink-test");
        let link = root.join("link");
        std::os::unix::fs::symlink(&fixtures, &link).unwrap();
        let result = safe_output(&link.join("forbidden.sqlite"), &root, &fixtures);
        fs::remove_dir_all(root).unwrap();
        assert!(result.is_err());
    }
}
