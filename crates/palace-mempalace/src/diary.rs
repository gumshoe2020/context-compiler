use crate::Error;
use palace_core::traits::DiaryReader;
use std::{
    cell::Cell,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};
/// A path outside the diary root (`..`, or a symlink out of it): a refused input, not a broken
/// run (M4d-2d item 4, review L-2). Callers map it by type, never by message.
#[derive(Debug)]
pub struct OutsideRoot(&'static str);
impl std::fmt::Display for OutsideRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for OutsideRoot {}
impl OutsideRoot {
    pub fn is(error: &Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}

pub struct Diary {
    root: PathBuf,
    reads: Cell<usize>,
    fallbacks: Cell<usize>,
}
impl Diary {
    pub fn open(root: &Path) -> Result<Self, Error> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("diary {}: {e}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("diary {}: not a directory", root.display()).into());
        }
        Ok(Self {
            root,
            reads: Cell::new(0),
            fallbacks: Cell::new(0),
        })
    }
    pub fn header_counts(&self) -> (usize, usize) {
        (self.reads.get(), self.fallbacks.get())
    }
    pub fn report_headers(&self) {
        let (reads, fallbacks) = self.header_counts();
        eprintln!("evidence headers: reads={reads} fallbacks={fallbacks}");
        if reads == 0 && fallbacks > 0 {
            eprintln!("warning: every evidence hit fell back to chunk text; check --diary root");
        }
    }
    fn read_header(&self, source_path: &str) -> Result<String, Error> {
        self.read_header_inner(source_path)
    }
    fn confined(&self, source: &str) -> Result<PathBuf, Error> {
        let path = Path::new(source);
        if !path.is_absolute()
            || !path.starts_with(&self.root)
            || path.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err(Box::new(OutsideRoot("source_path outside diary root")));
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(&self.root) || !canonical.is_file() {
            return Err(Box::new(OutsideRoot("source_path escapes diary root")));
        }
        Ok(canonical)
    }
}
impl Diary {
    fn read_header_inner(&self, source_path: &str) -> Result<String, Error> {
        let path = self.confined(source_path)?;
        let mut bytes = Vec::new();
        File::open(path)?.take(3200).read_to_end(&mut bytes)?;
        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(e) if e.error_len().is_none() => std::str::from_utf8(&bytes[..e.valid_up_to()])?,
            Err(e) => return Err(e.into()),
        };
        Ok(text.chars().take(800).collect())
    }
}

impl DiaryReader for Diary {
    type Error = Error;
    fn names(&self) -> Result<Vec<String>, Error> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if let Some(name) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".md"))
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }
    fn read(&self, name: &str) -> Result<Option<String>, Error> {
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err("diary filename must be a basename".into());
        }
        let name = if name.ends_with(".md") {
            name.to_owned()
        } else {
            format!("{name}.md")
        };
        let path = self.root.join(name);
        if std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(None);
        }
        let path = self.confined(path.to_str().ok_or("non-UTF8 diary path")?)?;
        Ok(Some(std::fs::read_to_string(path)?))
    }

    fn header(&self, path: &str) -> Result<String, Error> {
        let result = self.read_header(path);
        if result.is_ok() {
            self.reads.set(self.reads.get() + 1);
        } else {
            self.fallbacks.set(self.fallbacks.get() + 1);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_symlink_out_of_the_root_is_outside_it() {
        let dir = tempfile::tempdir().unwrap();
        let (root, outside) = (dir.path().join("diary"), dir.path().join("secret.md"));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(&outside, "# secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.md")).unwrap();
        std::fs::write(root.join("inside.md"), "# inside").unwrap();
        let diary = Diary::open(&root).unwrap();
        let escaped = diary.read("link").unwrap_err();
        assert!(OutsideRoot::is(&escaped), "{escaped}");
        assert_eq!(escaped.to_string(), "source_path escapes diary root");
        assert_eq!(diary.read("inside").unwrap().as_deref(), Some("# inside"));
    }
}
