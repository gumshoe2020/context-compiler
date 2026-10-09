use palace_store::Error;
use serde_json::{Value, json};
pub fn arguments(args: &Value) -> Result<Value, Error> {
    if !args.is_object() || args.to_string().len() > 131072 {
        return Err("graph arguments must be an object of at most 128 KiB".into());
    }
    let mut args = args.clone();
    if let Some(value) = args.get("source_closet").and_then(Value::as_str)
        && let Ok(mut provenance) = serde_json::from_str::<Value>(value)
        && provenance.is_object()
        && provenance.get("written_via").is_some()
    {
        provenance["written_via"] = json!("mcp");
        args["source_closet"] = json!(provenance.to_string());
    }
    Ok(args)
}
pub fn capped(mut value: Value) -> Result<String, Error> {
    let mut dropped = 0;
    loop {
        let text = value.to_string();
        if text.len() <= 1024 * 1024 {
            return Ok(text);
        }
        let rows = if value.is_array() {
            value.as_array_mut()
        } else {
            ["facts", "timeline", "live_triples"]
                .into_iter()
                .find(|key| value[*key].is_array())
                .and_then(|key| value[key].as_array_mut())
        }
        .ok_or("graph response exceeds byte cap")?;
        if rows.pop().is_none() {
            return Err("graph response exceeds byte cap".into());
        }
        dropped += 1;
        if value.is_object() {
            value["truncated"] = json!(true);
            value["dropped_rows"] = json!(dropped);
        }
    }
}
pub fn schemas() -> Vec<Value> {
    [
        ("add",vec!["subject","predicate","object"],vec!["subject","predicate","object","valid_from","valid_to","source_closet","source_file","source_drawer_id","adapter_name","from_type","to_type"]),
        ("query",vec!["entity"],vec!["entity","direction","as_of"]),
        ("invalidate",vec!["subject","predicate","object"],vec!["subject","predicate","object","ended"]),
        ("timeline",vec![],vec!["entity"]),
        ("stats",vec![],vec![]),
        ("entities",vec![],vec![]),
        ("set_entity",vec!["name"],vec!["name","type"]),
        ("retire_entity",vec!["name","by","reason"],vec!["name","by","reason","at"]),
    ].into_iter().map(|(name,required,fields)| {
        let mut properties=serde_json::Map::new();
        for field in fields {properties.insert(field.into(),json!({"type":"string"}));}
        if name=="add" {properties.insert("confidence".into(),json!({"type":"number","default":1.0}));}
        if name=="entities" {properties.insert("names".into(),json!({"type":"array","items":{"type":"string"},"maxItems":1000}));}
        if name=="set_entity" {properties.insert("properties".into(),json!({"type":"object"}));}
        json!({"name":format!("mempalace_kg_{name}"),"inputSchema":{"type":"object","properties":properties,"required":required}})
    }).collect()
}
