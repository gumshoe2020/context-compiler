#![forbid(unsafe_code)]
pub mod audit;
pub mod auto;
pub mod busy;
pub mod cas;
pub mod claims;
pub mod context;
pub mod crash;
pub mod delivery;
pub mod depends;
pub mod digest;
pub mod events;
pub mod fidelity;
pub mod graph;
pub mod graph_checks;
pub mod hold;
pub mod inspect;
pub mod lookup;
pub mod m3b_checks;
pub mod m5a_checks;
pub mod repeat;
pub mod shadow_diff;
pub mod terms;
mod write_log;
use rusqlite::Connection;
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
/// An improvement the store refuses (unknown target, invariant): exit 1, never "broken".
#[derive(Debug)]
pub struct Refused(pub String);
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "refused: {}", self.0)
    }
}
impl std::error::Error for Refused {}
impl Refused {
    pub fn error(reason: impl std::fmt::Display) -> Error {
        Box::new(Self(reason.to_string()))
    }
    pub fn is(error: &Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}
/// A call that has nothing to do (e.g. a second grounding attempt the same UTC day): exit 0, no
/// event (M4d-2a.1 item 3).
#[derive(Debug)]
pub struct Skipped(pub String);
impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Skipped {}
impl Skipped {
    pub fn error(reason: impl std::fmt::Display) -> Error {
        Box::new(Self(reason.to_string()))
    }
    pub fn is(error: &Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}
pub struct Store {
    connection: Connection,
    _lock: File,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, Error> {
        Self::open_with_wait(path, Duration::ZERO)
    }
    pub(crate) fn open_with_wait(path: &Path, wait: Duration) -> Result<Self, Error> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("sqlite.lock"))?;
        busy::opened("compiler store", path);
        writer_lock(&lock, wait)?;
        let existed = path.exists();
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        connection.execute_batch(include_str!("schema.sql"))?;
        claims::migrate(&connection)?;
        claims::memory_types(&connection, path, existed)?;
        let store = Self {
            connection,
            _lock: lock,
        };
        store.verify_chain()?;
        Ok(store)
    }
    pub fn verify_chain(&self) -> Result<(), Error> {
        write_log::verify(&self.connection)
    }
}
fn writer_lock(file: &File, wait: Duration) -> Result<(), Error> {
    let started = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(format!("writer lock unavailable ({error})").into()),
        }
    }
}
