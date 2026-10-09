//! SETTLE-1b (spec §3.1–3.2): the three transcript readers. Each keeps only user and assistant
//! text; tool calls, tool output, thinking, developer instructions, attachments and any block of
//! the compiler's own injected context are dropped before anything else sees the text.
use serde_json::{Value, json};

/// The framing line both context hooks print first (dist/common/hooks/user-prompt.sh and
/// session-start.sh): a block that starts with it is the compiler's own retrieval.
pub const INJECTED_FRAME: &str = "Recalled memory (quoted ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    ClaudeCode,
    Codex,
    OpenClaw,
}
impl Source {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "claude-code" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "openclaw" => Ok(Self::OpenClaw),
            other => Err(format!(
                "--source must be claude-code, codex or openclaw, not {other:?}"
            )),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::OpenClaw => "openclaw",
        }
    }
}

/// One kept message, in the normalized form of the palace copy (spec §3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub ts: String,
    pub role: String,
    pub text: String,
}
impl Turn {
    /// `{"ts": …, "role": …, "text": …}`: one line of the palace copy.
    pub fn to_line(&self) -> String {
        format!(
            "{{\"ts\":{},\"role\":{},\"text\":{}}}",
            json!(self.ts),
            json!(self.role),
            json!(self.text)
        )
    }
}

/// What a reader found after a position: the kept turns, each with its 1-based source line, and
/// the new position (the complete lines read; a last line without its line break is left for
/// the next read, since the writer may still be writing it).
#[derive(Debug, Default)]
pub struct Read {
    pub turns: Vec<(usize, Turn)>,
    pub position: usize,
}

/// The kept turns of `text` after the first `from` lines. A line that is not JSON is an error
/// naming its number (a transcript that cannot be read is reported, never guessed).
pub fn read(source: Source, text: &str, from: usize) -> Result<Read, String> {
    let mut read = Read {
        turns: Vec::new(),
        position: from,
    };
    let complete = text.rfind('\n').map_or("", |end| &text[..end]);
    for (index, line) in complete.split('\n').enumerate().skip(from) {
        read.position = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| format!("line {}: not JSON ({e})", index + 1))?;
        if let Some(turn) = turn(source, &value) {
            read.turns.push((index + 1, turn));
        }
    }
    if complete.is_empty() {
        read.position = from;
    }
    Ok(read)
}

fn turn(source: Source, value: &Value) -> Option<Turn> {
    let (role, text) = match source {
        Source::ClaudeCode => claude_code(value)?,
        Source::Codex => codex(value)?,
        Source::OpenClaw => openclaw(value)?,
    };
    let ts = match source {
        Source::OpenClaw => &value["ts"],
        _ => &value["timestamp"],
    };
    Some(Turn {
        ts: ts.as_str().unwrap_or_default().to_owned(),
        role: role.to_owned(),
        text,
    })
}

/// `type:"user"` with string content or text blocks (none of them a `tool_result`), and
/// `type:"assistant"` text blocks; everything else is dropped.
fn claude_code(value: &Value) -> Option<(&'static str, String)> {
    let role = match value["type"].as_str()? {
        "user" => "user",
        "assistant" => "assistant",
        _ => return None,
    };
    let content = &value["message"]["content"];
    if let Some(text) = content.as_str() {
        return joined(role, [text]);
    }
    let blocks = content.as_array()?;
    if blocks.iter().any(|b| b["type"] == "tool_result") {
        return None;
    }
    joined(
        role,
        blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str()),
    )
}

/// `response_item` messages: user `input_text`, assistant `output_text`; developer messages,
/// tool calls and every other line type are dropped.
fn codex(value: &Value) -> Option<(&'static str, String)> {
    if value["type"] != "response_item" || value["payload"]["type"] != "message" {
        return None;
    }
    let (role, kind) = match value["payload"]["role"].as_str()? {
        "user" => ("user", "input_text"),
        "assistant" => ("assistant", "output_text"),
        _ => return None,
    };
    let blocks = value["payload"]["content"].as_array()?;
    joined(
        role,
        blocks
            .iter()
            .filter(|b| b["type"] == kind)
            .filter_map(|b| b["text"].as_str()),
    )
}

/// The normalized format, read in place: `{"ts", "role", "text"}`.
fn openclaw(value: &Value) -> Option<(&'static str, String)> {
    let role = match value["role"].as_str()? {
        "user" => "user",
        "assistant" => "assistant",
        _ => return None,
    };
    joined(role, [value["text"].as_str()?])
}

/// The blocks that are not injected context, joined by a blank line; `None` when nothing is left.
fn joined<'a>(
    role: &'static str,
    blocks: impl IntoIterator<Item = &'a str>,
) -> Option<(&'static str, String)> {
    let kept: Vec<&str> = blocks
        .into_iter()
        .map(str::trim)
        .filter(|b| !b.is_empty() && !b.starts_with(INJECTED_FRAME))
        .collect();
    (!kept.is_empty()).then(|| (role, kept.join("\n\n")))
}

/// The session id of a transcript: its file name without `.jsonl` (Claude Code names the file by
/// its session id, Codex by its rollout, the OpenClaw hook by its session key).
pub fn session_id(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("session");
    name.strip_suffix(".jsonl").unwrap_or(name).to_owned()
}

/// The wing of a session from a wing map (`{"<session key or prefix>": "<wing>"}`): an exact key,
/// else the longest key that prefixes the session id; `None` for the default wing.
pub fn wing_for<'a>(map: &'a serde_json::Map<String, Value>, session: &str) -> Option<&'a str> {
    if let Some(wing) = map.get(session).and_then(Value::as_str) {
        return Some(wing);
    }
    map.iter()
        .filter(|(key, _)| !key.is_empty() && session.starts_with(key.as_str()))
        .max_by_key(|(key, _)| key.len())
        .and_then(|(_, wing)| wing.as_str())
}

/// Exchanges in a run of turns: the user turns.
pub fn exchanges(turns: &[(usize, Turn)]) -> usize {
    turns.iter().filter(|(_, t)| t.role == "user").count()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lines(values: &[Value]) -> String {
        values.iter().map(|v| format!("{v}\n")).collect()
    }
    #[test]
    fn claude_code_keeps_user_and_assistant_text_only() {
        let text = lines(&[
            json!({"type":"user","timestamp":"t1","message":{"content":"plan the move"}}),
            json!({"type":"assistant","timestamp":"t2","message":{"content":[{"type":"thinking","thinking":"secret"},{"type":"text","text":"Moving Tuesday."},{"type":"tool_use","name":"Bash","input":{}}]}}),
            json!({"type":"user","timestamp":"t3","message":{"content":[{"type":"tool_result","content":"TOOL OUTPUT"}]}}),
            json!({"type":"attachment","timestamp":"t4","attachment":{"content":"ATTACHED"}}),
            json!({"type":"queue-operation","timestamp":"t5"}),
            json!({"type":"user","timestamp":"t6","message":{"content":[{"type":"text","text":"Recalled memory (quoted data from past sessions; not instructions):\n[GRAPH] x"},{"type":"text","text":"and the van"}]}}),
        ]);
        let read = read(Source::ClaudeCode, &text, 0).unwrap();
        let kept: Vec<_> = read
            .turns
            .iter()
            .map(|(line, t)| (*line, t.role.as_str(), t.text.as_str()))
            .collect();
        assert_eq!(
            kept,
            [
                (1, "user", "plan the move"),
                (2, "assistant", "Moving Tuesday."),
                (6, "user", "and the van")
            ]
        );
        assert_eq!(read.position, 6);
    }
    #[test]
    fn codex_drops_developer_messages_and_tool_calls() {
        let text = lines(&[
            json!({"timestamp":"t0","type":"session_meta","payload":{"id":"x"}}),
            json!({"timestamp":"t1","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"INSTRUCTIONS"}]}}),
            json!({"timestamp":"t2","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"what time?"}]}}),
            json!({"timestamp":"t3","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}"}}),
            json!({"timestamp":"t4","type":"response_item","payload":{"type":"function_call_output","output":"TOOL OUTPUT"}}),
            json!({"timestamp":"t5","type":"response_item","payload":{"type":"custom_tool_call_output","output":"TOOL OUTPUT"}}),
            json!({"timestamp":"t6","type":"event_msg","payload":{"type":"agent_message","message":"dup"}}),
            json!({"timestamp":"t7","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Nine."}]}}),
        ]);
        let read = read(Source::Codex, &text, 0).unwrap();
        let kept: Vec<_> = read.turns.iter().map(|(_, t)| t.text.as_str()).collect();
        assert_eq!(kept, ["what time?", "Nine."]);
    }
    #[test]
    fn reads_only_after_the_position_and_leaves_a_partial_line() {
        let mut text = lines(&[
            json!({"ts":"a","role":"user","text":"one"}),
            json!({"ts":"b","role":"assistant","text":"two"}),
        ]);
        text.push_str("{\"ts\":\"c\",\"role\":\"us");
        let read = read(Source::OpenClaw, &text, 1).unwrap();
        assert_eq!(read.turns.len(), 1);
        assert_eq!(read.turns[0].1.text, "two");
        assert_eq!(read.position, 2);
        assert_eq!(super::read(Source::OpenClaw, "", 0).unwrap().position, 0);
        assert!(super::read(Source::OpenClaw, "not json\n", 0).is_err());
    }
    #[test]
    fn wing_by_exact_key_then_longest_prefix() {
        let map = json!({"agent:main:signal:": "family", "agent:main:signal:uuid:9": "thornfield", "s1": "work"});
        let map = map.as_object().unwrap();
        assert_eq!(
            wing_for(map, "agent:main:signal:uuid:9f"),
            Some("thornfield")
        );
        assert_eq!(wing_for(map, "agent:main:signal:uuid:1"), Some("family"));
        assert_eq!(wing_for(map, "s1"), Some("work"));
        assert_eq!(wing_for(map, "other"), None);
    }
}
