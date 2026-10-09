//! A second connection holding a database's write lock, for the M5a lock tests and the m5a
//! doctor's `lock_retry` and `consistent_copy` gates — on COPIES only. The transaction is never
//! committed: dropping the holder rolls back whatever it wrote.
use crate::Error;
use rusqlite::Connection;
use std::path::Path;

pub struct Held {
    connection: Connection,
}

impl Held {
    /// `BEGIN IMMEDIATE` on `path` (which must exist): every other writer now waits.
    pub fn begin(path: &Path) -> Result<Self, Error> {
        if !path.is_file() {
            return Err(format!("{}: no such database", path.display()).into());
        }
        let connection = Connection::open(path)?;
        connection.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self { connection })
    }

    /// `BEGIN EXCLUSIVE` on `path` (which must exist): in rollback-journal mode every reader
    /// now waits too (M5b-1 item 8, the busy source).
    pub fn exclusive(path: &Path) -> Result<Self, Error> {
        if !path.is_file() {
            return Err(format!("{}: no such database", path.display()).into());
        }
        let connection = Connection::open(path)?;
        connection.execute_batch("BEGIN EXCLUSIVE")?;
        Ok(Self { connection })
    }

    /// A write inside the open transaction (seen by nobody else until it commits, which it
    /// never does).
    pub fn write(&self, sql: &str) -> Result<(), Error> {
        self.connection.execute_batch(sql)?;
        Ok(())
    }
}
