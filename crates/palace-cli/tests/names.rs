//! PKG-1.2 item 4: `--person` and `--assistant` name the two roles; without them the owner's
//! names hold as before, and in a build without them (PKG-1.3 item 4, the release build; PUB-1a
//! item 2, a build without owner names) a command that may record a role is refused, naming the
//! flags. An empty claim store under target/; nothing else is read or written.
use std::{path::PathBuf, process::Command};

fn store() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.canonicalize().unwrap().join("target/names-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("compiler.sqlite");
    drop(palace_store::claims::ClaimStore::open(&store).unwrap());
    store
}

fn findings(names: &[&str], queue: &str, store: &PathBuf) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_palace"))
        .args(names)
        .args(["findings", "--queue", queue, "--store"])
        .arg(store)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The owner's names this binary falls back to: with the cargo feature `owner-defaults` and
/// owner names compiled in (`palace_core::names::OWNER`), else none.
fn defaults() -> Option<(&'static str, &'static str)> {
    palace_core::names::OWNER.filter(|_| cfg!(feature = "owner-defaults"))
}

#[test]
fn the_queue_is_the_assistants() {
    let Some((_, owner_assistant)) = defaults() else {
        return; // a build without defaults: a_build_without_defaults_needs_the_names
    };
    let store = store();
    assert_eq!(findings(&[], owner_assistant, &store).0, Some(0));
    let (code, stderr) = findings(&[], "codex", &store);
    assert_eq!(code, Some(2), "{stderr}");
    let others = ["--person", "maria", "--assistant", "codex"];
    assert_eq!(findings(&others, "codex", &store).0, Some(0));
    let (code, stderr) = findings(&others, owner_assistant, &store);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("the only queue is codex's"), "{stderr}");
}

#[test]
fn a_bad_name_exits_two_before_dispatch() {
    for names in [["--person", "Maria"], ["--assistant", "rule:x"]] {
        let (code, stderr) = findings(&names, "fergus", &PathBuf::from("/nonexistent"));
        assert_eq!(code, Some(2), "{names:?}: {stderr}");
        assert!(stderr.contains("palace: name "), "{stderr}");
    }
}

#[test]
fn a_build_without_defaults_needs_the_names() {
    if defaults().is_some() {
        return; // the development build: the_queue_is_the_assistants
    }
    let store = store();
    let (code, stderr) = findings(&[], "codex", &store);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("--person <name> --assistant <name>"),
        "{stderr}"
    );
    for owner in palace_core::names::OWNER.map_or(vec![], |(p, a)| vec![p, a]) {
        assert!(!stderr.contains(owner), "{stderr}");
    }
    let (code, stderr) = findings(&["--person", "maria"], "codex", &store);
    assert_eq!(code, Some(2), "{stderr}");
    let others = ["--person", "maria", "--assistant", "codex"];
    assert_eq!(findings(&others, "codex", &store).0, Some(0));
}
