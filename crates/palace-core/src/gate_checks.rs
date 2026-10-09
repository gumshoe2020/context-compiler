//! Executable reference-port cases, shared by unit tests and the acceptance doctor.
use crate::{
    batch::parse_batch,
    evidence::is_bookkeeping,
    proposal::{EvidenceReference, Proposal},
    traits::{EvidenceSearch, GraphStore},
    write_gate::propose_relation,
};
use serde_json::{Value, json};
#[derive(Default)]
struct Fake {
    headers: Vec<String>,
    writes: Vec<Value>,
}
impl EvidenceSearch for Fake {
    type Error = String;
    fn search_headers(&self, _: &str, limit: usize) -> Result<Vec<String>, String> {
        if limit != 3 {
            return Err("wrong evidence limit".into());
        }
        Ok(self.headers.clone())
    }
}
impl GraphStore for Fake {
    fn query_entity(
        &self,
        _: &str,
        _: crate::triple::Direction,
        _: Option<&str>,
    ) -> Result<Vec<crate::triple::Triple>, Self::Error> {
        Ok(vec![])
    }
    fn list_entities(&self) -> Result<Vec<crate::entity::Entity>, Self::Error> {
        Ok(vec![])
    }
    fn entity(&self, _: &str) -> Result<Option<crate::entity::Entity>, Self::Error> {
        Ok(None)
    }

    type Error = String;
    fn add_triple(&mut self, args: &Value) -> Result<Value, String> {
        self.writes.push(args.clone());
        Ok(json!({"ok":true}))
    }
}
fn base() -> Proposal {
    Proposal {
        from: "a".into(),
        to: "b".into(),
        relation: "supports".into(),
        evidence: vec![EvidenceReference {
            query: "direct source".into(),
        }],
        confidence: 0.82,
        supplied_by: "callum".into(),
        from_type: None,
        to_type: None,
        claim: None,
        claim_id: None,
        transcript: None,
    }
}
fn run(p: &Proposal, headers: &[&str], graph: &mut Fake) -> Result<Value, String> {
    propose_relation(
        &Fake {
            headers: headers.iter().map(|s| (*s).into()).collect(),
            ..Default::default()
        },
        graph,
        p,
        "2026-09-24T12:00:00Z",
        "palace-rs",
    )
}
fn check(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("ported gate assertion failed".into())
    }
}
pub fn write_resolves() -> Result<(), String> {
    let mut g = Fake::default();
    let p = base();
    let result = run(&p, &["# Primary"], &mut g)?;
    check(result == json!({"ok":true,"value":{"ok":true}}) && g.writes.len() == 1)?;
    let provenance: Value = serde_json::from_str(g.writes[0]["source_closet"].as_str().unwrap())
        .map_err(|e| e.to_string())?;
    check(
        provenance
            == json!({"confidence":0.82,"supplied_by":"callum","evidence_refs":["direct source"],"assessed_at":"2026-09-24T12:00:00Z","written_via":"palace-rs"})
            && g.writes[0].get("from_type").is_none(),
    )
}
pub fn write_empty() -> Result<(), String> {
    let mut g = Fake::default();
    let mut p = base();
    p.evidence.clear();
    check(
        run(&p, &["primary"], &mut g)?
            == json!({"ok":false,"reason":"direct first-order evidence is required"})
            && g.writes.is_empty(),
    )
}
pub fn write_missing() -> Result<(), String> {
    let mut g = Fake::default();
    check(
        run(&base(), &[], &mut g)?
            == json!({"ok":false,"reason":"evidence does not resolve: direct source"})
            && g.writes.is_empty(),
    )
}
pub fn write_types() -> Result<(), String> {
    let mut g = Fake::default();
    let mut p = base();
    for (field, value) in [("to_type", "widget"), ("from_type", "Person")] {
        p.from_type = None;
        p.to_type = None;
        if field == "to_type" {
            p.to_type = Some(value.into());
        } else {
            p.from_type = Some(value.into());
        }
        check(
            run(&p, &["primary"], &mut g)?["reason"]
                == format!(
                    "{field} \"{value}\" is not one of person, project, decision, fact, document, system, thread, event"
                ),
        )?;
    }
    check(g.writes.is_empty())?;
    p.from_type = Some("person".into());
    p.to_type = Some("project".into());
    check(
        run(&p, &["primary"], &mut g)?["ok"] == true
            && g.writes[0]["from_type"] == "person"
            && g.writes[0]["to_type"] == "project",
    )
}
pub fn evidence_headers() -> Result<(), String> {
    check(
        !is_bookkeeping("# Lumina Game Overview\nTags: lumina, callum.\nRust")
            && is_bookkeeping("# KG precision audit 2026-09-16\nbody")
            && is_bookkeeping(
                "# Alias fix applied\n2026-09-09. Tags: callum; graph-write; alias-gap.",
            )
            && is_bookkeeping("# ROLLUP: some graph relations"),
    )
}
pub fn evidence_mixed() -> Result<(), String> {
    let mut g = Fake::default();
    let p = base();
    check(
        run(&p, &["# KG precision audit", "Tags: alias-gap."], &mut g)?
            == json!({"ok":false,"reason":"evidence resolves only to graph bookkeeping entries (circular): direct source"})
            && g.writes.is_empty(),
    )?;
    check(
        run(&p, &["# KG precision audit", "# Primary"], &mut g)?["ok"] == true
            && g.writes.len() == 1,
    )
}
pub fn batch_independent() -> Result<(), String> {
    let mut p = serde_json::to_value(base()).map_err(|e| e.to_string())?;
    p.as_object_mut().unwrap().remove("from_type");
    p.as_object_mut().unwrap().remove("to_type");
    let mut typed = p.clone();
    typed["from_type"] = json!("person");
    typed["to_type"] = json!("project");
    let mut bad = p.clone();
    bad["from_type"] = json!("banana");
    let proposals = parse_batch(&json!([typed, bad, p]).to_string(), None)?;
    let mut g = Fake::default();
    let mut oks = Vec::new();
    for p in proposals {
        oks.push(run(&p, &["primary"], &mut g)?["ok"].clone());
    }
    check(
        oks == vec![json!(true), json!(false), json!(true)]
            && g.writes.len() == 2
            && g.writes[1].get("from_type").is_none(),
    )
}
pub fn batch_shape() -> Result<(), String> {
    check(parse_batch("{}", None).unwrap_err() == "--file must contain a JSON array")?;
    let mut p = serde_json::to_value(base()).map_err(|e| e.to_string())?;
    p["from_type"] = json!(7);
    check(
        parse_batch(&json!([p]).to_string(), None).unwrap_err()
            == "batch entry 0 from_type must be a string",
    )
}
pub const CASES: [fn() -> Result<(), String>; 8] = [
    write_resolves,
    write_empty,
    write_missing,
    write_types,
    evidence_headers,
    evidence_mixed,
    batch_independent,
    batch_shape,
];
#[cfg(test)]
mod tests {
    macro_rules! case {
        ($name:ident) => {
            #[test]
            fn $name() {
                super::$name().unwrap();
            }
        };
    }
    case!(write_resolves);
    case!(write_empty);
    case!(write_missing);
    case!(write_types);
    case!(evidence_headers);
    case!(evidence_mixed);
    case!(batch_independent);
    case!(batch_shape);
    #[test]
    fn order_and_non_finite() {
        let mut p = super::base();
        let mut g = super::Fake::default();
        p.confidence = f64::NAN;
        p.supplied_by.clear();
        assert_eq!(
            super::run(&p, &["primary"], &mut g).unwrap()["reason"],
            "confidence must be a finite number"
        );
        p.evidence[0].query = " ".into();
        assert_eq!(
            super::run(&p, &[], &mut g).unwrap()["reason"],
            "every evidence reference needs a search query"
        );
        assert!(g.writes.is_empty());
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    struct Broken;
    impl EvidenceSearch for Broken {
        type Error = String;
        fn search_headers(&self, _: &str, _: usize) -> Result<Vec<String>, String> {
            Err("vector_disabled".into())
        }
    }
    #[test]
    fn search_failure_rejects_one_proposal_and_next_can_write() {
        let mut graph = Fake::default();
        let rejected = propose_relation(&Broken, &mut graph, &base(), "now", "test").unwrap();
        assert_eq!(rejected["ok"], false);
        assert!(
            rejected["reason"]
                .as_str()
                .unwrap()
                .contains("vector_disabled")
        );
        assert!(graph.writes.is_empty());
        assert_eq!(run(&base(), &["primary"], &mut graph).unwrap()["ok"], true);
        assert_eq!(graph.writes.len(), 1);
    }
}

#[cfg(test)]
mod dry_run_tests {
    use super::*;
    #[test]
    fn dry_run_writes_nothing_and_claims_reach_provenance() {
        let search = Fake {
            headers: vec!["# Entry\nTags: callum.".into()],
            ..Default::default()
        };
        let mut p = base();
        assert_eq!(
            crate::write_gate::dry_run(&search, &p),
            json!({"ok":true,"reason":null})
        );
        p.evidence.clear();
        assert_eq!(crate::write_gate::dry_run(&search, &p)["ok"], false);
        let mut p = base();
        p.claim = Some("C7".into());
        p.claim_id = Some("c_0123456789ab".into());
        let mut graph = Fake::default();
        run(&p, &["# Entry\nTags: callum."], &mut graph).unwrap();
        let provenance: Value =
            serde_json::from_str(graph.writes[0]["source_closet"].as_str().unwrap()).unwrap();
        assert_eq!(provenance["claim"], "C7");
        assert_eq!(provenance["claim_id"], "c_0123456789ab");
    }
}
