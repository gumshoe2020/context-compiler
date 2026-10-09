//! The read-only inputs of `palace doctor --shadow-diff` (M5a item 2): the palace write log's
//! lines, the compiler store's recovery lines (its hash chain verified first) and the KG's
//! triples. Nothing is locked, created or written: both databases are opened read-only, and a
//! database without -shm and -wal files (nobody has it open, so the main file holds every
//! committed row) is opened `immutable=1`, since a read-only open would create them.
use crate::Error;
use palace_core::shadow_diff::{Logged, Row};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

/// The graph's triples and the compiler log's recovery lines at or after `since`.
pub fn read(
    compiler: &Path,
    kg: &Path,
    since: Option<&str>,
) -> Result<(Vec<Row>, Vec<Logged>), Error> {
    let c = open(compiler)?;
    attach(&c, kg)?;
    crate::write_log::verify(&c)?;
    Ok((rows(&c)?, recoveries(&c, since)?))
}

/// The palace write log's lines in file order (none when the file does not exist).
pub fn wal_lines(wal: &Path) -> Result<Vec<Logged>, Error> {
    let text = match std::fs::read_to_string(wal) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", wal.display()).into()),
    };
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            let line = serde_json::from_str(l)
                .map_err(|e| format!("{} line {}: {e}", wal.display(), i + 1))?;
            Ok(Logged {
                source: format!("wal:{}", i + 1),
                line,
            })
        })
        .collect()
}

pub(crate) fn open(path: &Path) -> Result<Connection, Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    Ok(Connection::open_with_flags(uri(path)?, flags)?)
}

fn attach(c: &Connection, kg: &Path) -> Result<(), Error> {
    c.execute("ATTACH DATABASE ?1 AS kg", [uri(kg)?])?;
    Ok(())
}

/// `mode=ro` beside an open database's -wal or -shm; else `immutable=1`, because a read-only
/// connection would otherwise create both files (or fail where it cannot).
fn uri(path: &Path) -> Result<String, Error> {
    if !path.is_file() {
        return Err(format!("{}: no such database", path.display()).into());
    }
    let side = |suffix: &str| Path::new(&format!("{}{suffix}", path.display())).exists();
    let query = if side("-wal") || side("-shm") {
        "mode=ro"
    } else {
        "immutable=1"
    };
    Ok(format!("{}?{query}", file_uri(path)?))
}

fn file_uri(path: &Path) -> Result<String, Error> {
    let path = path.canonicalize()?;
    let encoded: String = path
        .to_str()
        .ok_or("non-UTF8 path")?
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    Ok(format!("file:{encoded}"))
}

fn rows(c: &Connection) -> Result<Vec<Row>, Error> {
    let mut s = c.prepare(
        "SELECT id,subject,predicate,object,valid_to,coalesce(confidence,1.0) FROM kg.triples ORDER BY id",
    )?;
    let rows = s
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                fact: (r.get(1)?, r.get(2)?, r.get(3)?),
                valid_to: r.get(4)?,
                confidence: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn recoveries(c: &Connection, since: Option<&str>) -> Result<Vec<Logged>, Error> {
    let mut s = c.prepare("SELECT sequence,payload FROM write_log ORDER BY sequence")?;
    let lines = s
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut found = Vec::new();
    for (sequence, payload) in lines {
        let Ok(line) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let recovery = line["operation"]
            .as_str()
            .is_some_and(|o| o.starts_with("recovered_"));
        let recent = since.is_none_or(|t| line["at"].as_str().is_some_and(|at| at >= t));
        if recovery && recent {
            found.push(Logged {
                source: format!("log:{sequence}"),
                line,
            });
        }
    }
    Ok(found)
}
