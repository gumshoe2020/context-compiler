//! SETTLE-1b (spec §8): the `fidelity` table in compiler.sqlite, one row per recall check
//! (point, file, type), written once: a rerun of the same check adds nothing.
use crate::{Error, claims::ClaimStore, write_log};
use palace_core::recall::Counts;
use rusqlite::params;
use serde_json::{Value, json};

/// One recall check's row.
pub struct Row<'a> {
    /// `distill` | `consolidation`
    pub point: &'a str,
    /// The rollup's file, or the distilled transcript copy with its line range.
    pub file: &'a str,
    /// `specific` | `concept`
    pub kind: &'a str,
    pub counts: &'a Counts,
    pub rollup_depth: Option<i64>,
    pub primary_share: Option<f64>,
    /// `judge:<model>`, `rule:exact` (specifics) or `none` (no judge: concepts unjudged).
    pub judge: &'a str,
    /// The sampled items and their verdicts.
    pub items: &'a Value,
}
impl ClaimStore {
    /// Records the row unless this check was recorded before; whether it was new.
    pub fn record_fidelity(&mut self, row: &Row, at: &str) -> Result<bool, Error> {
        let tx = self.store.connection.transaction()?;
        let c = row.counts;
        let n = |v: usize| i64::try_from(v).unwrap_or(i64::MAX);
        let added = tx.execute(
            "INSERT OR IGNORE INTO fidelity(point,file,type,sampled,supported,missing,contradicted,unjudged,rollup_depth,primary_share,judge,items,at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![row.point, row.file, row.kind, n(c.sampled), n(c.supported), n(c.missing), n(c.contradicted), n(c.unjudged), row.rollup_depth, row.primary_share, row.judge, row.items.to_string(), at],
        )? == 1;
        if added {
            let line = json!({"operation":"fidelity","point":row.point,"file":row.file,"type":row.kind,"sampled":c.sampled,"supported":c.supported,"missing":c.missing,"contradicted":c.contradicted,"unjudged":c.unjudged,"at":at});
            write_log::append(&tx, &line.to_string())?;
        }
        tx.commit()?;
        Ok(added)
    }
    /// Whether a check of this point and file was recorded.
    pub fn fidelity_scored(&self, point: &str, file: &str) -> Result<bool, Error> {
        Ok(self
            .store
            .connection
            .prepare("SELECT 1 FROM fidelity WHERE point=?1 AND file=?2")?
            .exists([point, file])?)
    }
    /// The rows recorded at or after `since` (an ISO date or time), or in the last 30 days, with
    /// their items (JSON).
    pub fn fidelity_rows(&self, since: Option<&str>) -> Result<Vec<Value>, Error> {
        let mut s = self.store.connection.prepare(
            "SELECT point,file,type,sampled,supported,missing,contradicted,unjudged,judge,at,items FROM fidelity WHERE at >= coalesce(?1, strftime('%Y-%m-%dT%H:%M:%S','now','-30 days')) ORDER BY at, rowid",
        )?;
        let rows = s
            .query_map([since], |r| {
                let items: Value = serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(Value::Null);
                Ok(json!({"point":r.get::<_,String>(0)?,"file":r.get::<_,String>(1)?,"type":r.get::<_,String>(2)?,"sampled":r.get::<_,i64>(3)?,"supported":r.get::<_,i64>(4)?,"missing":r.get::<_,i64>(5)?,"contradicted":r.get::<_,i64>(6)?,"unjudged":r.get::<_,i64>(7)?,"judge":r.get::<_,String>(8)?,"at":r.get::<_,String>(9)?,"items":items}))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
    /// Every claim of `kind` stated by one of these entries (`<name>` or `<name>.md`), as
    /// `{id, subject, predicate, object, quote, status}`, oldest first.
    pub fn claims_of(&self, entries: &[String], kind: &str) -> Result<Vec<Value>, Error> {
        let mut s = self.store.connection.prepare(
            "SELECT id,subject,predicate,object,quote,status,rollup FROM claims WHERE kind=?1 ORDER BY first_seen, rowid",
        )?;
        let rows: Vec<Value> = s
            .query_map([kind], |r| {
                let t = |i: usize| r.get::<_, Option<String>>(i);
                Ok(json!({"id":t(0)?,"subject":t(1)?,"predicate":t(2)?,"object":t(3)?,"quote":t(4)?,"status":t(5)?,"entry":t(6)?}))
            })?
            .collect::<Result<_, _>>()?;
        let bare = |e: &str| e.strip_suffix(".md").unwrap_or(e).to_owned();
        Ok(rows
            .into_iter()
            .filter(|row| {
                let entry = bare(row["entry"].as_str().unwrap_or_default());
                entries.iter().any(|e| bare(e) == entry)
            })
            .collect())
    }
}
