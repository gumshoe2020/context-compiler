//! M5b-5 item 1 (review M-1): a standing finding is acted on once. `improve --auto` records, after
//! each event it applies, the claim's status and confidence right after it (an `auto_marks` row
//! `applied <event>` on the finding). A finding that recurs on a (claim, kind) whose latest
//! deciding event is that `rule:auto` event, still standing, on an unchanged claim, is a repeat:
//! `record_finding` bumps the decided finding's `seen_count` instead of inserting a new one, so
//! the loop has nothing to apply again. `repeats` lists them for the loop's SKIP lines.
use crate::{Error, lookup};
use palace_core::auto::{AfterEvent, BY, is_repeat};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::path::Path;

/// The `auto_marks` mark that holds the claim's state right after `event`.
pub fn mark_name(event: &str) -> String {
    format!("applied {event}")
}

/// The mark's detail: the claim (`lookup::claim` JSON) right after `event`.
pub fn detail(event: &str, claim: &Value) -> Value {
    json!({"event": event, "status": claim["status"], "confidence": claim["confidence"]})
}

/// A decided finding that the latest recurrence repeated, with its standing event.
pub struct Repeat {
    pub finding: Value,
    pub claim: Value,
    pub event: String,
}

/// `(finding, event)` when a recurrence of (claim, kind) now would repeat a standing `rule:auto`
/// event; `None` when it would be a new finding.
pub(crate) fn standing(
    c: &Connection,
    claim: &str,
    kind: &str,
) -> Result<Option<(String, String)>, Error> {
    let Some((finding, event)) = latest_decision(c, claim, kind)? else {
        return Ok(None);
    };
    let Some(after) = marked_after(c, &finding, &event)? else {
        return Ok(None);
    };
    let status: String = c.query_row("SELECT status FROM claims WHERE id=?1", [claim], |r| {
        r.get(0)
    })?;
    let confidence = crate::claims::derived(c, claim)?;
    let now = AfterEvent {
        undone: undone(c, &event)?,
        status: after["status"].as_str().unwrap_or_default(),
        confidence: after["confidence"].as_f64().unwrap_or(f64::NAN),
    };
    Ok(is_repeat(&now, &status, confidence).then_some((finding, event)))
}

/// The latest event that decided a finding of (claim, kind), when it is `rule:auto`'s:
/// (finding, event).
fn latest_decision(
    c: &Connection,
    claim: &str,
    kind: &str,
) -> Result<Option<(String, String)>, Error> {
    let latest: Option<(String, String, String)> = c
        .query_row(
            "SELECT f.id, e.id, e.by FROM findings f JOIN improvement_events e ON e.finding=f.id WHERE f.claim=?1 AND f.kind=?2 AND f.decision IS NOT NULL ORDER BY e.rowid DESC LIMIT 1",
            [claim, kind],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(latest
        .filter(|(_, _, by)| by == BY)
        .map(|(finding, event, _)| (finding, event)))
}

/// The claim's state recorded right after `event` on `finding`, if the loop recorded it.
fn marked_after(c: &Connection, finding: &str, event: &str) -> Result<Option<Value>, Error> {
    let detail: Option<String> = c
        .query_row(
            "SELECT detail FROM auto_marks WHERE finding=?1 AND mark=?2",
            [finding, &mark_name(event)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(detail.and_then(|d| serde_json::from_str(&d).ok()))
}

/// Whether `event`'s undo (its `reversible_by`) has been applied.
fn undone(c: &Connection, event: &str) -> Result<bool, Error> {
    Ok(c.prepare("SELECT 1 FROM improvement_events WHERE id=(SELECT reversible_by FROM improvement_events WHERE id=?1)")?
        .exists([event])?)
}

/// Every decided finding seen again after its decision that still repeats a standing `rule:auto`
/// event, read-only (compiler.sqlite and the KG opened `mode=ro`).
pub fn repeats(compiler: &Path, kg: &Path) -> Result<Vec<Repeat>, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    let mut out = Vec::new();
    if !crate::auto::has_table(&c, "auto_marks")? {
        return Ok(out);
    }
    for (claim, kind) in seen_after_decision(&c)? {
        let Some((finding, event)) = standing(&c, &claim, &kind)? else {
            continue;
        };
        let filter = lookup::FindingFilter {
            claim: Some(claim.clone()),
            ..Default::default()
        };
        let found = lookup::findings(&c, &filter)?
            .into_iter()
            .find(|f| f["id"] == finding);
        if let (Some(finding), Some(claim)) = (found, lookup::claim(&c, &claim)?) {
            out.push(Repeat {
                finding,
                claim,
                event,
            });
        }
    }
    Ok(out)
}

/// (claim, kind) of every decided finding whose `last_seen` is later than its decision.
fn seen_after_decision(c: &Connection) -> Result<Vec<(String, String)>, Error> {
    let mut s = c.prepare("SELECT DISTINCT claim, kind FROM findings WHERE claim IS NOT NULL AND decision IS NOT NULL AND last_seen > json_extract(decision,'$.at') ORDER BY claim, kind")?;
    let rows = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}
