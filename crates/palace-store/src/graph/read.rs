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
    readers: Option<&[String]>,
) -> Result<Vec<Value>, Error> {
    let order = if timeline {
        " ORDER BY t.valid_from ASC NULLS LAST LIMIT 100"
    } else {
        ""
    };
    let admit = readers
        .map(|r| crate::privacy::visible("triple", "t.id", r))
        .unwrap_or_else(|| "1".into());
    let conjunction = if condition.is_empty() { "WHERE" } else { "AND" };
    let sql = format!(
        "SELECT s.name,t.predicate,o.name,t.valid_from,t.valid_to,t.confidence,t.source_closet,t.id FROM kg.triples t JOIN kg.entities s ON s.id=t.subject JOIN kg.entities o ON o.id=t.object {condition} {conjunction} {admit}{order}"
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
        if readers.is_none() {
            let markers = crate::privacy::markers(c, "triple", &r.get::<_, String>(7)?)?;
            let markers: Vec<_> = markers
                .into_iter()
                .filter(|m| m.private || !m.people.is_empty())
                .collect();
            if !markers.is_empty() {
                row["privacy"] = json!(
                    markers
                        .iter()
                        .map(crate::privacy::Marker::json)
                        .collect::<Vec<_>>()
                );
                row["annotation"] = json!(
                    markers
                        .iter()
                        .map(crate::privacy::Marker::annotation)
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
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
    let readers: Option<Vec<String>> = args
        .get("_readers")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?;
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
            readers.as_deref(),
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
        "WHERE (t.subject=?1 OR t.object=?1)"
    } else {
        ""
    };
    let readers: Option<Vec<String>> = args
        .get("_readers")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?;
    let rows = facts(c, condition, id.as_deref(), true, readers.as_deref())?;
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

/// All aggregates use the same edge scope as query and traversal.
pub fn scoped_stats(c: &Connection, readers: Option<&[String]>) -> Result<Value, Error> {
    let Some(readers) = readers else {
        return stats(c);
    };
    let admitted = crate::privacy::visible("triple", "t.id", readers);
    let count = |condition: &str| -> Result<i64, Error> {
        Ok(c.query_row(
            &format!("SELECT count(*) FROM kg.triples t WHERE {admitted} {condition}"),
            [],
            |r| r.get(0),
        )?)
    };
    let triples = count("")?;
    let current = count("AND valid_to IS NULL")?;
    let entities: i64 = c.query_row(&format!("SELECT count(*) FROM kg.entities e WHERE EXISTS (SELECT 1 FROM kg.triples t WHERE {admitted} AND (t.subject=e.id OR t.object=e.id))"), [], |r| r.get(0))?;
    let types = c
        .prepare(&format!(
            "SELECT DISTINCT predicate FROM kg.triples t WHERE {admitted} ORDER BY predicate"
        ))?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"entities":entities,"triples":triples,"current_facts":current,"expired_facts":triples-current,"relationship_types":types}),
    )
}
