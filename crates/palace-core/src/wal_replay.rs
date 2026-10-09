//! Replaying MemPalace-shaped WAL lines over a KG's live set (M4d-2a item 4, review S-3): the
//! live triples (id → confidence) before, plus the `kg_add`, `kg_invalidate` (by triple id),
//! `kg_reopen` and `kg_set_confidence` lines written since, must give the live set after. Lines of
//! other operations do not change the live set. `effect` is what one line does to one triple
//! (also read by `palace doctor --shadow-diff`, M5a item 2).
use serde_json::Value;
use std::collections::BTreeMap;

pub type Live = BTreeMap<String, f64>;

/// What one write-log line does to the graph's triples.
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// A created triple (a `kg_add` with `result.created`), live unless `valid_to` is set.
    Add {
        id: String,
        confidence: f64,
        valid_to: Option<String>,
    },
    /// A triple ended by id; `ended` when the line records it.
    Invalidate {
        id: String,
        ended: Option<String>,
    },
    /// Every live triple of one fact ended (MemPalace's own `kg_invalidate` names no id).
    InvalidateFact {
        subject: String,
        predicate: String,
        object: String,
        ended: Option<String>,
    },
    Reopen {
        id: String,
        confidence: Option<f64>,
    },
    Confidence {
        id: String,
        confidence: f64,
    },
    None,
}

/// The effect of one line; a line of a graph operation missing what it needs is an error.
pub fn effect(line: &Value) -> Result<Effect, String> {
    let (params, result) = (&line["params"], &line["result"]);
    let triple = || {
        params["triple_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{} without triple_id", line["operation"]))
    };
    Ok(match line["operation"].as_str().unwrap_or_default() {
        "kg_add" if result["created"] == true => Effect::Add {
            id: result["triple_id"]
                .as_str()
                .ok_or("kg_add without triple_id")?
                .to_owned(),
            confidence: result["confidence"].as_f64().unwrap_or(1.0),
            valid_to: params["valid_to"].as_str().map(str::to_owned),
        },
        "kg_invalidate" if params["triple_id"].is_null() => Effect::InvalidateFact {
            subject: text(params, "subject")?,
            predicate: text(params, "predicate")?,
            object: text(params, "object")?,
            ended: params["ended"]
                .as_str()
                .or(result["ended"].as_str())
                .map(str::to_owned),
        },
        "kg_invalidate" => Effect::Invalidate {
            id: triple()?,
            ended: params["ended"].as_str().map(str::to_owned),
        },
        "kg_reopen" => Effect::Reopen {
            id: triple()?,
            confidence: params["confidence"].as_f64(),
        },
        "kg_set_confidence" => Effect::Confidence {
            id: triple()?,
            confidence: params["confidence"].as_f64().ok_or("no confidence")?,
        },
        _ => Effect::None,
    })
}

fn text(params: &Value, key: &str) -> Result<String, String> {
    params[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("kg_invalidate without triple_id or {key}"))
}

/// `base` with every line applied in order; an invalidate without a triple id is an error.
pub fn replay(mut live: Live, lines: &[Value]) -> Result<Live, String> {
    for line in lines {
        apply(&mut live, effect(line)?)?;
    }
    Ok(live)
}

fn apply(live: &mut Live, effect: Effect) -> Result<(), String> {
    match effect {
        Effect::Add {
            id,
            confidence,
            valid_to: None,
        } => {
            live.insert(id, confidence);
        }
        Effect::Invalidate { id, .. } => {
            live.remove(&id);
        }
        Effect::InvalidateFact { .. } => {
            return Err("kg_invalidate without triple_id cannot be replayed".into());
        }
        Effect::Reopen { id, confidence } => {
            live.insert(id, confidence.unwrap_or(1.0));
        }
        Effect::Confidence { id, confidence } => {
            if let Some(slot) = live.get_mut(&id) {
                *slot = confidence;
            }
        }
        Effect::Add { .. } | Effect::None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn add_invalidate_reopen_and_rescore_replay_in_order() {
        let base: Live = [("t_a".to_owned(), 0.9)].into();
        let lines = [
            json!({"operation":"kg_add","params":{},"result":{"created":true,"triple_id":"t_b","confidence":0.6}}),
            json!({"operation":"kg_add","params":{},"result":{"created":false,"triple_id":"t_a","confidence":0.9}}),
            json!({"operation":"kg_set_confidence","params":{"triple_id":"t_b","confidence":0.45}}),
            json!({"operation":"kg_invalidate","params":{"triple_id":"t_a","ended":"2026-09-26"}}),
            json!({"operation":"kg_reopen","params":{"triple_id":"t_a","confidence":0.9}}),
            json!({"operation":"kg_invalidate","params":{"triple_id":"t_b"}}),
            json!({"operation":"kg_set_entity","params":{"name":"x"}}),
        ];
        let after = replay(base, &lines).unwrap();
        assert_eq!(after, [("t_a".to_owned(), 0.9)].into());
        let by_name = json!({"operation":"kg_invalidate","params":{"subject":"a"}});
        assert!(replay(Live::new(), &[by_name]).is_err());
    }
}
