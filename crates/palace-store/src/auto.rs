//! The automatic loop's records in compiler.sqlite (M4d item 4, 5): `asks` (one row per asked
//! finding: `asked_at`, `answered_at`), `replies` (idempotency per reply token and normalised
//! text) and `auto_marks` (a recorded proposal, a veto, per finding). Every insert also writes a
//! line to the hash-chained write log. The candidate view is read-only (`--dry-run` writes nothing).
use crate::{Error, claims::ClaimStore, lookup, write_log};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

/// One open claim finding and what the loop needs to know about it.
pub struct Candidate {
    pub finding: Value,
    pub claim: Value,
    pub asked: bool,
    pub vetoed: bool,
    pub person_acted: bool,
}

/// Every open finding on a claim, oldest first, read-only (compiler.sqlite and the KG opened
/// `mode=ro`). An older store without the M4d-2b tables has no asks and no marks.
pub fn candidates(compiler: &Path, kg: &Path) -> Result<Vec<Candidate>, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    let filter = lookup::FindingFilter {
        open: true,
        ..Default::default()
    };
    let mut out = Vec::new();
    for finding in lookup::findings(&c, &filter)? {
        let Some(claim_id) = finding["claim"].as_str().map(str::to_owned) else {
            continue;
        };
        let Some(claim) = lookup::claim(&c, &claim_id)? else {
            continue;
        };
        let id = finding["id"].as_str().unwrap_or_default().to_owned();
        out.push(Candidate {
            asked: open_ask(&c, &id)?,
            vetoed: marked(&c, &id, "vetoed")?,
            person_acted: person_acted(&c, &claim_id)?,
            finding,
            claim,
        });
    }
    Ok(out)
}

pub(crate) fn has_table(c: &Connection, table: &str) -> Result<bool, Error> {
    Ok(
        c.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")?
            .exists([table])?,
    )
}

fn open_ask(c: &Connection, finding: &str) -> Result<bool, Error> {
    if !has_table(c, "asks")? {
        return Ok(false);
    }
    Ok(
        c.prepare("SELECT 1 FROM asks WHERE finding=?1 AND answered_at IS NULL")?
            .exists([finding])?,
    )
}

fn marked(c: &Connection, finding: &str, mark: &str) -> Result<bool, Error> {
    if !has_table(c, "auto_marks")? {
        return Ok(false);
    }
    Ok(
        c.prepare("SELECT 1 FROM auto_marks WHERE finding=?1 AND mark=?2")?
            .exists([finding, mark])?,
    )
}

/// the person or the assistant authored an event on the claim (any verb, undo included): the loop leaves it.
fn person_acted(c: &Connection, claim_id: &str) -> Result<bool, Error> {
    let (person, assistant) = (
        palace_core::names::person(),
        palace_core::names::assistant(),
    );
    Ok(
        c.prepare("SELECT 1 FROM improvement_events WHERE from_claim=?1 AND by IN (?2, ?3)")?
            .exists([claim_id, person, assistant])?,
    )
}

/// One ask as recorded (the Signal line is kept verbatim): `ungrounded` (the loop's) or
/// `escalated` (Fergus's, carrying his proposed claim).
pub struct Ask<'a> {
    pub finding: &'a str,
    pub claim: &'a str,
    pub kind: &'a str,
    pub token: &'a str,
    pub line: &'a str,
    pub proposal: Option<&'a str>,
}

const ASK_COLUMNS: &str =
    "finding,claim,kind,token,line,asked_at,proposal,answered_at,answer,sent_at";

fn ask_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let text = |i: usize| r.get::<_, Option<String>>(i);
    Ok(
        json!({"finding":text(0)?,"claim":text(1)?,"kind":text(2)?,"token":text(3)?,"line":text(4)?,"asked_at":text(5)?,"proposal":text(6)?,"answered_at":text(7)?,"answer":text(8)?,"sent_at":text(9)?}),
    )
}

impl ClaimStore {
    /// Records an ask; `false` (nothing written) when this finding was already asked.
    pub fn record_ask(&mut self, ask: &Ask, at: &str) -> Result<bool, Error> {
        let tx = self.store.connection.transaction()?;
        let added = tx.execute(
            "INSERT OR IGNORE INTO asks(finding,claim,kind,token,line,asked_at,proposal) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![ask.finding, ask.claim, ask.kind, ask.token, ask.line, at, ask.proposal],
        )? == 1;
        if added {
            let line = json!({"operation":"ask","finding":ask.finding,"claim":ask.claim,"kind":ask.kind,"token":ask.token,"asked_at":at,"proposal":ask.proposal});
            write_log::append(&tx, &line.to_string())?;
        }
        tx.commit()?;
        Ok(added)
    }

    /// Open asks (not answered, their finding undecided), oldest first. An orphan — an ask left
    /// open although its finding was decided (before M4d-2c) — is not open (review S-3).
    pub fn open_asks(&self) -> Result<Vec<Value>, Error> {
        let sql = format!(
            "SELECT {ASK_COLUMNS} FROM asks WHERE answered_at IS NULL AND NOT EXISTS (SELECT 1 FROM findings f WHERE f.id=asks.finding AND f.decision IS NOT NULL) ORDER BY asked_at, finding"
        );
        let mut s = self.store.connection.prepare(&sql)?;
        let rows = s.query_map([], ask_json)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// The ask for `finding`, open or answered.
    pub fn ask(&self, finding: &str) -> Result<Option<Value>, Error> {
        let sql = format!("SELECT {ASK_COLUMNS} FROM asks WHERE finding=?1");
        Ok(self
            .store
            .connection
            .query_row(&sql, [finding], ask_json)
            .optional()?)
    }

    /// Records the reply that answered the finding's ask (no-op when it was never asked). The
    /// deciding event may have closed the ask already (`<verb> by <who>`): the reply's own words
    /// replace that answer; the first answer time is kept.
    pub fn answer_ask(&mut self, finding: &str, answer: &str, at: &str) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE asks SET answered_at=coalesce(answered_at, ?2), answer=?3 WHERE finding=?1",
            params![finding, at, answer],
        )?;
        if changed > 0 {
            let line = json!({"operation":"ask_answered","finding":finding,"answer":answer,"answered_at":at});
            write_log::append(&tx, &line.to_string())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records `mark` (`proposal`, `vetoed`) on a finding once; `false` when already recorded.
    pub fn mark(
        &mut self,
        finding: &str,
        mark: &str,
        claim: &str,
        detail: &Value,
    ) -> Result<bool, Error> {
        let tx = self.store.connection.transaction()?;
        let at = crate::graph::now(&tx)?;
        let added = tx.execute(
            "INSERT OR IGNORE INTO auto_marks(finding,mark,claim,detail,at) VALUES (?1,?2,?3,?4,?5)",
            params![finding, mark, claim, detail.to_string(), at],
        )? == 1;
        if added {
            let line = json!({"operation":"auto_mark","finding":finding,"mark":mark,"claim":claim,"detail":detail,"at":at});
            write_log::append(&tx, &line.to_string())?;
        }
        tx.commit()?;
        Ok(added)
    }

    /// The event a reply `(token, text)` already applied (`Some("")` when it applied none).
    pub fn replied(&self, token: &str, text: &str) -> Result<Option<String>, Error> {
        Ok(self
            .store
            .connection
            .query_row(
                "SELECT coalesce(event,'') FROM replies WHERE token=?1 AND text=?2",
                [token, text],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Records an applied reply (idempotency key: token and normalised text).
    pub fn record_reply(
        &mut self,
        token: &str,
        text: &str,
        event: Option<&str>,
    ) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        let at = crate::graph::now(&tx)?;
        tx.execute(
            "INSERT OR IGNORE INTO replies(token,text,event,at) VALUES (?1,?2,?3,?4)",
            params![token, text, event, at],
        )?;
        let line = json!({"operation":"reply","token":token,"text":text,"event":event,"at":at});
        write_log::append(&tx, &line.to_string())?;
        tx.commit()?;
        Ok(())
    }
}

/// The sources the rollup's claims cite — the thread's other entries — read-only, sorted.
pub fn thread_sources(compiler: &Path, kg: &Path, rollup: &str) -> Result<Vec<String>, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    let mut s = c.prepare("SELECT DISTINCT source FROM claims WHERE rollup=?1 ORDER BY source")?;
    let rows = s
        .query_map([rollup], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// A later live claim with the same subject and predicate (the second check of `stale`).
pub fn later_claim(compiler: &Path, kg: &Path, claim_id: &str) -> Result<Option<String>, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    Ok(c.query_row(
        "SELECT l.id FROM claims o JOIN claims l ON lower(l.subject)=lower(o.subject) AND l.predicate=o.predicate AND l.date>o.date AND l.status='live' AND l.id<>o.id WHERE o.id=?1 ORDER BY l.date LIMIT 1",
        [claim_id],
        |r| r.get(0),
    )
    .optional()?)
}

/// The claim's derived confidence without `finding` (read-only): what §5.1's "above 0.80, re-check
/// once" is measured on (M4d-2c item 5, review S-4).
pub fn confidence_without(
    compiler: &Path,
    kg: &Path,
    claim_id: &str,
    finding: &str,
) -> Result<f64, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    crate::claims::derived_without(&c, claim_id, finding)
}

/// Whether the claim store holds `claim_id` (any status: an undone claim still counts).
pub fn claim_exists(compiler: &Path, kg: &Path, claim_id: &str) -> Result<bool, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    Ok(lookup::claim(&c, claim_id)?.is_some())
}
