//! Intent records (M4d-2a.1 item 2, review S-1). compiler.sqlite and the KG are both WAL-mode,
//! so one COMMIT over both is atomic per file only. A KG-touching commit therefore runs twice:
//! a rehearsal (rolled back) learns its KG rows from the journal; the intent — `{table, key,
//! column, before, after}` per row — commits in compiler.sqlite alone; then the real cross-file
//! transaction runs and marks the intent done beside its event; once COMMIT returns, the intent
//! is settled. `Graph::reconcile` handles any intent left unsettled.
use super::journal;
use crate::Error;
use palace_core::ids::{length_prefixed, sha256};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS kg_intents(id TEXT PRIMARY KEY, at TEXT NOT NULL, operation TEXT NOT NULL, rows TEXT NOT NULL, done_at TEXT, settled TEXT);";

/// Runs `body` as one cross-file transaction guarded by an intent (none when it touches no KG
/// row). `settles`: an earlier intent this transaction settles (recovery's own writes).
pub fn guarded<T>(
    c: &mut Connection,
    (operation, at): (&str, &str),
    settles: Option<(&str, &str)>,
    body: impl Fn(&Connection) -> Result<T, Error>,
) -> Result<T, Error> {
    journal::install(c)?;
    let rows = rehearse(c, &body)?;
    let intent = if rows.is_empty() {
        None
    } else {
        Some(record(c, operation, at, &rows)?)
    };
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let out = body(&tx)?;
    let rows = journal::take(&tx)?;
    if let Some(id) = &intent {
        done(&tx, id, &rows, at)?;
    }
    if let Some((earlier, outcome)) = settles {
        settle(&tx, earlier, outcome)?;
    }
    tx.commit()?;
    if let Some(id) = &intent {
        // A failure here only leaves the intent for the next `reconcile` to settle.
        settle(c, id, "committed").ok();
    }
    Ok(out)
}

/// The KG rows `body` would change; nothing is kept.
fn rehearse<T>(
    c: &mut Connection,
    body: &impl Fn(&Connection) -> Result<T, Error>,
) -> Result<Vec<Value>, Error> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    body(&tx)?;
    journal::take(&tx)
}

/// Commits the intent in compiler.sqlite alone; returns its id.
fn record(c: &mut Connection, operation: &str, at: &str, rows: &[Value]) -> Result<String, Error> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count: i64 = tx.query_row("SELECT count(*) FROM kg_intents", [], |r| r.get(0))?;
    let text = serde_json::to_string(rows)?;
    let parts = [operation, at, &text, &count.to_string()];
    let id = format!("k_{}", &sha256(length_prefixed(&parts).as_bytes())[..12]);
    tx.execute(
        "INSERT INTO kg_intents(id,at,operation,rows) VALUES (?1,?2,?3,?4)",
        params![id, at, operation, text],
    )?;
    tx.commit()?;
    Ok(id)
}

/// Marks the intent done with the rows the transaction actually wrote.
fn done(c: &Connection, id: &str, rows: &[Value], at: &str) -> Result<(), Error> {
    c.execute(
        "UPDATE kg_intents SET rows=?2, done_at=?3 WHERE id=?1",
        params![id, serde_json::to_string(rows)?, at],
    )?;
    Ok(())
}

pub fn settle(c: &Connection, id: &str, outcome: &str) -> Result<(), Error> {
    c.execute(
        "UPDATE kg_intents SET settled=?2 WHERE id=?1",
        params![id, outcome],
    )?;
    Ok(())
}

/// Whether the store has the intents table (a read-only check of an older store may not).
fn has_intents(c: &Connection) -> Result<bool, Error> {
    let exists: Option<i64> = c
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE name='kg_intents'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

/// M4d-2b pre-item S-2 (REVIEW-opus55-m4d2a1 S-2), the conservative option: a graph write is
/// refused while any intent is unsettled, so a writer that never reconciles (`palace-mcp`,
/// `palace kg …`, `retire-entity`) cannot write over a torn commit that the next reconciling
/// command would then revert. Reconciling commands settle every intent before they write.
pub fn refuse_unsettled(c: &Connection) -> Result<(), Error> {
    let pending: Vec<String> = unsettled(c)?.into_iter().map(|i| i.id).collect();
    if pending.is_empty() {
        return Ok(());
    }
    let diverged = super::reconcile::judge(c)?
        .into_iter()
        .find(|j| !j.diverged().is_empty());
    let next = match diverged {
        Some(j) => palace_core::reconcile::settle_hint(&j.id),
        None => "then a reconciling command (palace improve, rollup, propose-batch, watch --store, palace kg or palace-mcp) before writing again".into(),
    };
    Err(crate::Refused::error(format!(
        "unsettled intent(s) {} from an interrupted write: run `palace doctor --check-divergence`; {next}",
        pending.join(", ")
    )))
}

/// An intent not yet settled.
pub struct Unsettled {
    pub id: String,
    pub operation: String,
    pub rows: Vec<Value>,
    pub done: bool,
}

/// Every unsettled intent, newest first (a recovery's own intent is reconciled before the
/// intent it recovers).
pub fn unsettled(c: &Connection) -> Result<Vec<Unsettled>, Error> {
    if !has_intents(c)? {
        return Ok(Vec::new());
    }
    let mut s = c.prepare(
        "SELECT id, operation, rows, done_at IS NOT NULL FROM kg_intents WHERE settled IS NULL ORDER BY rowid DESC",
    )?;
    let rows = s.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get(1)?,
            r.get::<_, String>(2)?,
            r.get(3)?,
        ))
    })?;
    rows.map(|row| {
        let (id, operation, text, done) = row?;
        Ok(Unsettled {
            id,
            operation,
            rows: serde_json::from_str(&text)?,
            done,
        })
    })
    .collect()
}
