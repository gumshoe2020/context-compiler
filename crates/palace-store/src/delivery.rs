//! Ask delivery (M4d-2e item 2, review M-1): `asks.sent_at` marks an ask the wrapper has sent.
//! `unsent_asks` lists every open ask not yet sent, oldest first; `mark_sent` sets `sent_at` once
//! per ask (idempotent), refusing the whole call when a token names no ask. Each ask marked also
//! writes an `ask_sent` line to the hash-chained write log.
use crate::{Error, Refused, claims::ClaimStore, write_log};
use rusqlite::{Transaction, params};
use serde_json::{Value, json};

/// What `mark_sent` did for one token.
#[derive(Debug, PartialEq)]
pub struct Marked {
    pub token: String,
    /// The ask's `sent_at` (now, or the earlier one).
    pub sent_at: String,
    /// `false` when the ask was already marked sent (a no-op).
    pub newly: bool,
}

impl ClaimStore {
    /// Open asks (as `open_asks`) whose `sent_at` is NULL, oldest first.
    pub fn unsent_asks(&self) -> Result<Vec<Value>, Error> {
        Ok(self
            .open_asks()?
            .into_iter()
            .filter(|a| a["sent_at"].is_null())
            .collect())
    }

    /// Marks the asks named by `tokens` (`#f_<8 hex>`) sent; refuses (nothing written) when any
    /// token names no ask or more than one.
    pub fn mark_sent(&mut self, tokens: &[String]) -> Result<Vec<Marked>, Error> {
        let tx = self.store.connection.transaction()?;
        let at = crate::graph::now(&tx)?;
        let mut marked = Vec::new();
        for token in tokens {
            marked.push(mark_one(&tx, token, &at)?);
        }
        tx.commit()?;
        Ok(marked)
    }
}

fn mark_one(tx: &Transaction, token: &str, at: &str) -> Result<Marked, Error> {
    let finding = ask_named(tx, token)?;
    let newly = tx.execute(
        "UPDATE asks SET sent_at=?2 WHERE finding=?1 AND sent_at IS NULL",
        params![finding, at],
    )? == 1;
    if newly {
        let line = json!({"operation":"ask_sent","finding":finding,"token":token,"sent_at":at});
        write_log::append(tx, &line.to_string())?;
    }
    let sent_at = tx.query_row(
        "SELECT sent_at FROM asks WHERE finding=?1",
        [&finding],
        |r| r.get(0),
    )?;
    Ok(Marked {
        token: token.to_owned(),
        sent_at,
        newly,
    })
}

/// The finding of the one ask whose reply token is `token`.
fn ask_named(tx: &Transaction, token: &str) -> Result<String, Error> {
    let findings: Vec<String> = tx
        .prepare("SELECT finding FROM asks WHERE token=?1")?
        .query_map([token], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    match findings.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(Refused::error(format!("{token}: no ask has this token"))),
        _ => Err(Refused::error(format!("{token}: names more than one ask"))),
    }
}
