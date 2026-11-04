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
    let marker = support_marker(c, args)?;
    crate::privacy::put(c, "triple", &id, &marker)?;
    Ok(
        json!({"success":true,"triple_id":id,"created":created,"confidence":confidence,"subject":super::entities::get(c,&subject)?,"object":super::entities::get(c,&object)?,"fact":format!("{subject} → {pred} → {object}")}),
    )
}
pub(super) fn support_marker(
    c: &Connection,
    args: &Value,
) -> Result<crate::privacy::Marker, Error> {
    let closet = optional(args, "source_closet", 65536)?;
    let source = optional(args, "source_file", 4096)?;
    let provenance: Value = closet
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let locator = provenance["transcript"]
        .as_str()
        .or(source)
        .unwrap_or_default();
    let original = crate::privacy::origin(c, locator)?;
    if let Some(entry) = provenance["evidence_entry"].as_str() {
        let cited = crate::privacy::origin(c, entry)?;
        return Ok(crate::privacy::Marker::meet(
            &[original, cited.clone()],
            &cited.source,
        ));
    }
    Ok(original)
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
    if let Some(session) = &crate::privacy::conversation_id() {
        let incoming = crate::privacy::conversation(session, "");
        let ids:Vec<String>=c.prepare("SELECT id FROM kg.triples WHERE subject=?1 AND predicate=?2 AND object=?3 AND valid_to IS NULL")?.query_map(params![sub,pred,obj],|r|r.get(0))?.collect::<Result<_,_>>()?;
        for id in ids {
            for marker in crate::privacy::markers(c, "triple", &id)? {
                if marker.private == incoming.private
                    && (!marker.private || marker.audience == incoming.audience)
                    && !incoming.quarantine
                {
                    c.execute("INSERT OR IGNORE INTO privacy_retired(kind,id,source) VALUES ('triple',?1,?2)",params![id,marker.source])?;
                }
            }
            let remains=c.prepare("SELECT 1 FROM privacy_markers m WHERE kind='triple' AND id=?1 AND NOT EXISTS (SELECT 1 FROM privacy_retired r WHERE r.kind=m.kind AND r.id=m.id AND r.source=m.source)")?.exists([&id])?;
            if !remains && !incoming.quarantine {
                c.execute(
                    "UPDATE kg.triples SET valid_to=?2 WHERE id=?1",
                    params![id, ended],
                )?;
            }
        }
        return Ok(json!({"success":true,"invalidated":true,"ended":ended}));
    }
    c.execute("UPDATE kg.triples SET valid_to=?1 WHERE subject=?2 AND predicate=?3 AND object=?4 AND valid_to IS NULL",params![ended,sub,pred,obj])?;
    Ok(
        json!({"success":true,"invalidated":true,"fact":format!("{subject} → {pred} → {object}"),"ended":ended}),
    )
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    #[test]
    fn duplicate_add_records_both_supports_without_changing_kg_schema() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(crate::privacy::SCHEMA).unwrap();
        c.execute_batch("ATTACH DATABASE ':memory:' AS kg;
            CREATE TABLE kg.entities(id TEXT PRIMARY KEY,name TEXT,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP);
            CREATE TABLE kg.triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,valid_from TEXT,valid_to TEXT,confidence REAL,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT);").unwrap();
        let private = "wren"
            .parse::<crate::privacy::Audience>()
            .unwrap()
            .marker("synthetic-private");
        let shared = crate::privacy::Marker::shared("synthetic-shared");
        crate::privacy::register(&c, "private.jsonl", &private).unwrap();
        crate::privacy::register(&c, "shared.jsonl", &shared).unwrap();
        let schema = || {
            c.query_row("SELECT group_concat(sql) FROM kg.sqlite_master", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
        };
        let before = schema();
        let args = |path: &str| json!({"subject":"Invented Finch","predicate":"uses","object":"cobalt heron","source_closet":json!({"transcript":format!("{path}:1")}).to_string()});
        let first = add(&c, &args("private.jsonl"), "2031-01-01").unwrap();
        let second = add(&c, &args("shared.jsonl"), "2031-01-02").unwrap();
        assert_eq!(first["created"], true);
        assert_eq!(second["created"], false);
        assert_eq!(first["triple_id"], second["triple_id"]);
        let id = first["triple_id"].as_str().unwrap();
        assert_eq!(crate::privacy::markers(&c, "triple", id).unwrap().len(), 2);
        assert!(!crate::privacy::private(&c, "triple", id).unwrap());
        assert_eq!(before, schema());

        // Repointing must inherit the selected entry, even with a shared transcript.
        crate::privacy::register(&c, "selected.md", &private).unwrap();
        let mut repointed = args("shared.jsonl");
        repointed["predicate"] = json!("selected");
        repointed["source_closet"] = json!(
            json!({"transcript":"shared.jsonl:1","evidence_entry":"selected.md"}).to_string()
        );
        let selected = add(&c, &repointed, "2031-01-03").unwrap();
        let markers =
            crate::privacy::markers(&c, "triple", selected["triple_id"].as_str().unwrap()).unwrap();
        assert_eq!(markers.len(), 1);
        assert!(markers[0].private);
        assert_eq!(markers[0].source, private.source);
    }
}
