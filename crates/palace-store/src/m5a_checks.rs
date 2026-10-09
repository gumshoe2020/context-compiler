//! Read-only probes (and one hand edit) for the m5a doctor — on COPIES only.
use crate::Error;
use crate::shadow_diff::open as read_only;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

/// Claims and open findings in a claim store, and how many of each repeat another: a claim with
/// the same rollup and number, an open finding with the same claim, kind and checker.
pub fn duplicates(store: &Path) -> Result<[i64; 4], Error> {
    let c = read_only(store)?;
    let count = |sql: &str| -> Result<i64, Error> { Ok(c.query_row(sql, [], |r| r.get(0))?) };
    Ok([
        count("SELECT count(*) FROM claims")?,
        count("SELECT count(*) - count(DISTINCT rollup||'|'||number) FROM claims")?,
        count("SELECT count(*) FROM findings WHERE decision IS NULL")?,
        count(
            "SELECT count(*) - count(DISTINCT coalesce(claim,'')||'|'||kind||'|'||checker) FROM findings WHERE decision IS NULL",
        )?,
    ])
}

/// How many triples the KG holds, and whether one has this id.
pub fn triples(kg: &Path, id: &str) -> Result<(i64, bool), Error> {
    let c = read_only(kg)?;
    let count = c.query_row("SELECT count(*) FROM triples", [], |r| r.get(0))?;
    let found = c
        .query_row("SELECT 1 FROM triples WHERE id=?1", [id], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
        .is_some();
    Ok((count, found))
}

/// A hand edit of one triple's confidence, committed (the "someone edited the graph" case).
pub fn set_confidence(kg: &Path, id: &str, confidence: f64) -> Result<(), Error> {
    let changed = Connection::open(kg)?.execute(
        "UPDATE triples SET confidence=?2 WHERE id=?1",
        rusqlite::params![id, confidence],
    )?;
    if changed != 1 {
        return Err(format!("no triple {id} to edit").into());
    }
    Ok(())
}
