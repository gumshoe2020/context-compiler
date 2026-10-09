use crate::{Error, Store};
use palace_core::traits::GraphStore;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
mod audit;
mod edges;
mod effects;
mod entities;
mod improve;
mod intent;
mod journal;
mod pending;
mod read;
mod reconcile;
mod settle;
mod terms;
mod write;
pub use edges::{cited_edges, own_edges};
pub use improve::{Applied, Prepared, Request, fact};
pub use reconcile::{Diverged, check as check_divergence, write as reconcile_write};
pub use settle::Choice;
pub use terms::TermChange;
pub struct Graph {
    store: Store,
    _lock: File,
    wal: PathBuf,
}
impl Graph {
    pub fn open(palace: &Path, kg: &Path, wal: Option<&Path>) -> Result<Self, Error> {
        if !kg.is_file() {
            return Err(format!(
                "KG {} must already exist; schema is never created or migrated",
                kg.display()
            )
            .into());
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(kg.with_extension("sqlite.lock"))?;
        crate::busy::opened("KG", kg);
        crate::writer_lock(&lock, std::time::Duration::from_secs(5))?;
        let store = Store::open_with_wait(palace, std::time::Duration::from_secs(5))?;
        attach(&store.connection, kg)?;
        let graph = Self {
            store,
            _lock: lock,
            wal: wal.map(Path::to_owned).unwrap_or_else(|| {
                kg.parent()
                    .unwrap_or(Path::new("."))
                    .join("wal/write_log.jsonl")
            }),
        };
        Ok(graph)
    }
    pub fn now(&self) -> Result<String, Error> {
        now(&self.store.connection)
    }
    /// Appends `payload` to the compiler store's hash-chained write log (which this graph holds
    /// locked), e.g. a per-relation `rollup_step` between gated writes.
    pub fn log(&mut self, payload: &Value) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        crate::write_log::append(&tx, &payload.to_string())?;
        tx.commit()?;
        Ok(())
    }
    pub fn verify_chain(&self) -> Result<(), Error> {
        self.store.verify_chain()
    }
    pub fn schema(&self) -> Result<Vec<(String, Option<String>)>, Error> {
        let mut s = self
            .store
            .connection
            .prepare("SELECT name,sql FROM kg.sqlite_master ORDER BY name")?;
        Ok(s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }
    pub fn call(&mut self, operation: &str, args: &Value) -> Result<Value, Error> {
        if !args.is_object() {
            return Err("arguments must be an object".into());
        }
        match operation {
            "query" => read::query(&self.store.connection, args),
            "entities" => entities::list(&self.store.connection, args),
            "timeline" => read::timeline(&self.store.connection, args),
            "stats" => read::stats(&self.store.connection),
            "add" | "invalidate" | "set_entity" | "retire_entity" => self.mutate(operation, args),
            _ => Err("unknown graph operation".into()),
        }
    }
    /// A graph write, guarded by an intent (M4d-2a.1 item 2) and audited in the same transaction.
    fn mutate(&mut self, operation: &str, args: &Value) -> Result<Value, Error> {
        intent::refuse_unsettled(&self.store.connection)?;
        let timestamp = self.now()?;
        let wal = self.wal.clone();
        let label = format!("kg_{operation}");
        let result = intent::guarded(
            &mut self.store.connection,
            (&label, &timestamp),
            None,
            |tx| {
                let result = match operation {
                    "add" => write::add(tx, args, &timestamp),
                    "invalidate" => write::invalidate(tx, args),
                    "set_entity" => entities::set(tx, args, &timestamp),
                    "retire_entity" => entities::retire(tx, args, &timestamp),
                    _ => unreachable!(),
                }?;
                audit::record(tx, &wal, operation, args, &result, &timestamp)?;
                Ok(result)
            },
        )?;
        let mut result = result;
        if let Err(error) = self.flush_wal() {
            result["ok"] = json!(true);
            result["wal_warning"] = json!(error.to_string());
        }
        Ok(result)
    }
}
impl GraphStore for Graph {
    fn query_entity(
        &self,
        name: &str,
        direction: palace_core::triple::Direction,
        as_of: Option<&str>,
    ) -> Result<Vec<palace_core::triple::Triple>, Error> {
        let value = read::query(
            &self.store.connection,
            &json!({"entity":name,"direction":direction.as_str(),"as_of":as_of}),
        )?;
        Ok(serde_json::from_value(value["facts"].clone())?)
    }
    fn list_entities(&self) -> Result<Vec<palace_core::entity::Entity>, Error> {
        Ok(serde_json::from_value(entities::list(
            &self.store.connection,
            &json!({}),
        )?)?)
    }
    fn entity(&self, name: &str) -> Result<Option<palace_core::entity::Entity>, Error> {
        entities::get(&self.store.connection, name)?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    type Error = Error;
    fn add_triple(&mut self, args: &Value) -> Result<Value, Error> {
        self.call("add", args)
    }
}
/// The KG attached as `kg` (its schema checked, never changed) and the compiler's graph tables.
fn attach(c: &Connection, kg: &Path) -> Result<(), Error> {
    c.execute(
        "ATTACH DATABASE ? AS kg",
        [kg.to_str().ok_or("non-UTF8 KG path")?],
    )?;
    c.prepare("SELECT source_drawer_id,adapter_name FROM kg.triples LIMIT 0")?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS graph_wal_outbox (sequence INTEGER PRIMARY KEY, path TEXT NOT NULL, line TEXT NOT NULL);")?;
    c.execute_batch(intent::SCHEMA)?;
    Ok(())
}
pub(crate) fn now(c: &Connection) -> Result<String, Error> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let date: String = c.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%S',?,'unixepoch')",
        [seconds.as_secs() as i64],
        |r| r.get(0),
    )?;
    Ok(format!("{date}.{:06}Z", seconds.subsec_micros()))
}
/// Today in local time, as the KG's `valid_to` dates are written.
pub(crate) fn day(c: &Connection) -> Result<String, Error> {
    Ok(c.query_row("SELECT date('now','localtime')", [], |r| r.get(0))?)
}
fn properties(value: Option<String>) -> Value {
    value
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Graph-only readers never open the palace, acquire advisory locks, or flush the audit WAL.
pub struct GraphReader {
    connection: Connection,
}
impl GraphReader {
    pub fn open(kg: &Path) -> Result<Self, Error> {
        let path = kg
            .canonicalize()
            .map_err(|e| format!("KG {}: {e}", kg.display()))?;
        crate::busy::opened("KG", &path);
        let encoded: String = path
            .to_str()
            .ok_or("non-UTF8 KG path")?
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"/-_.".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let connection = Connection::open_in_memory()?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute(
            "ATTACH DATABASE ? AS kg",
            [format!("file:{encoded}?mode=ro")],
        )?;
        connection.prepare("SELECT id,name,type,properties,created_at FROM kg.entities LIMIT 0")?;
        connection.prepare("SELECT subject,predicate,object,confidence,source_closet,valid_from,valid_to FROM kg.triples LIMIT 0")?;
        Ok(Self { connection })
    }
    pub fn call(&self, operation: &str, args: &Value) -> Result<Value, Error> {
        if !args.is_object() {
            return Err("arguments must be an object".into());
        }
        match operation {
            "query" => read::query(&self.connection, args),
            "entities" => entities::list(&self.connection, args),
            "timeline" => read::timeline(&self.connection, args),
            "stats" => read::stats(&self.connection),
            _ => Err("unknown graph read operation".into()),
        }
    }
}

impl GraphStore for GraphReader {
    type Error = Error;

    fn query_entity(
        &self,
        name: &str,
        direction: palace_core::triple::Direction,
        as_of: Option<&str>,
    ) -> Result<Vec<palace_core::triple::Triple>, Error> {
        let value = read::query(
            &self.connection,
            &json!({"entity":name,"direction":direction.as_str(),"as_of":as_of}),
        )?;
        Ok(serde_json::from_value(value["facts"].clone())?)
    }
    fn list_entities(&self) -> Result<Vec<palace_core::entity::Entity>, Error> {
        Ok(serde_json::from_value(entities::list(
            &self.connection,
            &json!({}),
        )?)?)
    }
    fn entity(&self, name: &str) -> Result<Option<palace_core::entity::Entity>, Error> {
        entities::get(&self.connection, name)?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    fn add_triple(&mut self, _: &Value) -> Result<Value, Error> {
        Err("read-only graph".into())
    }
}

impl palace_core::traits::GraphHistory for Graph {
    fn stats(&self) -> Result<Value, Error> {
        read::stats(&self.store.connection)
    }
    fn timeline(&self, entity: Option<&str>) -> Result<Vec<palace_core::triple::Triple>, Error> {
        let value = read::timeline(&self.store.connection, &json!({"entity":entity}))?;
        Ok(serde_json::from_value(value["timeline"].clone())?)
    }
}

impl palace_core::traits::GraphHistory for GraphReader {
    fn stats(&self) -> Result<Value, Error> {
        read::stats(&self.connection)
    }
    fn timeline(&self, entity: Option<&str>) -> Result<Vec<palace_core::triple::Triple>, Error> {
        let value = read::timeline(&self.connection, &json!({"entity":entity}))?;
        Ok(serde_json::from_value(value["timeline"].clone())?)
    }
}

impl palace_core::traits::GraphMutation for Graph {
    fn invalidate(&mut self, subject: &str, predicate: &str, object: &str) -> Result<Value, Error> {
        self.call(
            "invalidate",
            &json!({"subject":subject,"predicate":predicate,"object":object}),
        )
    }
    fn retire_entity(&mut self, name: &str, reason: &str, by: &str) -> Result<Value, Error> {
        self.call(
            "retire_entity",
            &json!({"name":name,"reason":reason,"by":by}),
        )
    }
}

pub use audit::verify_single_write;
