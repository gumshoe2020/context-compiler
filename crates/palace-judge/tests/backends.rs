use palace_core::{canary::Question, traits::Judge};
use palace_judge::{
    command::{CommandJudge, CommandModel},
    replay::Replay,
};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn question() -> Question {
    Question {
        rollup: "r".into(),
        id: "c".into(),
        statement: "test statement".into(),
        passage: "é".repeat(13000),
    }
}
fn command() -> CommandJudge {
    CommandJudge(CommandModel {
        executable: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command.py"),
        args: vec![],
        timeout: Duration::from_secs(2),
        cwd: None,
        fresh_cwd: false,
    })
}
#[test]
fn stdin_and_unicode_passage_cap() {
    assert_eq!(
        command().judge(&question()).unwrap()["quote"],
        "stdin received"
    );
}
#[test]
fn timeout_and_nonzero_exit() {
    let mut command = command();
    command.0.args = vec!["--sleep".into()];
    command.0.timeout = Duration::from_millis(50);
    let start = Instant::now();
    assert!(
        command
            .judge(&question())
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    command.0.args = vec!["--fail".into()];
    command.0.timeout = Duration::from_secs(2);
    assert!(
        command
            .judge(&question())
            .unwrap_err()
            .to_string()
            .contains("exited")
    );
}
#[test]
fn replay_requires_both_keys_and_rejects_conflicts() {
    let mut replay = Replay::default();
    let mut report = json!({"results":[{"check":"canary","rollup":"r","canary_id":"c","verdict":"reversed","quote":"direction"}]});
    replay.add(&report).unwrap();
    assert_eq!(replay.judge(&question()).unwrap()["verdict"], "reversed");
    let mut q = question();
    q.id = "other".into();
    assert!(replay.judge(&q).is_err());
    q = question();
    q.rollup = "other".into();
    assert!(replay.judge(&q).is_err());
    report["results"][0]["verdict"] = json!("preserved");
    assert!(replay.add(&report).is_err());
}
