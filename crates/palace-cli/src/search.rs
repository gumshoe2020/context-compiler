//! Marker-aware memory search for adapters. Identity is supplied by the authenticated caller.
use clap::Args;
use palace_core::search_hit::{SearchHit, distinct_entries};
use palace_mempalace::MemPalace;
use palace_store::{Error, privacy};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Args)]
pub struct Options {
    /// Search as the single owner declared in people.json.
    #[arg(long, conflicts_with = "conversation")]
    owner: bool,
    #[arg(long)]
    store: PathBuf,
    #[arg(long)]
    palace: PathBuf,
    /// Semantic query, at most 250 characters.
    #[arg(long)]
    query: String,
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=100))]
    limit: u64,
    #[arg(long, default_value_t = 0.0)]
    max_distance: f64,
    #[arg(long, default_value_t = 10000, value_parser = clap::value_parser!(u64).range(1..))]
    timeout_ms: u64,
    #[arg(long, default_value = "mempalace-mcp")]
    mempalace_mcp: PathBuf,
}

pub fn run(o: Options) -> Result<(), Error> {
    let identity = privacy::identity();
    if o.owner && identity.conversation.is_some() {
        return Err("--owner cannot be used with --conversation".into());
    }
    let readers: Vec<String> = if o.owner {
        vec![
            identity
                .people
                .as_ref()
                .ok_or("--owner requires people.json")?
                .unique_owner()?,
        ]
    } else {
        let resolved = identity
            .people
            .as_ref()
            .zip(identity.conversation.as_deref())
            .map(|(people, conversation)| people.resolve(conversation, None));
        let Some(marker) = resolved.filter(|m| !m.quarantine && !m.people.is_empty()) else {
            eprintln!(
                "palace search: unknown or missing conversation identity; no results returned"
            );
            return Ok(());
        };
        marker.people.into_iter().collect()
    };
    if o.query.is_empty() || o.query.chars().count() > 250 || !o.max_distance.is_finite() {
        return Err(
            "search query must contain 1–250 characters and max-distance must be finite".into(),
        );
    }
    let hits = search(&o, &readers).map_err(|_| -> Error {
        // Transport errors may contain unfiltered server output. Never echo that to a reader.
        "memory search unavailable; no results returned".into()
    })?;
    println!("{}", json!({"results":hits}));
    Ok(())
}

fn search(o: &Options, readers: &[String]) -> Result<Vec<SearchHit>, Error> {
    let store = crate::paths::input(&o.store)?;
    let c = Connection::open_with_flags(store, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if !c
        .prepare("SELECT 1 FROM sqlite_master WHERE name='privacy_markers' AND type='table'")?
        .exists([])?
    {
        return Ok(Vec::new());
    }
    // Standard rule only: no same-source quarantine exception, owner override or wing policy.
    let predicate = privacy::visible("entry", "privacy_markers.id", readers);
    let admitted: BTreeSet<String> = c
        .prepare(&format!(
            "SELECT DISTINCT id FROM privacy_markers WHERE kind='entry' AND {predicate} ORDER BY id"
        ))?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    if admitted.is_empty() {
        return Ok(Vec::new());
    }
    let palace = crate::paths::input(&o.palace)?;
    let deadline = Instant::now() + Duration::from_millis(o.timeout_ms);
    let memory = MemPalace::open_unpinned(&palace, &o.mempalace_mcp, deadline)?;
    let allowed = |hit: &SearchHit| {
        // Only exact entry basenames can authorize returned chunks and their metadata.
        !hit.source_file.contains(['/', '\\'])
            && admitted.contains(
                hit.source_file
                    .strip_suffix(".md")
                    .unwrap_or(&hit.source_file),
            )
    };
    let mut hits = Vec::new();
    // Four calls at most, independent of the number of admitted entries. A full final
    // ranked window prevents hidden chunks from starving visible sources indefinitely.
    for window in [100, 400, 1600, i32::MAX as usize] {
        let candidates = memory.search_window(&o.query, window, o.max_distance, None, None)?;
        let exhausted = candidates.len() < window;
        hits = distinct_entries(candidates, o.limit as usize, &allowed);
        if exhausted || hits.len() >= o.limit as usize {
            break;
        }
    }
    Ok(hits)
}
