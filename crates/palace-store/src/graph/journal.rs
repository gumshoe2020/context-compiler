//! Which KG rows a transaction changes (M4d-2a.1 item 2): connection-local TEMP triggers on
//! `kg.triples` and `kg.entities` copy every insert and every changed column into
//! `temp.kg_journal`. The KG's schema is never touched (TEMP objects live in this connection).
use crate::Error;
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};

pub const TABLES: [&str; 2] = ["triples", "entities"];

/// Creates the journal table and triggers once per connection; clears the journal.
pub fn install(c: &Connection) -> Result<(), Error> {
    c.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS kg_journal(seq INTEGER PRIMARY KEY, tbl TEXT, key TEXT, col TEXT, before, after);
         DELETE FROM temp.kg_journal;",
    )?;
    for table in TABLES {
        let columns = columns(c, table)?;
        c.execute_batch(&insert_trigger(table, &columns))?;
        c.execute_batch(&update_trigger(table, &columns))?;
    }
    Ok(())
}

/// The KG table's column names.
pub fn columns(c: &Connection, table: &str) -> Result<Vec<String>, Error> {
    let mut s = c.prepare(&format!("PRAGMA kg.table_info({table})"))?;
    let names = s.query_map([], |r| r.get(1))?.collect::<Result<_, _>>()?;
    Ok(names)
}

fn insert_trigger(table: &str, columns: &[String]) -> String {
    let fields: Vec<String> = columns
        .iter()
        .map(|k| format!("'{k}', new.\"{k}\""))
        .collect();
    format!(
        "CREATE TEMP TRIGGER IF NOT EXISTS kg_journal_{table}_insert AFTER INSERT ON kg.{table} BEGIN
         INSERT INTO kg_journal(tbl,key,col,before,after) VALUES ('{table}', new.id, '*', NULL, json_object({}));
         END;",
        fields.join(",")
    )
}

fn update_trigger(table: &str, columns: &[String]) -> String {
    let rows: Vec<String> = columns
        .iter()
        .filter(|k| *k != "id")
        .map(|k| {
            format!(
                "INSERT INTO kg_journal(tbl,key,col,before,after) SELECT '{table}', new.id, '{k}', old.\"{k}\", new.\"{k}\" WHERE old.\"{k}\" IS NOT new.\"{k}\";"
            )
        })
        .collect();
    format!(
        "CREATE TEMP TRIGGER IF NOT EXISTS kg_journal_{table}_update AFTER UPDATE ON kg.{table} BEGIN {} END;",
        rows.join(" ")
    )
}

/// The rows journaled so far as `{table, key, column, before, after}` (a column changed twice
/// keeps its first `before` and last `after`; a change back to `before` is dropped), and clears
/// the journal. `column` `*` is an inserted row: `before` null, `after` the row.
pub fn take(c: &Connection) -> Result<Vec<Value>, Error> {
    let mut s = c.prepare("SELECT tbl,key,col,before,after FROM temp.kg_journal ORDER BY seq")?;
    let raw: Vec<Value> = s
        .query_map([], |r| {
            Ok(
                json!({"table": r.get::<_, String>(0)?, "key": r.get::<_, String>(1)?,
                "column": r.get::<_, String>(2)?, "before": value(r.get_ref(3)?),
                "after": value(r.get_ref(4)?)}),
            )
        })?
        .collect::<Result<_, _>>()?;
    c.execute("DELETE FROM temp.kg_journal", [])?;
    Ok(merged(raw))
}

fn merged(raw: Vec<Value>) -> Vec<Value> {
    let mut rows: Vec<Value> = Vec::new();
    for mut row in raw {
        if row["column"] == "*" {
            row["after"] =
                serde_json::from_str(row["after"].as_str().unwrap_or("{}")).unwrap_or(Value::Null);
        }
        let same = |r: &&mut Value| {
            r["table"] == row["table"] && r["key"] == row["key"] && r["column"] == row["column"]
        };
        match rows.iter_mut().find(same) {
            Some(earlier) => earlier["after"] = row["after"].take(),
            None => rows.push(row),
        }
    }
    rows.retain(|r| r["column"] == "*" || !crate::cas::same(&r["before"], &r["after"]));
    rows
}

/// A SQLite value as JSON (a blob, which the KG never stores, as null).
pub fn value(v: ValueRef) -> Value {
    match v {
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Null | ValueRef::Blob(_) => Value::Null,
    }
}
