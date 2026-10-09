//! SPEC §2.3 ImprovementEvent rows (M4d items 1 and 3). An event is one `improvement_events` row
//! plus one hash-chained write-log line carrying its state changes, each `{kind, id, before,
//! after}`. Undo applies the `before` values in reverse order as a new event whose id was fixed
//! in advance (`reversible_by`), so every event names the event that undoes it.
use crate::{Error, write_log};
use palace_core::ids::{length_prefixed, sha256};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// A new event; `changes` have already been applied by the caller in the same transaction.
pub struct Event<'a> {
    pub by: &'a str,
    pub verb: &'a str,
    pub from_claim: Option<&'a str>,
    pub to_claim: Option<&'a str>,
    pub finding: Option<&'a str>,
    pub text: &'a str,
    pub axis: &'a [&'a str],
    pub changes: Vec<Value>,
}

/// The id of the event that undoes `id`.
pub fn undo_id(id: &str) -> String {
    format!(
        "e_{}",
        &sha256(length_prefixed(&["undo", id]).as_bytes())[..12]
    )
}

/// Appends `event` (refused when `axis_improved` is empty, SPEC §3 "Monotone"); returns its id.
pub fn append(c: &Connection, event: &Event, at: &str) -> Result<String, Error> {
    let id = new_id(c, event, at)?;
    insert(c, &id, event, at)?;
    Ok(id)
}

/// The id `append` gives `event` now: its verb, by, claim, finding, text, instant and the number
/// of events so far (known before its changes, so audit lines can carry it).
pub fn new_id(c: &Connection, event: &Event, at: &str) -> Result<String, Error> {
    let count: i64 = c.query_row("SELECT count(*) FROM improvement_events", [], |r| r.get(0))?;
    let parts = [
        event.verb,
        event.by,
        event.from_claim.unwrap_or(""),
        event.finding.unwrap_or(""),
        event.text,
        at,
        &count.to_string(),
    ];
    Ok(format!(
        "e_{}",
        &sha256(length_prefixed(&parts).as_bytes())[..12]
    ))
}

/// Inserts `event` under a given id (an undo's id is its target's `reversible_by`).
pub fn insert(c: &Connection, id: &str, event: &Event, at: &str) -> Result<(), Error> {
    if event.axis.is_empty() {
        return Err(
            "refused: axis_improved is empty (SPEC §3: a verb must improve an axis)".into(),
        );
    }
    let axis = json!(event.axis).to_string();
    let reversible_by = undo_id(id);
    c.execute(
        "INSERT INTO improvement_events(id,at,by,verb,from_claim,to_claim,finding,text,axis_improved,reversible_by) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![id, at, event.by, event.verb, event.from_claim, event.to_claim, event.finding, event.text, axis, reversible_by],
    )?;
    let line = json!({"operation":"improvement_event","id":id,"at":at,"by":event.by,"verb":event.verb,"from_claim":event.from_claim,"to_claim":event.to_claim,"finding":event.finding,"text":event.text,"axis_improved":event.axis,"reversible_by":reversible_by,"changes":event.changes});
    write_log::append(c, &line.to_string())
}

/// The logged form of event `id` (with its changes), if any.
pub fn logged(c: &Connection, id: &str) -> Result<Option<Value>, Error> {
    let payload: Option<String> = c
        .query_row(
            "SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')='improvement_event' AND json_extract(payload,'$.id')=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    payload
        .map(|p| serde_json::from_str(&p).map_err(Into::into))
        .transpose()
}

/// A state change record; `apply(c, &change, Side::After)` performs it.
pub fn change(kind: &str, id: &str, before: Value, after: Value) -> Value {
    json!({"kind": kind, "id": id, "before": before, "after": after})
}

#[derive(Clone, Copy)]
pub enum Side {
    Before,
    After,
}

/// Sets the state a change describes on one side. Kinds: `claim` ({status, superseded_by}),
/// `claim_counters` ({survived, weakened}, absolute; events before M4d-2a),
/// `claim_counters_delta` (the side minus the other side is added), `finding` (decision JSON or null),
/// `citation_withdrawn` ({claim_id, withdrawn_at} on every citation row of the triple by the claim),
/// `triple_valid_to`, `triple_confidence`, `entity_properties` (the last three need the KG
/// attached as `kg`).
pub fn apply(c: &Connection, change: &Value, side: Side) -> Result<(), Error> {
    let (value, other) = match side {
        Side::Before => (&change["before"], &change["after"]),
        Side::After => (&change["after"], &change["before"]),
    };
    let id = change["id"].as_str().ok_or("change without id")?;
    let kind = change["kind"].as_str().unwrap_or_default();
    match kind {
        "claim" => set_status(c, id, value)?,
        "claim_counters_delta" => add_counters(c, id, value, other)?,
        "claim_counters" => c.execute(
            "UPDATE claims SET survived=?2, weakened=?3 WHERE id=?1",
            params![id, value["survived"].as_i64(), value["weakened"].as_i64()],
        )?,
        "citation_withdrawn" => set_withdrawn(c, id, value)?,
        "triple_confidence" => c.execute(
            "UPDATE kg.triples SET confidence=?2 WHERE id=?1",
            params![id, value.as_f64()],
        )?,
        "finding" if !value.is_null() => decide(c, id, value)?,
        _ => c.execute(&text_column(kind)?, params![id, as_text(value)])?,
    };
    Ok(())
}

/// Sets a finding's decision and closes its open ask, whatever path decided it (M4d-2c item 4,
/// review S-3: `rule:recheck` included), with an `ask_closed` line in the write log.
fn decide(c: &Connection, id: &str, decision: &Value) -> Result<usize, Error> {
    let changed = c.execute(&text_column("finding")?, params![id, as_text(decision)])?;
    let answer = format!(
        "{} by {}",
        decision["verb"].as_str().unwrap_or("?"),
        decision["by"].as_str().unwrap_or("?")
    );
    let at = decision["at"].as_str().unwrap_or_default();
    let closed = c.execute(
        "UPDATE asks SET answered_at=?2, answer=?3 WHERE finding=?1 AND answered_at IS NULL",
        params![id, at, answer],
    )?;
    if closed > 0 {
        let line = json!({"operation":"ask_closed","finding":id,"answer":answer,"answered_at":at});
        crate::write_log::append(c, &line.to_string())?;
    }
    Ok(changed)
}

fn set_status(c: &Connection, id: &str, value: &Value) -> Result<usize, Error> {
    Ok(c.execute(
        "UPDATE claims SET status=?2, superseded_by=?3 WHERE id=?1",
        params![
            id,
            value["status"].as_str(),
            value["superseded_by"].as_str()
        ],
    )?)
}

fn set_withdrawn(c: &Connection, triple: &str, value: &Value) -> Result<usize, Error> {
    Ok(c.execute(
        "UPDATE citations SET withdrawn_at=?3 WHERE triple_id=?1 AND claim_id=?2",
        params![
            triple,
            value["claim_id"].as_str(),
            value["withdrawn_at"].as_str()
        ],
    )?)
}

/// A counter delta: `value − other` added to each counter.
fn add_counters(c: &Connection, id: &str, value: &Value, other: &Value) -> Result<usize, Error> {
    let delta = |k: &str| value[k].as_i64().unwrap_or(0) - other[k].as_i64().unwrap_or(0);
    Ok(c.execute(
        "UPDATE claims SET survived=survived+?2, weakened=weakened+?3 WHERE id=?1",
        params![id, delta("survived"), delta("weakened")],
    )?)
}

fn text_column(kind: &str) -> Result<String, Error> {
    let (table, column) = match kind {
        "finding" => ("findings", "decision"),
        "triple_valid_to" => ("kg.triples", "valid_to"),
        "entity_properties" => ("kg.entities", "properties"),
        other => return Err(format!("unknown change kind {other:?}").into()),
    };
    Ok(format!("UPDATE {table} SET {column}=?2 WHERE id=?1"))
}

/// A string as itself, null as NULL, anything else as its JSON text.
fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}
