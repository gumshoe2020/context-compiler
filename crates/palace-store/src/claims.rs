//! Claim store (SPEC §2, M4b P7) in `compiler.sqlite`: every watch run records the claims it
//! checked and the findings it raised, so derived confidence decays across runs (SPEC §5.1).
use crate::{
    Error, Store,
    events::{self, Event, Side},
    write_log,
};
use palace_core::{
    claim::{Claim, confidence},
    finding::kind_of,
    ids::{length_prefixed, sha256},
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
pub struct ClaimStore {
    pub(crate) store: Store,
}
/// One rollup's claims and the watch rows produced for them (claim rows and malformed rows).
pub struct Checked<'a> {
    pub rollup: &'a str,
    /// The rollup's as-of date and thread (its title), for `<date> C7` lookups and IMPROVE tags.
    pub date: &'a str,
    pub thread: &'a str,
    /// `judge:<model>`: the checker named on a judge's finding.
    pub judge: &'a str,
    pub claims: &'a [Claim],
    pub rows: &'a mut [Value],
}
/// The distilled entry that states a batch of claims (the claims' `rollup` column).
pub struct DistilledEntry<'a> {
    pub file: &'a str,
    pub date: &'a str,
    /// The entry's title, as the claim numbers' thread.
    pub thread: &'a str,
}
/// One distilled claim as stored (spec §4.3).
pub struct Distilled<'a> {
    pub id: &'a str,
    /// `D1`, `D2`, … within the entry.
    pub number: &'a str,
    pub kind: &'a str,
    pub subject: &'a str,
    pub predicate: &'a str,
    pub object: &'a str,
    pub date: &'a str,
    pub quote: Option<&'a str>,
    /// `<palace copy path>:<line>`.
    pub transcript: &'a str,
    /// A `specific` written as a `concept` (spec §4.4): why.
    pub demoted: Option<&'a str>,
}
/// The rollup a batch of rows belongs to.
struct Meta<'a> {
    rollup: &'a str,
    date: &'a str,
    thread: &'a str,
    judge: &'a str,
}
impl ClaimStore {
    pub fn open(path: &Path) -> Result<Self, Error> {
        Ok(Self {
            store: Store::open_with_wait(path, Duration::from_secs(5))?,
        })
    }
    pub fn now(&self) -> Result<String, Error> {
        crate::graph::now(&self.store.connection)
    }
    /// Records the run and rewrites each claim row's `confidence` to the value derived from
    /// every run so far (survived checks and findings summed over history).
    pub fn record(&mut self, checked: Checked, at: &str) -> Result<usize, Error> {
        let tx = self.store.connection.transaction()?;
        let meta = Meta {
            rollup: checked.rollup,
            date: checked.date,
            thread: checked.thread,
            judge: checked.judge,
        };
        let mut findings = 0;
        for row in checked.rows.iter_mut() {
            findings += record_row(&tx, &meta, checked.claims, row, at)?;
        }
        write_log::append(
            &tx,
            &json!({"operation":"claims_record","rollup":checked.rollup,"claims":checked.claims.len(),"findings":findings,"at":at})
                .to_string(),
        )?;
        tx.commit()?;
        Ok(findings)
    }
    /// M4d-2b item 8: the `vocabulary` rows of one rollup (`palace_core::terms::gap_row`) as
    /// `vocabulary_gap` findings on claims already recorded (open ones are deduped as any finding).
    pub fn record_vocabulary(
        &mut self,
        rollup: &str,
        rows: &[Value],
        at: &str,
    ) -> Result<usize, Error> {
        let tx = self.store.connection.transaction()?;
        let mut recorded = 0;
        for row in rows
            .iter()
            .filter(|r| r["check"] == "vocabulary" && r["rollup"] == rollup)
        {
            let claim = row["claim_id"].as_str().unwrap_or_default();
            let finding = Finding {
                subject: claim,
                claim: Some(claim),
                rollup,
                kind: "vocabulary_gap",
                evidence: row["evidence"].clone(),
                proposal: row["proposal"].clone(),
                checker: "rule:terms",
            };
            record_finding(&tx, &finding, at)?;
            recorded += 1;
        }
        let line =
            json!({"operation":"vocabulary_record","rollup":rollup,"findings":recorded,"at":at});
        write_log::append(&tx, &line.to_string())?;
        tx.commit()?;
        Ok(recorded)
    }
    /// SETTLE-1b (spec §3.4, §4): the claims a distilled entry stated, each with its kind and
    /// transcript locator (idempotent per claim id); a demoted `specific` raises its finding.
    /// Returns the claims new to the store.
    pub fn record_distilled(
        &mut self,
        entry: &DistilledEntry,
        claims: &[Distilled],
        at: &str,
    ) -> Result<usize, Error> {
        let tx = self.store.connection.transaction()?;
        let mut added = 0;
        for c in claims {
            added += tx.execute(
                "INSERT OR IGNORE INTO claims(id,number,rollup,subject,predicate,object,date,source,quote,confidence_base,first_seen,kind,transcript) VALUES (?1,?2,?3,?4,?5,?6,?7,?3,?8,0.60,?9,?10,?11)",
                params![c.id, c.number, entry.file, c.subject, c.predicate, c.object, c.date, c.quote, at, c.kind, c.transcript],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO claim_numbers(rollup,number,claim_id,date,thread) VALUES (?1,?2,?3,?4,?5)",
                params![entry.file, c.number, c.id, entry.date, entry.thread],
            )?;
            if c.kind == "specific" {
                replace_specifics(&tx, c, at)?;
            }
            if let Some(reason) = c.demoted {
                let finding = Finding {
                    subject: c.id,
                    claim: Some(c.id),
                    rollup: entry.file,
                    kind: "unquoted_specific",
                    evidence: json!({"source":entry.file,"passage":c.quote.unwrap_or_default(),"locator":c.transcript,"reason":reason}),
                    proposal: Value::Null,
                    checker: "rule:distill",
                };
                record_finding(&tx, &finding, at)?;
            }
        }
        let line = json!({"operation":"distilled_claims","entry":entry.file,"claims":claims.len(),"added":added,"at":at});
        write_log::append(&tx, &line.to_string())?;
        tx.commit()?;
        Ok(added)
    }
    /// Every claim with its memory type (SETTLE-1b): id, kind, the stating entry, statement
    /// fields, quote, transcript locator, status and replacement, oldest first.
    pub fn typed_claims(&self) -> Result<Vec<Value>, Error> {
        let mut s = self.store.connection.prepare(
            "SELECT id,kind,rollup,subject,predicate,object,date,quote,transcript,status,superseded_by FROM claims ORDER BY first_seen, rowid",
        )?;
        let rows = s
            .query_map([], |r| {
                let t = |i: usize| r.get::<_, Option<String>>(i);
                Ok(json!({"id":t(0)?,"kind":t(1)?,"entry":t(2)?,"subject":t(3)?,"predicate":t(4)?,"object":t(5)?,"date":t(6)?,"quote":t(7)?,"transcript":t(8)?,"status":t(9)?,"superseded_by":t(10)?}))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
    /// The live specifics stated by these entries (see [`live_specifics_of`]).
    pub fn live_specifics(&self, entries: &[String]) -> Result<Vec<Value>, Error> {
        live_specifics_of(&self.store.connection, entries)
    }
    /// Appends one audited event (e.g. a rollup commit) to the hash-chained write log.
    pub fn log(&mut self, payload: &Value) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        write_log::append(&tx, &payload.to_string())?;
        tx.commit()?;
        Ok(())
    }
    /// Payloads of every write-log event with the given `operation`, oldest first.
    pub fn events(&self, operation: &str) -> Result<Vec<Value>, Error> {
        let mut s = self.store.connection.prepare(
            "SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')=?1 ORDER BY sequence",
        )?;
        let rows = s
            .query_map([operation], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.iter()
            .map(|p| serde_json::from_str(p).map_err(Into::into))
            .collect()
    }
    /// M4b.1 S-5: a rollup relation that resolved to an EXISTING triple is recorded as a citation
    /// of `claim_id`; the triple and its history are not touched. Idempotent per
    /// (triple, claim, rollup). Returns whether a new row was written.
    pub fn cite(&mut self, triple_id: &str, claim_id: &str, rollup: &str) -> Result<bool, Error> {
        let tx = self.store.connection.transaction()?;
        let at = crate::graph::now(&tx)?;
        let added = tx.execute(
            "INSERT OR IGNORE INTO citations(triple_id,claim_id,rollup,cited_at) VALUES (?1,?2,?3,?4)",
            params![triple_id, claim_id, rollup, at],
        )? == 1;
        if added {
            write_log::append(
                &tx,
                &json!({"operation":"citation","triple_id":triple_id,"claim_id":claim_id,"rollup":rollup,"cited_at":at})
                    .to_string(),
            )?;
        }
        tx.commit()?;
        Ok(added)
    }
    /// `(claim_id, rollup)` of every citation of a triple.
    pub fn citations(&self, triple_id: &str) -> Result<Vec<(String, String)>, Error> {
        let mut s = self
            .store
            .connection
            .prepare("SELECT claim_id, rollup FROM citations WHERE triple_id=?1 ORDER BY rowid")?;
        let rows = s
            .query_map([triple_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// How many of these claim ids the store holds.
    pub fn known(&self, ids: &[String]) -> Result<usize, Error> {
        let mut s = self
            .store
            .connection
            .prepare("SELECT 1 FROM claims WHERE id=?1")?;
        let mut count = 0;
        for id in ids {
            count += usize::from(s.exists([id])?);
        }
        Ok(count)
    }
    /// `(kind, seen_count)` of each open finding on a claim.
    pub fn open_findings(&self, claim_id: &str) -> Result<Vec<(String, i64)>, Error> {
        let mut s = self.store.connection.prepare(
            "SELECT kind, seen_count FROM findings WHERE claim=?1 AND decision IS NULL ORDER BY checked_at",
        )?;
        let rows = s
            .query_map([claim_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// `(claims, findings, improvement_events)` row counts.
    pub fn counts(&self) -> Result<(i64, i64, i64), Error> {
        let count = |table: &str| -> Result<i64, Error> {
            Ok(self.store.connection.query_row(
                &format!("SELECT count(*) FROM {table}"),
                [],
                |r| r.get(0),
            )?)
        };
        Ok((
            count("claims")?,
            count("findings")?,
            count("improvement_events")?,
        ))
    }
    pub fn verify_chain(&self) -> Result<(), Error> {
        self.store.verify_chain()
    }
    /// A claim, finding or `<rollup date> C7` target resolved to its claim (M4d item 1).
    pub fn resolve(&self, target: &str) -> Result<crate::lookup::Resolved, Error> {
        crate::lookup::resolve(&self.store.connection, target)
    }
    /// The latest event that names `finding` (for a decided finding: the one that decided it).
    pub fn deciding_event(&self, finding: &str) -> Result<Option<String>, Error> {
        Ok(self
            .store
            .connection
            .query_row(
                "SELECT id FROM improvement_events WHERE finding=?1 ORDER BY rowid DESC LIMIT 1",
                [finding],
                |r| r.get(0),
            )
            .optional()?)
    }
    /// Ids of the claims in `status` (e.g. `disputed`), sorted.
    pub fn with_status(&self, status: &str) -> Result<Vec<String>, Error> {
        let mut s = self
            .store
            .connection
            .prepare("SELECT id FROM claims WHERE status=?1 ORDER BY id")?;
        let ids = s
            .query_map([status], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids)
    }
    pub fn claim(&self, id: &str) -> Result<Option<Value>, Error> {
        crate::lookup::claim(&self.store.connection, id)
    }
    pub fn findings(&self, filter: &crate::lookup::FindingFilter) -> Result<Vec<Value>, Error> {
        crate::lookup::findings(&self.store.connection, filter)
    }
    /// Improvement events (id, verb, by, from_claim, to_claim, finding), oldest first.
    pub fn improvement_events(&self) -> Result<Vec<Value>, Error> {
        let mut s = self.store.connection.prepare(
            "SELECT id,verb,by,from_claim,to_claim,finding,at,axis_improved,reversible_by FROM improvement_events ORDER BY rowid",
        )?;
        let rows = s
            .query_map([], |r| {
                Ok(json!({"id":r.get::<_,String>(0)?,"verb":r.get::<_,String>(1)?,"by":r.get::<_,String>(2)?,"from_claim":r.get::<_,Option<String>>(3)?,"to_claim":r.get::<_,Option<String>>(4)?,"finding":r.get::<_,Option<String>>(5)?,"at":r.get::<_,String>(6)?,"axis_improved":r.get::<_,String>(7)?,"reversible_by":r.get::<_,Option<String>>(8)?}))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}
/// Spec §4.1: a specific is compressed by replacement. Each live specific with the same subject
/// and predicate (normalised as claim ids normalise them), another value and a date no later
/// than the new one's is marked `superseded` by it; both rows keep their dates, so history stays
/// answerable.
fn replace_specifics(tx: &Transaction, new: &Distilled, at: &str) -> Result<(), Error> {
    use palace_core::claim::{normalise_entity, normalise_predicate};
    let live: Vec<(String, String, String, String, String)> = tx
        .prepare("SELECT id,subject,predicate,object,date FROM claims WHERE kind='specific' AND status='live' AND id<>?1 ORDER BY rowid")?
        .query_map([new.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<Result<_, _>>()?;
    let (subject, predicate) = (
        normalise_entity(new.subject),
        normalise_predicate(new.predicate),
    );
    for (old, s, p, object, date) in live {
        let same = normalise_entity(&s) == subject && normalise_predicate(&p) == predicate;
        if !same || object.trim() == new.object.trim() || date.as_str() > new.date {
            continue;
        }
        tx.execute(
            "UPDATE claims SET status='superseded', superseded_by=?2 WHERE id=?1",
            params![old, new.id],
        )?;
        let line =
            json!({"operation":"specific_replaced","claim":old,"superseded_by":new.id,"at":at});
        write_log::append(tx, &line.to_string())?;
    }
    Ok(())
}
/// The live specifics stated by any of these entries (`<name>` or `<name>.md`), oldest first
/// (spec §5: a rollup of those entries may not restate their values).
pub fn live_specifics_of(
    c: &rusqlite::Connection,
    entries: &[String],
) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare(
        "SELECT id,subject,predicate,object,date,rollup FROM claims WHERE kind='specific' AND status='live' ORDER BY first_seen, rowid",
    )?;
    let rows: Vec<Value> = s
        .query_map([], |r| {
            let t = |i: usize| r.get::<_, String>(i);
            Ok(json!({"id":t(0)?,"subject":t(1)?,"predicate":t(2)?,"object":t(3)?,"date":t(4)?,"entry":t(5)?}))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            let entry = row["entry"].as_str().unwrap_or_default();
            let name = entry.strip_suffix(".md").unwrap_or(entry);
            entries
                .iter()
                .any(|e| e.strip_suffix(".md").unwrap_or(e) == name)
        })
        .collect())
}
fn record_row(
    tx: &Transaction,
    meta: &Meta,
    claims: &[Claim],
    row: &mut Value,
    at: &str,
) -> Result<usize, Error> {
    if row["status"] == "malformed" {
        return record_malformed(tx, meta.rollup, row, at);
    }
    let Some(claim) = claims.iter().find(|c| row["claim"] == c.number.as_str()) else {
        return Ok(0);
    };
    let found = record_claim_finding(tx, meta, claim, row, at)?;
    record_claim(tx, meta, claim, row, at)?;
    Ok(usize::from(found))
}
fn record_malformed(tx: &Transaction, rollup: &str, row: &Value, at: &str) -> Result<usize, Error> {
    let locator = format!("{rollup}:{}", row["line"]);
    let mut evidence = row["evidence"].clone();
    evidence["raw"] = row["finding"]["raw"].clone();
    evidence["reason"] = row["finding"]["reason"].clone();
    let finding = Finding {
        subject: &locator,
        claim: None,
        rollup,
        kind: "malformed_claim",
        evidence,
        proposal: row["proposal"].clone(),
        checker: "rule:grammar",
    };
    record_finding(tx, &finding, at)?;
    Ok(1)
}
/// The finding a failing claim row raises, if any; `true` when one was raised (new or repeat).
/// A passing row (`ok`/`preserved`) closes the claim's open findings instead (M4d "Findings
/// close": `rule:recheck` with a `confirm` event).
fn record_claim_finding(
    tx: &Transaction,
    meta: &Meta,
    claim: &Claim,
    row: &Value,
    at: &str,
) -> Result<bool, Error> {
    let status = row["status"].as_str().unwrap_or_default();
    let Some(kind) = kind_of(status) else {
        if matches!(status, "ok" | "preserved") {
            close_on_recheck(tx, &claim.id, status, at)?;
        }
        return Ok(false);
    };
    let checker = match row["checker"].as_str() {
        Some("judge") => meta.judge,
        Some(named) => named,
        None => "rule:claims-check",
    };
    let finding = Finding {
        subject: &claim.id,
        claim: Some(&claim.id),
        rollup: meta.rollup,
        kind,
        evidence: evidence(claim, row),
        proposal: row["proposal"].clone(),
        checker,
    };
    record_finding(tx, &finding, at)?;
    Ok(true)
}
/// `{source, passage, locator}` from the check (SPEC §2.2) plus the row's status and reason.
fn evidence(claim: &Claim, row: &Value) -> Value {
    let mut evidence = row["evidence"].clone();
    if evidence.is_null() {
        evidence = json!({"source":claim.source,"passage":"","locator":null});
    }
    evidence["status"] = row["status"].clone();
    evidence["reason"] = row.get("reason").cloned().unwrap_or(Value::Null);
    evidence
}
/// Every open finding on `claim_id` is decided `confirm` by `rule:recheck`, one event each —
/// except a `reversal`, which only a reply or the person or the assistant closes (M4d-2a item 6, L-14), and a
/// `vocabulary_gap`, which the claim check does not test (M4d-2b item 8).
fn close_on_recheck(tx: &Transaction, claim_id: &str, status: &str, at: &str) -> Result<(), Error> {
    let open: Vec<String> = tx
        .prepare("SELECT id FROM findings WHERE claim=?1 AND decision IS NULL AND kind NOT IN ('reversal','vocabulary_gap') ORDER BY checked_at")?
        .query_map([claim_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for finding in open {
        let decision = json!({"by":"rule:recheck","at":at,"verb":"confirm","note":format!("check returned {status}")});
        let change = events::change("finding", &finding, Value::Null, decision);
        events::apply(tx, &change, Side::After)?;
        let text = format!("recheck: the claim's check returned {status}");
        let event = Event {
            by: "rule:recheck",
            verb: "confirm",
            from_claim: Some(claim_id),
            to_claim: None,
            finding: Some(&finding),
            text: &text,
            axis: &["currency"],
            changes: vec![change],
        };
        events::append(tx, &event, at)?;
    }
    Ok(())
}
/// A rollup that marks a claim superseded or corrected has resolved its open reversal, disputed or
/// not: the finding leaves Fergus's queue, decided by `rule:recheck` with status `superseded`, one
/// event each (M4d-2e item 1, review M-2).
fn close_on_marker(tx: &Transaction, rollup: &str, claim: &Claim, at: &str) -> Result<(), Error> {
    let (Some(by), status) = (&claim.superseded_by, claim_status(claim)) else {
        return Ok(());
    };
    let open: Vec<String> = tx
        .prepare("SELECT id FROM findings WHERE claim=?1 AND decision IS NULL AND kind='reversal' ORDER BY checked_at")?
        .query_map([&claim.id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let note = format!("{rollup} marks the claim {status} by {by}");
    for finding in open {
        let decision =
            json!({"by":"rule:recheck","at":at,"verb":"recheck","status":"superseded","note":note});
        let change = events::change("finding", &finding, Value::Null, decision);
        events::apply(tx, &change, Side::After)?;
        let event = Event {
            by: "rule:recheck",
            verb: "recheck",
            from_claim: Some(&claim.id),
            to_claim: None,
            finding: Some(&finding),
            text: &note,
            axis: &["currency"],
            changes: vec![change],
        };
        events::append(tx, &event, at)?;
    }
    Ok(())
}
/// Upsert the claim and its `(rollup, number)`, count the check, take the base from the
/// provenance found this run, and rewrite the row's confidence from its whole history. A marker (superseded/corrected) sets the status; an
/// improvement status (expired, retracted, …) is never reset to live by a later run.
fn record_claim(
    tx: &Transaction,
    meta: &Meta,
    claim: &Claim,
    row: &mut Value,
    at: &str,
) -> Result<(), Error> {
    let base = row["confidence_base"].as_f64().unwrap_or(0.60);
    upsert_claim(tx, meta.rollup, claim, base, at)?;
    tx.execute(
        "INSERT OR IGNORE INTO claim_numbers(rollup,number,claim_id,date,thread) VALUES (?1,?2,?3,?4,?5)",
        params![meta.rollup, claim.number, claim.id, meta.date, meta.thread],
    )?;
    let survived = survival(tx, meta, &claim.id, row, at)?;
    close_on_marker(tx, meta.rollup, claim, at)?;
    tx.execute(
        "UPDATE claims SET checks=checks+1, survived=survived+?2, last_checked=?3, status=CASE WHEN ?4='live' THEN status ELSE ?4 END, confidence_base=?5 WHERE id=?1",
        params![claim.id, survived, at, claim_status(claim), base],
    )?;
    let runs: i64 = tx.query_row("SELECT checks FROM claims WHERE id=?1", [&claim.id], |r| {
        r.get(0)
    })?;
    row["confidence"] = json!(derived(tx, &claim.id)?);
    row["confidence_runs"] = json!(runs);
    Ok(())
}
/// M4d-2c item 5 (review S-4): a `preserved` verdict earns a survived check once per (claim,
/// judge, sha256 of the source text): a repeated watch of the same source by the same judge adds
/// nothing; a new judge or an edited source counts once. 1 when this check is new, else 0.
fn survival(
    tx: &Transaction,
    meta: &Meta,
    claim_id: &str,
    row: &Value,
    at: &str,
) -> Result<i64, Error> {
    if row["status"] != "preserved" {
        return Ok(0);
    }
    let source = row["source_sha256"].as_str().unwrap_or_default();
    let added = tx.execute(
        "INSERT OR IGNORE INTO survived_checks(claim,checker,source_sha256,at) VALUES (?1,?2,?3,?4)",
        params![claim_id, meta.judge, source, at],
    )?;
    Ok(i64::try_from(added)?)
}
/// Derived confidence (M4d "One decrement per fact"): base + 0.05 per survived check (and
/// confirm) − 0.15 per distinct finding kind not resolved by `confirm` − 0.15 per `weaken` that
/// had no finding to decide.
pub fn derived(c: &rusqlite::Connection, claim_id: &str) -> Result<f64, Error> {
    derived_without(c, claim_id, "")
}
/// Derived confidence as if the finding `without` did not exist (M4d-2c item 5: the §5.1
/// second-check threshold is measured without the finding under decision).
pub fn derived_without(
    c: &rusqlite::Connection,
    claim_id: &str,
    without: &str,
) -> Result<f64, Error> {
    let (base, survived, weakened): (f64, i64, i64) = c.query_row(
        "SELECT confidence_base, survived, weakened FROM claims WHERE id=?1",
        [claim_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let findings: i64 = c.query_row(
        "SELECT count(DISTINCT kind) FROM findings WHERE claim=?1 AND id<>?2 AND (decision IS NULL OR json_extract(decision,'$.verb') IS NOT 'confirm')",
        [claim_id, without],
        |r| r.get(0),
    )?;
    Ok(confidence(
        base,
        survived as usize,
        (findings + weakened) as usize,
    ))
}
/// `corrected` for a reversal marker, `superseded` for a currency marker, else `live`.
fn claim_status(claim: &Claim) -> &'static str {
    match (&claim.superseded_by, claim.corrected) {
        (Some(_), true) => "corrected",
        (Some(_), false) => "superseded",
        (None, _) => "live",
    }
}
fn upsert_claim(
    tx: &Transaction,
    rollup: &str,
    c: &Claim,
    base: f64,
    at: &str,
) -> Result<(), Error> {
    tx.execute(
        "INSERT OR IGNORE INTO claims(id,number,rollup,subject,predicate,object,date,source,quote,superseded_by,confidence_base,first_seen) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![c.id, c.number, rollup, c.subject, c.predicate, c.object, c.date, c.source, c.quote, c.superseded_by, base, at],
    )?;
    Ok(())
}
fn finding_id(subject: &str, kind: &str, at: &str) -> String {
    format!(
        "f_{}",
        &sha256(length_prefixed(&[subject, kind, at]).as_bytes())[..12]
    )
}
struct Finding<'a> {
    /// Claim id, or `<rollup>:<line>` for a malformed line.
    subject: &'a str,
    claim: Option<&'a str>,
    rollup: &'a str,
    kind: &'a str,
    evidence: Value,
    proposal: Value,
    checker: &'a str,
}
/// M4b.1 S-7: open findings (no decision yet) are unique by (claim, kind) — by (rollup, kind,
/// raw line) for a malformed line. A repeat bumps `last_seen`, and `seen_count` once per run
/// (M4d: runs, not observations), and inserts nothing, so confidence decays once per NEW
/// finding (SPEC §5.1). A decided finding that recurs is a new finding, unless it repeats a
/// standing `rule:auto` event on an unchanged claim (M5b-5 item 1): then the decided finding is
/// seen again and nothing is inserted.
fn record_finding(tx: &Transaction, f: &Finding, at: &str) -> Result<(), Error> {
    let evidence = f.evidence.to_string();
    let repeated = tx.execute(
        "UPDATE findings SET seen_count=seen_count+(last_seen IS NOT ?4), last_seen=?4 WHERE decision IS NULL AND kind=?2 AND (claim=?1 OR (claim IS NULL AND ?1 IS NULL AND rollup=?3 AND json_extract(evidence,'$.raw')=?5))",
        params![f.claim, f.kind, f.rollup, at, f.evidence["raw"].as_str()],
    )?;
    if repeated > 0 || seen_again(tx, f, at)? {
        return Ok(());
    }
    tx.execute(
        "INSERT OR IGNORE INTO findings(id,claim,rollup,kind,evidence,proposal,checker,checked_at,last_seen,seen_count) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8,1)",
        params![finding_id(f.subject, f.kind, at), f.claim, f.rollup, f.kind, evidence, f.proposal.to_string(), f.checker, at],
    )?;
    Ok(())
}
/// A recurrence that repeats a standing `rule:auto` event: its decided finding seen again.
fn seen_again(tx: &Transaction, f: &Finding, at: &str) -> Result<bool, Error> {
    let Some(claim) = f.claim else {
        return Ok(false);
    };
    let Some((finding, _)) = crate::repeat::standing(tx, claim, f.kind)? else {
        return Ok(false);
    };
    tx.execute(
        "UPDATE findings SET seen_count=seen_count+(last_seen IS NOT ?2), last_seen=?2 WHERE id=?1",
        params![finding, at],
    )?;
    Ok(true)
}
/// Adds the S-7 columns to a `findings` table created before M4b.1, M4d's `weakened` counter to
/// `claims`, M4d-2a's `citations.withdrawn_at` and M4d-2c's `asks.proposal` (additive only;
/// compiler.sqlite is ours), and M4d-2e's `asks.sent_at` (the delivery marker).
pub(crate) fn migrate(c: &rusqlite::Connection) -> Result<(), Error> {
    add_column(c, ("claims", "weakened"), "INTEGER NOT NULL DEFAULT 0")?;
    add_column(c, ("citations", "withdrawn_at"), "TEXT")?;
    add_column(c, ("asks", "proposal"), "TEXT")?;
    add_column(c, ("asks", "sent_at"), "TEXT")?;
    if add_column(c, ("findings", "last_seen"), "TEXT")? {
        c.execute_batch("UPDATE findings SET last_seen=checked_at;")?;
    }
    add_column(c, ("findings", "seen_count"), "INTEGER NOT NULL DEFAULT 1")?;
    Ok(())
}
/// The copy [`memory_types`] takes of a store before it migrates it, beside the store.
pub fn memory_types_backup(store: &Path) -> std::path::PathBuf {
    store.with_extension("sqlite.before-memory-types")
}
/// SETTLE-1b item 1 (spec §4.3): `claims.kind` (`specific` | `concept`, every existing claim a
/// `concept`) and `claims.transcript` (the `<palace copy>:<line>` locator of a distilled claim).
/// The `rollup` column now reads "the entry that stated it": a rollup or a distilled entry.
/// A store that existed before this open is copied first (`VACUUM INTO`, the first copy kept),
/// so the migration can be undone; a store that has `kind` is left alone. Additive: a binary from
/// before it reads and writes the migrated store as before (its inserts get `concept`).
pub(crate) fn memory_types(
    c: &rusqlite::Connection,
    path: &Path,
    existed: bool,
) -> Result<(), Error> {
    if has_column(c, "claims", "kind")? {
        add_column(c, ("claims", "transcript"), "TEXT")?;
        return Ok(());
    }
    let backup = memory_types_backup(path);
    if existed && !backup.exists() {
        let to = backup.to_str().ok_or("store path is not UTF-8")?;
        c.execute("VACUUM INTO ?1", [to])?;
    }
    add_column(c, ("claims", "kind"), "TEXT NOT NULL DEFAULT 'concept'")?;
    add_column(c, ("claims", "transcript"), "TEXT")?;
    Ok(())
}
fn has_column(c: &rusqlite::Connection, table: &str, column: &str) -> Result<bool, Error> {
    Ok(c.prepare(&format!(
        "SELECT 1 FROM pragma_table_info('{table}') WHERE name=?1"
    ))?
    .exists([column])?)
}
/// `ALTER TABLE … ADD COLUMN` unless the column exists; whether it was added.
fn add_column(
    c: &rusqlite::Connection,
    (table, column): (&str, &str),
    ddl: &str,
) -> Result<bool, Error> {
    let exists = has_column(c, table, column)?;
    if !exists {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl};"))?;
    }
    Ok(!exists)
}
