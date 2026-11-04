//! Offline synthetic apply/undo; the categories come from the people map, never from code.
mod common;
use palace_core::ids::sha256;
use palace_store::{
    claims::{ClaimStore, Distilled, DistilledEntry},
    privacy::{self, Audience, Marker},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
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
fn run(work: &Path, flag: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_palace"))
        .arg(format!("--roots={}", work.display()))
        .args(["privacy", "classify", flag])
        .arg(path)
        .output()
        .unwrap()
}
#[test]
fn tightens_all_derived_rows_preserves_private_and_undo_restores_bytes() {
    let work = root().join(format!("target/priv2-tagline/apply-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(work.join("diary")).unwrap();
    let store_path = work.join("compiler.sqlite");
    let kg = work.join("knowledge_graph.sqlite3");
    let people = work.join("people.json");
    fs::write(&people, common::PEOPLE).unwrap();
    let mut database = ClaimStore::open(&store_path).unwrap();
    for (name, audience) in [
        ("note", "shared"),
        ("private", "wren"),
        ("public", "shared"),
        ("kept", "wren"),
        ("theology", "shared"),
        ("no-tags", "shared"),
        ("inline-tags", "shared"),
    ] {
        let file = format!("{name}.md");
        database
            .mark_entry(&file, &audience.parse::<Audience>().unwrap().marker(&file))
            .unwrap();
        fs::write(work.join("diary").join(&file),format!("# Invented {name}\r\n\r\nTags: telescope.\r\n\r\nOrin Vale planned a cobalt observatory.\r\n")).unwrap();
    }
    fs::write(
        work.join("diary/no-tags.md"),
        "# Invented\n\nOrin built a dome.",
    )
    .unwrap();
    fs::write(
        work.join("diary/inline-tags.md"),
        "# Invented\r\n\r\nOrin used Signal. Tags: telescope, dome.\r\n",
    )
    .unwrap();
    database
        .record_distilled(
            &DistilledEntry {
                file: "note.md",
                date: "2031-06-01",
                thread: "Invented",
            },
            &[Distilled {
                id: "c_note",
                number: "D1",
                kind: "concept",
                subject: "Orin Vale",
                predicate: "plans",
                object: "cobalt observatory",
                date: "2031-06-01",
                quote: None,
                transcript: "note.md:1",
                demoted: None,
            }],
            "2031-06-01T12:00:00Z",
        )
        .unwrap();
    database
        .mark_entry("derived.md", &Audience::default().marker("derived.md"))
        .unwrap();
    database
        .record_distilled(
            &DistilledEntry {
                file: "derived.md",
                date: "2031-06-02",
                thread: "Invented derived",
            },
            &[Distilled {
                id: "c_derived",
                number: "D1",
                kind: "concept",
                subject: "Orin Vale",
                predicate: "plans",
                object: "cobalt observatory",
                date: "2031-06-01",
                quote: None,
                transcript: "note.md:1-4",
                demoted: None,
            }],
            "2031-06-02T12:00:00Z",
        )
        .unwrap();
    drop(database);
    let c = Connection::open(&store_path).unwrap();
    c.execute_batch("INSERT INTO open_items(id,person,owner,kind,title,opened_at,touched_at,source,quote) VALUES ('i_note','orin','person','task','Invented observatory','2031-06-01','2031-06-01','note.md:1-4','cobalt observatory');
        INSERT INTO fidelity(point,file,type,sampled,supported,missing,contradicted,unjudged,judge,items,at) VALUES ('consolidation','note.md:1-4','concept',1,1,0,0,0,'none','{}','2031-06-01');
        INSERT INTO asks(finding,claim,kind,token,line,asked_at) VALUES ('a_note','c_note','ungrounded','#a_note','Invented observatory','2031-06-01');").unwrap();
    for (kind, id) in [
        ("item", "i_note"),
        ("fidelity", "note.md:1-4"),
        ("ask", "a_note"),
        ("triple", "t_note"),
    ] {
        privacy::put(&c, kind, id, &Marker::shared("note.md")).unwrap();
    }
    drop(c);
    let g = Connection::open(&kg).unwrap();
    g.execute_batch("CREATE TABLE triples(id TEXT PRIMARY KEY,source_closet TEXT); INSERT INTO triples VALUES ('t_note','{\"claim_id\":\"c_note\"}');").unwrap();
    drop(g);
    let mut originals = BTreeMap::new();
    for file in [
        &store_path,
        &kg,
        &people,
        &work.join("diary/note.md"),
        &work.join("diary/private.md"),
        &work.join("diary/public.md"),
        &work.join("diary/kept.md"),
        &work.join("diary/theology.md"),
        &work.join("diary/no-tags.md"),
        &work.join("diary/inline-tags.md"),
    ] {
        originals.insert(file.clone(), fs::read(file).unwrap());
    }
    let entries: Vec<_> = ["note","private","public","kept","theology","no-tags","inline-tags"].into_iter().map(|name| {let path=work.join(format!("diary/{name}.md"));json!({"path":path,"hash":sha256(&fs::read(&path).unwrap()),"title":format!("Invented {name}"),"decision":{"category":match name {"public" => "shared", "kept" => "keep", "theology" => "audience-study", _ => "audience-household"},"reason":"Synthetic review."}})}).collect();
    let report = work.join("report.json");
    fs::write(&report,json!({"version":1,"diary":work,"store":store_path,"store_hash":sha256(&fs::read(&store_path).unwrap()),"people":people,"kg":kg,"entries":entries,"counts":{}}).to_string()).unwrap();
    // A category the people map does not hold is refused before backups or any changes.
    let good_report = fs::read(&report).unwrap();
    let mut unknown: Value = serde_json::from_slice(&good_report).unwrap();
    unknown["entries"][0]["decision"]["category"] = json!("audience-nobody");
    fs::write(&report, unknown.to_string()).unwrap();
    let refused = run(&work, "--apply", &report);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("invalid audience category"));
    fs::write(&report, good_report).unwrap();
    // Stale entry refuses before backups or any changes.
    let note = work.join("diary/note.md");
    fs::write(&note, "changed").unwrap();
    let refused = run(&work, "--apply", &report);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("note.md"));
    fs::write(&note, &originals[&note]).unwrap();
    assert_eq!(fs::read(&store_path).unwrap(), originals[&store_path]);
    let report_bytes = fs::read(&report).unwrap();
    for invalid in [
        "Tags: dome.\n  tAgS: brass.\n",
        "Tags: audience-household.\nTags: dome.\n",
        "---\ntags: dome\n---\nInvented prose.\n",
        "---\ntags:\n  - dome\n---\nInvented prose.\n",
    ] {
        fs::write(&note, invalid).unwrap();
        let mut changed: Value = serde_json::from_slice(&report_bytes).unwrap();
        changed["entries"][0]["hash"] = json!(sha256(invalid.as_bytes()));
        fs::write(&report, serde_json::to_vec(&changed).unwrap()).unwrap();
        let refused = run(&work, "--apply", &report);
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("note.md"));
        assert_eq!(fs::read(&note).unwrap(), invalid.as_bytes());
        for (file, bytes) in &originals {
            if file != &note {
                assert_eq!(&fs::read(file).unwrap(), bytes, "{}", file.display());
            }
        }
        assert!(
            !fs::read_dir(&work).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".run-"))
        );
    }
    fs::write(&note, &originals[&note]).unwrap();
    fs::write(&report, report_bytes).unwrap();
    let result = run(&work, "--apply", &report);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: Value = serde_json::from_slice(&result.stdout).unwrap();
    let undo = PathBuf::from(output["run"].as_str().unwrap());
    assert_eq!(
        fs::read(&note).unwrap(),
        String::from_utf8(originals[&note].clone())
            .unwrap()
            .replace("Tags: telescope.", "Tags: telescope, audience-household.")
            .into_bytes()
    );
    assert_eq!(
        fs::read(work.join("diary/public.md")).unwrap(),
        originals[&work.join("diary/public.md")]
    );
    for (name, suffix) in [
        ("no-tags", "\n\nTags: audience-household.\n"),
        ("inline-tags", "\r\nTags: audience-household.\r\n"),
    ] {
        let file = work.join(format!("diary/{name}.md"));
        let mut expected = originals[&file].clone();
        expected.extend_from_slice(suffix.as_bytes());
        assert_eq!(fs::read(&file).unwrap(), expected);
        assert!(
            privacy::entry_tags(&fs::read_to_string(file).unwrap()).contains("audience-household")
        );
    }
    let c = Connection::open(&store_path).unwrap();
    for (kind, id) in [
        ("entry", "note"),
        ("entry", "no-tags"),
        ("entry", "inline-tags"),
        ("claim", "c_note"),
        ("entry", "derived"),
        ("claim", "c_derived"),
        ("item", "i_note"),
        ("fidelity", "note.md:1-4"),
        ("ask", "a_note"),
        ("triple", "t_note"),
    ] {
        let ms = privacy::markers(&c, kind, id).unwrap();
        assert!(!ms.is_empty(), "{kind}/{id}");
        assert!(
            ms.iter()
                .all(|m| m.private && !m.admits(&["wren".into()]) && !m.admits(&["nella".into()])),
            "{kind}/{id}: {ms:?}"
        );
        assert!(
            ms.iter().all(|m| m.admits(&["orin".into()])),
            "{kind}/{id}: {ms:?}"
        );
    }
    assert!(
        privacy::markers(&c, "entry", "private")
            .unwrap()
            .iter()
            .all(|m| m.private && m.audience.is_empty())
    );
    let kept = privacy::markers(&c, "entry", "kept").unwrap();
    assert!(!kept.is_empty());
    assert!(
        kept.iter()
            .all(|m| m.private && m.audience == ["wren".to_owned()].into())
    );
    assert_eq!(
        fs::read(work.join("diary/kept.md")).unwrap(),
        originals[&work.join("diary/kept.md")]
    );
    let theology = privacy::markers(&c, "entry", "theology").unwrap();
    assert!(!theology.is_empty());
    assert!(theology.iter().all(|m| m.private
        && m.audience == ["orin".to_owned(), "torin".to_owned(), "nella".to_owned()].into()));
    assert!(
        fs::read_to_string(work.join("diary/theology.md"))
            .unwrap()
            .contains("Tags: telescope, audience-study.")
    );
    drop(c);
    // The categories are read, never written: people.json is untouched by apply.
    assert_eq!(fs::read(&people).unwrap(), originals[&people]);
    // Do not undo over later work; validate all targets before restoring any.
    fs::write(&note, "later work").unwrap();
    let refused = run(&work, "--undo", &undo);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("note.md"));
    let changed = String::from_utf8(originals[&note].clone())
        .unwrap()
        .replace("Tags: telescope.", "Tags: telescope, audience-household.");
    fs::write(&note, changed).unwrap();
    let result = run(&work, "--undo", &undo);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (file, bytes) in &originals {
        assert_eq!(&fs::read(file).unwrap(), bytes, "{}", file.display());
    }
    assert!(run(&work, "--undo", &undo).status.success());
}
