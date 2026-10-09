//! `palace improve escalate <finding> "<proposed claim>" --by <assistant>` (M4d-2c item 1; SPEC §5.1
//! revised 2026-09-27): Fergus hands a reversal from his queue to Callum with the claim he
//! proposes (M4d-2d item 2: reversals only; conflation and omission stay his, by split/add).
//! The proposal must parse under the claim grammar; the ask (kind `escalated`) carries it, and
//! its line the replies `keep / accept / fix: <text>`. Prints `ESCALATED <token> <line>`;
//! prints, never sends.
use crate::findings::{QUEUE_KINDS, signal_for};
use crate::improve::Options;
use palace_core::{finding::reply_token, reply::normalise};
use palace_store::{Error, Refused, auto::Ask, claims::ClaimStore};

pub fn run(o: &Options) -> i32 {
    match escalated(o) {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(e) => crate::improve::fail("palace improve escalate", e),
    }
}

fn escalated(o: &Options) -> Result<String, Error> {
    let assistant = palace_core::names::assistant();
    if o.by.as_deref() != Some(assistant) {
        return Err(Refused::error(format!(
            "escalate is {assistant}'s (--by {assistant})"
        )));
    }
    let proposal = normalise(o.text.as_deref().unwrap_or_default());
    if proposal.is_empty() {
        return Err(Refused::error("escalate needs the proposed claim"));
    }
    let compiler = crate::rollup_commit::compiler_path(&o.graph_paths()?.kg)?;
    let mut store = ClaimStore::open(&compiler)?;
    let (finding, claim) = queued(&store, o.target.as_deref().unwrap_or_default())?;
    crate::reply::grammatical(&claim, &proposal)
        .map_err(|problem| Refused::error(format!("proposal: {problem}")))?;
    let line = signal_for(&finding, &claim, Some(&proposal));
    let token = reply_token(finding["id"].as_str().unwrap_or_default());
    let ask = Ask {
        finding: finding["id"].as_str().unwrap_or_default(),
        claim: claim["id"].as_str().unwrap_or_default(),
        kind: "escalated",
        token: &token,
        line: &line,
        proposal: Some(&proposal),
    };
    let at = store.now()?;
    if !store.record_ask(&ask, &at)? {
        return Err(Refused::error(format!("{token} was asked already")));
    }
    Ok(format!("ESCALATED {token} {line}"))
}

/// The open queue finding `target` names, and its claim.
fn queued(
    store: &ClaimStore,
    target: &str,
) -> Result<(serde_json::Value, serde_json::Value), Error> {
    let id = store
        .resolve(target)?
        .finding
        .ok_or_else(|| Refused::error(format!("{target} names no finding")))?;
    let finding = crate::reply::finding(store, &id)?;
    let kind = finding["kind"].as_str().unwrap_or_default();
    if !finding["decision"].is_null() || !QUEUE_KINDS.contains(&kind) {
        return Err(Refused::error(format!(
            "{id} is not in {}'s queue (an open reversal, conflation or omission finding)",
            palace_core::names::assistant()
        )));
    }
    palace_core::reply::escalatable(kind).map_err(|why| Refused::error(format!("{id}: {why}")))?;
    let claim = store
        .claim(finding["claim"].as_str().unwrap_or_default())?
        .ok_or_else(|| Refused::error(format!("finding {id} has no claim")))?;
    Ok((finding, claim))
}
