use crate::{
    graph_order::utf16_compare,
    traits::{GraphHistory, GraphMutation},
    triple::{Direction, Triple},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressInput {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub reviewer: String,
    pub verdict: String,
    pub note: String,
}
#[derive(Debug, PartialEq, Serialize)]
pub struct LoadBearingGap {
    pub entity: String,
    #[serde(rename = "inDegree")]
    pub in_degree: usize,
}
/// "stress reviewer must be non-<Assistant>", as the `&'static str` the graph's errors take.
fn non_assistant<E: From<&'static str>>() -> E {
    static MESSAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let assistant = crate::names::display(crate::names::assistant());
    MESSAGE
        .get_or_init(|| format!("stress reviewer must be non-{assistant}"))
        .as_str()
        .into()
}
pub fn record_stress<G: GraphMutation>(
    graph: &mut G,
    input: &StressInput,
    at: &str,
) -> Result<Value, G::Error>
where
    G::Error: From<&'static str>,
{
    let assistant = crate::names::assistant();
    if input.reviewer.trim().is_empty() || input.reviewer.trim().eq_ignore_ascii_case(assistant) {
        return Err(non_assistant());
    }
    let invalidated = graph.invalidate(&input.subject, &input.predicate, &input.object)?;
    let provenance = json!({"stress_history":[{"reviewer":input.reviewer,"verdict":input.verdict,"note":input.note,"assessed_at":at}]}).to_string();
    let mut result = graph.add_triple(&json!({"subject":input.subject,"predicate":input.predicate,"object":input.object,"source_closet":provenance}))?;
    if let Some(warning) = invalidated.get("wal_warning") {
        result["wal_warning"] = warning.clone();
        result["ok"] = json!(true);
    }
    Ok(result)
}
pub fn load_bearing_gaps<G: GraphHistory>(
    graph: &G,
    threshold: usize,
) -> Result<Vec<LoadBearingGap>, G::Error> {
    let stats = graph.stats()?;
    let mut names = ["entities", "entity_names", "entityNames"]
        .iter()
        .find_map(|k| stats[k].as_array())
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
    if names.is_none() {
        names = Some(
            graph
                .timeline(None)?
                .into_iter()
                .flat_map(|t| [t.subject, t.object])
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
        );
    }
    let mut names = names.unwrap_or_default();
    names.sort_by(|a, b| utf16_compare(a, b));
    let mut gaps = Vec::new();
    for entity in names {
        let incoming = graph.query_entity(&entity, Direction::Incoming, None)?;
        if incoming.len() > threshold
            && !graph
                .timeline(Some(&entity))?
                .iter()
                .any(has_external_assessment)
        {
            gaps.push(LoadBearingGap {
                entity,
                in_degree: incoming.len(),
            });
        }
    }
    Ok(gaps)
}
fn has_external_assessment(triple: &Triple) -> bool {
    let Some(p) = triple
        .source_closet
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    else {
        return false;
    };
    let external = |v: &Value| {
        v.as_str()
            .is_some_and(|s| s.to_lowercase() != crate::names::assistant())
    };
    external(&p["supplied_by"])
        || p["stress_history"]
            .as_array()
            .is_some_and(|a| a.iter().any(|e| external(&e["reviewer"])))
}
