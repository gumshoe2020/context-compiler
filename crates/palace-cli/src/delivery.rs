//! `palace improve --unsent --store <compiler.sqlite>` and `palace improve --mark-sent <#f_…>…
//! --store <compiler.sqlite>` (M4d-2e item 2, review M-1): the wrapper's source of truth for ask
//! delivery. `--unsent` prints every open ask not yet sent, one Signal line each, oldest first
//! (nothing when there is none); `--mark-sent` prints `SENT <token> sent_at=<time>` (or
//! `SENT <token> already sent_at=<time>`, a no-op) per token. Prints, never sends. Exit 0,
//! 1 refused (an unknown token: nothing marked), 2 broken.
use crate::improve::Options;
use palace_store::{Error, claims::ClaimStore};

pub fn run(o: &Options) -> i32 {
    let lines = if o.unsent { unsent(o) } else { mark_sent(o) };
    match lines {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(e) => {
            let mode = if o.unsent { "unsent" } else { "mark-sent" };
            crate::improve::fail(&format!("palace improve --{mode}"), e)
        }
    }
}

fn store(o: &Options) -> Result<ClaimStore, Error> {
    ClaimStore::open(&crate::digest::store_path(o)?)
}

fn unsent(o: &Options) -> Result<Vec<String>, Error> {
    Ok(store(o)?
        .unsent_asks()?
        .iter()
        .filter_map(|a| a["line"].as_str().map(str::to_owned))
        .collect())
}

fn mark_sent(o: &Options) -> Result<Vec<String>, Error> {
    let marked = store(o)?.mark_sent(&o.mark_sent)?;
    Ok(marked
        .iter()
        .map(|m| {
            let already = if m.newly { "" } else { " already" };
            format!("SENT {}{already} sent_at={}", m.token, m.sent_at)
        })
        .collect())
}
