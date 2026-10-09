//! PKG-1.3 item 5 (review S3): `palace_reply` answers one of the nightly job's questions with the
//! person's reply, exactly as the owner's corrections path does: `palace --person <p> --assistant
//! <a> improve --reply <reply> --token <token> --by <p>` on this memory's graph, palace and diary
//! root, under the names palace-mcp was started with. Offered only with `--palace-bin`.
use crate::{child, options::ReplyOptions};
use palace_store::Error;
use serde_json::{Value, json};
use std::{path::PathBuf, process::Command, time::Duration};

pub const NAME: &str = "palace_reply";
/// `palace improve --reply` may search MemPalace (`source:`); past this it is ended.
const DEADLINE: Duration = Duration::from_secs(60);
const MAX_REPLY: usize = 2000;
const MAX_MESSAGE: usize = 500;

pub struct ReplyTool {
    options: ReplyOptions,
}
impl ReplyTool {
    pub fn new(options: ReplyOptions) -> Self {
        Self { options }
    }
    pub fn schema() -> Value {
        json!({"name":NAME,
            "description":"Answer a question of the nightly memory job (shown at session start; it ends with a reply token #f_… and the replies it accepts) with the user's own reply. Only when the user has answered it; never answer for the user.",
            "inputSchema":{"type":"object","properties":{
                "token":{"type":"string","description":"The question's reply token, e.g. #f_1a2b3c4d."},
                "reply":{"type":"string","description":"The user's reply, one of the words the question accepts: keep, accept, drop, fix: <claim>, source: <file>."}},
            "required":["token","reply"]}})
    }
    pub fn call(&self, args: &Value) -> Result<String, Error> {
        let (token, reply) = arguments(args)?;
        let done = child::run(self.command(&token, &reply), DEADLINE)?;
        let code = done.status.and_then(|s| s.code());
        let message = tail(&done.stderr);
        let result = match code {
            Some(0) => json!({"applied":true,"result":done.stdout.trim()}),
            Some(1) => json!({"applied":false,"refused":message}),
            Some(3) => json!({"applied":false,"retry_later":message}),
            Some(code) => json!({"applied":false,"failed":format!("exit {code}: {message}")}),
            None if done.status.is_some() => json!({"applied":false,"failed":"ended by a signal"}),
            None => json!({"applied":false,"failed":format!(
                "no answer within {} s; it was ended", DEADLINE.as_secs())}),
        };
        Ok(result.to_string())
    }
    fn command(&self, token: &str, reply: &str) -> Command {
        let o = &self.options;
        let mut command = Command::new(&o.palace_bin);
        command.args(["--person", &o.person, "--assistant", &o.assistant]);
        if let Some(check) = &o.check {
            command.arg("--mempalace-check").arg(check);
        }
        command
            .arg("improve")
            .arg(format!("--reply={reply}"))
            .arg(format!("--token={token}"))
            .args(["--by", &o.person])
            .arg("--kg")
            .arg(&o.kg)
            .arg("--palace")
            .arg(&o.palace)
            .arg("--diary")
            .arg(&o.diary)
            .arg("--mempalace")
            .arg(&o.mempalace)
            .arg("--mempalace-mcp")
            .arg(mempalace_mcp(&o.mempalace));
        command
    }
}

/// `mempalace-mcp` beside `--mempalace` (the pinned install's two programs), else on PATH.
fn mempalace_mcp(mempalace: &std::path::Path) -> PathBuf {
    match mempalace.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("mempalace-mcp"),
        _ => PathBuf::from("mempalace-mcp"),
    }
}

/// The token (`#f_` and its hex; a missing `#` is added) and the reply (one non-blank line).
fn arguments(args: &Value) -> Result<(String, String), Error> {
    let text = |key: &str| {
        args[key]
            .as_str()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    };
    let (token, reply) = (text("token"), text("reply"));
    let token = if token.starts_with("f_") {
        format!("#{token}")
    } else {
        token
    };
    let hex = token.strip_prefix("#f_").unwrap_or_default();
    if hex.is_empty() || hex.len() > 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("token must be the question's reply token, #f_ and its hex digits".into());
    }
    if reply.is_empty() || reply.contains(['\n', '\r']) || reply.chars().count() > MAX_REPLY {
        return Err(
            format!("reply must be one non-blank line of at most {MAX_REPLY} characters").into(),
        );
    }
    Ok((token, reply))
}

/// The end of a child's stderr, trimmed, for the reply.
fn tail(text: &str) -> String {
    let text = text.trim();
    let skip = text.chars().count().saturating_sub(MAX_MESSAGE);
    text.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_are_checked() {
        let ok = arguments(&json!({"token":"f_1a2b3c4d","reply":" drop "})).unwrap();
        assert_eq!(ok, ("#f_1a2b3c4d".to_owned(), "drop".to_owned()));
        for bad in [
            json!({"reply":"drop"}),
            json!({"token":"#f_","reply":"drop"}),
            json!({"token":"#f_zz","reply":"drop"}),
            json!({"token":"--by","reply":"drop"}),
            json!({"token":"#f_1a","reply":" "}),
            json!({"token":"#f_1a","reply":"keep\nfix: x"}),
            json!({"token":"#f_1a","reply":7}),
        ] {
            assert!(arguments(&bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn mempalace_mcp_is_beside_mempalace() {
        let beside = mempalace_mcp(std::path::Path::new("/u/bin/mempalace"));
        assert_eq!(beside, PathBuf::from("/u/bin/mempalace-mcp"));
        let bare = mempalace_mcp(std::path::Path::new("mempalace"));
        assert_eq!(bare, PathBuf::from("mempalace-mcp"));
    }
}
