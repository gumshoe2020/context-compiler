//! In-process equivalents of the three Python bridge regression cases.
use crate::{Error, graph::Graph};
use serde_json::json;
fn check(ok: bool) -> Result<(), Error> {
    if ok {
        Ok(())
    } else {
        Err("ported graph assertion failed".into())
    }
}
pub fn confidence_types(g: &mut Graph) -> Result<(), Error> {
    let value=g.call("add",&json!({"subject":"bridge-callum","predicate":"works_on","object":"bridge-luma","confidence":0.42,"from_type":"person","to_type":"project","source_closet":"{\"written_via\":\"palace-rs\"}"}))?;
    check(
        value["created"] == true
            && value["confidence"] == 0.42
            && value["subject"]["type"] == "person"
            && value["object"]["type"] == "project",
    )?;
    let rows = g.call(
        "query",
        &json!({"entity":"bridge-callum","direction":"outgoing"}),
    )?;
    check(
        rows["facts"][0]["confidence"] == 0.42
            && rows["facts"][0]["source_closet"] == "{\"written_via\":\"palace-rs\"}",
    )
}
pub fn omitted_types(g: &mut Graph) -> Result<(), Error> {
    g.call(
        "set_entity",
        &json!({"name":"bridge-preserve","type":"project","properties":{"aliases":["lumina"]}}),
    )?;
    g.call("add",&json!({"subject":"bridge-owner","predicate":"works_on","object":"bridge-preserve","confidence":0.9}))?;
    check(g.call("entities", &json!({"names":["bridge-preserve"]}))?[0]["type"] == "project")?;
    g.call("add",&json!({"subject":"bridge-owner","predicate":"owns","object":"bridge-preserve","confidence":0.9,"from_type":"person","to_type":"system"}))?;
    let row = g.call("entities", &json!({"names":["bridge-preserve"]}))?;
    check(row[0]["type"] == "system" && row[0]["properties"] == json!({"aliases":["lumina"]}))?;
    check(
        g.call(
            "set_entity",
            &json!({"name":"bridge-preserve","type":"system"}),
        )?["changed"]
            == false,
    )
}
pub fn duplicate(g: &mut Graph) -> Result<(), Error> {
    let mut args =
        json!({"subject":"bridge-a","predicate":"supports","object":"bridge-b","confidence":0.5});
    let first = g.call("add", &args)?;
    args["confidence"] = json!(0.6);
    let second = g.call("add", &args)?;
    check(
        first["created"] == true
            && second["created"] == false
            && first["triple_id"] == second["triple_id"],
    )?;
    let rows = g.call("query", &json!({"entity":"bridge-a"}))?;
    check(rows["facts"].as_array().unwrap().len() == 1 && rows["facts"][0]["confidence"] == 0.5)
}
pub type Case = fn(&mut Graph) -> Result<(), Error>;
pub const CASES: [Case; 3] = [confidence_types, omitted_types, duplicate];
