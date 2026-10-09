//! Read-side lookups over compiler.sqlite's claim store (M4d items 1–3, 6): resolving a target
//! (`c_…`, `f_…`, a reply token `#f_…`, or `<rollup date> C7` / `<rollup> C7` through the
//! `(rollup, number)` table) and reading claims and findings as JSON.
use crate::{Error, Refused};
use palace_core::claim::is_claim_number;
use rusqlite::{Connection, OptionalExtension, Row};
use serde_json::{Value, json};

/// A target resolved to its claim (and the finding it named, if it named one).
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub claim_id: String,
    pub finding: Option<String>,
}

pub fn resolve(c: &Connection, target: &str) -> Result<Resolved, Error> {
    let target = target.trim();
    if let Some(prefix) = target
        .strip_prefix('#')
        .or(Some(target))
        .filter(|t| t.starts_with("f_"))
    {
        return finding_target(c, prefix);
    }
    if target.starts_with("c_") {
        return match claim(c, target)? {
            Some(_) => Ok(Resolved {
                claim_id: target.to_owned(),
                finding: None,
            }),
            None => Err(Refused::error(format!("unknown claim {target}"))),
        };
    }
    numbered(c, target)
}

fn finding_target(c: &Connection, prefix: &str) -> Result<Resolved, Error> {
    let mut s = c.prepare("SELECT id, claim FROM findings WHERE substr(id,1,length(?1))=?1")?;
    let rows: Vec<(String, Option<String>)> = s
        .query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    match rows.as_slice() {
        [(id, Some(claim))] => Ok(Resolved {
            claim_id: claim.clone(),
            finding: Some(id.clone()),
        }),
        [(id, None)] => Err(Refused::error(format!(
            "finding {id} is about a malformed line, not a claim"
        ))),
        [] => Err(Refused::error(format!("unknown finding {prefix}"))),
        _ => Err(Refused::error(format!(
            "ambiguous finding {prefix} ({} matches)",
            rows.len()
        ))),
    }
}

/// `<date> C7` (the rollup's as-of date) or `<rollup name> C7`.
fn numbered(c: &Connection, target: &str) -> Result<Resolved, Error> {
    let parts: Vec<&str> = target.split_whitespace().collect();
    let [left, number] = parts.as_slice() else {
        return Err(Refused::error(format!(
            "unknown target {target:?} (use c_…, f_…, #f_… or \"<rollup date> C7\")"
        )));
    };
    let number = number.to_uppercase();
    if !is_claim_number(&number) {
        return Err(Refused::error(format!("unknown target {target:?}")));
    }
    let rollup = left.strip_suffix(".md").unwrap_or(left);
    let mut s = c.prepare("SELECT DISTINCT claim_id, rollup FROM claim_numbers WHERE number=?1 AND (date=?2 OR rollup=?3) ORDER BY rollup")?;
    let rows: Vec<(String, String)> = s
        .query_map(rusqlite::params![number, left, rollup], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    one_claim(target, &rows)
}

/// The single `(claim_id, rollup)` a numbered target names, else unknown or ambiguous.
fn one_claim(target: &str, rows: &[(String, String)]) -> Result<Resolved, Error> {
    match rows {
        [(id, _)] => Ok(Resolved {
            claim_id: id.clone(),
            finding: None,
        }),
        [] => Err(Refused::error(format!("unknown claim {target}"))),
        _ => Err(Refused::error(format!(
            "ambiguous claim {target}: rollups {}",
            rows.iter()
                .map(|(_, r)| r.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// A claim row as JSON (with `numbers`: every `(rollup, number, date, thread)` it has), if any.
pub fn claim(c: &Connection, id: &str) -> Result<Option<Value>, Error> {
    let row = c
        .query_row(
            "SELECT id,number,rollup,subject,predicate,object,date,source,quote,status,superseded_by,confidence_base,checks,survived,weakened FROM claims WHERE id=?1",
            [id],
            claim_json,
        )
        .optional()?;
    let Some(mut row) = row else {
        return Ok(None);
    };
    row["numbers"] = json!(numbers(c, id)?);
    row["confidence"] = json!(crate::claims::derived(c, id)?);
    Ok(Some(row))
}

fn claim_json(r: &Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, String>(0)?, "number": r.get::<_, String>(1)?, "rollup": r.get::<_, String>(2)?,
        "subject": r.get::<_, String>(3)?, "predicate": r.get::<_, String>(4)?, "object": r.get::<_, String>(5)?,
        "date": r.get::<_, String>(6)?, "source": r.get::<_, String>(7)?, "quote": r.get::<_, Option<String>>(8)?,
        "status": r.get::<_, String>(9)?, "superseded_by": r.get::<_, Option<String>>(10)?,
        "confidence_base": r.get::<_, f64>(11)?, "checks": r.get::<_, i64>(12)?,
        "survived": r.get::<_, i64>(13)?, "weakened": r.get::<_, i64>(14)?,
    }))
}

/// Every `{rollup, number, date, thread}` a claim is listed under, oldest rollup first.
pub fn numbers(c: &Connection, id: &str) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare("SELECT rollup, number, date, thread FROM claim_numbers WHERE claim_id=?1 ORDER BY date, rollup")?;
    let rows = s
        .query_map([id], |r| {
            Ok(json!({"rollup": r.get::<_, String>(0)?, "number": r.get::<_, String>(1)?, "date": r.get::<_, String>(2)?, "thread": r.get::<_, String>(3)?}))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// The next free claim number in `rollup` (`C<max + 1>`).
pub fn next_number(c: &Connection, rollup: &str) -> Result<String, Error> {
    let max: i64 = c.query_row(
        "SELECT coalesce(max(CAST(substr(number,2) AS INTEGER)),0) FROM claim_numbers WHERE rollup=?1",
        [rollup],
        |r| r.get(0),
    )?;
    Ok(format!("C{}", max + 1))
}

/// Findings as JSON, newest first; `open` keeps undecided ones, `claim` / `rollup` filter.
pub fn findings(c: &Connection, filter: &FindingFilter) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare(
        "SELECT id,claim,rollup,kind,evidence,proposal,checker,checked_at,decision,last_seen,seen_count FROM findings
         WHERE (?1=0 OR decision IS NULL) AND (?2 IS NULL OR claim=?2) AND (?3 IS NULL OR rollup=?3)
         ORDER BY checked_at, id",
    )?;
    let rows = s
        .query_map(
            rusqlite::params![filter.open, filter.claim, filter.rollup],
            finding_json,
        )?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

#[derive(Default)]
pub struct FindingFilter {
    pub open: bool,
    pub claim: Option<String>,
    pub rollup: Option<String>,
}

fn finding_json(r: &Row) -> rusqlite::Result<Value> {
    let parse = |text: Option<String>| {
        text.and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .unwrap_or(Value::Null)
    };
    Ok(json!({
        "id": r.get::<_, String>(0)?, "claim": r.get::<_, Option<String>>(1)?, "rollup": r.get::<_, String>(2)?,
        "kind": r.get::<_, String>(3)?, "evidence": parse(r.get(4)?), "proposal": parse(r.get(5)?),
        "checker": r.get::<_, String>(6)?, "checked_at": r.get::<_, String>(7)?, "decision": parse(r.get(8)?),
        "last_seen": r.get::<_, Option<String>>(9)?, "seen_count": r.get::<_, i64>(10)?,
    }))
}
