use crate::traits::GraphMutation;
use serde_json::{Value, json};
pub const FIXTURE_NAMES: [&str; 5] = [
    "pgs-m1-confidence-probe",
    "mcp-schema-no-confidence",
    "pgs-m5-cli",
    "pgs-m5-evidence",
    "pgs-m7-batch",
];
pub const FIXTURE_REASON: &str = "2026-08-18 milestone test fixture";
pub fn retire_fixtures<G: GraphMutation>(graph: &mut G, by: &str) -> Result<Value, G::Error> {
    let mut outcomes = Vec::new();
    let mut changed = 0;
    for name in FIXTURE_NAMES {
        let result = graph.retire_entity(name, FIXTURE_REASON, by)?;
        let status = status_of(&result);
        changed += usize::from(status == "retired");
        outcomes.push(json!({"name":name,"status":status,"result":result}));
    }
    Ok(json!({"outcomes":outcomes,"changed":changed}))
}
pub fn status_of(result: &Value) -> &'static str {
    if result["retired"] == true {
        if result["changed"] == true {
            "retired"
        } else {
            "already-retired"
        }
    } else if result.get("refused").is_some() {
        "refused"
    } else {
        "missing"
    }
}
