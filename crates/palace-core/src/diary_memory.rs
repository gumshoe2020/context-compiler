//! A diary held in memory, keyed by entry name (without `.md`): the watch's tests and the
//! doctors read through it (moved out of palace-testkit, PKG-1.2 item 5). With it, a judge that
//! always gives one verdict (PUB-1a item 3: palace-testkit is gone).
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

/// A judge that always answers `verdict` with a test quote, or times out when it is "timeout".
pub struct FakeJudge(pub &'static str);
impl Judge for FakeJudge {
    type Error = String;
    fn judge(&self, _: &Question) -> Result<Value, String> {
        if self.0 == "timeout" {
            Err("judge timed out".into())
        } else {
            Ok(json!({"verdict":self.0,"quote":"test quote"}))
        }
    }
}
