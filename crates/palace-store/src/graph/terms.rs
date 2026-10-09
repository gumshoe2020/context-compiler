//! A term's status change (M4d item 8): `adopt` by `rule:adopt` (the survival bar met) or
//! `veto` by the person (retired) — one ImprovementEvent on the term's definition claim whose one
//! change is the entity's properties, applied through the audited path under an intent, so undo
//! and recovery treat it like any verb's graph effect.
use super::{Graph, effects::Audited, improve::Applied, intent};
use crate::{
    Error, Refused,
    events::{self, Event, change},
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

/// The status change asked for.
pub struct TermChange<'a> {
    pub word: &'a str,
    pub status: &'a str,
    pub by: &'a str,
    pub verb: &'a str,
    pub text: &'a str,
}

impl Graph {
    pub fn term_status(&mut self, t: &TermChange) -> Result<Applied, Error> {
        let (at, wal) = (self.now()?, self.wal.clone());
        let label = format!("term {}", t.verb);
        let applied = intent::guarded(&mut self.store.connection, (&label, &at), None, |tx| {
            term_in(tx, &wal, t, &at)
        })?;
        Ok(self.flushed_term(applied))
    }

    fn flushed_term(&self, mut applied: Applied) -> Applied {
        applied.wal_warning = self.flush_wal().err().map(|e| e.to_string());
        applied
    }
}

fn term_in(
    tx: &Connection,
    wal: &std::path::Path,
    t: &TermChange,
    at: &str,
) -> Result<Applied, Error> {
    let id = palace_core::ids::entity_id(t.word);
    let before = properties(tx, &id)?;
    if before["status"] == t.status {
        return Err(Refused::error(format!(
            "term {} is already {}",
            t.word, t.status
        )));
    }
    let mut after = before.clone();
    after["status"] = json!(t.status);
    after[format!("{}_by", t.verb)] = json!({"by": t.by, "at": at});
    let changes = vec![change("entity_properties", &id, before, after)];
    let definition = definition(tx, t.word)?;
    let mut event = Event {
        by: t.by,
        verb: t.verb,
        from_claim: definition.as_deref(),
        to_claim: None,
        finding: None,
        text: t.text,
        axis: &["precision"],
        changes: Vec::new(),
    };
    let event_id = events::new_id(tx, &event, at)?;
    let audited = Audited {
        tx,
        wal,
        event: &event_id,
        at,
    };
    audited.apply(&changes)?;
    event.changes = changes;
    events::insert(tx, &event_id, &event, at)?;
    let listed = super::improve::listed(tx, event.from_claim)?;
    Ok(Applied::of(&event_id, &event, listed, at.to_owned()))
}

/// A term entity's properties; refused when `id` is not a term.
fn properties(tx: &Connection, id: &str) -> Result<Value, Error> {
    let props: Option<Option<String>> = tx
        .query_row(
            "SELECT properties FROM kg.entities WHERE id=?1 AND type='term'",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let props = props.ok_or_else(|| Refused::error(format!("no term {id}")))?;
    Ok(serde_json::from_str(props.as_deref().unwrap_or("{}"))?)
}

fn definition(tx: &Connection, word: &str) -> Result<Option<String>, Error> {
    Ok(tx
        .query_row(
            "SELECT id FROM claims WHERE lower(subject)=lower(?1) AND predicate='means' ORDER BY first_seen, id LIMIT 1",
            [word],
            |r| r.get(0),
        )
        .optional()?)
}
