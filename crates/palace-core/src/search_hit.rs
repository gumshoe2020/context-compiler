use serde::{Deserialize, Serialize};
/// MemPalace owns ranking; the compiler consumes text and provenance only.
#[derive(Debug, Deserialize, Serialize)]
pub struct SearchHit {
    pub text: String,
    pub source_path: String,
    pub source_file: String,
    pub wing: String,
    pub room: String,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}
