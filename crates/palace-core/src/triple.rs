use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Triple {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub confidence: Option<f64>,
    pub source_closet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}
impl Triple {
    pub fn surface_confidence(mut self) -> Self {
        let assessed = self
            .source_closet
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v.get("confidence").and_then(Value::as_f64))
            .filter(|n| n.is_finite());
        if (self.confidence.is_none() || self.confidence == Some(1.0))
            && assessed.is_some()
            && assessed != self.confidence
        {
            self.confidence = assessed;
            self.confidence_source = Some("provenance".into());
        } else if self.confidence.is_some() {
            self.confidence_source = Some("column".into());
        }
        self
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
    Both,
}
impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Both => "both",
        }
    }
}
