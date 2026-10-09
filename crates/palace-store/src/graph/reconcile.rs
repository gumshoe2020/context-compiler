//! `recover`'s first half (M4d-2a.1 item 2): every unsettled intent, newest first, is reconciled
//! row by row with `palace_core::reconcile::action`. A re-apply writes `after` again and logs
//! `recovered_reapply`; a restore writes `before` back and logs `recovered_rollback` — both as a
//! guarded transaction that also settles the intent. Append-only: an inserted triple is ended,
//! never deleted, and an inserted entity is kept. Any diverged row stops recovery untouched.
use super::{Graph, intent, journal::TABLES};
use crate::{Error, write_log};
use palace_core::reconcile::{Action, action, outcome};
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value as Sql};
use serde_json::{Value, json};

/// Diverged rows (`DIVERGED <table> <key> <column>` lines) of one intent: nothing was changed.
/// Its message names the one command that settles it (M4d-2c item 6).
#[derive(Debug)]
pub struct Diverged {
    pub intent: String,
    pub lines: Vec<String>,
}
impl std::fmt::Display for Diverged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hint = palace_core::reconcile::settle_hint(&self.intent);
        write!(f, "{}\n{hint}", self.lines.join("\n"))
    }
}
impl std::error::Error for Diverged {}
impl Diverged {
    pub fn of(error: &Error) -> Option<&Self> {
        error.downcast_ref::<Self>()
    }
}

/// One unsettled intent judged against the KG now.
pub struct Judged {
    pub id: String,
    pub operation: String,
    pub done: bool,
    pub actions: Vec<(Value, Action)>,
}

impl Judged {
    pub fn diverged(&self) -> Vec<String> {
        self.actions
            .iter()
            .filter(|(_, a)| *a == Action::Diverged)
            .map(|(r, _)| {
                format!(
                    "DIVERGED {} {} {}",
                    text(&r["table"]),
                    text(&r["key"]),
                    text(&r["column"])
                )
            })
            .collect()
    }
    pub fn outcome(&self) -> &'static str {
        let actions: Vec<Action> = self.actions.iter().map(|(_, a)| *a).collect();
        outcome(self.done, &actions)
    }
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("?")
}

/// Every unsettled intent, newest first, with each row's action (read-only).
pub fn judge(c: &Connection) -> Result<Vec<Judged>, Error> {
    intent::unsettled(c)?
        .into_iter()
        .map(|i| {
            let actions = i
                .rows
                .into_iter()
                .map(|row| {
                    let (b, a) = sides(c, &row)?;
                    Ok((row, action(i.done, b, a)))
                })
                .collect::<Result<_, Error>>()?;
            Ok(Judged {
                id: i.id,
                operation: i.operation,
                done: i.done,
                actions,
            })
        })
        .collect()
}

/// Whether the KG row now equals its `before` and its `after`.
fn sides(c: &Connection, row: &Value) -> Result<(bool, bool), Error> {
    let now = current(c, row)?;
    if row["column"] == "*" {
        let exists = now == true;
        return Ok((!exists, exists));
    }
    Ok((
        crate::cas::same(&now, &row["before"]),
        crate::cas::same(&now, &row["after"]),
    ))
}

/// The KG's value now for an intent row (a `*` row: whether it exists).
pub(super) fn current(c: &Connection, row: &Value) -> Result<Value, Error> {
    let (table, key) = (table(row)?, text(&row["key"]));
    if row["column"] == "*" {
        let exists = c
            .prepare(&format!("SELECT 1 FROM kg.{table} WHERE id=?1"))?
            .exists([key])?;
        return Ok(Value::Bool(exists));
    }
    let column = column(c, table, row)?;
    let now: Option<Value> = c
        .query_row(
            &format!("SELECT \"{column}\" FROM kg.{table} WHERE id=?1"),
            [key],
            |r| Ok(super::journal::value(r.get_ref(0)?)),
        )
        .optional()?;
    Ok(now.unwrap_or(Value::Null))
}

fn table(row: &Value) -> Result<&'static str, Error> {
    TABLES
        .into_iter()
        .find(|t| row["table"] == *t)
        .ok_or_else(|| format!("intent row on an unknown table {}", row["table"]).into())
}

/// The row's column, checked against the KG table (it is spliced into SQL).
fn column<'a>(c: &Connection, table: &str, row: &'a Value) -> Result<&'a str, Error> {
    let name = text(&row["column"]);
    if super::journal::columns(c, table)?.iter().any(|k| k == name) {
        return Ok(name);
    }
    Err(format!("intent row on an unknown column {table}.{name}").into())
}

impl Graph {
    /// Reconciles every unsettled intent (newest first); returns one `{intent, outcome, rows}`
    /// per intent that needed a write. A diverged row stops here with `Diverged`.
    pub fn reconcile(&mut self) -> Result<Vec<Value>, Error> {
        let mut acted = Vec::new();
        while let Some(judged) = judge(&self.store.connection)?.into_iter().next() {
            let lines = judged.diverged();
            if !lines.is_empty() {
                let intent = judged.id.clone();
                return Err(Box::new(Diverged { intent, lines }));
            }
            let outcome = judged.outcome();
            if matches!(outcome, "committed" | "dropped") {
                intent::settle(&self.store.connection, &judged.id, outcome)?;
                continue;
            }
            acted.push(self.repair(&judged, outcome)?);
        }
        if !acted.is_empty() {
            self.flush_wal()?;
        }
        Ok(acted)
    }

    /// Writes the rows back (guarded itself) and settles the intent in the same transaction.
    fn repair(&mut self, judged: &Judged, outcome: &str) -> Result<Value, Error> {
        let at = self.now()?;
        let rows: Vec<(Value, Action)> = judged
            .actions
            .iter()
            .filter(|(_, a)| *a != Action::Keep)
            .cloned()
            .collect();
        let line = json!({"operation": outcome, "intent": judged.id, "of": judged.operation,
            "rows": rows.iter().map(|(r, _)| r).collect::<Vec<_>>(), "at": at});
        let day = super::day(&self.store.connection)?;
        intent::guarded(
            &mut self.store.connection,
            (outcome, &at),
            Some((&judged.id, outcome)),
            |tx| {
                for (row, act) in rows.iter().rev() {
                    write(tx, row, *act, &day)?;
                }
                write_log::append(tx, &line.to_string())
            },
        )?;
        Ok(line)
    }
}

/// Re-applies `after` or restores `before` for one row.
pub fn write(c: &Connection, row: &Value, act: Action, day: &str) -> Result<(), Error> {
    let (table, key) = (table(row)?, text(&row["key"]));
    match (row["column"] == "*", act) {
        (true, Action::Reapply) => insert(c, table, &row["after"]),
        (true, _) if table == "triples" => {
            c.execute(
                "UPDATE kg.triples SET valid_to=?2 WHERE id=?1 AND valid_to IS NULL",
                params_from_iter([key, day]),
            )?;
            Ok(())
        }
        (true, _) => Ok(()),
        (false, act) => {
            let side = if act == Action::Reapply {
                "after"
            } else {
                "before"
            };
            let column = column(c, table, row)?;
            let sql = format!("UPDATE kg.{table} SET \"{column}\"=?2 WHERE id=?1");
            c.execute(
                &sql,
                params_from_iter([Sql::Text(key.to_owned()), sql_value(&row[side])]),
            )?;
            Ok(())
        }
    }
}

/// Inserts a logged row again (its every column, as logged).
fn insert(c: &Connection, table: &str, row: &Value) -> Result<(), Error> {
    let columns = super::journal::columns(c, table)?;
    let names: Vec<String> = columns.iter().map(|k| format!("\"{k}\"")).collect();
    let marks: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT OR IGNORE INTO kg.{table}({}) VALUES ({})",
        names.join(","),
        marks.join(",")
    );
    c.execute(
        &sql,
        params_from_iter(columns.iter().map(|k| sql_value(&row[k]))),
    )?;
    Ok(())
}

fn sql_value(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Number(n) if n.is_i64() => Sql::Integer(n.as_i64().unwrap_or_default()),
        Value::Number(n) => Sql::Real(n.as_f64().unwrap_or_default()),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

/// `palace doctor --check-divergence`: every unsettled intent (`PENDING <intent> <outcome>`) and
/// diverged row, read-only.
pub fn check(compiler: &std::path::Path, kg: &std::path::Path) -> Result<Vec<String>, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    let mut lines = Vec::new();
    for judged in judge(&c)? {
        let diverged = judged.diverged();
        let outcome = if diverged.is_empty() {
            judged.outcome()
        } else {
            "diverged"
        };
        lines.push(format!(
            "PENDING {} {} ({})",
            judged.id, outcome, judged.operation
        ));
        lines.extend(diverged);
    }
    Ok(lines)
}
