use crate::{
    evidence::is_bookkeeping,
    proposal::Proposal,
    traits::{EvidenceSearch, GraphStore},
};
use serde_json::{Value, json};
pub const ENTITY_TYPES: &[&str] = &[
    "person", "project", "decision", "fact", "document", "system", "thread", "event",
];
fn rejection(reason: impl Into<String>) -> Value {
    json!({"ok":false,"reason":reason.into()})
}
/// Every gate check, in `write.ts` order, without touching the graph. `Some(rejection)` or `None`.
pub fn check_proposal<S: EvidenceSearch>(search: &S, proposal: &Proposal) -> Option<Value>
where
    S::Error: std::fmt::Display,
{
    for (field, value) in [
        ("from_type", &proposal.from_type),
        ("to_type", &proposal.to_type),
    ] {
        if let Some(value) = value
            && !ENTITY_TYPES.contains(&value.as_str())
        {
            return Some(rejection(format!(
                "{field} \"{value}\" is not one of {}",
                ENTITY_TYPES.join(", ")
            )));
        }
    }
    if proposal.evidence.is_empty() {
        return Some(rejection("direct first-order evidence is required"));
    }
    if proposal.evidence.iter().any(|r| r.query.trim().is_empty()) {
        return Some(rejection("every evidence reference needs a search query"));
    }
    let checks: Vec<_> = proposal
        .evidence
        .iter()
        .map(|reference| check_evidence(search, &reference.query))
        .collect();
    if checks.iter().all(Option::is_some) {
        return checks.into_iter().next().flatten();
    }
    if !proposal.confidence.is_finite() {
        return Some(rejection("confidence must be a finite number"));
    }
    if proposal.supplied_by.trim().is_empty() {
        return Some(rejection("supplied_by is required"));
    }
    None
}
fn check_evidence<S: EvidenceSearch>(search: &S, query: &str) -> Option<Value>
where
    S::Error: std::fmt::Display,
{
    if query.trim().is_empty() {
        return Some(rejection("every evidence reference needs a search query"));
    }
    let hits = match search.search_headers(query, 3) {
        Ok(hits) => hits,
        Err(error) => return Some(rejection(format!("evidence search failed: {error}"))),
    };
    if hits.is_empty() {
        return Some(rejection(format!("evidence does not resolve: {query}")));
    }
    if hits.iter().all(|h| is_bookkeeping(h)) {
        return Some(rejection(format!(
            "evidence resolves only to graph bookkeeping entries (circular): {query}"
        )));
    }
    None
}
/// The write-gate dry run: `{ok, reason}` and nothing written.
pub fn dry_run<S: EvidenceSearch>(search: &S, proposal: &Proposal) -> Value
where
    S::Error: std::fmt::Display,
{
    check_proposal(search, proposal).unwrap_or_else(|| json!({"ok":true,"reason":null}))
}
pub fn propose_relation<S: EvidenceSearch, G: GraphStore<Error = S::Error>>(
    search: &S,
    graph: &mut G,
    proposal: &Proposal,
    now: &str,
    via: &str,
) -> Result<Value, S::Error>
where
    S::Error: std::fmt::Display,
{
    if let Some(rejected) = check_proposal(search, proposal) {
        if skipped_no_evidence(&rejected) {
            log_no_evidence(proposal);
        }
        return Ok(rejected);
    }
    let mut names = match &proposal.evidence_names {
        Some([from, to]) => [vec![from.clone()], vec![to.clone()]],
        None => [vec![proposal.from.clone()], vec![proposal.to.clone()]],
    };
    let ends = proposal
        .evidence_names
        .is_none()
        .then_some([&proposal.from, &proposal.to]);
    for (name, variants) in ends.into_iter().flatten().zip(&mut names) {
        if let Some(entity) = graph.entity(name)? {
            variants.push(entity.name);
            if let Some(aliases) = entity.properties.get("aliases").and_then(Value::as_array) {
                variants.extend(aliases.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
    }
    let Some(entry) = select_evidence(search, proposal, &names)? else {
        log_no_evidence(proposal);
        return Ok(json!({"ok":false,"reason":"relation_skipped_no_evidence"}));
    };
    let value = graph.add_triple(&write_arguments(proposal, now, via, &entry.source))?;
    Ok(json!({"ok":true,"value":value}))
}

pub fn skipped_no_evidence(result: &Value) -> bool {
    result["ok"] == false
        && result["reason"].as_str().is_some_and(|reason| {
            reason == "relation_skipped_no_evidence"
                || reason.starts_with("evidence does not resolve:")
                || reason.starts_with("evidence resolves only to graph bookkeeping entries")
        })
}
fn log_no_evidence(p: &Proposal) {
    let id =
        crate::ids::sha256(crate::ids::length_prefixed(&[&p.from, &p.relation, &p.to]).as_bytes());
    eprintln!("relation_skipped_no_evidence id={id}");
}

/// First passing entry in reference order, then search rank. Both complete names (or a
/// complete alias) must occur in one entry; never combine mentions from separate entries.
fn select_evidence<S: EvidenceSearch>(
    search: &S,
    p: &Proposal,
    names: &[Vec<String>; 2],
) -> Result<Option<crate::traits::EvidenceEntry>, S::Error> {
    for reference in &p.evidence {
        for entry in search.evidence_entries(&reference.query, 3)? {
            if !is_bookkeeping(&entry.text)
                && names
                    .iter()
                    .all(|variants| variants.iter().any(|name| mentions(&entry.text, name)))
            {
                return Ok(Some(entry));
            }
        }
    }
    Ok(None)
}
fn mentions(text: &str, name: &str) -> bool {
    let text: Vec<_> = crate::entity_match::words(text).collect();
    let name: Vec<_> = crate::entity_match::words(name).collect();
    !name.is_empty() && text.windows(name.len()).any(|part| part == name)
}
fn write_arguments(p: &Proposal, now: &str, via: &str, entry: &str) -> Value {
    let mut provenance = json!({"confidence":p.confidence,"supplied_by":p.supplied_by,"evidence_refs":[entry],"assessed_at":now,"written_via":via});
    provenance["evidence_entry"] = json!(entry);
    for (key, value) in [
        ("claim", &p.claim),
        ("claim_id", &p.claim_id),
        ("transcript", &p.transcript),
    ] {
        if let Some(value) = value {
            provenance[key] = json!(value);
        }
    }
    let mut args = json!({"subject":p.from,"predicate":p.relation,"object":p.to,"confidence":p.confidence,"source_closet":provenance.to_string(),"source_file":entry});
    for (key, value) in [("from_type", &p.from_type), ("to_type", &p.to_type)] {
        if let Some(value) = value {
            args[key] = json!(value);
        }
    }
    args
}

#[cfg(test)]
mod evidence_tests {
    use super::*;
    use crate::traits::EvidenceEntry;

    struct Entries(Vec<(&'static str, &'static str)>);
    #[derive(Default)]
    struct Graph(Vec<Value>);
    impl GraphStore for Graph {
        type Error = String;
        fn query_entity(
            &self,
            _: &str,
            _: crate::triple::Direction,
            _: Option<&str>,
        ) -> Result<Vec<crate::triple::Triple>, String> {
            unreachable!()
        }
        fn list_entities(&self) -> Result<Vec<crate::entity::Entity>, String> {
            unreachable!()
        }
        fn entity(&self, name: &str) -> Result<Option<crate::entity::Entity>, String> {
            Ok((name == "violet glider").then(|| crate::entity::Entity {
                id: "violet_glider".into(),
                name: name.into(),
                entity_type: None,
                properties: serde_json::from_value(json!({"aliases":["VG"]})).unwrap(),
                created_at: None,
            }))
        }
        fn add_triple(&mut self, args: &Value) -> Result<Value, String> {
            self.0.push(args.clone());
            Ok(json!({"created":true,"triple_id":"synthetic-id"}))
        }
    }
    impl EvidenceSearch for Entries {
        type Error = String;
        fn search_headers(&self, _: &str, _: usize) -> Result<Vec<String>, String> {
            Ok(self.0.iter().map(|(_, text)| text.to_string()).collect())
        }
        fn evidence_entries(&self, _: &str, _: usize) -> Result<Vec<EvidenceEntry>, String> {
            Ok(self
                .0
                .iter()
                .map(|(source, text)| EvidenceEntry {
                    source: source.to_string(),
                    text: text.to_string(),
                })
                .collect())
        }
    }
    fn proposal() -> Proposal {
        Proposal {
            from: "Invented Tern".into(),
            to: "violet glider".into(),
            relation: "uses".into(),
            evidence: vec![crate::proposal::EvidenceReference {
                query: "invented source".into(),
            }],
            confidence: 0.9,
            supplied_by: "synthetic".into(),
            from_type: None,
            to_type: None,
            claim: None,
            claim_id: None,
            transcript: None,
            evidence_names: None,
        }
    }
    #[test]
    fn repoints_to_best_single_entry_and_accepts_complete_alias() {
        let search = Entries(vec![
            ("first.md", "Invented Tern made a plan."),
            ("second.md", "INVENTED-TERN uses VG."),
            ("third.md", "Invented Tern uses violet glider."),
        ]);
        let mut graph = Graph::default();
        let result =
            propose_relation(&search, &mut graph, &proposal(), "2032-01-01", "synthetic").unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(graph.0.len(), 1);
        assert_eq!(graph.0[0]["source_file"], "second.md");
        let provenance: Value =
            serde_json::from_str(graph.0[0]["source_closet"].as_str().unwrap()).unwrap();
        assert_eq!(provenance["evidence_entry"], "second.md");
    }
    #[test]
    fn fact_node_edge_needs_an_entry_naming_the_claim_ends() {
        let mut edge = proposal();
        (edge.from, edge.to) = ("claim c_old".into(), "claim c_new".into());
        edge.evidence_names = Some(["Invented Tern".into(), "violet glider".into()]);
        let mut graph = Graph::default();
        let said = Entries(vec![("a.md", "Invented Tern uses violet glider.")]);
        let result = propose_relation(&said, &mut graph, &edge, "2032-01-01", "synthetic").unwrap();
        assert_eq!(result["ok"], true);
        let silent = Entries(vec![("b.md", "Invented Tern made a plan.")]);
        let result =
            propose_relation(&silent, &mut graph, &edge, "2032-01-01", "synthetic").unwrap();
        assert!(skipped_no_evidence(&result));
        edge.evidence_names = None;
        let result = propose_relation(&said, &mut graph, &edge, "2032-01-01", "synthetic").unwrap();
        assert!(skipped_no_evidence(&result));
    }
    #[test]
    fn no_cross_entry_partial_substring_or_bookkeeping_evidence() {
        let names = [vec!["Invented Tern".into()], vec!["violet glider".into()]];
        let search = Entries(vec![
            ("subject.md", "Invented Tern made a plan."),
            ("object.md", "violet glider is ready."),
            ("partial.md", "Invented Finch uses violet glider."),
            ("substring.md", "Invented Tern uses ultraviolet gliders."),
            (
                "audit.md",
                "# KG precision audit\nInvented Tern uses violet glider.",
            ),
        ]);
        assert!(
            select_evidence(&search, &proposal(), &names)
                .unwrap()
                .is_none()
        );
        let mut graph = Graph::default();
        let result =
            propose_relation(&search, &mut graph, &proposal(), "2032-01-01", "synthetic").unwrap();
        assert!(skipped_no_evidence(&result));
        assert!(graph.0.is_empty());
        assert!(!mentions("Invented Tern", ""));
    }
}
