use crate::{Error, write_log};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
pub fn verify(path: &Path) -> Result<(), Error> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    write_log::verify(&c)
}
pub fn rows(path: &Path) -> Result<Vec<serde_json::Value>, Error> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    write_log::verify(&c)?;
    let mut s = c.prepare("SELECT payload FROM write_log WHERE json_extract(payload,'$.operation') LIKE 'kg_%' ORDER BY sequence")?;
    let rows = s
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.iter()
        .map(|s| serde_json::from_str(s).map_err(Into::into))
        .collect()
}
