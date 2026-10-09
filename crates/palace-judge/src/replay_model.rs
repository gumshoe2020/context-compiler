//! `--llm replay`: answers the rollup's two Model calls offline. Select names the replay body's
//! thread and the offered candidates whose tag the matching archived rollup carries; write
//! returns the replay body verbatim; a repair (M5b-10 item 2) returns the asked lines unchanged.
use crate::Error;
use palace_core::{
    rollup::Entry,
    rollup_repair::{REPAIR_MARKER, asked_lines},
    rollup_select::SELECT_MARKER,
    rollup_validate::{WRITE_MARKER, split_title},
    traits::Model,
};
use serde_json::json;

pub struct ReplayModel {
    body: String,
    thread: String,
    archived: Vec<Entry>,
}
impl ReplayModel {
    /// `archived` = the archived rollups (select/pull); `body` = the rollup the write step returns.
    pub fn new(body: String, archived: Vec<Entry>) -> Result<Self, Error> {
        let (title, _) = split_title(&body);
        let thread = title
            .strip_prefix("ROLLUP: ")
            .and_then(|t| t.split(" — ").next())
            .filter(|t| !t.is_empty())
            .ok_or("replay body title must be `# ROLLUP: <thread> — state as of <date>`")?
            .to_owned();
        Ok(Self {
            body,
            thread,
            archived,
        })
    }
    pub fn thread(&self) -> &str {
        &self.thread
    }
    fn select(&self, prompt: &str) -> String {
        let Some(archive) = self.archived.iter().find(|e| e.tags.contains(&self.thread)) else {
            return json!({"bail":true,"reason":format!("replay: no archived rollup carries {}", self.thread)}).to_string();
        };
        let ids: Vec<u64> = offered(prompt)
            .into_iter()
            .filter(|(_, tag)| archive.tags.contains(tag))
            .map(|(id, _)| id)
            .collect();
        if ids.is_empty() {
            return json!({"bail":true,"reason":"replay: no offered candidate matches the archived rollup"}).to_string();
        }
        json!({"thread":self.thread,"reason":format!("replay of archived {}", archive.name),"candidate_ids":ids}).to_string()
    }
}
/// Candidate lines `[<id>] tag: <tag> — …` as offered in the select prompt.
fn offered(prompt: &str) -> Vec<(u64, String)> {
    prompt
        .lines()
        .filter_map(|line| {
            let (id, rest) = line.strip_prefix('[')?.split_once("] tag: ")?;
            let tag = rest.split(" — ").next()?;
            Some((id.parse().ok()?, tag.to_owned()))
        })
        .collect()
}
impl Model for ReplayModel {
    type Error = Error;
    fn complete(&self, prompt: &str) -> Result<String, Error> {
        if prompt.contains(SELECT_MARKER) {
            Ok(self.select(prompt))
        } else if prompt.contains(REPAIR_MARKER) {
            Ok(asked_lines(prompt).join("\n"))
        } else if prompt.contains(WRITE_MARKER) {
            Ok(self.body.clone())
        } else {
            Err("replay model has no answer for this prompt".into())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn select_matches_archived_tags_and_write_returns_body() {
        let body = "# ROLLUP: halloran-thread — state as of 2026-09-23\nbody".to_owned();
        let archived = vec![Entry::parse(
            "2026-09-22-fcd6130ad2a9",
            "# ROLLUP: x\nTags: halloran-thread, halloran, hollander.".into(),
        )];
        let model = ReplayModel::new(body.clone(), archived).unwrap();
        let prompt = format!(
            "{SELECT_MARKER}\n[1] tag: callum — 3\n[2] tag: halloran — 2\n[3] tag: hollander — 2\n"
        );
        let answer: serde_json::Value =
            serde_json::from_str(&model.complete(&prompt).unwrap()).unwrap();
        assert_eq!(answer["thread"], "halloran-thread");
        assert_eq!(answer["candidate_ids"], json!([2, 3]));
        assert_eq!(model.complete(WRITE_MARKER).unwrap(), body);
        assert!(model.complete("other").is_err());
    }
}
