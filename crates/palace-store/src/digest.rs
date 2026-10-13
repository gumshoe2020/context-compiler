//! The corrections digest's reads (M4d-2d item 3): the improvement events and Fergus's
//! escalations since a time, oldest first, each with its claim's label, statement and status
//! change. An event already undone is left out (there is nothing left to undo); the undo itself
//! is listed. Read-only; the lines are `palace_core::digest`'s.
use crate::{Error, claims::ClaimStore, lookup};
use palace_core::digest::Correction;
use rusqlite::Connection;
use serde_json::Value;

impl ClaimStore {
    pub fn digest_events(&self, since: &str) -> Result<Vec<Correction>, Error> {
        let c = &self.store.connection;
        let mut timed = Vec::new();
        for logged in logged_events(c, since)? {
            if !undone(c, &logged)? {
                timed.push((text(&logged, "at"), event(c, &logged)?));
            }
        }
        timed.extend(escalations(c, since)?);
        timed.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(timed.into_iter().map(|(_, e)| e).collect())
    }

    /// Twenty-four hours ago, in the event times' format: the digest's default window.
    pub fn day_ago(&self) -> Result<String, Error> {
        self.days_ago(1)
    }
    /// `days` times twenty-four hours ago, in the event times' format.
    pub fn days_ago(&self, days: u32) -> Result<String, Error> {
        let sql = "SELECT strftime('%Y-%m-%dT%H:%M:%S','now','-'||?1||' days')";
        Ok(self.store.connection.query_row(sql, [days], |r| r.get(0))?)
    }
}

fn logged_events(c: &Connection, since: &str) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare(
        "SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')='improvement_event'
           AND json_extract(payload,'$.at')>=?1 ORDER BY sequence",
    )?;
    let rows: Vec<String> = s
        .query_map([since], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    rows.iter()
        .map(|p| serde_json::from_str(p).map_err(Into::into))
        .collect()
}

fn undone(c: &Connection, logged: &Value) -> Result<bool, Error> {
    let undo = text(logged, "reversible_by");
    Ok(c.prepare("SELECT 1 FROM improvement_events WHERE id=?1")?
        .exists([undo])?)
}

fn event(c: &Connection, logged: &Value) -> Result<Correction, Error> {
    let claim_id = text(logged, "from_claim");
    let claim = lookup::claim(c, &claim_id)?.unwrap_or_default();
    let successor = logged["to_claim"]
        .as_str()
        .map(|id| lookup::claim(c, id))
        .transpose()?
        .flatten();
    Ok(Correction {
        id: text(logged, "id"),
        by: text(logged, "by"),
        verb: text(logged, "verb"),
        label: label(&claim),
        statement: statement(&claim),
        status: status_change(logged, &claim_id, &claim),
        successor: successor.as_ref().map(statement),
    })
}

/// The claim's status before and after the event, from its logged changes; the current status
/// twice when the event did not change it.
fn status_change(logged: &Value, claim_id: &str, claim: &Value) -> (String, String) {
    let changes = logged["changes"].as_array().cloned().unwrap_or_default();
    let change = changes
        .iter()
        .find(|ch| ch["kind"] == "claim" && ch["id"] == claim_id);
    let status = |side: &str, fallback: &Value| {
        change
            .map_or(fallback, |ch| &ch[side]["status"])
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    (
        status("before", &claim["status"]),
        status("after", &claim["status"]),
    )
}

fn escalations(c: &Connection, since: &str) -> Result<Vec<(String, Correction)>, Error> {
    let mut s = c.prepare(
        "SELECT asked_at, claim, token, proposal FROM asks WHERE kind='escalated' AND asked_at>=?1",
    )?;
    let rows: Vec<(String, String, String, Option<String>)> = s
        .query_map([since], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (at, claim_id, token, proposal) in rows {
        let claim = lookup::claim(c, &claim_id)?.unwrap_or_default();
        let now = text(&claim, "status");
        out.push((at, escalation(&claim, token, proposal, now)));
    }
    Ok(out)
}

fn escalation(claim: &Value, token: String, proposal: Option<String>, now: String) -> Correction {
    Correction {
        id: token,
        by: palace_core::names::assistant().into(),
        verb: "escalate".into(),
        label: label(claim),
        statement: statement(claim),
        status: (now.clone(), now),
        successor: proposal,
    }
}

/// `<rollup date> Cn` where the claim is first listed.
fn label(claim: &Value) -> String {
    let first = &claim["numbers"][0];
    format!("{} {}", text(first, "date"), text(first, "number"))
}

fn statement(claim: &Value) -> String {
    format!(
        "{} {} {}",
        text(claim, "subject"),
        text(claim, "predicate").replace('_', " "),
        text(claim, "object")
    )
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
