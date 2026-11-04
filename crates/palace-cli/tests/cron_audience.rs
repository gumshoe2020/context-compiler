//! Cron routing with a repository-local stand-in; no models, delivery or live processes.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
#[test]
fn cron_passes_audience_only_when_requested() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let work = root.join(format!("target/settle1k-m3/cron-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("diary-root/diary")).unwrap();
    fs::create_dir_all(work.join("palace")).unwrap();
    fs::write(work.join("kg.sqlite"), "invented staged graph").unwrap();
    fs::write(work.join("canaries.json"), "[]").unwrap();
    let stub = work.join("palace-stub.py");
    fs::write(&stub, "#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ['CRON_CAPTURE'],'a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\n").unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
    for audience in [None, Some("each")] {
        let capture = work.join("args.jsonl");
        let _ = fs::remove_file(&capture);
        let mut cmd = Command::new("sh");
        cmd.arg(root.join("scripts/palace-cron.sh"))
            .args(["--rollup", "--no-deliver-stages", "--llm", "replay"])
            .arg("--palace")
            .arg(work.join("palace"))
            .arg("--diary-root")
            .arg(work.join("diary-root"))
            .arg("--kg")
            .arg(work.join("kg.sqlite"))
            .arg("--state")
            .arg(work.join("state"))
            .arg("--palace-bin")
            .arg(&stub)
            .arg("--send")
            .arg(&stub)
            .arg("--page")
            .arg(&stub)
            .arg("--mempalace-check")
            .arg(&stub)
            .arg("--canaries")
            .arg(work.join("canaries.json"))
            .env("CRON_CAPTURE", &capture);
        if let Some(value) = audience {
            cmd.args(["--audience", value]);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let calls: Vec<Vec<String>> = fs::read_to_string(capture)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let rollup = calls
            .iter()
            .find(|args| args.iter().any(|s| s == "rollup"))
            .unwrap();
        let flag = rollup.iter().position(|s| s == "--audience");
        if let Some(value) = audience {
            assert_eq!(rollup[flag.unwrap() + 1], value);
        } else {
            assert!(flag.is_none());
        }
    }
}
