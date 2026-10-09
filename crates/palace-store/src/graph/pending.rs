//! Atomicity across stores for `palace improve` (M4d-2a item 5, review L-3), the `rollup_step`
//! pattern: before the write gate or the coined entity touch the KG, an `improve_step` `pending`
//! record names what the call may create (its new claim ids and term entity); the event's own
//! transaction logs `committed`. At the next call, a pending record with neither `committed` nor
//! `rolled_back` is rolled back: the triples its claims created are ended and its entity retired
//! (append-only, audited to the WAL). Committed events whose IMPROVE entry is missing are
//! returned so the caller can write it (resume).
use super::{Graph, effects::Audited, improve::Prepared};
use crate::{Error, events::change, write_log};
use palace_core::ids::{entity_id, length_prefixed, sha256};
use rusqlite::Connection;
use serde_json::{Value, json};

impl Graph {
    /// Logs the `pending` step for `p` and stores its key on it.
    pub fn begin(&mut self, p: &mut Prepared) -> Result<(), Error> {
        let at = self.now()?;
        let claim_id = p.claim["id"].as_str().unwrap_or_default();
        let key = format!(
            "i_{}",
            &sha256(length_prefixed(&[p.verb.name(), claim_id, &p.by, &p.text, &at]).as_bytes())
                [..12]
        );
        let claim_ids: Vec<&str> = p.new_claims.iter().map(|c| c.id.as_str()).collect();
        let entity = p.coinage.as_ref().map(|k| entity_id(&k.word));
        self.log(&json!({"operation":"improve_step","step":"pending","key":key,"verb":p.verb.name(),"by":p.by,"claim":claim_id,"claim_ids":claim_ids,"entity":entity,"at":at}))?;
        p.pending = Some(key);
        Ok(())
    }

    /// Rolls back every pending step that neither committed nor was rolled back; returns one
    /// `{key, verb, changes}` per rollback.
    pub fn recover(&mut self) -> Result<Vec<Value>, Error> {
        let open = open_steps(&self.store.connection)?;
        let mut rolled = Vec::new();
        for step in open {
            rolled.push(self.roll_back(&step)?);
        }
        Ok(rolled)
    }

    fn roll_back(&mut self, step: &Value) -> Result<Value, Error> {
        let (at, wal) = (self.now()?, self.wal.clone());
        let key = step["key"].as_str().unwrap_or_default();
        let line = super::intent::guarded(
            &mut self.store.connection,
            ("improve_step rolled_back", &at),
            None,
            |tx| {
                let changes = orphans(tx, step, &at)?;
                let audited = Audited {
                    tx,
                    wal: &wal,
                    event: key,
                    at: &at,
                };
                audited.apply(&changes)?;
                let line = json!({"operation":"improve_step","step":"rolled_back","key":key,"verb":step["verb"],"changes":changes,"at":at});
                write_log::append(tx, &line.to_string())?;
                Ok(line)
            },
        )?;
        self.flush_wal()?;
        Ok(line)
    }
}

/// `pending` steps with no later `committed` or `rolled_back` step of the same key.
fn open_steps(c: &Connection) -> Result<Vec<Value>, Error> {
    let steps = super::improve::payloads(c, "improve_step")?;
    let closed: Vec<&Value> = steps
        .iter()
        .filter(|s| s["step"] != "pending")
        .map(|s| &s["key"])
        .collect();
    Ok(steps
        .iter()
        .filter(|s| s["step"] == "pending" && !closed.contains(&&s["key"]))
        .cloned()
        .collect())
}

/// `valid_to` for each live triple created for the step's claims; `retired` on its entity.
fn orphans(c: &Connection, step: &Value, at: &str) -> Result<Vec<Value>, Error> {
    let day: String = c.query_row("SELECT date('now','localtime')", [], |r| r.get(0))?;
    let mut changes = Vec::new();
    for claim in step["claim_ids"].as_array().into_iter().flatten() {
        let claim = claim.as_str().unwrap_or_default();
        for (triple, _) in super::edges::own_edges(c, claim)? {
            changes.push(change("triple_valid_to", &triple, Value::Null, json!(day)));
        }
    }
    if let Some(entity) = step["entity"].as_str() {
        changes.extend(retired(c, entity, step, at)?);
    }
    Ok(changes)
}

fn retired(c: &Connection, entity: &str, step: &Value, at: &str) -> Result<Option<Value>, Error> {
    let props: Option<String> = rusqlite::OptionalExtension::optional(c.query_row(
        "SELECT properties FROM kg.entities WHERE id=?1",
        [entity],
        |r| r.get(0),
    ))?;
    let Some(props) = props else {
        return Ok(None);
    };
    let before: Value = serde_json::from_str(&props).unwrap_or_else(|_| json!({}));
    let mut after = before.clone();
    after["retired"] = json!({"at": at, "by": step["by"], "reason": "improve rolled back"});
    Ok(Some(change("entity_properties", entity, before, after)))
}
