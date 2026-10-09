//! SPEC §3 verbs applied to compiler.sqlite + the attached KG (M4d item 3). `prepare` resolves
//! and checks (a `Refused` on any invariant); the caller gates the new triples through the write
//! gate; `commit` records every state change, the event row and its write-log line in ONE
//! transaction. Append-only: rows are added; only status, decision, `valid_to` and confidence
//! change, and each change is logged with its before and after so `undo` can restore it.
use super::{Graph, effects::Audited, intent};
use crate::{
    Error, Refused,
    events::{self, Event, change},
    lookup,
};
use palace_core::{
    claim::Claim,
    improve::{Coinage, Verb, coinage, split_statements, statement, term_claims},
    proposal::{EvidenceReference, Proposal},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::Path;

pub struct Request<'a> {
    pub verb: Verb,
    pub target: &'a str,
    pub text: Option<&'a str>,
    pub by: &'a str,
    /// `add --supersede`: the anchor is superseded by the new claim even when it is grounded.
    pub supersede: bool,
    /// `weaken --grounding-failed`: a failed grounding attempt (SPEC §5.1 row 2).
    pub grounding_failed: bool,
}

/// A checked improvement, not yet written.
pub struct Prepared {
    pub verb: Verb,
    pub by: String,
    pub text: String,
    pub claim: Value,
    pub finding: Option<String>,
    pub new_claims: Vec<Claim>,
    pub coinage: Option<Coinage>,
    /// New triples for the write gate (contradicts old→new; the claims of `add`/`split`).
    pub proposals: Vec<Proposal>,
    /// Where the claim is listed: `{rollup, number, date, thread}`.
    pub listed: Value,
    /// `add` supersedes its anchor (ungrounded anchor, or `--supersede`; M4d-2a item 3).
    pub supersede: bool,
    /// The `improve_step` key once `Graph::begin` logged it (M4d-2a item 5).
    pub pending: Option<String>,
    /// A failed grounding attempt: decides no finding and always subtracts 0.15 (item 11).
    pub grounding_failed: bool,
    /// `weaken` on a reversal finding (M4d-2c item 1): the claim becomes `disputed`, the finding
    /// stays open for Fergus, and nothing is subtracted beyond the finding's own 0.15.
    pub disputes: bool,
    /// A person's `confirm` that decides the reversal of a `disputed` claim makes it live again.
    pub undisputes: bool,
}

/// A written event.
pub struct Applied {
    pub event: String,
    pub reversible_by: String,
    pub verb: String,
    pub by: String,
    pub text: String,
    pub from_claim: Option<String>,
    pub to_claim: Option<String>,
    pub finding: Option<String>,
    pub axis: Vec<String>,
    pub listed: Value,
    pub at: String,
    /// Set when the palace WAL could not be written now (the outbox retries on the next write).
    pub wal_warning: Option<String>,
}

impl Graph {
    pub fn prepare(&self, request: &Request) -> Result<Prepared, Error> {
        let c = &self.store.connection;
        let resolved = lookup::resolve(c, request.target)?;
        let claim = lookup::claim(c, &resolved.claim_id)?.ok_or("claim vanished")?;
        let listed = first_listed(&claim)?;
        let (text, (disputes, undisputes)) =
            checked(c, request, &claim, resolved.finding.as_deref())?;
        let supersede = supersedes(c, request, &claim)?;
        let anchor = anchor_claim(&claim)?;
        let (new_claims, coinage) = new_claims(c, request.verb, &text, &anchor, &listed)?;
        check_regrounding(c, request, &anchor, &new_claims)?;
        let proposals = proposals(request.verb, request.by, &anchor, &new_claims);
        Ok(Prepared {
            verb: request.verb,
            by: request.by.to_owned(),
            text,
            claim,
            finding: resolved.finding,
            new_claims,
            coinage,
            proposals,
            listed,
            supersede,
            pending: None,
            grounding_failed: request.grounding_failed,
            disputes,
            undisputes,
        })
    }

    /// Writes the event; `created` are the triples the gate created for `prepared.proposals`.
    /// Graph changes are audited to the palace WAL in the same transaction (M4d-2a item 4), which
    /// an intent guards (M4d-2a.1 item 2).
    pub fn commit(&mut self, p: &Prepared, created: &[String]) -> Result<Applied, Error> {
        let (at, wal) = (self.now()?, self.wal.clone());
        let label = format!("improve {}", p.verb.name());
        let applied = intent::guarded(&mut self.store.connection, (&label, &at), None, |tx| {
            commit_in(tx, &wal, p, (created, &at))
        })?;
        Ok(self.flushed(applied))
    }

    /// Undo `event_id`: its `before` values restored in reverse order, as a new event whose id
    /// is the undone event's `reversible_by`. Undoing an undo re-applies the original. Refused
    /// unless every row still holds the event's `after` (compare-and-swap, M4d-2a item 2), and
    /// for a rule or judge when the event is a person's (M4d-2a.1 item 1).
    pub fn undo(&mut self, event_id: &str, by: &str) -> Result<Applied, Error> {
        let (at, wal) = (self.now()?, self.wal.clone());
        let applied = intent::guarded(&mut self.store.connection, ("undo", &at), None, |tx| {
            undo_in(tx, &wal, (event_id, by), &at)
        })?;
        Ok(self.flushed(applied))
    }

    /// Flushes the WAL outbox; a failure is reported on the result (the outbox keeps the lines).
    fn flushed(&self, mut applied: Applied) -> Applied {
        applied.wal_warning = self.flush_wal().err().map(|e| e.to_string());
        applied
    }
}

/// The event's transaction body: its changes, the event row and log line, the `committed` step.
fn commit_in(
    tx: &Connection,
    wal: &Path,
    p: &Prepared,
    (created, at): (&[String], &str),
) -> Result<Applied, Error> {
    let claim_id = p.claim["id"].as_str().ok_or("claim id")?.to_owned();
    let mut event = p.event(&claim_id);
    let id = events::new_id(tx, &event, at)?;
    let audited = Audited {
        tx,
        wal,
        event: &id,
        at,
    };
    let (changes, to_claim) = effects(&audited, p, (&claim_id, created))?;
    let decided = decided_finding(&changes);
    event.to_claim = to_claim.as_deref();
    event.finding = event.finding.or(decided.as_deref());
    event.changes = changes;
    events::insert(tx, &id, &event, at)?;
    log_steps(tx, p, (&id, &claim_id), at)?;
    Ok(Applied::of(&id, &event, p.listed.clone(), at.to_owned()))
}

/// The `committed` step of a pending call (M4d-2a item 5) and a grounding attempt's day
/// (M4d-2a.1 item 3), in the event's transaction.
fn log_steps(
    tx: &Connection,
    p: &Prepared,
    (id, claim_id): (&str, &str),
    at: &str,
) -> Result<(), Error> {
    if let Some(key) = &p.pending {
        let step = json!({"operation":"improve_step","step":"committed","key":key,"event":id});
        crate::write_log::append(tx, &step.to_string())?;
    }
    if p.grounding_failed {
        let attempt =
            json!({"operation":"grounding_attempt","claim":claim_id,"day":&at[..10],"event":id});
        crate::write_log::append(tx, &attempt.to_string())?;
    }
    Ok(())
}

/// The undo's transaction body.
fn undo_in(
    tx: &Connection,
    wal: &Path,
    (event_id, by): (&str, &str),
    at: &str,
) -> Result<Applied, Error> {
    let logged = undoable(tx, event_id)?;
    people_first(tx, event_id, &logged, by)?;
    crate::cas::check(tx, event_id, &logged)?;
    let undo = events::undo_id(event_id);
    let changes = reversed(&logged);
    let audited = Audited {
        tx,
        wal,
        event: &undo,
        at,
    };
    audited.apply(&changes)?;
    let text = undo_text(event_id, &logged);
    let axis: Vec<String> = serde_json::from_value(logged["axis_improved"].clone())?;
    let axis: Vec<&str> = axis.iter().map(String::as_str).collect();
    let event = undo_event(&logged, by, (&text, &axis), changes);
    events::insert(tx, &undo, &event, at)?;
    let listed = listed(tx, event.from_claim)?;
    Ok(Applied::of(&undo, &event, listed, at.to_owned()))
}

impl Prepared {
    /// The event's fields known before its changes (for its id).
    fn event<'a>(&'a self, claim_id: &'a str) -> Event<'a> {
        Event {
            by: &self.by,
            verb: self.verb.name(),
            from_claim: Some(claim_id),
            to_claim: None,
            finding: self.finding.as_deref(),
            text: &self.text,
            axis: self.verb.axis(),
            changes: Vec::new(),
        }
    }
}

/// Applies the claim changes, then the edge changes (which read the new counters); returns all
/// of them and the successor claim.
fn effects(
    a: &Audited,
    p: &Prepared,
    (claim_id, created): (&str, &[String]),
) -> Result<(Vec<Value>, Option<String>), Error> {
    let day: String =
        a.tx.query_row("SELECT date('now','localtime')", [], |r| r.get(0))?;
    let (mut changes, to_claim) = claim_changes(a.tx, p, (claim_id, created), (a.at, &day))?;
    a.apply(&changes)?;
    let edges = super::edges::changes(a.tx, p, claim_id, &day)?;
    a.apply(&edges)?;
    changes.extend(edges);
    Ok((changes, to_claim))
}

/// The first finding the event decided (M4d-2a item 6, review L-9: recorded even when the
/// target was a claim).
fn decided_finding(changes: &[Value]) -> Option<String> {
    changes
        .iter()
        .find(|c| c["kind"] == "finding")
        .and_then(|c| c["id"].as_str().map(str::to_owned))
}

fn undo_text(event_id: &str, logged: &Value) -> String {
    format!(
        "undo {event_id} ({} by {})",
        logged["verb"].as_str().unwrap_or_default(),
        logged["by"].as_str().unwrap_or_default()
    )
}

fn undo_event<'a>(
    logged: &'a Value,
    by: &'a str,
    (text, axis): (&'a str, &'a [&'a str]),
    changes: Vec<Value>,
) -> Event<'a> {
    let field = |key: &str| logged[key].as_str();
    Event {
        by,
        verb: "undo",
        from_claim: field("from_claim"),
        to_claim: field("to_claim"),
        finding: field("finding"),
        text,
        axis,
        changes,
    }
}

impl Graph {
    /// Committed events (except log-only `rule:recheck` ones) whose IMPROVE entry was never
    /// logged: a crash between the event and its diary entry (M4d-2a item 5). Oldest first.
    pub fn unrecorded(&self) -> Result<Vec<Applied>, Error> {
        let c = &self.store.connection;
        let entries: Vec<Value> = payloads(c, "improve_entry")?
            .into_iter()
            .map(|e| e["event"].clone())
            .collect();
        let mut missing = Vec::new();
        for logged in payloads(c, "improvement_event")? {
            if logged["by"] != "rule:recheck" && !entries.contains(&logged["id"]) {
                let listed = listed(c, logged["from_claim"].as_str())?;
                missing.push(Applied::from_logged(&logged, listed));
            }
        }
        Ok(missing)
    }
}

/// Payloads of every write-log line with `operation`, oldest first.
pub(super) fn payloads(c: &Connection, operation: &str) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare(
        "SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')=?1 ORDER BY sequence",
    )?;
    let rows: Vec<String> = s
        .query_map([operation], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    rows.iter()
        .map(|p| serde_json::from_str(p).map_err(Into::into))
        .collect()
}

/// The logged event, refused when unknown or already undone.
fn undoable(c: &Connection, event_id: &str) -> Result<Value, Error> {
    let logged = events::logged(c, event_id)?
        .ok_or_else(|| Refused::error(format!("unknown event {event_id}")))?;
    let undo = events::undo_id(event_id);
    if events::logged(c, &undo)?.is_some() {
        return Err(Refused::error(format!(
            "event {event_id} is already undone by {undo}"
        )));
    }
    Ok(logged)
}

/// Refused when `by` is a rule or judge and the event is a person's, or decides a reversal
/// (M4d-2a.1 item 1, review S-2).
fn people_first(c: &Connection, event_id: &str, logged: &Value, by: &str) -> Result<(), Error> {
    let author = logged["by"].as_str().unwrap_or_default();
    let reversal = decides_reversal(c, logged)?;
    palace_core::improve::may_undo(by, author, reversal).map_err(|why| {
        Refused::error(format!(
            "{by} may not undo {event_id} ({} by {author}): {why}",
            logged["verb"].as_str().unwrap_or_default()
        ))
    })
}

/// Whether any logged change decides (or reopens) a `reversal` finding.
fn decides_reversal(c: &Connection, logged: &Value) -> Result<bool, Error> {
    let changes = logged["changes"].as_array().cloned().unwrap_or_default();
    let mut reversal = c.prepare("SELECT 1 FROM findings WHERE id=?1 AND kind='reversal'")?;
    for change in changes.iter().filter(|c| c["kind"] == "finding") {
        if reversal.exists([change["id"].as_str().unwrap_or_default()])? {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Applied {
    fn from_logged(logged: &Value, listed: Value) -> Self {
        let text = |k: &str| logged[k].as_str().unwrap_or_default().to_owned();
        let owned = |k: &str| logged[k].as_str().map(str::to_owned);
        let axis = logged["axis_improved"].as_array().into_iter().flatten();
        Self {
            event: text("id"),
            reversible_by: text("reversible_by"),
            verb: text("verb"),
            by: text("by"),
            text: text("text"),
            from_claim: owned("from_claim"),
            to_claim: owned("to_claim"),
            finding: owned("finding"),
            axis: axis.filter_map(|a| a.as_str().map(str::to_owned)).collect(),
            listed,
            at: text("at"),
            wal_warning: None,
        }
    }

    pub(super) fn of(id: &str, e: &Event, listed: Value, at: String) -> Self {
        let owned = |v: Option<&str>| v.map(str::to_owned);
        Self {
            event: id.to_owned(),
            reversible_by: events::undo_id(id),
            verb: e.verb.to_owned(),
            by: e.by.to_owned(),
            text: e.text.to_owned(),
            from_claim: owned(e.from_claim),
            to_claim: owned(e.to_claim),
            finding: owned(e.finding),
            axis: e.axis.iter().map(|a| (*a).to_owned()).collect(),
            listed,
            at,
            wal_warning: None,
        }
    }
}

/// A logged event's changes in reverse order with before/after swapped: its undo.
fn reversed(logged: &Value) -> Vec<Value> {
    let changes = logged["changes"].as_array().cloned().unwrap_or_default();
    changes
        .iter()
        .rev()
        .map(|c| {
            change(
                c["kind"].as_str().unwrap_or_default(),
                c["id"].as_str().unwrap_or_default(),
                c["after"].clone(),
                c["before"].clone(),
            )
        })
        .collect()
}

/// Where a claim is first listed (`{rollup, number, date, thread}`), or null.
pub(super) fn listed(c: &Connection, claim_id: Option<&str>) -> Result<Value, Error> {
    let numbers = claim_id.map(|id| lookup::numbers(c, id)).transpose()?;
    Ok(numbers
        .and_then(|n| n.into_iter().next())
        .unwrap_or(Value::Null))
}

/// Every change except the edges', and the successor claim: new claims, created triples, the
/// coined term, decided findings, the claim's status and counters.
fn claim_changes(
    c: &Connection,
    p: &Prepared,
    (claim_id, created): (&str, &[String]),
    (at, day): (&str, &str),
) -> Result<(Vec<Value>, Option<String>), Error> {
    let mut changes = insert_claims(c, p, at)?;
    changes.extend(
        created
            .iter()
            .map(|t| change("triple_valid_to", t, json!(day), Value::Null)),
    );
    changes.extend(entity_change(c, p, at)?);
    let decided = decisions(c, p, claim_id, at)?;
    let decided_any = !decided.is_empty();
    changes.extend(decided);
    let to_claim = successor(c, p, claim_id)?;
    changes.extend(status_change(p, to_claim.as_deref()));
    changes.extend(counter_change(p, decided_any));
    Ok((changes, to_claim))
}

/// Where the claim is first listed (`{rollup, number, date, thread}`).
fn first_listed(claim: &Value) -> Result<Value, Error> {
    Ok(claim["numbers"]
        .get(0)
        .cloned()
        .ok_or("claim has no (rollup, number) row")?)
}

/// The named finding's kind.
fn finding_kind(c: &Connection, finding: Option<&str>) -> Result<Option<String>, Error> {
    finding
        .map(|id| c.query_row("SELECT kind FROM findings WHERE id=?1", [id], |r| r.get(0)))
        .transpose()
        .map_err(Into::into)
}

/// `(disputes, undisputes)`: a non-grounding `weaken` of a named reversal disputes the claim; a
/// person's `confirm` of a disputed claim, whatever it names, decides its reversal and so
/// undisputes it (M4d-2e item 1).
fn dispute_flags(request: &Request, claim: &Value, kind: Option<&str>) -> (bool, bool) {
    let reversal = kind == Some("reversal");
    let disputes = request.verb == Verb::Weaken && reversal && !request.grounding_failed;
    let undisputes = request.verb == Verb::Confirm
        && claim["status"] == "disputed"
        && request.verb.decides_for("reversal", request.by);
    (disputes, undisputes)
}

/// Every refusal `prepare` can give before anything is built; returns the verb's text and its
/// `dispute_flags`.
fn checked(
    c: &Connection,
    request: &Request,
    claim: &Value,
    finding: Option<&str>,
) -> Result<(String, (bool, bool)), Error> {
    palace_core::improve::may_assert(request.verb, request.by, true).map_err(Refused::error)?;
    let disputed = claim["status"] == "disputed";
    palace_core::improve::may_settle(request.verb, request.by, disputed).map_err(Refused::error)?;
    let kind = finding_kind(c, finding)?;
    let flags = dispute_flags(request, claim, kind.as_deref());
    check_status(request.verb, claim, flags.0)?;
    check_finding(request, kind.as_deref())?;
    let text = text_of(request)?;
    check_grounding(c, request, claim)?;
    Ok((text, flags))
}

/// A rule's or judge's `add` only re-grounds (M4d-2d item 1): the same fact as an ungrounded
/// anchor, with another source or quote.
fn check_regrounding(
    c: &Connection,
    request: &Request,
    anchor: &Claim,
    new: &[Claim],
) -> Result<(), Error> {
    if request.verb != Verb::Add {
        return Ok(());
    }
    let same = new
        .iter()
        .all(|n| palace_core::improve::same_fact(anchor, n));
    let regrounds = same && ungrounded(c, &anchor.id)?;
    palace_core::improve::may_assert(Verb::Add, request.by, regrounds).map_err(Refused::error)?;
    Ok(())
}

/// A named reversal finding is decided only by the person or the assistant with confirm, contradict,
/// retract, split or expire (M4d-2a item 6, review L-14; M4d-2e item 1); `weaken` only disputes
/// it (M4d-2c item 1).
fn check_finding(request: &Request, kind: Option<&str>) -> Result<(), Error> {
    let reversal = kind == Some("reversal");
    if reversal && request.verb != Verb::Weaken && !request.verb.decides_for("reversal", request.by)
    {
        return Err(Refused::error(format!(
            "{} is a reversal: only {} decide it, with confirm, contradict, retract, split or expire (weaken disputes it)",
            request.target,
            palace_core::names::either()
        )));
    }
    Ok(())
}

fn text_of(request: &Request) -> Result<String, Error> {
    if request.grounding_failed && request.verb != Verb::Weaken {
        return Err(Refused::error("--grounding-failed applies only to weaken"));
    }
    let default = if request.grounding_failed {
        "grounding attempt failed"
    } else {
        ""
    };
    let text = request.text.unwrap_or(default).trim().to_owned();
    if !palace_core::improve::one_line(&text) {
        return Err(Refused::error("the text must be one line (no newlines)"));
    }
    if request.verb.needs_text() && text.is_empty() {
        return Err(Refused::error(format!(
            "{} needs the text of the new claim(s)",
            request.verb.name()
        )));
    }
    Ok(text)
}

/// `--grounding-failed` (M4d-2a.1 item 3, review L-5): by a rule, the assistant or the person, on an
/// ungrounded claim only; a second attempt the same UTC day is `Skipped` (exit 0, no event).
fn check_grounding(c: &Connection, request: &Request, claim: &Value) -> Result<(), Error> {
    if !request.grounding_failed {
        return Ok(());
    }
    let id = claim["id"].as_str().unwrap_or_default();
    if !palace_core::improve::may_report_grounding(request.by) {
        return Err(Refused::error(format!(
            "--grounding-failed is reported by rule:*, {} or {}, not {}",
            palace_core::names::assistant(),
            palace_core::names::person(),
            request.by
        )));
    }
    if !open_absent_source(c, id)? {
        return Err(Refused::error(format!(
            "--grounding-failed needs an ungrounded claim (an open unsupported finding whose source is absent); {id} has none"
        )));
    }
    let today = &super::now(c)?[..10];
    if attempted(c, id, today)? {
        return Err(crate::Skipped::error(format!(
            "already attempted today: {id} ({today} UTC)"
        )));
    }
    Ok(())
}

fn open_absent_source(c: &Connection, claim_id: &str) -> Result<bool, Error> {
    Ok(c.prepare(
        "SELECT 1 FROM findings WHERE claim=?1 AND kind='unsupported' AND decision IS NULL
           AND json_extract(evidence,'$.status')='absent-source'",
    )?
    .exists([claim_id])?)
}

fn attempted(c: &Connection, claim_id: &str, day: &str) -> Result<bool, Error> {
    Ok(c.prepare(
        "SELECT 1 FROM write_log WHERE json_extract(payload,'$.operation')='grounding_attempt'
           AND json_extract(payload,'$.claim')=?1 AND json_extract(payload,'$.day')=?2",
    )?
    .exists([claim_id, day])?)
}

/// Whether `add` supersedes its anchor: asked with `--supersede`, or the anchor is ungrounded
/// (an unconfirmed `absent-source` finding: its source file was never found). Either way the
/// anchor must be live.
fn supersedes(c: &Connection, request: &Request, claim: &Value) -> Result<bool, Error> {
    if request.verb != Verb::Add {
        if request.supersede {
            return Err(Refused::error("--supersede applies only to add"));
        }
        return Ok(false);
    }
    let id = claim["id"].as_str().unwrap_or_default();
    let live = claim["status"] == "live";
    if request.supersede && !live {
        return Err(Refused::error(format!(
            "--supersede needs a live anchor; {id} is {}",
            claim["status"]
        )));
    }
    Ok(live && (request.supersede || ungrounded(c, id)?))
}

fn ungrounded(c: &Connection, claim_id: &str) -> Result<bool, Error> {
    Ok(c.prepare(
        "SELECT 1 FROM findings WHERE claim=?1 AND json_extract(evidence,'$.status')='absent-source'
           AND (decision IS NULL OR json_extract(decision,'$.verb') IS NOT 'confirm')",
    )?
    .exists([claim_id])?)
}

/// A disputed claim counts as live for every verb except a second dispute.
fn check_status(verb: Verb, claim: &Value, disputes: bool) -> Result<(), Error> {
    let status = claim["status"].as_str().unwrap_or("live");
    let open = status == "live" || (status == "disputed" && !disputes);
    if verb.requires_live() && !open {
        return Err(Refused::error(format!(
            "{} needs a live claim; {} is {status}",
            verb.name(),
            claim["id"].as_str().unwrap_or("")
        )));
    }
    Ok(())
}

/// The stored claim as a `Claim` (the anchor for defaults and ids).
fn anchor_claim(row: &Value) -> Result<Claim, Error> {
    let text = |k: &str| row[k].as_str().unwrap_or_default().to_owned();
    Ok(Claim {
        number: text("number"),
        id: text("id"),
        statement: format!(
            "{} {} {}",
            text("subject"),
            text("predicate").replace('_', " "),
            text("object")
        ),
        subject: text("subject"),
        predicate: text("predicate"),
        object: text("object"),
        date: text("date"),
        source: text("source"),
        quote: row["quote"].as_str().map(str::to_owned),
        superseded_by: None,
        corrected: false,
    })
}

/// The claims `contradict`, `add`, `split` and `coin` create, numbered after the rollup's last;
/// refused when one already exists (or the coined term does).
fn new_claims(
    c: &Connection,
    verb: Verb,
    text: &str,
    anchor: &Claim,
    listed: &Value,
) -> Result<(Vec<Claim>, Option<Coinage>), Error> {
    let rollup = listed["rollup"].as_str().unwrap_or_default();
    let next = lookup::next_number(c, rollup)?;
    let after = format!("C{}", next[1..].parse::<u64>().unwrap_or(0) + 1);
    let (claims, coined) = parsed_claims(verb, text, anchor, [&next, &after])
        .map_err(|e| Refused::error(format!("{} text: {e}", verb.name())))?;
    refuse_existing(c, &claims, coined.as_ref())?;
    Ok((claims, coined))
}

type Parsed = (Vec<Claim>, Option<Coinage>);

fn parsed_claims(
    verb: Verb,
    text: &str,
    anchor: &Claim,
    numbers: [&str; 2],
) -> Result<Parsed, String> {
    Ok(match verb {
        Verb::Contradict | Verb::Add => (vec![statement(text, numbers[0], anchor)?], None),
        Verb::Split => (split_statements(text, numbers, anchor)?.to_vec(), None),
        Verb::Coin => {
            let coined = coinage(text)?;
            let claims = term_claims(&coined, numbers, anchor)?.to_vec();
            (claims, Some(coined))
        }
        _ => (Vec::new(), None),
    })
}

fn refuse_existing(
    c: &Connection,
    claims: &[Claim],
    coined: Option<&Coinage>,
) -> Result<(), Error> {
    if let Some(word) = coined.map(|k| palace_core::ids::entity_id(&k.word)) {
        let exists = c
            .prepare("SELECT 1 FROM kg.entities WHERE id=?1")?
            .exists([&word])?;
        if exists {
            return Err(Refused::error(format!("term {word} already exists")));
        }
    }
    for claim in claims {
        if lookup::claim(c, &claim.id)?.is_some() {
            return Err(Refused::error(format!(
                "claim {} already exists ({})",
                claim.id, claim.statement
            )));
        }
    }
    Ok(())
}

/// New triples: `contradicts` between the two claims' fact nodes, or each new claim's own edge.
fn proposals(verb: Verb, by: &str, old: &Claim, new: &[Claim]) -> Vec<Proposal> {
    match verb {
        Verb::Contradict => new
            .iter()
            .map(|n| edge(by, (&fact(old), &fact(n)), "contradicts", n, Some("fact")))
            .collect(),
        Verb::Add | Verb::Split => new
            .iter()
            .map(|n| edge(by, (&n.subject, &n.object), &n.predicate, n, None))
            .collect(),
        _ => Vec::new(),
    }
}

/// The graph node a claim is promoted to (SPEC §7: `contradicts` attaches to claims as facts).
pub fn fact(claim: &Claim) -> String {
    format!("claim {}", claim.id)
}

fn edge(
    by: &str,
    (from, to): (&str, &str),
    relation: &str,
    claim: &Claim,
    kind: Option<&str>,
) -> Proposal {
    let query: String = claim
        .quote
        .clone()
        .unwrap_or_else(|| claim.statement.clone())
        .chars()
        .take(250)
        .collect();
    Proposal {
        from: from.to_owned(),
        to: to.to_owned(),
        relation: relation.to_owned(),
        evidence: vec![EvidenceReference { query }],
        confidence: 0.60,
        supplied_by: by.to_owned(),
        from_type: kind.map(str::to_owned),
        to_type: kind.map(str::to_owned),
        claim: Some(claim.number.clone()),
        claim_id: Some(claim.id.clone()),
        transcript: None,
    }
}

/// Inserts each new claim (live, base 0.60 until checked) and its `(rollup, number)`.
fn insert_claims(c: &Connection, p: &Prepared, at: &str) -> Result<Vec<Value>, Error> {
    let l = &p.listed;
    let mut changes = Vec::new();
    for n in &p.new_claims {
        c.execute(
            "INSERT INTO claims(id,number,rollup,subject,predicate,object,date,source,quote,status,confidence_base,first_seen) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'live',0.60,?10)",
            params![n.id, n.number, l["rollup"].as_str(), n.subject, n.predicate, n.object, n.date, n.source, n.quote, at],
        )?;
        c.execute(
            "INSERT INTO claim_numbers(rollup,number,claim_id,date,thread) VALUES (?1,?2,?3,?4,?5)",
            params![
                l["rollup"].as_str(),
                n.number,
                n.id,
                l["date"].as_str(),
                l["thread"].as_str()
            ],
        )?;
        changes.push(change(
            "claim",
            &n.id,
            json!({"status":"undone","superseded_by":null}),
            json!({"status":"live","superseded_by":null}),
        ));
    }
    Ok(changes)
}

/// `coin`: the term entity (created by the caller through the graph) is recorded so undo can
/// retire it; nothing is deleted.
fn entity_change(c: &Connection, p: &Prepared, at: &str) -> Result<Option<Value>, Error> {
    let Some(coined) = &p.coinage else {
        return Ok(None);
    };
    let id = palace_core::ids::entity_id(&coined.word);
    let props: String = c.query_row(
        "SELECT properties FROM kg.entities WHERE id=?1",
        [&id],
        |r| r.get(0),
    )?;
    let after: Value = serde_json::from_str(&props)?;
    let mut before = after.clone();
    before["retired"] = json!({"at": at, "by": p.by, "reason": "coin undone"});
    Ok(Some(change("entity_properties", &id, before, after)))
}

/// The open findings the verb decides: the named finding, else every open one of a kind it decides.
fn decisions(c: &Connection, p: &Prepared, claim_id: &str, at: &str) -> Result<Vec<Value>, Error> {
    if p.grounding_failed {
        return Ok(Vec::new());
    }
    let filter = lookup::FindingFilter {
        open: true,
        claim: Some(claim_id.to_owned()),
        rollup: None,
    };
    let decision = json!({"by": p.by, "at": at, "verb": p.verb.name(), "note": p.text});
    Ok(lookup::findings(c, &filter)?
        .iter()
        .filter(|f| p.decides(f))
        .map(|f| {
            let id = f["id"].as_str().unwrap_or_default();
            change("finding", id, Value::Null, decision.clone())
        })
        .collect())
}

impl Prepared {
    /// Whether this call decides the open finding `f`: the named one (a reversal only for a
    /// person), else every kind the verb decides for `by`.
    fn decides(&self, f: &Value) -> bool {
        let kind = f["kind"].as_str().unwrap_or_default();
        match &self.finding {
            Some(named) => {
                (f["id"] == named.as_str()
                    && (kind != "reversal" || self.verb.decides_for(kind, &self.by)))
                    || self.corrects_reversal(kind)
            }
            None => self.verb.decides_for(kind, &self.by),
        }
    }
}

impl Prepared {
    /// A person's verb that ends, corrects or confirms the claim decides its open reversal
    /// whatever the target named, so it leaves Fergus's queue (M4d-2d item 1, M4d-2e item 1).
    fn corrects_reversal(&self, kind: &str) -> bool {
        kind == "reversal" && self.verb.decides_for(kind, &self.by)
    }
}

/// The claim that replaces this one: the new claim of contradict/add/split/coin, or for
/// `expire` a later live claim of the same rollup with the same subject and predicate.
fn successor(c: &Connection, p: &Prepared, claim_id: &str) -> Result<Option<String>, Error> {
    if let Some(first) = p.new_claims.first() {
        return Ok(Some(first.id.clone()));
    }
    if p.verb != Verb::Expire {
        return Ok(None);
    }
    let later = c.query_row(
        "SELECT l.id FROM claims o JOIN claims l ON lower(l.subject)=lower(o.subject) AND l.predicate=o.predicate AND l.date>o.date AND l.status='live' AND l.id<>o.id WHERE o.id=?1 ORDER BY l.date LIMIT 1",
        [claim_id],
        |r| r.get(0),
    );
    match later {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn status_change(p: &Prepared, successor: Option<&str>) -> Option<Value> {
    let after = if p.supersede {
        "superseded"
    } else if p.disputes {
        "disputed"
    } else if p.undisputes {
        "live"
    } else {
        p.verb.status_after()?
    };
    let before = json!({"status": p.claim["status"], "superseded_by": p.claim["superseded_by"]});
    let replaced_by = if p.verb == Verb::Retract {
        None
    } else {
        successor
    };
    Some(change(
        "claim",
        p.claim["id"].as_str()?,
        before,
        json!({"status": after, "superseded_by": replaced_by}),
    ))
}

/// `confirm` counts a survived check; a `weaken` that decided no finding is its own decrement,
/// and so is every failed grounding attempt, open finding or not (M4d-2a item 11: C13 0.45 →
/// 0.30). Logged as a delta (item 2): undo subtracts it whatever watch has counted since.
fn counter_change(p: &Prepared, decided_any: bool) -> Option<Value> {
    let delta = match p.verb {
        Verb::Confirm => json!({"survived": 1, "weakened": 0}),
        Verb::Weaken if p.grounding_failed || (!decided_any && !p.disputes) => {
            json!({"survived": 0, "weakened": 1})
        }
        _ => return None,
    };
    Some(change(
        "claim_counters_delta",
        p.claim["id"].as_str()?,
        json!({"survived": 0, "weakened": 0}),
        delta,
    ))
}
