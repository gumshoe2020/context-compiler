//! `palace improve --digest --store <compiler.sqlite> [--since <date>] [--max 10]` (M4d-2d item 3;
//! SPEC §5.1 revised point 3): Callum's view of the corrections made without him. At most
//! `--max` lines: one per correction by the assistant (`… — undo e_…`; reply `undo e_…` to undo it),
//! one summary line for rule and judge events, one line `queue=… asks=… unsent=…`. No output (exit 0)
//! when the window holds nothing, so the wrapper sends no empty message. Never canaries or terms.
//! Read-only; prints, never sends. Exit 0 printed, 1 refused, 2 broken.
use crate::findings::QUEUE_KINDS;
use crate::improve::Options;
use palace_core::digest::Open;
use palace_store::{Error, Refused, claims::ClaimStore, lookup::FindingFilter};
use std::path::PathBuf;

pub fn run(o: &Options) -> i32 {
    match lines(o) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(e) => crate::improve::fail("palace improve --digest", e),
    }
}

fn lines(o: &Options) -> Result<Vec<String>, Error> {
    let store = ClaimStore::open(&store_path(o)?)?;
    let since = match o.since.as_deref() {
        Some(since) if palace_core::digest::valid_since(since) => since.to_owned(),
        Some(since) => {
            return Err(Refused::error(format!(
                "--since {since:?}: use YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS"
            )));
        }
        None => store.day_ago()?,
    };
    let events = store.digest_events(&since)?;
    let open = Open {
        queue: queued(&store)?,
        asks: store.open_asks()?.len(),
        unsent: store.unsent_asks()?.len(),
    };
    let mut lines = palace_core::digest::lines(&events, open, o.max);
    // SETTLE-1b (spec §8): one recall line when the window holds fidelity checks.
    let (window, month) = (
        store.fidelity_rows(Some(&since))?,
        store.fidelity_rows(None)?,
    );
    lines.extend(palace_core::recall::digest_line(&window, &month));
    Ok(lines)
}

/// `--store`, else the claim store beside `--kg` (`--digest`, `--unsent`, `--mark-sent`).
pub fn store_path(o: &Options) -> Result<PathBuf, Error> {
    let path = match (&o.store, &o.paths) {
        (Some(store), _) => store.clone(),
        (None, Some(paths)) => crate::rollup_commit::compiler_path(&paths.kg)?,
        (None, None) => return Err(Refused::error("needs --store (or --kg)")),
    };
    crate::paths::input(&path)
}

/// Open findings in Fergus's queue.
fn queued(store: &ClaimStore) -> Result<usize, Error> {
    let open = FindingFilter {
        open: true,
        ..Default::default()
    };
    let findings = store.findings(&open)?;
    Ok(findings
        .iter()
        .filter(|f| QUEUE_KINDS.contains(&f["kind"].as_str().unwrap_or_default()))
        .count())
}
