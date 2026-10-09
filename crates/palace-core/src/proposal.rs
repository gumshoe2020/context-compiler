use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvidenceReference {
    pub query: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Proposal {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub relation: String,
    pub evidence: Vec<EvidenceReference>,
    pub confidence: f64,
    pub supplied_by: String,
    pub from_type: Option<String>,
    pub to_type: Option<String>,
    /// The rollup claim number this relation derives from (`"C7"`), recorded in provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<String>,
    /// The stable claim id (`c_…`), resolved from `claim` when the rollup is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
    /// SETTLE-1b: the `<palace copy>:<line>` locator of a distilled relation, kept in provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
}
