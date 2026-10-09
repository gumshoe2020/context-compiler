use crate::diary_tool::{self, DiaryTool};
use crate::options::Options;
use crate::reply_tool::{self, ReplyTool};
use palace_store::Error;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
pub struct Service {
    graph: (PathBuf, PathBuf, Option<PathBuf>),
    /// Offered only with `--diary-root` (PKG-1.2 item 2).
    diary: Option<DiaryTool>,
    /// Offered only with `--palace-bin` (PKG-1.3 item 5).
    reply: Option<ReplyTool>,
}
impl Service {
    pub fn open(options: Options) -> Self {
        let directory = options.kg.parent().unwrap_or(Path::new("."));
        let audit = directory.join("compiler.sqlite");
        let wal = options.wal.or_else(|| {
            Some(
                options
                    .palace
                    .as_deref()
                    .unwrap_or(directory)
                    .join("wal/write_log.jsonl"),
            )
        });
        Self {
            graph: (audit, options.kg, wal),
            diary: options.diary.map(DiaryTool::new),
            reply: options.reply.map(ReplyTool::new),
        }
    }
    pub fn call(&mut self, params: &Value) -> Result<Value, Error> {
        let text = match params["name"].as_str() {
            Some(name) if name.starts_with("mempalace_kg_") => {
                let (palace, kg, wal) = &self.graph;
                let operation = &name[13..];
                let args = crate::kg_tools::arguments(&params["arguments"])?;
                let result = match operation {
                    "query" | "entities" | "timeline" | "stats" => {
                        palace_store::graph::GraphReader::open(kg)?.call(operation, &args)?
                    }
                    "add" | "invalidate" | "set_entity" | "retire_entity" => {
                        let mut graph =
                            palace_store::graph::Graph::open(palace, kg, wal.as_deref())?;
                        // M4d-2c item 6: settle any interrupted write before writing (a DIVERGED
                        // intent is an error naming `palace doctor --settle`).
                        graph.reconcile()?;
                        graph.call(operation, &args)?
                    }
                    _ => return Err("unknown graph operation".into()),
                };
                crate::kg_tools::capped(result)?
            }
            Some(diary_tool::NAME) => match &self.diary {
                Some(tool) => tool.call(&params["arguments"])?,
                None => return Err("unknown tool".into()),
            },
            Some(reply_tool::NAME) => match &self.reply {
                Some(tool) => tool.call(&params["arguments"])?,
                None => return Err("unknown tool".into()),
            },
            _ => return Err("unknown tool".into()),
        };
        Ok(json!({"content":[{"type":"text","text":text}]}))
    }
    pub fn tools(&self) -> Value {
        let mut tools = crate::kg_tools::schemas();
        if self.diary.is_some() {
            tools.push(DiaryTool::schema());
        }
        if self.reply.is_some() {
            tools.push(ReplyTool::schema());
        }
        json!({"tools":tools})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    fn service(args: &[&str]) -> Service {
        Service::open(Options::parse(args.iter().map(OsString::from)).unwrap())
    }
    fn names(service: &Service) -> Vec<String> {
        let tools = service.tools();
        let tools = tools["tools"].as_array().unwrap();
        tools
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }
    #[test]
    fn the_diary_tool_only_with_a_diary_root() {
        let old = names(&service(&["--palace", "/p"]));
        assert_eq!(old.len(), 8);
        assert!(old.iter().all(|n| n.starts_with("mempalace_kg_")));
        let mut with_diary = names(&service(&["--palace", "/p", "--diary-root", "/d"]));
        assert_eq!(with_diary.pop().as_deref(), Some(diary_tool::NAME));
        assert_eq!(with_diary, old);
        let call = json!({"name":diary_tool::NAME,"arguments":{"text":"x"}});
        let refused = service(&["--palace", "/p"]).call(&call).unwrap_err();
        assert_eq!(refused.to_string(), "unknown tool");
    }
}
