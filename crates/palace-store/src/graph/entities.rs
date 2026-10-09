use super::properties;
use crate::Error;
use palace_core::{
    graph_input::{name, optional},
    ids::entity_id,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
fn entity(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"type":row.get::<_,Option<String>>(2)?,"properties":properties(row.get(3)?),"created_at":row.get::<_,Option<String>>(4)?}),
    )
}
pub fn get(c: &Connection, name: &str) -> Result<Option<Value>, Error> {
    Ok(c.query_row(
        "SELECT id,name,type,properties,created_at FROM kg.entities WHERE id=?",
        [entity_id(name)],
        entity,
    )
    .optional()?)
}
pub fn list(c: &Connection, args: &Value) -> Result<Value, Error> {
    let names = match args.get("names") {
        None | Some(Value::Null) => None,
        Some(Value::Array(values)) if values.len() <= 1000 => Some(
            values
                .iter()
                .map(|v| name(&json!({"name":v}), "name").map(|n| entity_id(&n)))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => return Err("names must be an array of at most 1000 names".into()),
    };
    let mut s =
        c.prepare("SELECT id,name,type,properties,created_at FROM kg.entities ORDER BY id")?;
    let rows = s.query_map([], entity)?.collect::<Result<Vec<_>, _>>()?;
    Ok(json!(
        rows.into_iter()
            .filter(|r| names
                .as_ref()
                .is_none_or(|ns| ns.iter().any(|n| r["id"] == *n)))
            .collect::<Vec<_>>()
    ))
}
/// `at` is the intent's time: `created_at` is written from it, so the rehearsal and the real write
/// agree (M4d-2c item 6, review S-5: no false DIVERGED when a torn commit crosses a second).
pub fn set(c: &Connection, args: &Value, at: &str) -> Result<Value, Error> {
    let name = name(args, "name")?;
    let existing = get(c, &name)?;
    let old = existing.as_ref();
    let stored_name = old.and_then(|e| e["name"].as_str()).unwrap_or(&name);
    let kind = optional(args, "type", 128)?
        .filter(|s| !s.is_empty())
        .or_else(|| old.and_then(|e| e["type"].as_str()))
        .unwrap_or("unknown");
    let mut props = old
        .map(|e| e["properties"].clone())
        .unwrap_or_else(|| json!({}));
    if let Some(value) = args.get("properties").filter(|v| !v.is_null()) {
        let object = value.as_object().ok_or("properties must be an object")?;
        if value.to_string().len() > 65536 {
            return Err("properties exceeds 65536 bytes".into());
        }
        props.as_object_mut().unwrap().extend(object.clone());
    }
    let changed = old.is_none_or(|e| e["type"] != kind || e["properties"] != props);
    if changed {
        let row = [&entity_id(&name), stored_name, kind, &props.to_string()];
        write_row(c, row, (old.is_some(), at))?;
    }
    Ok(json!({"entity":get(c,&name)?,"changed":changed}))
}
/// An existing entity is UPDATEd (the same columns an `INSERT OR REPLACE` would rewrite, including
/// `created_at`) so the intent journal records each changed column with its real `before`; a
/// REPLACE fires only the insert trigger and left recovery blind (M4d-2b pre-item S-1).
/// `created_at` is `datetime(at)`: CURRENT_TIMESTAMP's format, the intent's time. It is rewritten
/// on UPDATE too, for parity: MemPalace 3.5.0's own `add_entity` (INSERT OR REPLACE) resets it on
/// every update (verified on a copy, M4d-2d item 4, review L-8).
fn write_row(
    c: &Connection,
    [id, name, kind, props]: [&str; 4],
    (exists, at): (bool, &str),
) -> Result<(), Error> {
    let sql = if exists {
        "UPDATE kg.entities SET name=?2,type=?3,properties=?4,created_at=datetime(?5) WHERE id=?1"
    } else {
        "INSERT INTO kg.entities(id,name,type,properties,created_at) VALUES (?1,?2,?3,?4,datetime(?5))"
    };
    c.execute(sql, params![id, name, kind, props, at])?;
    Ok(())
}
pub fn retire(c: &Connection, args: &Value, now: &str) -> Result<Value, Error> {
    let name = name(args, "name")?;
    let by = optional(args, "by", 4096)?.ok_or("by is required")?;
    let reason = optional(args, "reason", 4096)?.ok_or("reason is required")?;
    let at = optional(args, "at", 128)?
        .filter(|s| !s.is_empty())
        .unwrap_or(now);
    let Some(existing) = get(c, &name)? else {
        return Ok(json!({"retired":false,"missing":true,"name":name}));
    };
    let facts = super::read::query(c, &json!({"entity":name,"direction":"both"}))?;
    let mut live = Vec::new();
    for fact in facts["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["current"] == true)
    {
        let row = json!({"subject":fact["subject"],"predicate":fact["predicate"],"object":fact["object"],"valid_from":fact["valid_from"],"confidence":fact["confidence"]});
        if !live.contains(&row) {
            live.push(row);
        }
    }
    if !live.is_empty() {
        return Ok(json!({"retired":false,"refused":true,"name":name,"live_triples":live}));
    }
    if existing["properties"]
        .get("retired")
        .is_some_and(|v| !v.is_null() && *v != false)
    {
        return Ok(json!({"retired":true,"changed":false,"entity":existing}));
    }
    let result = set(
        c,
        &json!({"name":name,"properties":{"retired":{"at":at,"by":by,"reason":reason}}}),
        now,
    )?;
    Ok(json!({"retired":true,"changed":true,"entity":result["entity"]}))
}
