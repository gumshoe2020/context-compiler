use crate::{
    graph_order::utf16_compare,
    traits::GraphStore,
    traverse::{Neighborhood, Options, traverse},
};
use std::collections::BTreeSet;
#[derive(Debug, Clone, Copy)]
pub enum Task {
    Support,
    Contradict,
    Boundary,
    Analogies,
    AntiAnalogies,
    Dependencies,
    Provenance,
}
impl std::str::FromStr for Task {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "support" => Ok(Self::Support),
            "contradict" => Ok(Self::Contradict),
            "boundary" => Ok(Self::Boundary),
            "analogies" => Ok(Self::Analogies),
            "anti-analogies" => Ok(Self::AntiAnalogies),
            "dependencies" => Ok(Self::Dependencies),
            "provenance" => Ok(Self::Provenance),
            _ => Err("unknown compile task"),
        }
    }
}
pub fn compile<G: GraphStore>(
    graph: &G,
    entity: &str,
    task: Task,
    options: Options,
) -> Result<Neighborhood, G::Error> {
    let mut result = traverse(graph, entity, options)?;
    let claims: BTreeSet<_> = result
        .triples
        .iter()
        .filter(|t| t.predicate == "supports" && t.object == entity)
        .map(|t| t.subject.clone())
        .collect();
    result.triples.retain(|t| {
        let touches = t.subject == entity || t.object == entity;
        match task {
            Task::Support => {
                (t.predicate == "supports" && t.object == entity)
                    || (t.predicate == "depends_on" && claims.contains(&t.subject))
            }
            Task::Provenance => touches,
            _ => {
                touches
                    && t.predicate
                        == match task {
                            Task::Contradict => "contradicts",
                            Task::Boundary => "bounded_by",
                            Task::Analogies => "analogous_to",
                            Task::AntiAnalogies => "anti_analogous_to",
                            Task::Dependencies => "depends_on",
                            _ => unreachable!(),
                        }
            }
        }
    });
    if matches!(task, Task::Provenance) {
        for t in &mut result.triples {
            t.provenance = t
                .source_closet
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .filter(serde_json::Value::is_object);
        }
    }
    result.entities = std::iter::once(entity.to_owned())
        .chain(
            result
                .triples
                .iter()
                .flat_map(|t| [t.subject.clone(), t.object.clone()]),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    result.entities.sort_by(|a, b| utf16_compare(a, b));
    result.retired.retain(|e, _| result.entities.contains(e));
    Ok(result)
}
