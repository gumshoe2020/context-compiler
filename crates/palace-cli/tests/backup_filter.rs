//! Invented content, independent staged databases only.
use palace_store::{
    claims::ClaimStore,
    privacy::{self, Audience},
};
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn run(work: &Path, store: &Path, kg: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_palace"))
        .arg(format!("--roots={}", work.display()))
        .args([
            "--person",
            "ann",
            "--assistant",
            "finch",
            "privacy",
            "backup-filter",
        ])
        .arg("--store")
        .arg(store)
        .arg("--kg")
        .arg(kg)
        .arg("--diary")
        .arg(work.join("root"))
        .args(["--exclude-person", "wren"])
        .output()
        .unwrap()
}
fn setup(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let work = root().join(format!(
        "target/settle1k-m3/backup-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("root/diary")).unwrap();
    let store = work.join("compiler.sqlite");
    drop(ClaimStore::open(&store).unwrap());
    let kg = work.join("kg.sqlite");
    let c = Connection::open(&kg).unwrap();
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,object TEXT,source_closet TEXT);").unwrap();
    drop(c);
    (work, store, kg)
}

#[test]
fn filters_exact_audience_supports_and_duplicate_payloads_without_changing_sources() {
    let (work, store, kg) = setup("filter");
    let c = Connection::open(&store).unwrap();
    let g = Connection::open(&kg).unwrap();
    for (id, audience) in [
        ("shared", "shared"),
        ("secret", "wren"),
        ("group", "ann,wren"),
        ("other", "cy"),
        ("mixed", "wren"),
        ("retired", "wren"),
    ] {
        let marker = audience.parse::<Audience>().unwrap().marker(id);
        let entry = format!("2031-05-10-{id}");
        fs::write(
            work.join(format!("root/diary/{entry}.md")),
            format!("# Invented entry\n{id} copper narwhal"),
        )
        .unwrap();
        for (kind, key) in [
            ("claim", id),
            ("triple", id),
            ("item", id),
            ("fidelity", entry.as_str()),
            ("entry", entry.as_str()),
        ] {
            privacy::put(&c, kind, key, &marker).unwrap();
        }
        privacy::register(&c, &entry, &marker).unwrap();
        c.execute("INSERT INTO claims(id,number,rollup,subject,predicate,object,date,source,confidence_base,first_seen) VALUES (?1,'C1',?2,'Invented Finch','chose',?3,'2031-05-10',?2,0.8,'2031-05-10')", params![id,entry,format!("{id} copper narwhal")]).unwrap();
        c.execute("INSERT INTO fidelity(point,file,type,sampled,supported,missing,contradicted,unjudged,judge,items,at) VALUES ('consolidation',?1,'concept',1,1,0,0,0,'none',?2,'2031-05-10')", params![entry,format!("{id} copper narwhal")]).unwrap();
        c.execute("INSERT INTO open_items(id,person,owner,kind,title,opened_at,touched_at,source,quote) VALUES (?1,'ann','person','task',?2,'2031-05-10','2031-05-10',?1,?2)", params![id,format!("{id} copper narwhal")]).unwrap();
        g.execute(
            "INSERT INTO entities VALUES (?1,?2)",
            params![id, format!("{id} copper narwhal")],
        )
        .unwrap();
        g.execute(
            "INSERT INTO triples VALUES (?1,?1,?1,?2)",
            params![id, format!("{id} copper narwhal")],
        )
        .unwrap();
    }
    privacy::put(
        &c,
        "triple",
        "mixed",
        &Audience::default().marker("independent"),
    )
    .unwrap();
    c.execute(
        "INSERT INTO privacy_retired VALUES ('triple','retired','retired')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO write_log VALUES (1,'secret copper narwhal','hash','hash')",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO findings(id,claim,rollup,kind,evidence,checker,checked_at) VALUES ('finding','secret','secret','missing','secret copper narwhal','rule','2031-05-10')",[]).unwrap();
    drop(c);
    drop(g);
    // Keep independent source copies to prove filtering never follows a live/source path.
    let original_store = work.join("source-store.sqlite");
    let original_kg = work.join("source-kg.sqlite");
    fs::copy(&store, &original_store).unwrap();
    fs::copy(&kg, &original_kg).unwrap();
    let before = fs::read(&original_store).unwrap();
    let before_kg = fs::read(&original_kg).unwrap();
    let diary_before: Vec<_> = fs::read_dir(work.join("root/diary"))
        .unwrap()
        .map(|p| {
            let p = p.unwrap().path();
            (p.clone(), fs::read(p).unwrap())
        })
        .collect();
    let output = run(&work, &store, &kg);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let excluded = String::from_utf8(output.stdout).unwrap();
    assert_eq!(excluded.lines().count(), 4);
    for id in ["secret", "group", "mixed", "retired"] {
        assert!(excluded.contains(&format!("2031-05-10-{id}.md")));
    }
    let c = Connection::open(&store).unwrap();
    let g = Connection::open(&kg).unwrap();
    for table in ["claims", "open_items", "fidelity"] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    assert_eq!(
        g.query_row("SELECT count(*) FROM triples", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        g.query_row("SELECT count(*) FROM entities", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM findings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM write_log", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(c);
    drop(g);
    for path in [&store, &kg] {
        let bytes = fs::read(path).unwrap();
        assert!(
            !bytes
                .windows(b"secret copper narwhal".len())
                .any(|b| b == b"secret copper narwhal")
        );
    }
    assert_eq!(fs::read(original_store).unwrap(), before);
    assert_eq!(fs::read(original_kg).unwrap(), before_kg);
    for (path, bytes) in diary_before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(run(&work, &store, &kg).status.success(), "idempotent rerun");
}

#[test]
fn refuses_live_paths_aliases_hardlinks_and_sidecars_before_mutation() {
    let (work, store, kg) = setup("refusal");
    let before = fs::read(&store).unwrap();
    for dir in [
        "palace-settle",
        "palace-shadow",
        ".openclaw/palace-custom",
        ".mempalace",
        "plugins",
    ] {
        let live = work.join(dir);
        fs::create_dir_all(&live).unwrap();
        let path = live.join("compiler.sqlite");
        fs::copy(&store, &path).unwrap();
        assert!(!run(&work, &path, &kg).status.success());
        assert_eq!(fs::read(&path).unwrap(), before);
        let alias = work.join("alias.sqlite");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(!run(&work, &alias, &kg).status.success());
        fs::remove_file(alias).unwrap();
    }
    let hard = work.join("hard.sqlite");
    fs::hard_link(&store, &hard).unwrap();
    assert!(!run(&work, &hard, &kg).status.success());
    fs::remove_file(hard).unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", store.display()));
        fs::write(&sidecar, "invented").unwrap();
        assert!(!run(&work, &store, &kg).status.success());
        fs::remove_file(sidecar).unwrap();
        assert!(fs::read(&store).unwrap() == before, "store bytes changed");
    }
}

#[test]
fn corrupt_metadata_rolls_back_both_databases_and_emits_no_paths() {
    let (work, store, kg) = setup("rollback");
    let c = Connection::open(&store).unwrap();
    c.execute(
        "INSERT INTO privacy_sources VALUES ('invented','not json')",
        [],
    )
    .unwrap();
    drop(c);
    let before = fs::read(&store).unwrap();
    let before_kg = fs::read(&kg).unwrap();
    let output = run(&work, &store, &kg);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let c = Connection::open(&store).unwrap();
    assert_eq!(
        c.query_row("SELECT marker FROM privacy_sources", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "not json"
    );
    drop(c);
    assert!(fs::read(&store).unwrap() == before, "store bytes changed");
    assert!(fs::read(&kg).unwrap() == before_kg, "graph bytes changed");
}
