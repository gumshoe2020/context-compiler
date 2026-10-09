use crate::{
    graph_order::{estimate_tokens, locale_compare, utf16_compare},
    ids::entity_id,
    traits::GraphStore,
    triple::{Direction, Triple},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub max_hops: usize,
    pub token_budget: usize,
    pub include_retired: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_hops: 2,
            token_budget: 800,
            include_retired: false,
        }
    }
}
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Neighborhood {
    pub entities: Vec<String>,
    pub triples: Vec<Triple>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retired: BTreeMap<String, Value>,
}
pub fn triple_key(t: &Triple) -> String {
    format!(
        "{}\0{}\0{}\0{}\0{}",
        t.subject,
        t.predicate,
        t.object,
        t.valid_from.as_deref().unwrap_or(""),
        t.valid_to.as_deref().unwrap_or("")
    )
}
pub fn compare_triples(a: &Triple, b: &Triple) -> std::cmp::Ordering {
    locale_compare(&triple_key(a), &triple_key(b))
}
pub fn traverse<G: GraphStore>(
    graph: &G,
    start: &str,
    options: Options,
) -> Result<Neighborhood, G::Error> {
    let retired: BTreeMap<_, _> = graph
        .list_entities()?
        .into_iter()
        .filter_map(|e| e.retired().cloned().map(|m| (e.id, m)))
        .collect();
    let skip = |name: &str| !options.include_retired && retired.contains_key(&entity_id(name));
    if skip(start) {
        return Ok(Neighborhood::default());
    }
    let mut entities = BTreeSet::from([start.to_owned()]);
    let mut triples = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = vec![(start.to_owned(), 0)];
    let mut used = estimate_tokens(&start);
    let mut index = 0;
    while index < queue.len() && used < options.token_budget {
        let (current, hop) = queue[index].clone();
        index += 1;
        if hop >= options.max_hops {
            continue;
        }
        let mut adjacent = graph.query_entity(&current, Direction::Outgoing, None)?;
        adjacent.extend(graph.query_entity(&current, Direction::Incoming, None)?);
        let mut adjacent: Vec<_> = adjacent
            .into_iter()
            .map(Triple::surface_confidence)
            .collect();
        adjacent.sort_by(compare_triples);
        for triple in adjacent {
            let key = triple_key(&triple);
            if seen.contains(&key) {
                continue;
            }
            let next = if triple.subject == current {
                &triple.object
            } else {
                &triple.subject
            };
            if skip(next) {
                continue;
            }
            let cost = estimate_tokens(&triple)
                + if entities.contains(next) {
                    0
                } else {
                    estimate_tokens(next)
                };
            if used + cost > options.token_budget {
                continue;
            }
            seen.insert(key);
            used += cost;
            if entities.insert(next.clone()) {
                queue.push((next.clone(), hop + 1));
            }
            triples.push(triple);
        }
    }
    let mut entities: Vec<_> = entities.into_iter().collect();
    entities.sort_by(|a, b| utf16_compare(a, b));
    triples.sort_by(compare_triples);
    let marks = if options.include_retired {
        entities
            .iter()
            .filter_map(|e| retired.get(&entity_id(e)).map(|m| (e.clone(), m.clone())))
            .collect()
    } else {
        BTreeMap::new()
    };
    Ok(Neighborhood {
        entities,
        triples,
        retired: marks,
    })
}
