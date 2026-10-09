#![forbid(unsafe_code)]
pub mod command;
pub mod replay;
pub mod replay_model;
pub mod table;
use palace_core::canary::Question;
use serde_json::{Value, json};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub fn prompt(q: &Question) -> String {
    format!(
        "Canonical statement: {}\n\nPassage to evaluate:\n{}\n\nAnswer with one JSON object only: {{\"verdict\": \"preserved\"|\"reversed\"|\"unclear\", \"quote\": \"<the sentence in the passage that supports your verdict>\"}}. `reversed` means the passage attributes the correction, action, or claim to the opposite party or reverses its direction. `preserved` means the passage agrees with the statement or does not contradict it. `unclear` otherwise.",
        q.statement,
        q.passage.chars().take(12000).collect::<String>()
    )
}
pub fn parse_response(raw: &str) -> Value {
    let mut start = None;
    for (i, c) in raw.char_indices() {
        if c == '{' {
            start = Some(i);
        } else if c == '}'
            && let Some(start) = start
        {
            if let Ok(obj) = serde_json::from_str::<Value>(&raw[start..=i]) {
                let verdict = obj["verdict"]
                    .as_str()
                    .filter(|v| ["preserved", "reversed", "unclear"].contains(v))
                    .unwrap_or("unclear");
                return json!({"verdict":verdict,"quote":obj.get("quote").cloned().unwrap_or(json!(""))});
            }
            break;
        }
    }
    json!({"verdict":"unclear","quote":"","raw":raw.chars().take(200).collect::<String>()})
}
#[cfg(test)]
mod tests {
    #[test]
    fn first_flat_json_and_invalid_verdict() {
        assert_eq!(
            super::parse_response("preface {\"verdict\":\"reversed\",\"quote\":\"yes\"} tail")["verdict"],
            "reversed"
        );
        assert_eq!(
            super::parse_response("{bad} {\"verdict\":\"preserved\"}")["verdict"],
            "unclear"
        );
        assert_eq!(
            super::parse_response("{\"verdict\":\"invented\"}")["verdict"],
            "unclear"
        );
    }
}
