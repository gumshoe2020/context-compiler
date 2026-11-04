use std::{fs, path::PathBuf, process::Command};
#[test]
fn relation_evidence_acceptance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let out = root.join(format!("target/kgev1/cargo-{}", std::process::id()));
    fs::create_dir_all(&out).unwrap();
    let status = Command::new("python3")
        .arg(root.join("testdata/privacy/kg_evidence.py"))
        .arg(&root)
        .arg(&out)
        .arg(env!("CARGO_BIN_EXE_palace"))
        .stdout(fs::File::create(out.join("checks.log")).unwrap())
        .stderr(fs::File::create(out.join("errors.log")).unwrap())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "KG-EV1 acceptance failed: {}",
        out.display()
    );
}

#[test]
fn privacy_canary_acceptance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let out = root.join(format!("target/priv1/canary-{}", std::process::id()));
    fs::create_dir_all(&out).unwrap();
    let status = Command::new("python3")
        .arg(root.join("testdata/privacy/checks.py"))
        .arg(&root)
        .arg(&out)
        .arg(env!("CARGO_BIN_EXE_palace"))
        .stdout(fs::File::create(out.join("checks.log")).unwrap())
        .stderr(fs::File::create(out.join("errors.log")).unwrap())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "PRIV-1 acceptance failed: {}",
        out.display()
    );
}

#[test]
fn legacy_wing_backfill_acceptance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let out = root.join(format!("target/priv1b/legacy-{}", std::process::id()));
    fs::create_dir_all(&out).unwrap();
    let status = Command::new("python3")
        .arg(root.join("testdata/privacy/legacy_backfill.py"))
        .arg(&root)
        .arg(&out)
        .arg(env!("CARGO_BIN_EXE_palace"))
        .stdout(fs::File::create(out.join("checks.log")).unwrap())
        .stderr(fs::File::create(out.join("errors.log")).unwrap())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "PRIV-1b acceptance failed: {}",
        out.display()
    );
}

#[test]
fn real_shaped_recall_acceptance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let out = root.join(format!("target/priv1c/recall-{}", std::process::id()));
    fs::create_dir_all(&out).unwrap();
    let status = Command::new("python3")
        .arg(root.join("testdata/privacy/recall.py"))
        .arg(&root)
        .arg(&out)
        .arg(env!("CARGO_BIN_EXE_palace"))
        .stdout(fs::File::create(out.join("checks.log")).unwrap())
        .stderr(fs::File::create(out.join("errors.log")).unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "PRIV-1c recall failed: {}", out.display());
}

#[test]
fn sticky_tags_acceptance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let out = root.join(format!("target/priv1c/tags-{}", std::process::id()));
    fs::create_dir_all(&out).unwrap();
    let status = Command::new("python3")
        .arg(root.join("testdata/privacy/sticky_tags.py"))
        .arg(&root)
        .arg(&out)
        .arg(env!("CARGO_BIN_EXE_palace"))
        .stdout(fs::File::create(out.join("checks.log")).unwrap())
        .stderr(fs::File::create(out.join("errors.log")).unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "PRIV-1c tags failed: {}", out.display());
}
