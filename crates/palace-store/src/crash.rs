//! Staged crashes for the M4d-2a.1 recovery tests and the m4d1 doctor's `recovery` gate — on
//! COPIES only. Each helper produces one torn state the cross-file commit can leave: the event
//! half failing, the KG half committed alone by a second connection, the KG half lost after the
//! event committed, or a row someone else edited.
use crate::{Error, graph::reconcile_write};
use palace_core::reconcile::Action;
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

/// From now on the compiler's write log refuses lines while an intent is recorded and not yet
/// done: the real transaction (not the rehearsal) fails, so the event half never commits.
pub fn fail_event_half(compiler: &Path) -> Result<(), Error> {
    Connection::open(compiler)?.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS staged_crash BEFORE INSERT ON write_log
         WHEN EXISTS (SELECT 1 FROM kg_intents WHERE done_at IS NULL AND settled IS NULL)
         BEGIN SELECT RAISE(ABORT, 'staged crash: the event half failed'); END;",
    )?;
    Ok(())
}

pub fn clear(compiler: &Path) -> Result<(), Error> {
    Connection::open(compiler)?.execute_batch("DROP TRIGGER IF EXISTS staged_crash;")?;
    Ok(())
}

/// A second connection commits the KG half alone: every unsettled intent that is not done gets
/// its `after` rows written to the KG. Returns how many rows.
pub fn commit_kg_half(compiler: &Path, kg: &Path) -> Result<usize, Error> {
    let intents = unsettled(compiler, false)?;
    write_kg(kg, &intents, Action::Reapply)
}

/// The KG half of the newest done intent is lost and the process died before settling it: its
/// rows go back to `before` (column rows only) and it is unsettled again. Returns its id.
pub fn lose_kg_half(compiler: &Path, kg: &Path) -> Result<String, Error> {
    let c = Connection::open(compiler)?;
    let (id, rows): (String, String) = c.query_row(
        "SELECT id, rows FROM kg_intents WHERE done_at IS NOT NULL ORDER BY rowid DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let rows: Vec<Value> = serde_json::from_str(&rows)?;
    write_kg(kg, &[rows], Action::Restore)?;
    c.execute("UPDATE kg_intents SET settled=NULL WHERE id=?1", [&id])?;
    Ok(id)
}

/// Someone else sets a triple's confidence.
pub fn hand_edit(kg: &Path, triple: &str, confidence: f64) -> Result<(), Error> {
    Connection::open(kg)?.execute(
        "UPDATE triples SET confidence=?2 WHERE id=?1",
        rusqlite::params![triple, confidence],
    )?;
    Ok(())
}

/// Someone else sets an entity's properties.
pub fn hand_edit_entity(kg: &Path, id: &str, properties: &str) -> Result<(), Error> {
    Connection::open(kg)?.execute(
        "UPDATE entities SET properties=?2 WHERE id=?1",
        rusqlite::params![id, properties],
    )?;
    Ok(())
}

fn unsettled(compiler: &Path, done: bool) -> Result<Vec<Vec<Value>>, Error> {
    let c = Connection::open(compiler)?;
    let mut s = c.prepare(
        "SELECT rows FROM kg_intents WHERE settled IS NULL AND (done_at IS NOT NULL)=?1",
    )?;
    let texts: Vec<String> = s
        .query_map([done], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    texts
        .iter()
        .map(|t| serde_json::from_str(t).map_err(Into::into))
        .collect()
}

/// Writes one side of the rows through a connection that has only the KG attached.
fn write_kg(kg: &Path, intents: &[Vec<Value>], side: Action) -> Result<usize, Error> {
    let c = Connection::open_in_memory()?;
    c.execute(
        "ATTACH DATABASE ?1 AS kg",
        [kg.to_str().ok_or("non-UTF8 KG path")?],
    )?;
    let day: String = c.query_row("SELECT date('now','localtime')", [], |r| r.get(0))?;
    let mut written = 0;
    for rows in intents {
        for row in rows
            .iter()
            .filter(|r| side == Action::Reapply || r["column"] != "*")
        {
            reconcile_write(&c, row, side, &day)?;
            written += 1;
        }
    }
    Ok(written)
}
