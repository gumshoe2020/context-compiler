use std::{fs, path::PathBuf, process::Command};

#[test]
fn synthetic_rollup_selection() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root.join("target/rollup-pick/cargo-acceptance");
    fs::create_dir_all(&work).unwrap();
    let output = Command::new("python3")
        .arg(root.join("testdata/rollup-pick/checks.py"))
        .arg(&root)
        .arg(&work)
        .arg(env!("CARGO_BIN_EXE_palace"))
        .stdout(fs::File::create(work.join("checks.log")).unwrap())
        .stderr(fs::File::create(work.join("errors.log")).unwrap())
        .status()
        .unwrap();
    assert!(
        output.success(),
        "see target/rollup-pick/cargo-acceptance logs"
    );
}
