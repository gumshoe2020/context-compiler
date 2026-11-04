//! Destructive sanitization for offline backup copies only. The CLI owns path validation.
use crate::{Error, privacy};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Exclusions(pub BTreeMap<String, BTreeSet<String>>);
impl Exclusions {
    pub fn entries(&self) -> BTreeSet<String> {
        self.0.get("entry").cloned().unwrap_or_default()
    }
}

/// All supports count, including retired ones and supports also visible to other people.
/// Malformed metadata aborts before any deletion; quarantine is conservatively excluded.
pub fn plan(c: &Connection, person: &str) -> Result<Exclusions, Error> {
    // Validate source metadata during read-only planning as well as row supports.
    for row in c
        .prepare("SELECT marker FROM privacy_sources")?
        .query_map([], |r| r.get::<_, String>(0))?
    {
        privacy::from_json(&serde_json::from_str(&row?)?)?;
    }
    let mut plan = Exclusions::default();
    let mut q = c.prepare("SELECT kind,id,source,people,audience,private,quarantine FROM privacy_markers ORDER BY kind,id,source")?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            privacy::Marker {
                source: r.get(2)?,
                people: Default::default(),
                audience: Default::default(),
                private: r.get(5)?,
                quarantine: r.get(6)?,
            },
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (kind, id, mut marker, people, audience) = row?;
        marker.people = serde_json::from_str(&people)?;
        marker.audience = serde_json::from_str(&audience)?;
        if marker.audience.contains(person) || marker.quarantine {
            plan.0.entry(kind).or_default().insert(id);
        }
    }
    Ok(plan)
}

fn has(c: &Connection, table: &str) -> Result<bool, Error> {
    Ok(
        c.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")?
            .exists([table])?,
    )
}
fn delete_if_present(c: &Connection, table: &str, predicate: &str) -> Result<(), Error> {
    if has(c, table)? {
        c.execute(&format!("DELETE FROM {table} WHERE {predicate}"), [])?;
    }
    Ok(())
}

/// Both databases must be closed staged copies; `kg` is already attached. No migration,
/// model, filesystem discovery, backup creation or external write occurs here.
pub fn apply(c: &mut Connection, plan: &Exclusions, person: &str) -> Result<(), Error> {
    c.execute_batch(
        "PRAGMA secure_delete=ON; PRAGMA kg.secure_delete=ON;
        PRAGMA journal_mode=DELETE; PRAGMA kg.journal_mode=DELETE;",
    )?;
    let tx = c.transaction()?;
    tx.execute_batch("CREATE TEMP TABLE excluded(kind TEXT,id TEXT,PRIMARY KEY(kind,id));")?;
    for (kind, ids) in &plan.0 {
        for id in ids {
            tx.execute("INSERT INTO excluded VALUES (?1,?2)", params![kind, id])?;
        }
    }
    // Strip the extension when comparing entry IDs, whose markers normalize .md.
    tx.execute_batch(
        "INSERT OR IGNORE INTO excluded SELECT 'claim',id FROM claims WHERE
        replace(rollup,'.md','') IN (SELECT id FROM excluded WHERE kind='entry');
        INSERT OR IGNORE INTO excluded SELECT 'fidelity',file FROM fidelity WHERE
        replace(file,'.md','') IN (SELECT id FROM excluded WHERE kind='entry');
        INSERT OR IGNORE INTO excluded SELECT 'finding',id FROM findings WHERE claim IN
        (SELECT id FROM excluded WHERE kind='claim') OR replace(rollup,'.md','') IN
        (SELECT id FROM excluded WHERE kind='entry');",
    )?;
    delete_if_present(
        &tx,
        "citations",
        "claim_id IN (SELECT id FROM excluded WHERE kind='claim') OR triple_id IN (SELECT id FROM excluded WHERE kind='triple')",
    )?;
    delete_if_present(
        &tx,
        "claim_numbers",
        "claim_id IN (SELECT id FROM excluded WHERE kind='claim')",
    )?;
    delete_if_present(
        &tx,
        "survived_checks",
        "claim IN (SELECT id FROM excluded WHERE kind='claim')",
    )?;
    delete_if_present(
        &tx,
        "auto_marks",
        "claim IN (SELECT id FROM excluded WHERE kind='claim') OR finding IN (SELECT id FROM excluded WHERE kind='finding')",
    )?;
    delete_if_present(
        &tx,
        "asks",
        "claim IN (SELECT id FROM excluded WHERE kind='claim') OR finding IN (SELECT id FROM excluded WHERE kind IN ('finding','ask'))",
    )?;
    tx.execute(
        "DELETE FROM findings WHERE id IN (SELECT id FROM excluded WHERE kind='finding')",
        [],
    )?;
    tx.execute(
        "DELETE FROM claims WHERE id IN (SELECT id FROM excluded WHERE kind='claim')",
        [],
    )?;
    tx.execute(
        "DELETE FROM fidelity WHERE file IN (SELECT id FROM excluded WHERE kind='fidelity')",
        [],
    )?;
    delete_if_present(
        &tx,
        "open_items",
        "id IN (SELECT id FROM excluded WHERE kind='item')",
    )?;
    tx.execute(
        "DELETE FROM kg.triples WHERE id IN (SELECT id FROM excluded WHERE kind='triple')",
        [],
    )?;
    // Orphan graph entities can retain private names and properties after triple deletion.
    tx.execute("DELETE FROM kg.entities WHERE id NOT IN (SELECT subject FROM kg.triples UNION SELECT object FROM kg.triples)", [])?;
    // These payloads can contain full previous rows or verbatim text, without row markers.
    // Preserve their schema/triggers but discard the entire offline audit/recovery history.
    let triggers: Vec<(String,String)> = tx.prepare("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name IN ('write_log','improvement_events')")?
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<Result<_,_>>()?;
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))?;
    }
    for table in [
        "write_log",
        "improvement_events",
        "replies",
        "kg_intents",
        "graph_wal_outbox",
        "initiative_state",
    ] {
        delete_if_present(&tx, table, "1")?;
    }
    for (_, sql) in &triggers {
        tx.execute_batch(sql)?;
    }
    let mut sources = Vec::new();
    for row in tx
        .prepare("SELECT locator,marker FROM privacy_sources")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
    {
        let (locator, text) = row?;
        let marker = privacy::from_json(&serde_json::from_str(&text)?)?;
        if marker.audience.contains(person) || marker.quarantine {
            sources.push(locator);
        }
    }
    for locator in sources {
        tx.execute("DELETE FROM privacy_sources WHERE locator=?1", [locator])?;
    }
    for table in ["privacy_markers", "privacy_retired"] {
        tx.execute(&format!("DELETE FROM {table} WHERE EXISTS (SELECT 1 FROM excluded e WHERE e.kind={table}.kind AND e.id={table}.id)"), [])?;
    }
    tx.commit()?;
    // Remove deleted payloads from free pages in both artifacts, without keeping raw backups.
    c.execute_batch("VACUUM main; VACUUM kg;")?;
    Ok(())
}
