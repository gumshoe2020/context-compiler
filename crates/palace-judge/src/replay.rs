use crate::Error;
use palace_core::{canary::Question, traits::Judge};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};
#[derive(Default)]
pub struct Replay {
    rows: BTreeMap<(String, String), Value>,
}
impl Replay {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let mut replay = Self::default();
        let mut files = if path.is_dir() {
            fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![path.to_owned()]
        };
        files.sort();
        for path in files
            .into_iter()
            .filter(|p| p.extension().is_some_and(|s| s == "json"))
        {
            replay.add(&serde_json::from_slice::<Value>(&fs::read(path)?)?)?;
        }
        Ok(replay)
    }
    pub fn add(&mut self, report: &Value) -> Result<(), Error> {
        for row in report["results"]
            .as_array()
            .ok_or("replay report missing results")?
        {
            if row["check"] != "canary" && row["check"] != "claim" {
                continue;
            }
            let key = (
                row["rollup"]
                    .as_str()
                    .ok_or("replay missing rollup")?
                    .to_owned(),
                row["canary_id"]
                    .as_str()
                    .or_else(|| row["claim"].as_str())
                    .ok_or("replay missing question id")?
                    .to_owned(),
            );
            if self.rows.get(&key).is_some_and(|old| old != row) {
                return Err(format!("conflicting replay answers for {key:?}").into());
            }
            self.rows.insert(key, row.clone());
        }
        Ok(())
    }
}
impl Judge for Replay {
    type Error = Error;
    fn judge(&self, q: &Question) -> Result<Value, Error> {
        let row = self
            .rows
            .get(&(q.rollup.clone(), q.id.clone()))
            .ok_or_else(|| format!("no replay answer for {} / {}", q.rollup, q.id))?;
        if let Some(reason) = row["reason"].as_str() {
            return Err(reason.to_owned().into());
        }
        if !row["verdict"].is_string() {
            return Err("replay row has no verdict".into());
        }
        Ok(row.clone())
    }
}
