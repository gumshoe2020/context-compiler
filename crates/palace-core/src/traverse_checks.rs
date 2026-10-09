//! Reference-port cases shared with the acceptance doctor.
use crate::{
    compile::{Task, compile},
    entity::Entity,
    stress::{self, StressInput},
    traits::{GraphHistory, GraphMutation, GraphStore},
    traverse::{Options, traverse},
    triple::{Direction, Triple},
};
use serde_json::{Value, json};
#[derive(Default)]
struct Fake {
    triples: Vec<Triple>,
    entities: Vec<Entity>,
    calls: Vec<String>,
}
fn triple(a: &str, p: &str, b: &str) -> Triple {
    serde_json::from_value(json!({"subject":a,"predicate":p,"object":b})).unwrap()
}
impl GraphStore for Fake {
    type Error = String;
    fn query_entity(
        &self,
        name: &str,
        direction: Direction,
        _: Option<&str>,
    ) -> Result<Vec<Triple>, String> {
        Ok(self
            .triples
            .iter()
            .filter(|t| match direction {
                Direction::Outgoing => t.subject == name,
                Direction::Incoming => t.object == name,
                Direction::Both => t.subject == name || t.object == name,
            })
            .cloned()
            .collect())
    }
    fn list_entities(&self) -> Result<Vec<Entity>, String> {
        Ok(self.entities.clone())
    }
    fn entity(&self, name: &str) -> Result<Option<Entity>, String> {
        Ok(self.entities.iter().find(|e| e.name == name).cloned())
    }
    fn add_triple(&mut self, args: &Value) -> Result<Value, String> {
        self.calls.push("add".into());
        Ok(args.clone())
    }
}
impl GraphMutation for Fake {
    fn invalidate(&mut self, _: &str, _: &str, _: &str) -> Result<Value, String> {
        self.calls.push("invalidate".into());
        Ok(json!({}))
    }
    fn retire_entity(&mut self, _: &str, _: &str, _: &str) -> Result<Value, String> {
        Err("unused".into())
    }
}
impl GraphHistory for Fake {
    fn stats(&self) -> Result<Value, String> {
        Ok(json!({"entities":self.entities.len()}))
    }
    fn timeline(&self, name: Option<&str>) -> Result<Vec<Triple>, String> {
        Ok(self
            .triples
            .iter()
            .filter(|t| name.is_none_or(|n| t.subject == n || t.object == n))
            .cloned()
            .collect())
    }
}
fn graph() -> Fake {
    Fake {
        triples: [
            ("a", "b"),
            ("a", "c"),
            ("b", "d"),
            ("c", "e"),
            ("d", "f"),
            ("e", "g"),
            ("f", "h"),
            ("g", "i"),
        ]
        .map(|(a, b)| triple(a, "links", b))
        .to_vec(),
        ..Default::default()
    }
}
fn require(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("M3b reference assertion failed".into())
    }
}
pub fn bfs() -> Result<(), String> {
    let g = graph();
    let small_options = Options {
        max_hops: 3,
        token_budget: 150,
        ..Default::default()
    };
    let small = traverse(&g, "a", small_options)?;
    let large = traverse(
        &g,
        "a",
        Options {
            token_budget: 500,
            ..small_options
        },
    )?;
    require(
        small == traverse(&g, "a", small_options)?
            && small.entities.iter().all(|e| large.entities.contains(e))
            && large.entities == ["a", "b", "c", "d", "e", "f", "g"]
            && large.triples.len() == 6,
    )
}
pub fn retired() -> Result<(), String> {
    let mut g = graph();
    let mark = json!({"at":"2026-09-07T00:00:00Z","by":"fergus","reason":"fixture"});
    g.entities.push(
        serde_json::from_value(
            json!({"id":"b","name":"b","type":"unknown","properties":{"retired":mark}}),
        )
        .unwrap(),
    );
    let hidden = traverse(&g, "a", Options::default())?;
    require(
        hidden.entities == ["a", "c", "e"]
            && hidden.retired.is_empty()
            && traverse(&g, "b", Options::default())?.entities.is_empty(),
    )?;
    let opts = Options {
        include_retired: true,
        ..Default::default()
    };
    let shown = traverse(&g, "a", opts)?;
    require(
        shown.entities == ["a", "b", "c", "d", "e"]
            && shown.retired.get("b") == Some(&mark)
            && traverse(&g, "B", opts)?.retired.get("B") == Some(&mark),
    )
}
pub fn confidence() -> Result<(), String> {
    for (column, source, expected, kind) in [
        (
            Some(1.0),
            "{\"confidence\":0.85}",
            Some(0.85),
            Some("provenance"),
        ),
        (
            Some(0.42),
            "{\"confidence\":0.99}",
            Some(0.42),
            Some("column"),
        ),
        (Some(1.0), "{\"confidence\":1}", Some(1.0), Some("column")),
        (Some(1.0), "bad", Some(1.0), Some("column")),
        (None, "bad", None, None),
    ] {
        let mut t = triple("a", "p", "b");
        t.confidence = column;
        t.source_closet = Some(source.into());
        let t = t.surface_confidence();
        require(t.confidence == expected && t.confidence_source.as_deref() == kind)?;
    }
    Ok(())
}
pub fn surfaced() -> Result<(), String> {
    let mut t = triple("x", "p", "y");
    t.confidence = Some(1.0);
    t.source_closet = Some("{\"confidence\":0.7}".into());
    let g = Fake {
        triples: vec![t],
        ..Default::default()
    };
    let result = traverse(&g, "x", Options::default())?;
    require(
        result.triples[0].confidence == Some(0.7)
            && result.triples[0].confidence_source.as_deref() == Some("provenance"),
    )
}
fn compile_graph() -> Fake {
    let mut triples = [
        ("claim", "supports", "target"),
        ("claim", "depends_on", "basis"),
        ("target", "contradicts", "counterclaim"),
        ("target", "bounded_by", "condition"),
        ("target", "analogous_to", "analogue"),
        ("anti", "anti_analogous_to", "target"),
        ("target", "depends_on", "prerequisite"),
        ("consumer", "depends_on", "target"),
        ("target", "unrecognized_relation", "other"),
    ]
    .map(|(a, p, b)| triple(a, p, b))
    .to_vec();
    triples[0].source_closet=Some(json!({"confidence":0.8,"supplied_by":"reviewer","evidence_refs":["source"],"assessed_at":"2026-08-18T00:00:00.000Z","stress_history":[{"verdict":"holds"}]}).to_string());
    Fake {
        triples,
        ..Default::default()
    }
}
pub fn compile_kinds() -> Result<(), String> {
    let g = compile_graph();
    let opts = Options {
        token_budget: 1000,
        ..Default::default()
    };
    for (task, predicates) in [
        (Task::Support, vec!["depends_on", "supports"]),
        (Task::Contradict, vec!["contradicts"]),
        (Task::Boundary, vec!["bounded_by"]),
        (Task::Analogies, vec!["analogous_to"]),
        (Task::AntiAnalogies, vec!["anti_analogous_to"]),
        (Task::Dependencies, vec!["depends_on", "depends_on"]),
    ] {
        let result = compile(&g, "target", task, opts)?;
        require(
            result
                .triples
                .iter()
                .map(|t| t.predicate.as_str())
                .collect::<Vec<_>>()
                == predicates,
        )?;
    }
    Ok(())
}
pub fn compile_provenance() -> Result<(), String> {
    let g = compile_graph();
    let opts = Options {
        token_budget: 1000,
        ..Default::default()
    };
    require(
        traverse(&g, "target", opts)?
            .triples
            .iter()
            .any(|t| t.predicate == "unrecognized_relation"),
    )?;
    let result = compile(&g, "target", Task::Provenance, opts)?;
    let t = result
        .triples
        .iter()
        .find(|t| t.predicate == "supports")
        .ok_or("missing support")?;
    require(
        t.provenance
            == Some(serde_json::from_str(g.triples[0].source_closet.as_ref().unwrap()).unwrap()),
    )
}
pub fn stress_record() -> Result<(), String> {
    let mut g = Fake::default();
    let mut input = StressInput {
        subject: "a".into(),
        predicate: "supports".into(),
        object: "b".into(),
        reviewer: "mark".into(),
        verdict: "affirmed".into(),
        note: "reviewed".into(),
    };
    let result = stress::record_stress(&mut g, &input, "2026-09-24T00:00:00.000Z")?;
    let provenance: Value =
        serde_json::from_str(result["source_closet"].as_str().unwrap()).unwrap();
    require(
        g.calls == ["invalidate", "add"] && provenance["stress_history"][0]["reviewer"] == "mark",
    )?;
    // the assistant, padded and in mixed case ("fergus" → " FeRgUs ")
    let mixed: String = (crate::names::assistant().chars().enumerate())
        .map(|(i, c)| {
            if i % 2 == 0 {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect();
    for reviewer in [String::new(), format!(" {mixed} ")] {
        input.reviewer = reviewer;
        require(stress::record_stress(&mut g, &input, "now").is_err())?;
    }
    require(g.calls.len() == 2)
}
pub fn stress_gaps() -> Result<(), String> {
    let mut g = Fake::default();
    for name in ["load", "reviewed", "small"] {
        for subject in if name == "small" {
            vec!["a"]
        } else {
            vec!["a", "b", "c"]
        } {
            g.triples.push(triple(subject, "p", name));
        }
    }
    g.triples[3].source_closet = Some(json!({"stress_history":[{"reviewer":"Wren"}]}).to_string());
    let result = stress::load_bearing_gaps(&g, 2)?;
    require(result.len() == 1 && result[0].entity == "load" && result[0].in_degree == 3)
}
pub const CASES: [fn() -> Result<(), String>; 8] = [
    bfs,
    retired,
    confidence,
    surfaced,
    compile_kinds,
    compile_provenance,
    stress_record,
    stress_gaps,
];
#[cfg(test)]
mod tests {
    #[test]
    fn traverse_bfs() {
        super::bfs().unwrap();
    }
    #[test]
    fn traverse_retired() {
        super::retired().unwrap();
    }
    #[test]
    fn traverse_confidence() {
        super::confidence().unwrap();
    }
    #[test]
    fn traverse_surfaced() {
        super::surfaced().unwrap();
    }
    #[test]
    fn compile_kinds() {
        super::compile_kinds().unwrap();
    }
    #[test]
    fn compile_provenance() {
        super::compile_provenance().unwrap();
    }
    #[test]
    fn stress_record() {
        super::stress_record().unwrap();
    }
    #[test]
    fn stress_gaps() {
        super::stress_gaps().unwrap();
    }
}
