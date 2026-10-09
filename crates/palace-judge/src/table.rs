//! `table:<file>`: a judge answering from a table (moved out of palace-testkit, PKG-1.2 item 5,
//! so that `palace improve --judge table:<file>` needs no test code).
use palace_core::canary::Question;
use palace_core::traits::Judge;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Answers claim questions from a table keyed by claim number; unlisted claims are preserved.
pub struct TableJudge(pub BTreeMap<String, String>);
impl Judge for TableJudge {
    type Error = String;
    fn judge(&self, q: &Question) -> Result<Value, String> {
        let verdict = self.0.get(&q.id).map_or("preserved", String::as_str);
        Ok(json!({"verdict":verdict,"quote":"table"}))
    }
}
