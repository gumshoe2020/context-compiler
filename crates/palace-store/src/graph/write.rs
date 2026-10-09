use crate::Error;
use palace_core::{
    graph_input::{interval, name, optional, predicate, temporal},
    ids::{entity_id, temporal_triple_id},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
pub fn add(c: &Connection, args: &Value, now: &str) -> Result<Value, Error> {
    let (subject, pred, object) = (
        name(args, "subject")?,
        predicate(args)?,
        name(args, "object")?,
    );
    let (from, to) = (temporal(args, "valid_from")?, temporal(args, "valid_to")?);
    interval(from.as_deref(), to.as_deref())?;
    let confidence = args.get("confidence").map_or(Ok(1.0), |v| {
        v.as_f64()
            .filter(|v| v.is_finite())
            .ok_or("confidence must be a finite number")
    })?;
    let closet = optional(args, "source_closet", 65536)?;
    let source = optional(args, "source_file", 4096)?;
    let drawer = optional(args, "source_drawer_id", 4096)?;
    let adapter = optional(args, "adapter_name", 128)?;
    for (name, field) in [(&subject, "from_type"), (&object, "to_type")] {
        if let Some(kind) = optional(args, field, 128)?.filter(|s| !s.is_empty()) {
            super::entities::set(c, &json!({"name":name,"type":kind}), now)?;
        }
        c.execute(
            "INSERT OR IGNORE INTO kg.entities(id,name) VALUES (?1,?2)",
            params![entity_id(name), name],
        )?;
    }
    let (sub, obj) = (entity_id(&subject), entity_id(&object));
    let old: Option<String> = c.query_row("SELECT id FROM kg.triples WHERE subject=?1 AND predicate=?2 AND object=?3 AND valid_to IS NULL", params![sub,pred,obj], |r| r.get(0)).optional()?;
    let created = old.is_none();
    let id = old.unwrap_or_else(|| temporal_triple_id(&sub, &pred, &obj, from.as_deref(), now));
    if created {
        c.execute("INSERT INTO kg.triples(id,subject,predicate,object,valid_from,valid_to,confidence,source_closet,source_file,source_drawer_id,adapter_name) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![id,sub,pred,obj,from,to,confidence,closet,source,drawer,adapter])?;
    }
    Ok(
        json!({"success":true,"triple_id":id,"created":created,"confidence":confidence,"subject":super::entities::get(c,&subject)?,"object":super::entities::get(c,&object)?,"fact":format!("{subject} → {pred} → {object}")}),
    )
}
pub fn invalidate(c: &Connection, args: &Value) -> Result<Value, Error> {
    let (subject, pred, object) = (
        name(args, "subject")?,
        predicate(args)?,
        name(args, "object")?,
    );
    let ended = temporal(args, "ended")?
        .filter(|s| !s.is_empty())
        .unwrap_or(c.query_row("SELECT date('now','localtime')", [], |r| r.get(0))?);
    let mut s = c.prepare("SELECT valid_from FROM kg.triples WHERE subject=?1 AND predicate=?2 AND object=?3 AND valid_to IS NULL")?;
    let (sub, obj) = (entity_id(&subject), entity_id(&object));
    for from in s.query_map(params![sub, pred, obj], |r| r.get::<_, Option<String>>(0))? {
        interval(from?.as_deref(), Some(&ended))?;
    }
    c.execute("UPDATE kg.triples SET valid_to=?1 WHERE subject=?2 AND predicate=?3 AND object=?4 AND valid_to IS NULL",params![ended,sub,pred,obj])?;
    Ok(
        json!({"success":true,"invalidated":true,"fact":format!("{subject} → {pred} → {object}"),"ended":ended}),
    )
}
