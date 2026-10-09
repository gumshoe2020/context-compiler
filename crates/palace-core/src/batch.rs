use crate::proposal::Proposal;
use serde_json::Value;
pub fn parse_batch(
    contents: &str,
    supplied_by_default: Option<&str>,
) -> Result<Vec<Proposal>, String> {
    let value: Value = serde_json::from_str(contents).map_err(|e| e.to_string())?;
    let entries = value.as_array().ok_or("--file must contain a JSON array")?;
    entries
        .iter()
        .enumerate()
        .map(|(i, v)| parse_entry(v, i, supplied_by_default))
        .collect()
}
fn parse_entry(value: &Value, i: usize, default: Option<&str>) -> Result<Proposal, String> {
    let mut v = value.clone();
    if !v.is_object() {
        return Err(format!("batch entry {i} must be an object"));
    }
    if v.get("supplied_by").is_none_or(Value::is_null) {
        v["supplied_by"] = v
            .get("supplied-by")
            .filter(|v| !v.is_null())
            .cloned()
            .or_else(|| default.map(|s| Value::String(s.into())))
            .unwrap_or(Value::Null);
    }
    if ["from", "to", "type", "supplied_by"]
        .iter()
        .any(|k| !v[k].is_string())
        || !v["confidence"].is_number()
        || !v["evidence"].is_array()
    {
        return Err(format!("batch entry {i} does not match the proposal shape"));
    }
    if v["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| !e.is_object() || !e["query"].is_string())
    {
        return Err(format!("batch entry {i} has invalid evidence"));
    }
    for key in ["from_type", "to_type", "claim", "claim_id"] {
        if v.get(key).is_some_and(|v| !v.is_string()) {
            return Err(format!("batch entry {i} {key} must be a string"));
        }
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}
