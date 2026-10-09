use crate::Error;
use palace_core::{
    graph_input::{name, optional, temporal, temporal_key},
    ids::entity_id,
};
use rusqlite::Connection;
use serde_json::{Value, json};
fn facts(
    c: &Connection,
    condition: &str,
    id: Option<&str>,
    timeline: bool,
) -> Result<Vec<Value>, Error> {
    let order = if timeline {
        " ORDER BY t.valid_from ASC NULLS LAST LIMIT 100"
    } else {
        ""
    };
    let sql = format!(
        "SELECT s.name,t.predicate,o.name,t.valid_from,t.valid_to,t.confidence,t.source_closet FROM kg.triples t JOIN kg.entities s ON s.id=t.subject JOIN kg.entities o ON o.id=t.object {condition}{order}"
    );
    let mut s = c.prepare(&sql)?;
    let mut rows = if let Some(id) = id {
        s.query([id])?
    } else {
        s.query([])?
    };
    let mut result = Vec::new();
    while let Some(r) = rows.next()? {
        let to: Option<String> = r.get(4)?;
        let mut row = json!({"subject":r.get::<_,String>(0)?,"predicate":r.get::<_,String>(1)?,"object":r.get::<_,String>(2)?,"valid_from":r.get::<_,Option<String>>(3)?,"valid_to":to,"current":to.is_none()});
        if !timeline {
            row["confidence"] = json!(r.get::<_, Option<f64>>(5)?);
            row["source_closet"] = json!(r.get::<_, Option<String>>(6)?);
        }
        result.push(row);
    }
    Ok(result)
}
pub fn query(c: &Connection, args: &Value) -> Result<Value, Error> {
    let name = name(args, "entity")?;
    let as_of = temporal(args, "as_of")?;
    let direction = optional(args, "direction", 16)?.unwrap_or("both");
    if !["outgoing", "incoming", "both"].contains(&direction) {
        return Err("direction must be 'outgoing', 'incoming', or 'both'".into());
    }
    let mut result = Vec::new();
    for (dir, field, column) in [
        ("outgoing", "subject", "t.subject"),
        ("incoming", "object", "t.object"),
    ] {
        if direction != dir && direction != "both" {
            continue;
        }
        for mut row in facts(
            c,
            &format!("WHERE {column}=?1"),
            Some(&entity_id(&name)),
            false,
        )? {
            if as_of
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_some_and(|at| !visible(&row, at))
            {
                continue;
            }
            row["direction"] = json!(dir);
            row[field] = json!(name);
            result.push(row);
        }
    }
    Ok(json!({"entity":name,"as_of":as_of,"count":result.len(),"facts":result}))
}
fn visible(row: &Value, at: &str) -> bool {
    let at = temporal_key(at, false);
    row["valid_from"]
        .as_str()
        .is_none_or(|s| temporal_key(s, false) <= at)
        && row["valid_to"]
            .as_str()
            .is_none_or(|s| temporal_key(s, true) >= at)
}
pub fn timeline(c: &Connection, args: &Value) -> Result<Value, Error> {
    let name = optional(args, "entity", 128)?
        .map(|_| name(args, "entity"))
        .transpose()?;
    let id = name.as_deref().map(entity_id);
    let condition = if name.is_some() {
        "WHERE t.subject=?1 OR t.object=?1"
    } else {
        ""
    };
    let rows = facts(c, condition, id.as_deref(), true)?;
    Ok(json!({"entity":name.as_deref().unwrap_or("all"),"count":rows.len(),"timeline":rows}))
}
pub fn stats(c: &Connection) -> Result<Value, Error> {
    let count = |sql| c.query_row(sql, [], |r| r.get::<_, i64>(0));
    let entities = count("SELECT count(*) FROM kg.entities")?;
    let triples = count("SELECT count(*) FROM kg.triples")?;
    let current = count("SELECT count(*) FROM kg.triples WHERE valid_to IS NULL")?;
    let mut s = c.prepare("SELECT DISTINCT predicate FROM kg.triples ORDER BY predicate")?;
    let types = s
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"entities":entities,"triples":triples,"current_facts":current,"expired_facts":triples-current,"relationship_types":types}),
    )
}
