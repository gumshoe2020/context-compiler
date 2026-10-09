//! `depends` (SPEC §3, M4d item 6): the provenance around one claim, read-only — the rollups
//! asserting it, the edges created from it or citing it, its superseded/successor chain, the
//! improvement events touching it and any compile records naming it. Opened read-only; nothing
//! is locked or written.
use crate::{Error, lookup};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::path::Path;

/// Everything downstream and upstream of `target`'s claim, as JSON.
pub fn depends(compiler: &Path, kg: &Path, target: &str) -> Result<Value, Error> {
    let c = Connection::open_with_flags(
        compiler,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let kg = kg.to_str().ok_or("non-UTF8 KG path")?;
    c.execute("ATTACH DATABASE ?1 AS kg", [format!("file:{kg}?mode=ro")])?;
    let id = lookup::resolve(&c, target)?.claim_id;
    let claim = lookup::claim(&c, &id)?.ok_or("claim vanished")?;
    Ok(json!({
        "claim": claim,
        "rollups": claim["numbers"],
        "edges": edges(&c, &id)?,
        "predecessors": predecessors(&c, &claim)?,
        "successors": successors(&c, &claim)?,
        "events": events(&c, &id)?,
        "compiles": compiles(&c, &id)?,
    }))
}

/// Triples whose provenance names the claim (live or ended), triples citing it, and triples on
/// its fact node (`claim <id>`, where `contradicts` attaches).
fn edges(c: &Connection, id: &str) -> Result<Vec<Value>, Error> {
    let node = format!("claim_{id}");
    let mut s = c.prepare(
        "SELECT t.id, t.predicate, t.valid_to, t.confidence,
                CASE WHEN t.subject=?2 OR t.object=?2 THEN 'fact' WHEN t.id IN (SELECT triple_id FROM citations WHERE claim_id=?1) THEN 'cited' ELSE 'created' END
         FROM kg.triples t WHERE (CASE WHEN json_valid(t.source_closet) THEN json_extract(t.source_closet,'$.claim_id') END)=?1
            OR t.id IN (SELECT triple_id FROM citations WHERE claim_id=?1) OR t.subject=?2 OR t.object=?2 ORDER BY t.id",
    )?;
    let rows = s
        .query_map([id, &node], |r| {
            Ok(json!({"triple": r.get::<_, String>(0)?, "predicate": r.get::<_, String>(1)?, "valid_to": r.get::<_, Option<String>>(2)?, "confidence": r.get::<_, Option<f64>>(3)?, "link": r.get::<_, String>(4)?}))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// A `superseded_by` value as a claim id: `c_…` itself, or a number within the claim's rollup.
fn target_id(c: &Connection, claim: &Value) -> Result<Option<String>, Error> {
    let Some(by) = claim["superseded_by"].as_str().filter(|b| !b.is_empty()) else {
        return Ok(None);
    };
    if by.starts_with("c_") {
        return Ok(Some(by.to_owned()));
    }
    let rollup = claim["rollup"].as_str().unwrap_or_default();
    Ok(lookup::resolve(c, &format!("{rollup} {by}"))
        .ok()
        .map(|r| r.claim_id))
}

/// The chain forward: each claim that replaced the previous one (bounded at 50 links).
fn successors(c: &Connection, claim: &Value) -> Result<Vec<Value>, Error> {
    let mut chain = Vec::new();
    let mut current = claim.clone();
    while chain.len() < 50 {
        let Some(next) = target_id(c, &current)? else {
            break;
        };
        let Some(row) = lookup::claim(c, &next)? else {
            break;
        };
        chain.push(link(&current, &row));
        current = row;
    }
    Ok(chain)
}

/// Claims this one replaced (their `superseded_by` names it by id or by number in its rollup).
fn predecessors(c: &Connection, claim: &Value) -> Result<Vec<Value>, Error> {
    let id = claim["id"].as_str().unwrap_or_default();
    let mut s = c.prepare(
        "SELECT DISTINCT o.id FROM claims o LEFT JOIN claim_numbers n ON n.claim_id=?1 AND n.rollup=o.rollup
         WHERE o.id<>?1 AND (o.superseded_by=?1 OR o.superseded_by=n.number) ORDER BY o.id",
    )?;
    let ids: Vec<String> = s.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut rows = Vec::new();
    for old in ids {
        if let Some(row) = lookup::claim(c, &old)? {
            rows.push(link(&row, claim));
        }
    }
    Ok(rows)
}

fn link(old: &Value, new: &Value) -> Value {
    json!({"from": old["id"], "from_number": old["number"], "status": old["status"], "to": new["id"], "to_number": new["number"]})
}

fn events(c: &Connection, id: &str) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare("SELECT id, verb, by, at, from_claim, to_claim FROM improvement_events WHERE from_claim=?1 OR to_claim=?1 ORDER BY rowid")?;
    let rows = s
        .query_map([id], |r| {
            Ok(json!({"id": r.get::<_, String>(0)?, "verb": r.get::<_, String>(1)?, "by": r.get::<_, String>(2)?, "at": r.get::<_, String>(3)?, "from_claim": r.get::<_, Option<String>>(4)?, "to_claim": r.get::<_, Option<String>>(5)?}))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Write-log compile records that name the claim (none are written today; reported if present).
fn compiles(c: &Connection, id: &str) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare("SELECT payload FROM write_log WHERE json_extract(payload,'$.operation') LIKE '%compile%' AND instr(payload, ?1) > 0 ORDER BY sequence")?;
    let rows: Vec<String> = s.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    rows.iter()
        .map(|p| serde_json::from_str(p).map_err(Into::into))
        .collect()
}

/// Read-only state an improvement may change, one line per row: every claim, undone ones
/// included (status, successor, counters, derived confidence), entities (type, properties),
/// findings (decision), triples (`valid_to`, confidence) and citations (withdrawn or not).
/// Undo must restore it exactly (M4d-1 `undo` gate).
pub fn snapshot(compiler: &Path, kg: &Path) -> Result<Vec<String>, Error> {
    let c = Connection::open_with_flags(
        compiler,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let kg = kg.to_str().ok_or("non-UTF8 KG path")?;
    c.execute("ATTACH DATABASE ?1 AS kg", [format!("file:{kg}?mode=ro")])?;
    let mut lines = Vec::new();
    for sql in SNAPSHOT {
        lines.extend(strings(&c, sql)?);
    }
    for id in strings(&c, "SELECT id FROM claims ORDER BY id")? {
        lines.push(format!(
            "{id} confidence {}",
            crate::claims::derived(&c, &id)?
        ));
    }
    Ok(lines)
}

const SNAPSHOT: [&str; 5] = [
    "SELECT id||' '||status||' '||coalesce(superseded_by,'-')||' '||survived||' '||weakened FROM claims ORDER BY id",
    "SELECT 'entity '||id||' '||coalesce(type,'-')||' '||coalesce(properties,'-') FROM kg.entities ORDER BY id",
    "SELECT id||' '||coalesce(decision,'open') FROM findings ORDER BY id",
    "SELECT id||' '||coalesce(valid_to,'-')||' '||coalesce(confidence,'-') FROM kg.triples ORDER BY id",
    "SELECT 'cite '||triple_id||' '||claim_id||' '||coalesce(withdrawn_at,'-') FROM citations ORDER BY triple_id, claim_id, rollup",
];

fn strings(c: &Connection, sql: &str) -> Result<Vec<String>, Error> {
    let mut s = c.prepare(sql)?;
    let rows = s
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}
