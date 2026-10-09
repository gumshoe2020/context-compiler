//! Read-only views over compiler.sqlite and the KG for the M4d-2a doctor gates: nothing is locked
//! or written (both opened read-only).
use crate::{Error, cas};
use palace_core::wal_replay::Live;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::Path;

/// compiler.sqlite read-only with the KG attached read-only as `kg`.
pub(crate) fn open(compiler: &Path, kg: &Path) -> Result<Connection, Error> {
    let c = Connection::open_with_flags(
        compiler,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let kg = kg.to_str().ok_or("non-UTF8 KG path")?;
    c.execute("ATTACH DATABASE ?1 AS kg", [format!("file:{kg}?mode=ro")])?;
    Ok(c)
}

/// The changes of a logged event whose rows no longer hold their `before` value (counter deltas
/// excepted), as `kind id: current`: empty once the event is fully undone.
pub fn not_restored(compiler: &Path, kg: &Path, logged: &Value) -> Result<Vec<String>, Error> {
    let c = open(compiler, kg)?;
    let mut stale = Vec::new();
    for change in logged["changes"].as_array().into_iter().flatten() {
        if change["kind"] == "claim_counters_delta" {
            continue;
        }
        let current = cas::current(&c, change)?;
        if !cas::same(&current, &change["before"]) {
            stale.push(format!("{} {}: {current}", change["kind"], change["id"]));
        }
    }
    Ok(stale)
}

/// `id valid_to confidence source_closet` of every triple `claim_id` cites.
pub fn cited_rows(compiler: &Path, kg: &Path, claim_id: &str) -> Result<Vec<String>, Error> {
    let c = open(compiler, kg)?;
    let mut s = c.prepare(
        "SELECT t.id||' '||coalesce(t.valid_to,'-')||' '||coalesce(t.confidence,'-')||' '||coalesce(t.source_closet,'-')
         FROM kg.triples t JOIN citations ci ON ci.triple_id=t.id WHERE ci.claim_id=?1 ORDER BY t.id",
    )?;
    let rows = s
        .query_map([claim_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Live triples (id → confidence, 1.0 when unset).
pub fn live_triples(compiler: &Path, kg: &Path) -> Result<Live, Error> {
    let c = open(compiler, kg)?;
    let mut s =
        c.prepare("SELECT id, coalesce(confidence,1.0) FROM kg.triples WHERE valid_to IS NULL")?;
    let rows = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Whether the triple exists and has a `valid_to`.
pub fn triple_ended(compiler: &Path, kg: &Path, triple: &str) -> Result<bool, Error> {
    let c = open(compiler, kg)?;
    let valid_to: Option<String> = c.query_row(
        "SELECT valid_to FROM kg.triples WHERE id=?1",
        [triple],
        |r| r.get(0),
    )?;
    Ok(valid_to.is_some())
}
