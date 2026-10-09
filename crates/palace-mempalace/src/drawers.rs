//! Drawer count read straight from Chroma's sqlite, read-only (the MCP `mempalace_status` tool
//! takes MemPalace's writer lock under ~/.mempalace/locks, so it is avoided).
use crate::Error;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
pub fn count(palace: &Path) -> Result<usize, Error> {
    let c = Connection::open_with_flags(
        palace.join("chroma.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let n: i64 = c.query_row(
        "SELECT count(e.id) FROM collections c JOIN segments s ON s.collection=c.id \
         JOIN embeddings e ON e.segment_id=s.id \
         WHERE c.name='mempalace_drawers' AND s.scope='METADATA'",
        [],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}
