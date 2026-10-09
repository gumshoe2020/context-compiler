//! Error → canary (M4d item 7, M4d-2c item 2): every `contradict` by the person or the assistant (direct, or
//! through a `fix:`/`accept` reply) appends a canary built from the corrected claim, recording its
//! author, to the canaries-out file (default
//! `<palace>/canaries.json`; never under fixtures/); undoing that contradict removes it. Each
//! change is a line in the compiler's hash-chained write log. `watch` reads the file too.
use crate::improve::{Ctx, Options};
use palace_core::{
    auto::claim_of,
    canary::{Canary, from_correction, makes_canary},
};
use palace_store::{Error, Refused, claims::ClaimStore, events::undo_id, graph::Applied};
use serde_json::json;
use std::{fs, path::Path, path::PathBuf};

/// `--canaries-out`, else `<palace>/canaries.json` (the KG's directory without `--palace`);
/// refused anywhere under fixtures/ or outside the roots.
pub fn path(o: &Options) -> Result<PathBuf, Error> {
    let paths = o.graph_paths()?;
    let palace = match &paths.palace {
        Some(palace) => palace.clone(),
        None => paths.kg.parent().ok_or("KG parent missing")?.to_path_buf(),
    };
    let file = o
        .canaries_out
        .clone()
        .unwrap_or_else(|| palace.join("canaries.json"));
    crate::paths::database(&file).map_err(|e| Refused::error(format!("--canaries-out: {e}")))
}

/// After an event: a person's contradict adds its canary; an undo of one removes it.
pub fn follow(ctx: &Ctx, applied: &Applied) -> Result<(), Error> {
    if makes_canary(&applied.verb, &applied.by) {
        return add(ctx, applied);
    }
    if applied.verb == "undo" {
        return remove(ctx, &applied.event);
    }
    Ok(())
}

fn add(ctx: &Ctx, applied: &Applied) -> Result<(), Error> {
    let store = ClaimStore::open(&ctx.compiler()?)?;
    let claim = |id: Option<&str>| -> Result<_, Error> {
        let row = store
            .claim(id.unwrap_or_default())?
            .ok_or("contradict without its claims")?;
        Ok(claim_of(&row))
    };
    let (old, new) = (
        claim(applied.from_claim.as_deref())?,
        claim(applied.to_claim.as_deref())?,
    );
    drop(store);
    let canary = from_correction(&old, &new, (&applied.event, &applied.by));
    let mut all = read(&ctx.canaries_out)?;
    all.push(canary.clone());
    write(&ctx.canaries_out, &all)?;
    log(ctx, "canary_added", &canary)
}

/// Removes the canary whose origin this undo (`undo_id(origin) == event`) reverses.
fn remove(ctx: &Ctx, undo_event: &str) -> Result<(), Error> {
    let mut all = read(&ctx.canaries_out)?;
    let Some(at) = all.iter().position(|c| {
        c.origin
            .as_deref()
            .is_some_and(|o| undo_id(o) == undo_event)
    }) else {
        return Ok(());
    };
    let canary = all.remove(at);
    write(&ctx.canaries_out, &all)?;
    log(ctx, "canary_removed", &canary)
}

fn log(ctx: &Ctx, operation: &str, canary: &Canary) -> Result<(), Error> {
    let line = json!({"operation":operation,"canary":canary.id,"origin":canary.origin,"author":canary.author,"file":ctx.canaries_out.display().to_string()});
    ClaimStore::open(&ctx.compiler()?)?.log(&line)
}

/// The canaries in `file` (none when it does not exist yet).
pub fn read(file: &Path) -> Result<Vec<Canary>, Error> {
    match fs::read(file) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Replaces `file` atomically (a temporary beside it, then a rename).
fn write(file: &Path, all: &[Canary]) -> Result<(), Error> {
    let temporary = file.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_string_pretty(all)?)?;
    fs::rename(&temporary, file)?;
    Ok(())
}
