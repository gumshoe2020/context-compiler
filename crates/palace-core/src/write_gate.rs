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
    for reference in &proposal.evidence {
        if let Some(rejected) = check_evidence(search, &reference.query) {
            return Some(rejected);
        }
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
        return Ok(rejected);
    }
    let value = graph.add_triple(&write_arguments(proposal, now, via))?;
    Ok(json!({"ok":true,"value":value}))
}
fn write_arguments(p: &Proposal, now: &str, via: &str) -> Value {
    let mut provenance = json!({"confidence":p.confidence,"supplied_by":p.supplied_by,"evidence_refs":p.evidence.iter().map(|e| &e.query).collect::<Vec<_>>(),"assessed_at":now,"written_via":via});
    for (key, value) in [
        ("claim", &p.claim),
        ("claim_id", &p.claim_id),
        ("transcript", &p.transcript),
    ] {
        if let Some(value) = value {
            provenance[key] = json!(value);
        }
    }
    let mut args = json!({"subject":p.from,"predicate":p.relation,"object":p.to,"confidence":p.confidence,"source_closet":provenance.to_string()});
    for (key, value) in [("from_type", &p.from_type), ("to_type", &p.to_type)] {
        if let Some(value) = value {
            args[key] = json!(value);
        }
    }
    args
}
