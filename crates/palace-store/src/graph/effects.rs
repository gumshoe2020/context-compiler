//! A verb's and an undo's state changes, applied through the audited path (M4d-2a item 4, review
//! S-3): each graph change that alters its row (`valid_to`, confidence, entity properties) also
//! writes a MemPalace-shaped audit line — `kg_invalidate` / `kg_reopen` / `kg_set_confidence` by
//! triple id, `kg_set_entity` by entity id — carrying the improvement event id, to the compiler's
//! write log and the palace WAL outbox in the same transaction as the change.
use super::audit;
use crate::{
    Error, cas,
    events::{self, Side},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

pub(super) struct Audited<'a> {
    pub tx: &'a Connection,
    pub wal: &'a Path,
    pub event: &'a str,
    pub at: &'a str,
}

impl Audited<'_> {
    /// Applies each change in order, auditing the graph changes that alter their row.
    pub fn apply(&self, changes: &[Value]) -> Result<(), Error> {
        for change in changes {
            if let Some((operation, params, result)) = self.line(change)? {
                audit::record(self.tx, self.wal, operation, &params, &result, self.at)?;
            }
            events::apply(self.tx, change, Side::After)?;
        }
        Ok(())
    }

    /// The audit line of a graph change, or None (not a graph change, or a no-op).
    fn line(&self, change: &Value) -> Result<Option<Line>, Error> {
        let kind = change["kind"].as_str().unwrap_or_default();
        let graph = matches!(
            kind,
            "triple_valid_to" | "triple_confidence" | "entity_properties"
        );
        if !graph || cas::same(&cas::current(self.tx, change)?, &change["after"]) {
            return Ok(None);
        }
        let reopened = kind == "triple_valid_to" && change["after"].is_null();
        let id = change["id"].as_str().unwrap_or_default();
        let confidence = if reopened {
            confidence(self.tx, id)?
        } else {
            None
        };
        Ok(Some(line(kind, change, self.event, confidence)))
    }
}

type Line = (&'static str, Value, Value);

/// `(operation, params, result)` of a graph change; `confidence` is the reopened triple's.
fn line(kind: &str, change: &Value, event: &str, confidence: Option<f64>) -> Line {
    let (id, after) = (change["id"].as_str().unwrap_or_default(), &change["after"]);
    let ok = json!({"success": true});
    match kind {
        "triple_valid_to" if after.is_null() => (
            "reopen",
            json!({"triple_id": id, "confidence": confidence, "improvement_event": event}),
            ok,
        ),
        "triple_valid_to" => (
            "invalidate",
            json!({"triple_id": id, "ended": after, "improvement_event": event}),
            json!({"success": true, "invalidated": true, "ended": after}),
        ),
        "triple_confidence" => (
            "set_confidence",
            json!({"triple_id": id, "confidence": after, "improvement_event": event}),
            ok,
        ),
        _ => (
            "set_entity",
            json!({"id": id, "properties": after, "replace_properties": true, "improvement_event": event}),
            ok,
        ),
    }
}

fn confidence(c: &Connection, triple: &str) -> Result<Option<f64>, Error> {
    Ok(c.query_row(
        "SELECT confidence FROM kg.triples WHERE id=?1",
        [triple],
        |r| r.get(0),
    )?)
}
