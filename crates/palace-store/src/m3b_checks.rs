//! Executable store reference cases, shared with the M3b doctor.
use crate::{Error, graph::Graph};
use palace_core::{
    retire::{FIXTURE_NAMES, FIXTURE_REASON, retire_fixtures},
    traits::{GraphMutation, GraphStore},
    traverse::{Options, traverse},
    triple::Direction,
};
use serde_json::json;
fn require(ok: bool) -> Result<(), Error> {
    if ok {
        Ok(())
    } else {
        Err("M3b store reference assertion failed".into())
    }
}
pub fn refuses_live(g: &mut Graph) -> Result<(), Error> {
    g.call(
        "add",
        &json!({"subject":"luma","predicate":"uses","object":"rust"}),
    )?;
    let result = g.retire_entity("luma", "test", "tester")?;
    require(
        result["refused"] == true
            && result["live_triples"][0]["object"] == "rust"
            && g.entity("luma")?.unwrap().properties.is_empty(),
    )
}
pub fn orphan(g: &mut Graph) -> Result<(), Error> {
    g.call(
        "add",
        &json!({"subject":"fixture","predicate":"probe","object":"other"}),
    )?;
    g.call(
        "set_entity",
        &json!({"name":"fixture","properties":{"note":"keep me"}}),
    )?;
    g.call(
        "invalidate",
        &json!({"subject":"fixture","predicate":"probe","object":"other","ended":"2026-08-19"}),
    )?;
    let first = g.call(
        "retire_entity",
        &json!({"name":"fixture","reason":"done","by":"fergus","at":"2026-09-07T12:00:00Z"}),
    )?;
    let second = g.retire_entity("fixture", "done again", "fergus")?;
    require(
        first["changed"] == true
            && first["entity"]["properties"]
                == json!({"note":"keep me","retired":{"at":"2026-09-07T12:00:00Z","by":"fergus","reason":"done"}})
            && second["changed"] == false
            && second["entity"]["properties"]["retired"]["reason"] == "done",
    )?;
    let facts = g.query_entity("fixture", Direction::Outgoing, None)?;
    require(
        facts.len() == 1
            && facts[0].valid_to.as_deref() == Some("2026-08-19")
            && g.entity("other")?.is_some(),
    )?;
    require(
        g.retire_entity("never-existed", "x", "y")?
            == json!({"retired":false,"missing":true,"name":"never-existed"})
            && g.entity("never-existed")?.is_none(),
    )
}
pub fn fixtures(g: &mut Graph) -> Result<(), Error> {
    for name in FIXTURE_NAMES {
        g.call("set_entity", &json!({"name":name}))?;
    }
    let first = retire_fixtures(g, "fergus")?;
    let second = retire_fixtures(g, "fergus")?;
    require(
        first["changed"] == 5
            && second["changed"] == 0
            && second["outcomes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["status"] == "already-retired"),
    )?;
    for name in FIXTURE_NAMES {
        require(g.entity(name)?.unwrap().properties["retired"]["reason"] == FIXTURE_REASON)?;
    }
    require(
        traverse(g, "pgs-m5-cli", Options::default())?
            .entities
            .is_empty(),
    )?;
    let shown = traverse(
        g,
        "pgs-m5-cli",
        Options {
            include_retired: true,
            ..Default::default()
        },
    )?;
    require(
        shown.entities == ["pgs-m5-cli"] && shown.retired["pgs-m5-cli"]["reason"] == FIXTURE_REASON,
    )
}
pub fn mixed(g: &mut Graph) -> Result<(), Error> {
    for name in FIXTURE_NAMES.into_iter().take(4) {
        g.call("set_entity", &json!({"name":name}))?;
    }
    g.call(
        "add",
        &json!({"subject":"pgs-m5-cli","predicate":"p","object":"o"}),
    )?;
    let result = retire_fixtures(g, "fergus")?;
    require(
        result["changed"] == 3
            && result["outcomes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["status"].as_str().unwrap())
                .collect::<Vec<_>>()
                == ["retired", "retired", "refused", "retired", "missing"],
    )
}
pub fn roundtrip(g: &mut Graph) -> Result<(), Error> {
    g.call("add",&json!({"subject":"roundtrip","predicate":"p","object":"destination","confidence":0.42,"source_closet":"{\"confidence\":0.8}","valid_from":"2026-01-01"}))?;
    let rows = g.query_entity("roundtrip", Direction::Outgoing, None)?;
    require(
        rows.len() == 1
            && rows[0].confidence == Some(0.42)
            && rows[0].valid_from.as_deref() == Some("2026-01-01")
            && rows[0].source_closet.as_deref() == Some("{\"confidence\":0.8}"),
    )?;
    g.call(
        "set_entity",
        &json!({"name":"orphan-roundtrip","properties":{"retired":{"reason":"fixture"}}}),
    )?;
    require(
        g.list_entities()?
            .iter()
            .any(|e| e.name == "orphan-roundtrip" && e.retired().is_some()),
    )
}
pub type Case = fn(&mut Graph) -> Result<(), Error>;
pub const CASES: [Case; 5] = [refuses_live, orphan, fixtures, mixed, roundtrip];
/// Only for isolated acceptance databases, never used by the graph open path.
pub fn empty_fixture(path: &std::path::Path) -> Result<(), Error> {
    let c = rusqlite::Connection::open(path)?;
    c.execute_batch("CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT NOT NULL,predicate TEXT NOT NULL,object TEXT NOT NULL,valid_from TEXT,valid_to TEXT,confidence REAL DEFAULT 1.0,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP,FOREIGN KEY(subject) REFERENCES entities(id),FOREIGN KEY(object) REFERENCES entities(id));")?;
    Ok(())
}
