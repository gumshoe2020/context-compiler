//! Owner-only read-only overview. No entry content or identifying metadata is returned.
use clap::Args;
use palace_store::{Error, privacy};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::path::PathBuf;

#[derive(Args)]
pub struct Options {
    #[arg(long, required = true, conflicts_with = "conversation")]
    owner: bool,
    #[arg(long)]
    store: PathBuf,
    #[arg(long)]
    palace: PathBuf,
}

pub fn run(o: Options) -> Result<(), Error> {
    let identity = privacy::identity();
    if !o.owner || identity.conversation.is_some() {
        return Err("status requires owner-private identity".into());
    }
    identity
        .people
        .as_ref()
        .ok_or("status requires people.json")?
        .unique_owner()?;
    let c = Connection::open_with_flags(
        crate::paths::input(&o.store)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let count = |table: &str| -> Result<i64, Error> {
        Ok(c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
    };
    let entries: i64 = c.query_row(
        "SELECT count(DISTINCT id) FROM privacy_markers WHERE kind='entry'",
        [],
        |r| r.get(0),
    )?;
    let drawers = palace_mempalace::drawers::count(&crate::paths::input(&o.palace)?)?;
    println!(
        "{}",
        json!({"counts":{"entries":entries,"drawers":drawers,
        "claims":count("claims")?,"findings":count("findings")?,
        "improvement_events":count("improvement_events")?}})
    );
    Ok(())
}
