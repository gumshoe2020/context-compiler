//! Lock contention is "retry later" (M5a item 3, review S-1): SQLite's `database is locked` /
//! `database table is locked` after the busy timeout, or another process holding a writer lock
//! file, is not a broken run. `named` turns such an error into a typed `Busy` that names the
//! database held (every command maps it to exit 3; palace-mcp to a retryable error).
use crate::Error;
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

/// Another connection held a database past the busy timeout: retry later.
#[derive(Debug)]
pub struct Busy(pub String);
impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "retry later: {}", self.0)
    }
}
impl std::error::Error for Busy {}
impl Busy {
    pub fn is(error: &Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}

/// The databases this process has opened, by role, so a lock error can name the one held.
static OPENED: Mutex<Vec<(&'static str, PathBuf)>> = Mutex::new(Vec::new());

pub(crate) fn opened(role: &'static str, path: &Path) {
    if let Ok(mut opened) = OPENED.lock()
        && !opened.iter().any(|(_, p)| p == path)
    {
        opened.push((role, path.to_owned()));
    }
}

/// Whether `error` is lock contention (by SQLite's code where it survives, else by its message).
pub fn locked(error: &Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(failure, _)) =
        error.downcast_ref::<rusqlite::Error>()
        && matches!(
            failure.code,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
        )
    {
        return true;
    }
    let text = error.to_string();
    [
        "database is locked",
        "database table is locked",
        "writer lock",
    ]
    .iter()
    .any(|m| text.contains(m))
}

/// `error` as a `Busy` naming the database another connection holds (every opened one when the
/// holder has already let go); any other error unchanged.
pub fn named(error: Error) -> Error {
    if Busy::is(&error) || !locked(&error) {
        return error;
    }
    let opened = OPENED.lock().map(|o| o.clone()).unwrap_or_default();
    let held: Vec<&(&str, PathBuf)> = opened.iter().filter(|(_, path)| held(path)).collect();
    let named = if held.is_empty() {
        opened.iter().collect()
    } else {
        held
    };
    let names: Vec<String> = named
        .iter()
        .map(|(role, path)| format!("{role} {}", path.display()))
        .collect();
    let names = if names.is_empty() {
        "a database".to_owned()
    } else {
        names.join(" or ")
    };
    Box::new(Busy(format!(
        "{names} is locked by another writer ({error})"
    )))
}

/// Whether another connection holds `path` so that even a reader must wait (M5b-1 item 9, review
/// S-7). Never opens a database read-write. A WAL database is only read for its header: a WAL
/// writer blocks no reader, so no read-only probe can single it out, and opening one (even
/// read-only) may create its -wal and -shm; `named` then names every database the process opened.
/// A rollback-journal database is opened read-only and a zero-timeout deferred read takes only a
/// shared lock: an exclusive or pending holder makes it fail.
fn held(path: &Path) -> bool {
    if wal_mode(path).unwrap_or(true) {
        return false;
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(c) = Connection::open_with_flags(path, flags) else {
        return false;
    };
    c.busy_timeout(Duration::ZERO).is_ok()
        && c.execute_batch("BEGIN; SELECT count(*) FROM sqlite_master; ROLLBACK;")
            .is_err_and(|e| locked(&e.into()))
}

/// The database header's file-format bytes (18 and 19) say WAL; None when unreadable.
fn wal_mode(path: &Path) -> Option<bool> {
    use std::io::Read;
    let mut header = [0u8; 20];
    std::fs::File::open(path)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    Some(header[18] == 2 || header[19] == 2)
}

#[cfg(test)]
mod tests {
    use super::held;
    use rusqlite::Connection;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/busy-probe-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("db.sqlite3")
    }

    fn stamps(db: &Path) -> Vec<String> {
        let dir = db.parent().unwrap();
        let mut all: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                format!(
                    "{:?} {:?}",
                    e.file_name(),
                    e.metadata().unwrap().modified().unwrap()
                )
            })
            .collect();
        all.sort();
        all
    }

    /// M5b-1 item 9 (review S-7): the probe writes, creates and touches nothing; it identifies a
    /// reader-blocking holder of a rollback-journal database and no WAL writer.
    #[test]
    fn the_busy_probe_is_read_only() {
        let wal = scratch("wal");
        let c = Connection::open(&wal).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x);")
            .unwrap();
        drop(c);
        for side in ["-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{side}", wal.display()));
        }
        let before = stamps(&wal);
        assert!(!held(&wal));
        assert_eq!(
            stamps(&wal),
            before,
            "nothing created or touched beside a WAL database"
        );
        let writer = Connection::open(&wal).unwrap();
        writer
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO t VALUES (1);")
            .unwrap();
        assert!(!held(&wal), "a WAL writer blocks no reader");
        drop(writer);

        let journal = scratch("journal");
        let c = Connection::open(&journal).unwrap();
        c.execute_batch("CREATE TABLE t(x);").unwrap();
        let before = stamps(&journal);
        assert!(!held(&journal));
        assert_eq!(stamps(&journal), before);
        c.execute_batch("BEGIN EXCLUSIVE; INSERT INTO t VALUES (1);")
            .unwrap();
        assert!(held(&journal), "an exclusive holder is identified");
        let _ = fs::remove_dir_all(wal.parent().unwrap());
        let _ = fs::remove_dir_all(journal.parent().unwrap());
    }
}
