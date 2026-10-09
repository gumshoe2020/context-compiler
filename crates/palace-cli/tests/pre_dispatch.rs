//! M5b-1 item 9 (review S-6): an error before dispatch exits 2 ("broken"), never 1, which
//! `watch` and `doctor --shadow-diff` use for findings and differences. Nothing is read or written:
//! the named `--roots` does not exist.
use std::process::Command;

#[test]
fn a_bad_roots_argument_exits_two_before_dispatch() {
    let missing = "/nonexistent-palace-m5b1-root";
    for args in [
        vec!["watch", "--diary", "x", "--all", "--store", "y"],
        vec![
            "doctor",
            "--shadow-diff",
            "--palace",
            "x",
            "--kg",
            "y",
            "--store",
            "z",
        ],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_palace"))
            .arg(format!("--roots={missing}"))
            .args(&args)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(
            stderr.contains(&format!("--roots {missing}")),
            "{args:?}: {stderr}"
        );
    }
}
