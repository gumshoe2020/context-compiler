use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: Option<String>,
    pub properties: Map<String, Value>,
    pub created_at: Option<String>,
}
impl Entity {
    pub fn retired(&self) -> Option<&Value> {
        self.properties
            .get("retired")
            .filter(|v| v.is_object() || v.is_array())
    }
}
