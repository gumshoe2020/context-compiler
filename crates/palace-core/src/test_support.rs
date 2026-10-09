//! In-memory adapters for palace-core's own unit tests (other crates use diary_memory).
use crate::{
    canary::Question,
    traits::{DiaryReader, Judge},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Memory(pub BTreeMap<String, String>);
impl DiaryReader for Memory {
    type Error = String;
    fn header(&self, name: &str) -> Result<String, String> {
        Ok(self
            .read(name)?
            .unwrap_or_default()
            .chars()
            .take(800)
            .collect())
    }
    fn names(&self) -> Result<Vec<String>, String> {
        Ok(self.0.keys().cloned().collect())
    }
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .get(name.strip_suffix(".md").unwrap_or(name))
            .cloned())
    }
}
/// Answers every claim question "preserved".
pub struct Preserving;
impl Judge for Preserving {
    type Error = String;
    fn judge(&self, _: &Question) -> Result<Value, String> {
        Ok(json!({"verdict":"preserved","quote":"table"}))
    }
}
