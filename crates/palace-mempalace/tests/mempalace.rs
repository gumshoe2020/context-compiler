use palace_core::traits::DiaryReader;
use palace_mempalace::{diary::Diary, version};
use std::{fs, path::Path};

fn fake_version(dir: &Path, output: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("mempalace");
    fs::write(&path, format!("#!/bin/sh\necho '{output}'\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The repository's check script as the default, as palace names its own (`set_default_script`).
fn checkout_default() -> std::path::PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/mempalace-check.sh");
    version::set_default_script(script.canonicalize().unwrap());
    version::default_script()
}

#[test]
fn pin_is_read_from_the_real_check_script() {
    let script = checkout_default();
    assert_eq!(version::pin_from(&script).unwrap(), "3.5.0");
    assert_eq!(
        version::pin().unwrap(),
        "3.5.0",
        "default = the repository's script"
    );
    let missing = version::pin_from(Path::new("/nonexistent/mempalace-check.sh")).unwrap_err();
    assert!(
        missing.to_string().contains("--mempalace-check"),
        "{missing}"
    );
}

#[test]
fn a_mismatched_script_names_the_scripts_pin() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("mempalace-check.sh");
    fs::write(&script, "#!/bin/sh\nset -u\nPIN=\"3.6.0\"\n").unwrap();
    let cli = fake_version(dir.path(), "MemPalace 3.5.0");
    let pin = version::pin_from(&script).unwrap();
    let error = version::check_against(&cli, &pin, false)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("MemPalace 3.6.0") && error.contains("3.5.0"),
        "{error}"
    );
    fs::write(&script, "#!/bin/sh\n# no pin\n").unwrap();
    assert!(
        version::pin_from(&script)
            .unwrap_err()
            .to_string()
            .contains("no PIN= line")
    );
}

#[test]
fn version_pin_is_exact_unless_allowed() {
    checkout_default();
    let dir = tempfile::tempdir().unwrap();
    let pinned = fake_version(dir.path(), "MemPalace 3.5.0");
    assert_eq!(version::check(&pinned, false).unwrap(), "MemPalace 3.5.0");
    let other = fake_version(dir.path(), "MemPalace 3.5.1");
    assert!(version::check(&other, false).is_err());
    assert_eq!(version::check(&other, true).unwrap(), "MemPalace 3.5.1");
    assert!(version::check(&dir.path().join("missing"), true).is_err());
}

#[test]
fn diary_header_is_first_800_chars_inside_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("diary");
    fs::create_dir(&root).unwrap();
    let entry = root.join("2026-09-24-abc.md");
    fs::write(&entry, format!("# Title\n{}", "é".repeat(2000))).unwrap();
    let diary = Diary::open(&root).unwrap();
    let canonical = entry.canonicalize().unwrap();
    let header = diary.header(canonical.to_str().unwrap()).unwrap();
    assert_eq!(header.chars().count(), 800);
    assert!(header.starts_with("# Title\n"));
}

#[test]
fn diary_rejects_paths_outside_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("diary");
    fs::create_dir(&root).unwrap();
    let outside = dir.path().join("secret.md");
    fs::write(&outside, "# secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link.md")).unwrap();
    let diary = Diary::open(&root).unwrap();
    let root = root.canonicalize().unwrap();
    for source in [
        outside.canonicalize().unwrap().display().to_string(),
        format!("{}/../secret.md", root.display()),
        root.join("link.md").display().to_string(),
        "relative.md".to_owned(),
    ] {
        assert!(diary.header(&source).is_err(), "{source} was opened");
    }
}

#[test]
fn mid_entry_bookkeeping_header_and_outside_fallback() {
    use palace_core::{
        evidence::is_bookkeeping,
        search_hit::SearchHit,
        traits::{EvidenceSearch, Searcher},
    };
    struct Hit(String);
    impl Searcher for Hit {
        type Error = palace_mempalace::Error;
        fn search(&self, _: &str, _: usize, _: f64) -> Result<Vec<SearchHit>, Self::Error> {
            Ok(vec![serde_json::from_value(serde_json::json!({"text":"ordinary mid-entry chunk", "source_path":self.0,"source_file":"entry.md","wing":"diary","room":"notes"})).unwrap()])
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("entry.md");
    fs::write(&entry, "# Audit\nTags: kg-bookkeeping.\nbody").unwrap();
    let evidence = palace_mempalace::Evidence {
        searcher: Hit(entry.canonicalize().unwrap().display().to_string()),
        diary: Diary::open(dir.path()).unwrap(),
    };
    assert!(is_bookkeeping(&evidence.search_headers("q", 3).unwrap()[0]));
    assert_eq!(evidence.diary.header_counts(), (1, 0));
    let evidence = palace_mempalace::Evidence {
        searcher: Hit("/outside/entry.md".into()),
        diary: evidence.diary,
    };
    assert!(!is_bookkeeping(
        &evidence.search_headers("q", 3).unwrap()[0]
    ));
    assert_eq!(evidence.diary.header_counts(), (1, 1));
}
