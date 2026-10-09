//! A verb's edge effects (M4d-2a item 1, review S-2, SPEC §2.3 amendment d): a verb ends or
//! re-scores only the triples its claim CREATED (provenance `claim_id`). A triple the claim merely
//! cites keeps its `valid_to`, confidence and provenance; a verb that ends the claim records a
//! `citation_withdrawn` change instead, which undo reverses.
use super::improve::Prepared;
use crate::{Error, events::change};
use palace_core::improve::Verb;
use rusqlite::Connection;
use serde_json::{Value, json};

/// The edge changes of `p` on `claim_id`: `valid_to` on its own live edges and a withdrawn
/// citation per live cited edge for verbs that end the claim; the derived confidence on its own
/// edges for `confirm`/`weaken`; nothing otherwise.
pub fn changes(
    c: &Connection,
    p: &Prepared,
    claim_id: &str,
    day: &str,
) -> Result<Vec<Value>, Error> {
    let own = own_edges(c, claim_id)?;
    if p.verb.ends_edges() {
        let mut ended: Vec<Value> = own
            .iter()
            .map(|(id, _)| change("triple_valid_to", id, Value::Null, json!(day)))
            .collect();
        ended.extend(withdrawn(c, claim_id, day)?);
        return Ok(ended);
    }
    if !matches!(p.verb, Verb::Confirm | Verb::Weaken) {
        return Ok(Vec::new());
    }
    rescored(c, claim_id, &own)
}

fn rescored(c: &Connection, claim_id: &str, own: &[(String, f64)]) -> Result<Vec<Value>, Error> {
    let derived = crate::claims::derived(c, claim_id)?;
    Ok(own
        .iter()
        .filter(|(_, confidence)| *confidence != derived)
        .map(|(id, confidence)| change("triple_confidence", id, json!(confidence), json!(derived)))
        .collect())
}

/// One `citation_withdrawn` change per live triple the claim cites (and did not create).
fn withdrawn(c: &Connection, claim_id: &str, day: &str) -> Result<Vec<Value>, Error> {
    Ok(cited_edges(c, claim_id)?
        .iter()
        .map(|triple| {
            change(
                "citation_withdrawn",
                triple,
                json!({"claim_id": claim_id, "withdrawn_at": null}),
                json!({"claim_id": claim_id, "withdrawn_at": day}),
            )
        })
        .collect())
}

/// `(triple id, confidence)` of every live triple whose provenance `claim_id` is this claim.
pub fn own_edges(c: &Connection, claim_id: &str) -> Result<Vec<(String, f64)>, Error> {
    let mut s = c.prepare(
        "SELECT id, confidence FROM kg.triples WHERE valid_to IS NULL AND
           (CASE WHEN json_valid(source_closet) THEN json_extract(source_closet,'$.claim_id') END)=?1 ORDER BY id",
    )?;
    let rows = s
        .query_map([claim_id], |r| {
            Ok((r.get(0)?, r.get::<_, Option<f64>>(1)?.unwrap_or(1.0)))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Live triples the claim cites (an active citation) whose provenance is not the claim itself.
pub fn cited_edges(c: &Connection, claim_id: &str) -> Result<Vec<String>, Error> {
    let mut s = c.prepare(
        "SELECT DISTINCT t.id FROM kg.triples t JOIN citations ci ON ci.triple_id=t.id
         WHERE ci.claim_id=?1 AND ci.withdrawn_at IS NULL AND t.valid_to IS NULL AND
           (CASE WHEN json_valid(t.source_closet) THEN json_extract(t.source_closet,'$.claim_id') END) IS NOT ?1
         ORDER BY t.id",
    )?;
    let rows = s
        .query_map([claim_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}
