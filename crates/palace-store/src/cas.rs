//! Undo is compare-and-swap (M4d-2a item 2, review S-1, SPEC §2.3 amendment e): an event is
//! undone only while every row it changed still holds the value it wrote. Counter deltas commute
//! and are exempt. A refusal names the later event(s) that touched the row.
use crate::{Error, Refused};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

/// Refuses the undo of `event_id` when any logged change's current value differs from its `after`.
pub fn check(c: &Connection, event_id: &str, logged: &Value) -> Result<(), Error> {
    let changes = logged["changes"].as_array().cloned().unwrap_or_default();
    let mut stale = Vec::new();
    for change in changes
        .iter()
        .filter(|ch| ch["kind"] != "claim_counters_delta")
    {
        if !same(&current(c, change)?, &change["after"]) {
            stale.push(change);
        }
    }
    if stale.is_empty() {
        return Ok(());
    }
    Err(Refused::error(refusal(c, event_id, &stale)?))
}

fn refusal(c: &Connection, event_id: &str, stale: &[&Value]) -> Result<String, Error> {
    let rows: Vec<String> = stale
        .iter()
        .map(|ch| {
            format!(
                "{} {}",
                ch["kind"].as_str().unwrap_or("?"),
                ch["id"].as_str().unwrap_or("?")
            )
        })
        .collect();
    let ids: Vec<&str> = stale.iter().filter_map(|ch| ch["id"].as_str()).collect();
    let later = later_events(c, event_id, &ids)?;
    let by = if later.is_empty() {
        "outside any improvement event (a later watch run)".to_owned()
    } else {
        format!("by later event(s) {}", later.join(", "))
    };
    Ok(format!(
        "undo {event_id} refused: {} changed since, {by}; undo those first",
        rows.join(", ")
    ))
}

/// Logged improvement events after `event_id` whose changes touch any of `ids`, oldest first.
fn later_events(c: &Connection, event_id: &str, ids: &[&str]) -> Result<Vec<String>, Error> {
    let mut s = c.prepare(
        "SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')='improvement_event'
           AND sequence > (SELECT sequence FROM write_log WHERE json_extract(payload,'$.operation')='improvement_event' AND json_extract(payload,'$.id')=?1)
         ORDER BY sequence",
    )?;
    let payloads: Vec<String> = s
        .query_map([event_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut later = Vec::new();
    for payload in payloads {
        let event: Value = serde_json::from_str(&payload)?;
        let touches = event["changes"].as_array().is_some_and(|changes| {
            changes
                .iter()
                .any(|ch| ids.iter().any(|id| ch["id"] == *id))
        });
        if touches {
            later.extend(event["id"].as_str().map(str::to_owned));
        }
    }
    Ok(later)
}

/// The row's current value in the shape its change logs.
pub fn current(c: &Connection, change: &Value) -> Result<Value, Error> {
    let id = change["id"].as_str().ok_or("change without id")?;
    let one = |sql: &str| -> Result<Option<Option<String>>, Error> {
        Ok(c.query_row(sql, [id], |r| r.get(0)).optional()?)
    };
    Ok(match change["kind"].as_str().unwrap_or_default() {
        "claim" => claim_row(c, id)?,
        "claim_counters" => counters(c, id)?,
        "finding" => json_text(one("SELECT decision FROM findings WHERE id=?1")?.flatten())?,
        "entity_properties" => {
            json_text(one("SELECT properties FROM kg.entities WHERE id=?1")?.flatten())?
        }
        "triple_valid_to" => json!(one("SELECT valid_to FROM kg.triples WHERE id=?1")?.flatten()),
        "triple_confidence" => json!(
            c.query_row("SELECT confidence FROM kg.triples WHERE id=?1", [id], |r| r
                .get::<_, Option<f64>>(0))
                .optional()?
                .flatten()
        ),
        "citation_withdrawn" => citation(c, id, &change["after"])?,
        other => return Err(format!("unknown change kind {other:?}").into()),
    })
}

fn claim_row(c: &Connection, id: &str) -> Result<Value, Error> {
    Ok(c.query_row(
        "SELECT status, superseded_by FROM claims WHERE id=?1",
        [id],
        |r| {
            Ok(json!({"status": r.get::<_, String>(0)?, "superseded_by": r.get::<_, Option<String>>(1)?}))
        },
    )
    .optional()?
    .unwrap_or(Value::Null))
}

fn counters(c: &Connection, id: &str) -> Result<Value, Error> {
    Ok(c.query_row(
        "SELECT survived, weakened FROM claims WHERE id=?1",
        [id],
        |r| Ok(json!({"survived": r.get::<_, i64>(0)?, "weakened": r.get::<_, i64>(1)?})),
    )
    .optional()?
    .unwrap_or(Value::Null))
}

fn citation(c: &Connection, triple: &str, after: &Value) -> Result<Value, Error> {
    let claim = after["claim_id"].as_str().unwrap_or_default();
    let withdrawn: Option<Option<String>> = c
        .query_row(
            "SELECT withdrawn_at FROM citations WHERE triple_id=?1 AND claim_id=?2 ORDER BY rollup LIMIT 1",
            [triple, claim],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json!({"claim_id": claim, "withdrawn_at": withdrawn.flatten()}))
}

/// A JSON text column as its value (a plain string stays a string), NULL as null.
fn json_text(text: Option<String>) -> Result<Value, Error> {
    Ok(match text {
        None => Value::Null,
        Some(t) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
    })
}

/// Equal JSON, numbers within 1e-9 (a logged float may not round-trip bit-exactly).
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => x == y,
        },
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}
