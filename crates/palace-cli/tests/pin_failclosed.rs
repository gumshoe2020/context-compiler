//! M4d-1 0b (review S-2): rollup, watch, propose-batch, init and regress exit 2 naming the path
//! when the pin script is missing, has no `PIN=` line or two of them (M4d-2a item 7), from the
//! flag or the environment; the flag wins. Scratch copies under target/;
//! scripts/mempalace-check.sh is only read.
#[path = "../src/pin_probe.rs"]
mod pin_probe;
use std::path::Path;

#[test]
fn every_pin_reading_command_fails_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root
        .join("target")
        .join(format!("pin-failclosed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    pin_probe::scratch_scripts(&root.join("scripts/mempalace-check.sh"), &work).unwrap();
    let probes = pin_probe::run_all(Path::new(env!("CARGO_BIN_EXE_palace")), &work, &[]).unwrap();
    assert_eq!(probes, 31);
    std::fs::remove_dir_all(&work).unwrap();
}
